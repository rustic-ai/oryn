//! Engine/adapter integration regression source; no real network timing needed.
use super::*;
use crate::{
    shared_memory::*, v8_shared_memory::MappingLease, v8_shared_wait::WaitEnvironment,
    worker_wire::*,
};
use std::sync::{Arc, Mutex};
struct ImmediatelyNotified {
    calls: Mutex<Vec<SharedMemoryRequest>>,
    owners: Mutex<Vec<Creator>>,
}
impl WorkerTransport for ImmediatelyNotified {
    fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
        let AgentServiceRequest::SharedMemory { request, creator } = request else {
            panic!("unexpected route")
        };
        self.calls.lock().unwrap().push(request.clone());
        self.owners.lock().unwrap().push(creator);
        let descriptor = if let SharedMemoryRequest::Open { descriptor } = &request {
            Some(*descriptor)
        } else {
            None
        };
        let native_descriptor = descriptor.map(|_| {
            Arc::new(std::os::fd::OwnedFd::from(
                std::fs::File::open("/dev/null").unwrap(),
            ))
        });
        let outcome = match request {
            SharedMemoryRequest::WaitAdmission { .. } => {
                SharedMemoryOutcome::Wait(SharedWaitResult::Pending(SharedWaitId([8; 32])))
            }
            // The coordinator notification wins before the first native poll.
            SharedMemoryRequest::WaitPoll { .. } => SharedMemoryOutcome::Wait(SharedWaitResult::Ok),
            _ => SharedMemoryOutcome::None,
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
fn enqueued_wait_async_keeps_promise_when_notification_wins_before_first_poll() {
    super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let environment = WaitEnvironment::new().unwrap();
    let buffer = v8::SharedArrayBuffer::new(scope, 4).unwrap();
    let broker = Arc::new(ImmediatelyNotified {
        calls: Mutex::new(Vec::new()),
        owners: Mutex::new(Vec::new()),
    });
    let lease = Arc::new(MappingLease {
        mapping_live: std::sync::atomic::AtomicBool::new(true),
        transport: broker.clone(),
        creator: Creator::Agent(AgentId(7)),
        descriptor: SharedMemoryDescriptor {
            token: SharedMemoryToken([3; 32]),
            byte_length: 4,
            mapped_length: 8192,
            maximum_length: 4,
            data_offset: 4096,
            reserved_length: 4096,
            guard_length: 0,
            wasm: false,
            growable: false,
            memory64: false,
        },
    });
    environment.attach(&buffer, lease);
    environment.retire();
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<!doctype html><body>")
            .into_runtime()
            .unwrap(),
    ));
    let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    state.set_worker_transport(broker.clone(), Some(Creator::Agent(AgentId(8))), None);
    let key = v8::String::new(scope, "shared").unwrap();
    context.global(scope).set(scope, key.into(), buffer.into());
    let code=v8::String::new(scope,"globalThis.result=Atomics.waitAsync(new Int32Array(shared),0,0); result.async===true && result.value instanceof Promise").unwrap();
    assert!(
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    );
    let key = v8::String::new(scope, "result").unwrap();
    let result = context
        .global(scope)
        .get(scope, key.into())
        .unwrap()
        .to_object(scope)
        .unwrap();
    let key = v8::String::new(scope, "value").unwrap();
    let promise =
        v8::Local::<v8::Promise>::try_from(result.get(scope, key.into()).unwrap()).unwrap();
    assert_eq!(promise.state(), v8::PromiseState::Pending);
    assert_eq!(
        broker.calls.lock().unwrap().len(),
        1,
        "one atomic native admission precedes promise construction"
    );
    assert!(
        broker
            .owners
            .lock()
            .unwrap()
            .iter()
            .all(|owner| *owner == Creator::Agent(AgentId(8))),
        "wait belongs to current realm, never backing creator"
    );
    assert!(
        !environment.pending(),
        "retired creating realm must not own the new waiter"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while promise.state() == v8::PromiseState::Pending {
        state.shared_waits.borrow().as_ref().unwrap().poll(scope);
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
    assert_eq!(promise.result(scope).to_rust_string_lossy(scope), "ok");
    assert!(!state.shared_waits.borrow().as_ref().unwrap().pending());
    state.shared_waits.borrow().as_ref().unwrap().retire();
    assert!(
        !broker
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|request| matches!(request, SharedMemoryRequest::WaitCancel { .. })),
        "acknowledged completion has no outstanding cancellation lease"
    );
}

#[test]
fn wasm_mapping_validation_failure_releases_native_lease_once() {
    super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let broker = Arc::new(ImmediatelyNotified {
        calls: Mutex::new(Vec::new()),
        owners: Mutex::new(Vec::new()),
    });
    let fd: std::os::fd::OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let response = SharedMemoryResponse {
        isolated: true,
        outcome: SharedMemoryOutcome::None,
        descriptor: Some(SharedMemoryDescriptor {
            token: SharedMemoryToken([5; 32]),
            byte_length: 65536,
            maximum_length: 65536,
            mapped_length: 131072,
            data_offset: 65536,
            reserved_length: 65536,
            guard_length: 0,
            wasm: true,
            growable: false,
            memory64: false,
        }),
        native_descriptor: Some(Arc::new(fd)),
    };
    let waits = crate::v8_shared_wait::WaitEnvironment::new().unwrap();
    assert!(
        crate::v8_shared_memory::map_wasm_backing(
            scope,
            broker.clone(),
            Creator::Agent(AgentId(7)),
            response,
            &waits
        )
        .is_err()
    );
    let calls = broker.calls.lock().unwrap();
    assert!(
        matches!(calls.as_slice(),[SharedMemoryRequest::Release{token}] if *token==SharedMemoryToken([5;32]))
    );
}
