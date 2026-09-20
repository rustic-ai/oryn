//! Native sandbox mapping ownership. No author-selected descriptor or pointer
//! reaches this interface: callers supply a response from their bound broker.
use crate::shared_memory::*;
use crate::worker_wire::{AgentServiceRequest, AgentServiceResponse, Creator, WorkerTransport};
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

unsafe extern "C" {
    fn oryn_v8_map_shared_wasm_memory(
        isolate: v8::UnsafeRawIsolatePtr,
        fd: i32,
        byte_length: usize,
        mapped_length: usize,
        data_offset: usize,
        reserved_length: usize,
        guard_length: usize,
        maximum_length: usize,
        memory64: bool,
        grow_pages: unsafe extern "C" fn(*mut c_void, usize, usize, *mut usize) -> bool,
        release: unsafe extern "C" fn(*mut c_void),
        context: *mut c_void,
    ) -> *const v8::SharedArrayBuffer;
    fn oryn_v8_shared_memory_granularity(isolate: v8::UnsafeRawIsolatePtr) -> usize;
    fn oryn_v8_map_shared_memory(
        isolate: v8::UnsafeRawIsolatePtr,
        fd: i32,
        byte_length: usize,
        mapped_length: usize,
        data_offset: usize,
        reserved_length: usize,
        guard_length: usize,
        maximum_length: usize,
        growable: bool,
        grow: unsafe extern "C" fn(*mut c_void, usize) -> i32,
        release: unsafe extern "C" fn(*mut c_void),
        context: *mut c_void,
    ) -> *const v8::SharedArrayBuffer;
}

/// One admitted parent mapping reference. V8 owns the strong reference;
/// identity tables may retain only Weak references so GC releases the lease.
pub struct MappingLease {
    pub(crate) transport: Arc<dyn WorkerTransport>,
    pub(crate) creator: Creator,
    pub descriptor: SharedMemoryDescriptor,
    pub(crate) mapping_live: AtomicBool,
}
static MAPPINGS: OnceLock<Mutex<BTreeMap<usize, Weak<MappingLease>>>> = OnceLock::new();
/// Native identity only. Possession of this mapping does not authorize a new
/// recipient: the parent still validates the receiving document and cluster.
pub(crate) fn lookup(buffer: &v8::SharedArrayBuffer) -> Option<Arc<MappingLease>> {
    let address = buffer.get_backing_store().data()?.as_ptr() as usize;
    let mappings = MAPPINGS.get()?.lock().ok()?;
    let lease = mappings.get(&address)?.upgrade()?;
    lease.mapping_live.load(Ordering::Acquire).then_some(lease)
}
fn register(address: usize, lease: &Arc<MappingLease>) {
    let mut mappings = MAPPINGS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    mappings.retain(|_, entry| {
        entry
            .upgrade()
            .is_some_and(|lease| lease.mapping_live.load(Ordering::Acquire))
    });
    mappings.insert(address, Arc::downgrade(lease));
}
impl MappingLease {
    pub fn request(&self, request: SharedMemoryRequest) -> Result<SharedMemoryResponse, String> {
        match self
            .transport
            .request(AgentServiceRequest::SharedMemory {
                creator: self.creator.clone(),
                request,
            })
            .map_err(|error| error.message)?
        {
            AgentServiceResponse::SharedMemory(response) => Ok(response),
            _ => Err("shared memory response kind mismatch".into()),
        }
    }
}
impl Drop for MappingLease {
    fn drop(&mut self) {
        // Mapping destruction may run on a V8 background thread. This owns no
        // isolate handles and uses the route-bound, thread-safe native broker.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.request(SharedMemoryRequest::Release {
                token: self.descriptor.token,
            })
        }));
    }
}
unsafe extern "C" fn release(context: *mut c_void) {
    if !context.is_null() {
        // The shim consumes exactly this Box on success AND every failure.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let lease = unsafe { Box::from_raw(context.cast::<Arc<MappingLease>>()) };
            // Waiters may retain the physical capability after V8 frees this
            // mapping. Never allow address reuse to authenticate a new object.
            lease.mapping_live.store(false, Ordering::Release);
            drop(lease);
        }));
    }
}

unsafe extern "C" fn grow(context: *mut c_void, length: usize) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let lease = unsafe { &*context.cast::<Arc<MappingLease>>() };
        match lease.request(SharedMemoryRequest::GrowAbsolute {
            token: lease.descriptor.token,
            length: length as u64,
        }) {
            Ok(SharedMemoryResponse {
                outcome: SharedMemoryOutcome::Length(_),
                ..
            }) => 0,
            _ => 1,
        }
    }))
    .unwrap_or(1)
}

unsafe extern "C" fn grow_pages(
    context: *mut c_void,
    delta: usize,
    maximum: usize,
    previous: *mut usize,
) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() || previous.is_null() {
            return false;
        }
        let lease = unsafe { &*context.cast::<Arc<MappingLease>>() };
        if !lease.descriptor.wasm {
            return false;
        }
        match lease.request(SharedMemoryRequest::GrowByPages {
            token: lease.descriptor.token,
            pages: delta as u64,
            maximum_pages: maximum as u64,
        }) {
            Ok(SharedMemoryResponse {
                outcome: SharedMemoryOutcome::Length(old_pages),
                ..
            }) => {
                let Ok(old) = usize::try_from(old_pages) else {
                    return false;
                };
                if delta > maximum || old > maximum - delta {
                    return false;
                }
                unsafe {
                    *previous = old;
                }
                true
            }
            _ => false,
        }
    }))
    .unwrap_or(false)
}

pub fn granularity(scope: &mut v8::PinScope) -> usize {
    unsafe { oryn_v8_shared_memory_granularity(scope.as_raw_isolate_ptr()) }
}

/// Consume one Allocate/Open response, transferring its parent reference to
/// the engine. A failure also releases it; callers must not release it again.
pub fn map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transport: Arc<dyn WorkerTransport>,
    creator: Creator,
    response: SharedMemoryResponse,
    waits: &crate::v8_shared_wait::WaitEnvironment,
) -> Result<(v8::Local<'s, v8::SharedArrayBuffer>, Weak<MappingLease>), String> {
    map_kind(scope, transport, creator, response, false, waits)
}

/// Native engine factory only: returns a temporary SAB handle to an actual
/// shared Wasm backing. This does not construct a WebAssembly.Memory object.
pub(crate) fn map_wasm_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transport: Arc<dyn WorkerTransport>,
    creator: Creator,
    response: SharedMemoryResponse,
    waits: &crate::v8_shared_wait::WaitEnvironment,
) -> Result<(v8::Local<'s, v8::SharedArrayBuffer>, Weak<MappingLease>), String> {
    map_kind(scope, transport, creator, response, true, waits)
}
fn map_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transport: Arc<dyn WorkerTransport>,
    creator: Creator,
    response: SharedMemoryResponse,
    wasm: bool,
    waits: &crate::v8_shared_wait::WaitEnvironment,
) -> Result<(v8::Local<'s, v8::SharedArrayBuffer>, Weak<MappingLease>), String> {
    let descriptor = response
        .descriptor
        .ok_or("missing shared memory descriptor")?;
    let lease = Arc::new(MappingLease {
        transport,
        creator,
        descriptor,
        mapping_live: AtomicBool::new(true),
    });
    if !response.isolated || descriptor.wasm != wasm {
        return Err("shared memory mapping policy or engine kind mismatch".into());
    }
    let fd = response
        .native_descriptor
        .ok_or("missing native shared memory attachment")?;
    let size = |value: u64| {
        usize::try_from(value).map_err(|_| "shared mapping extent overflow".to_string())
    };
    let (length, mapped, offset, reserved, guard, maximum) = (
        size(descriptor.byte_length)?,
        size(descriptor.mapped_length)?,
        size(descriptor.data_offset)?,
        size(descriptor.reserved_length)?,
        size(descriptor.guard_length)?,
        size(descriptor.maximum_length)?,
    );
    let weak = Arc::downgrade(&lease);
    let context = Box::into_raw(Box::new(lease)).cast::<c_void>();
    // No Rust-owned strong lease survives this call. The native shim consumes
    // context at entry, including validation failures and allocation failures.
    let pointer = unsafe {
        if wasm {
            oryn_v8_map_shared_wasm_memory(
                scope.as_raw_isolate_ptr(),
                fd.as_raw_fd(),
                length,
                mapped,
                offset,
                reserved,
                guard,
                maximum,
                descriptor.memory64,
                grow_pages,
                release,
                context,
            )
        } else {
            oryn_v8_map_shared_memory(
                scope.as_raw_isolate_ptr(),
                fd.as_raw_fd(),
                length,
                mapped,
                offset,
                reserved,
                guard,
                maximum,
                descriptor.growable,
                grow,
                release,
                context,
            )
        }
    };
    let value = unsafe { pointer.as_ref() }.ok_or("native shared mapping failed")?;
    // Use rusty_v8's public backing-store API rather than constructing Local
    // handles by transmute. Both temporary and returned wrappers share ownership.
    let backing = value.get_backing_store();
    // The growth delegate is already installed by the mapping shim. Register
    // waits against that final range before making any wrapper author-visible.
    let retained = weak.upgrade().ok_or("shared mapping ownership lost")?;
    let address = backing
        .data()
        .ok_or("native shared mapping address missing")?
        .as_ptr() as usize;
    register(address, &retained);
    waits.attach(value, retained);
    Ok((
        v8::SharedArrayBuffer::with_backing_store(scope, &backing),
        weak,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Broker(Mutex<Vec<SharedMemoryRequest>>);
    impl WorkerTransport for Broker {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, crate::worker_wire::WorkerError> {
            let AgentServiceRequest::SharedMemory {
                creator:
                    Creator::Window {
                        target: crate::message_wire::WindowTarget::Root,
                        document_generation: 7,
                        stream_epoch: 3,
                    },
                request,
            } = request
            else {
                panic!("wrong native principal")
            };
            self.0.lock().unwrap().push(request);
            Ok(AgentServiceResponse::SharedMemory(SharedMemoryResponse {
                isolated: true,
                outcome: SharedMemoryOutcome::Length(8),
                descriptor: None,
                native_descriptor: None,
            }))
        }
    }
    #[test]
    fn wasm_growth_callback_preserves_atomic_previous_pages_and_call_limit() {
        let broker = Arc::new(Broker(Mutex::new(Vec::new())));
        let token = SharedMemoryToken([41; 32]);
        let lease = Arc::new(MappingLease {
            mapping_live: AtomicBool::new(true),
            transport: broker.clone(),
            creator: Creator::Window {
                target: crate::message_wire::WindowTarget::Root,
                document_generation: 7,
                stream_epoch: 3,
            },
            descriptor: SharedMemoryDescriptor {
                token,
                byte_length: 65536,
                mapped_length: 16 * 65536 + 4096,
                maximum_length: 16 * 65536,
                data_offset: 4096,
                reserved_length: 16 * 65536,
                guard_length: 0,
                wasm: true,
                growable: false,
                memory64: false,
            },
        });
        let context = Box::into_raw(Box::new(lease)).cast();
        let mut previous = 99;
        assert!(unsafe { grow_pages(context, 2, 16, &mut previous) });
        assert_eq!(previous, 8);
        previous = 99;
        assert!(!unsafe { grow_pages(context, 2, 7, &mut previous) });
        assert_eq!(
            previous, 99,
            "failed validation must not publish a previous size"
        );
        unsafe { release(context) };
        let calls = broker.0.lock().unwrap();
        assert!(matches!(
            calls.as_slice(),
            [
                SharedMemoryRequest::GrowByPages {
                    pages: 2,
                    maximum_pages: 16,
                    ..
                },
                SharedMemoryRequest::GrowByPages {
                    pages: 2,
                    maximum_pages: 7,
                    ..
                },
                SharedMemoryRequest::Release { .. }
            ]
        ));
    }
    #[test]
    fn retired_native_mapping_cannot_authenticate_reused_address_with_live_wait_lease() {
        let broker = Arc::new(Broker(Mutex::new(Vec::new())));
        let lease = Arc::new(MappingLease {
            mapping_live: AtomicBool::new(true),
            transport: broker.clone(),
            creator: Creator::Window {
                target: crate::message_wire::WindowTarget::Root,
                document_generation: 7,
                stream_epoch: 3,
            },
            descriptor: SharedMemoryDescriptor {
                token: SharedMemoryToken([43; 32]),
                byte_length: 8,
                mapped_length: 8192,
                maximum_length: 4096,
                data_offset: 4096,
                reserved_length: 4096,
                guard_length: 0,
                wasm: false,
                growable: true,
                memory64: false,
            },
        });
        let waiter = lease.clone();
        let context = Box::into_raw(Box::new(lease)).cast();
        unsafe { release(context) };
        assert!(!waiter.mapping_live.load(Ordering::Acquire));
        assert!(
            broker.0.lock().unwrap().is_empty(),
            "waiter retains physical capability until acknowledgement"
        );
        drop(waiter);
        assert!(matches!(
            broker.0.lock().unwrap().as_slice(),
            [SharedMemoryRequest::Release { .. }]
        ));
    }
    #[test]
    fn mapping_callback_owns_one_release_and_preserves_exact_creator() {
        let broker = Arc::new(Broker(Mutex::new(Vec::new())));
        let token = SharedMemoryToken([42; 32]);
        let lease = Arc::new(MappingLease {
            mapping_live: AtomicBool::new(true),
            transport: broker.clone(),
            creator: Creator::Window {
                target: crate::message_wire::WindowTarget::Root,
                document_generation: 7,
                stream_epoch: 3,
            },
            descriptor: SharedMemoryDescriptor {
                token,
                byte_length: 8,
                mapped_length: 8192,
                maximum_length: 4096,
                data_offset: 4096,
                reserved_length: 4096,
                guard_length: 0,
                wasm: false,
                growable: true,
                memory64: false,
            },
        });
        let weak = Arc::downgrade(&lease);
        let context = Box::into_raw(Box::new(lease)).cast();
        assert_eq!(unsafe { grow(context, 16) }, 0);
        assert!(weak.upgrade().is_some());
        unsafe { release(context) };
        assert!(weak.upgrade().is_none());
        let calls = broker.0.lock().unwrap();
        assert!(
            matches!(calls.as_slice(), [SharedMemoryRequest::GrowAbsolute { token: seen, length: 16 }, SharedMemoryRequest::Release { token: released }] if *seen == token && *released == token)
        );
    }
}
