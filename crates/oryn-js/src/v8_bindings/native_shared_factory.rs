//! Real V8 constructor allocation hook. Authority is resolved at each call from
//! the entered native context; the isolate delegate owns no document or worker.
use super::*;
use crate::shared_memory::{SharedMemoryRequest, SharedMemoryResponse};
use crate::worker_wire::{AgentServiceRequest, AgentServiceResponse};
use std::ffi::c_void;

unsafe extern "C" {
    fn oryn_v8_install_shared_memory_factory(isolate: v8::UnsafeRawIsolatePtr);
    fn oryn_v8_copy_shared_factory_backing(
        buffer: *const v8::SharedArrayBuffer,
        backing_out: *mut c_void,
    ) -> bool;
}

pub(super) fn install(scope: &mut v8::PinScope) {
    unsafe { oryn_v8_install_shared_memory_factory(scope.as_raw_isolate_ptr()) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn oryn_v8_allocate_native_shared_memory(
    context: v8::Local<v8::Context>,
    initial: usize,
    maximum: usize,
    growable: bool,
    wasm: bool,
    memory64: bool,
    backing_out: *mut c_void,
) -> bool {
    // V8 supplies a live rooted context during allocation. No JS is executed
    // here, and failure is returned to V8's original constructor error path.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if backing_out.is_null() {
            return false;
        }
        v8::callback_scope!(unsafe scope, context);
        let Some(state) = context.get_slot::<V8BindingState>() else {
            return false;
        };
        if state.execution_retired() {
            return false;
        }
        if !super::native_shared_policy::bind(scope, &state) {
            return false;
        }
        let Ok(creator) = state.native_worker_creator() else {
            return false;
        };
        let Some(transport) = state.worker_transport.borrow().clone() else {
            return false;
        };
        let granularity = crate::v8_shared_memory::granularity(scope);
        if granularity == 0 || initial > maximum || (memory64 && !wasm) {
            return false;
        }
        let request = SharedMemoryRequest::Allocate {
            byte_length: initial as u64,
            maximum_length: (growable || wasm).then_some(maximum as u64),
            granularity: granularity as u64,
            guard_length: 0,
            wasm,
            memory64,
        };
        let Ok(AgentServiceResponse::SharedMemory(response)) =
            transport.request(AgentServiceRequest::SharedMemory {
                creator: creator.clone(),
                request,
            })
        else {
            return false;
        };
        // Mapping consumes the parent's admitted reference even on error.
        let mut waits = state.shared_waits.borrow_mut();
        if waits.is_none() {
            *waits = crate::v8_shared_wait::WaitEnvironment::new();
        }
        let Some(waits) = waits.as_ref() else {
            release_unmapped(&*transport, creator, response);
            return false;
        };
        let mapped = if wasm {
            crate::v8_shared_memory::map_wasm_backing(scope, transport, creator, response, waits)
        } else {
            crate::v8_shared_memory::map(scope, transport, creator, response, waits)
        };
        let Ok((buffer, _lease)) = mapped else {
            return false;
        };
        unsafe { oryn_v8_copy_shared_factory_backing(&*buffer, backing_out) }
    }))
    .unwrap_or(false)
}

fn release_unmapped(
    transport: &dyn crate::worker_wire::WorkerTransport,
    creator: crate::worker_wire::Creator,
    response: SharedMemoryResponse,
) {
    if let Some(descriptor) = response.descriptor {
        let _ = transport.request(AgentServiceRequest::SharedMemory {
            creator,
            request: SharedMemoryRequest::Release {
                token: descriptor.token,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entered_context_without_native_authority_cannot_allocate_shared_storage() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install(scope);
        let source = v8::String::new(
            scope,
            r#"
          (() => {
            if (new ArrayBuffer(8).byteLength !== 8) return false;
            if (new WebAssembly.Memory({initial:1}).buffer.byteLength !== 65536)
              return false;
            for (const create of [
              () => new SharedArrayBuffer(8),
              () => new SharedArrayBuffer(8, {maxByteLength:16}),
              () => new WebAssembly.Memory({initial:1, maximum:2, shared:true})
            ]) {
              try { create(); return false; }
              catch (error) { if (!(error instanceof RangeError)) return false; }
            }
            return true;
          })()
        "#,
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod mapped_constructor_tests {
    use super::*;
    use crate::shared_memory::*;
    use crate::worker_wire::*;
    use std::sync::{Arc, Mutex};

    // Real anonymous file-backed mappings exercise the Rust/C++ factory ABI.
    // The test broker deliberately admits only zero-sized logical allocations.
    struct ZeroBroker {
        agent: u64,
        requests: Mutex<Vec<SharedMemoryRequest>>,
    }
    impl WorkerTransport for ZeroBroker {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            let AgentServiceRequest::SharedMemory {
                creator: Creator::Agent(AgentId(agent)),
                request,
            } = request
            else {
                panic!("allocation used the wrong native creator")
            };
            assert_eq!(
                agent, self.agent,
                "isolate delegate reused another realm authority"
            );
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            let mut descriptor = None;
            let mut native_descriptor = None;
            let outcome = match request {
                SharedMemoryRequest::Allocate {
                    byte_length,
                    maximum_length,
                    granularity,
                    guard_length,
                    wasm,
                    memory64,
                } => {
                    assert_eq!(byte_length, 0);
                    assert_eq!(maximum_length.unwrap_or(0), 0);
                    assert_eq!(guard_length, 0);
                    assert!(!memory64);
                    assert!(granularity.is_power_of_two());
                    let mut token = [0; 32];
                    getrandom::fill(&mut token).unwrap();
                    let filename = token
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let path = std::env::temp_dir().join(format!("oryn-shared-factory-{filename}"));
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .unwrap();
                    // Unlink immediately; only the owned descriptor retains the
                    // file, including when mapping or constructor validation fails.
                    std::fs::remove_file(path).unwrap();
                    file.set_len(2 * granularity).unwrap();
                    native_descriptor = Some(Arc::new(std::os::fd::OwnedFd::from(file)));
                    descriptor = Some(SharedMemoryDescriptor {
                        token: SharedMemoryToken(token),
                        byte_length: 0,
                        maximum_length: 0,
                        mapped_length: 2 * granularity,
                        data_offset: granularity,
                        reserved_length: granularity,
                        guard_length: 0,
                        wasm,
                        memory64,
                        growable: maximum_length.is_some(),
                    });
                    SharedMemoryOutcome::None
                }
                SharedMemoryRequest::GrowAbsolute { length: 0, .. } => {
                    SharedMemoryOutcome::Length(0)
                }
                SharedMemoryRequest::GrowByPages {
                    pages: 0,
                    maximum_pages: 0,
                    ..
                } => SharedMemoryOutcome::Length(0),
                SharedMemoryRequest::Bind { parent: None }
                | SharedMemoryRequest::Release { .. } => SharedMemoryOutcome::None,
                _ => panic!("unexpected zero-memory operation: {request:?}"),
            };
            Ok(AgentServiceResponse::SharedMemory(SharedMemoryResponse {
                isolated: true,
                outcome,
                descriptor,
                native_descriptor,
            }))
        }
    }

    #[test]
    fn zero_length_native_constructors_keep_logical_limits_and_retirement_authority() {
        super::super::tests::initialize_v8();
        let broker = Arc::new(ZeroBroker {
            agent: 73,
            requests: Mutex::new(Vec::new()),
        });
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_worker_transport(broker.clone(), Some(Creator::Agent(AgentId(73))), None);
        let source = v8::String::new(scope, r#"
          (() => {
            const fixed = new SharedArrayBuffer(0);
            const growable = new SharedArrayBuffer(0, {maxByteLength:0});
            const memory = new WebAssembly.Memory({initial:0, maximum:0, shared:true});
            if(fixed.byteLength!==0 || fixed.growable || fixed.maxByteLength!==0) return false;
            if(!growable.growable || growable.byteLength!==0 || growable.maxByteLength!==0) return false;
            growable.grow(0);
            if(memory.grow(0)!==0 || memory.buffer.byteLength!==0) return false;
            if(!(memory.buffer instanceof SharedArrayBuffer)) return false;
            for(const grow of [() => growable.grow(1), () => memory.grow(1)]) {
              try { grow(); return false; }
              catch(error) { if(!(error instanceof RangeError)) return false; }
            }
            return true;
          })()
        "#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let allocation_count = || {
            broker
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| matches!(request, SharedMemoryRequest::Allocate { .. }))
                .count()
        };
        assert_eq!(allocation_count(), 3);
        state.retire_execution();
        let source = v8::String::new(
            scope,
            "try { new SharedArrayBuffer(0); false } catch(e) { e instanceof RangeError }",
        )
        .unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        assert_eq!(
            allocation_count(),
            3,
            "retirement must reject before broker allocation"
        );
    }

    #[test]
    fn isolate_factory_resolves_each_current_realm_and_survives_peer_retirement() {
        super::super::tests::initialize_v8();
        let first = Arc::new(ZeroBroker {
            agent: 101,
            requests: Mutex::new(Vec::new()),
        });
        let second = Arc::new(ZeroBroker {
            agent: 102,
            requests: Mutex::new(Vec::new()),
        });
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let a = v8::Context::new(scope, Default::default());
        let b = v8::Context::new(scope, Default::default());
        let mut states = Vec::new();
        for (context, broker, realm) in [(a, first.clone(), 101), (b, second.clone(), 102)] {
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<!doctype html><body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state =
                install_generated_bindings(scope, context, dom, RealmId(realm as u32), 1).unwrap();
            state.set_worker_transport(broker, Some(Creator::Agent(AgentId(realm))), None);
            states.push(state);
        }
        // Both realms have installed the isolate hook. Its latest installation
        // must not capture B's identity when A subsequently invokes it.
        for context in [a, b, a] {
            let scope = &mut v8::ContextScope::new(scope, context);
            let source = v8::String::new(scope, "new SharedArrayBuffer(0).byteLength===0").unwrap();
            assert!(
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
        states[1].retire_execution();
        {
            let scope = &mut v8::ContextScope::new(scope, a);
            let source = v8::String::new(scope, "new SharedArrayBuffer(0).byteLength===0").unwrap();
            assert!(
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
        let allocations = |broker: &ZeroBroker| {
            broker
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| matches!(request, SharedMemoryRequest::Allocate { .. }))
                .count()
        };
        assert_eq!(allocations(&first), 3);
        assert_eq!(allocations(&second), 1);
    }
}
