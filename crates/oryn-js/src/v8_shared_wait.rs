//! Isolate-owned native waitAsync completion, backed by parent waiter tokens.
use crate::{shared_memory::*, v8_shared_memory::MappingLease};
use std::{ffi::c_void, marker::PhantomData, ptr::NonNull, rc::Rc, sync::Arc};
unsafe extern "C" {
    fn oryn_v8_wait_environment_new() -> *mut c_void;
    fn oryn_v8_wait_environment_retire(environment: *mut c_void);
    fn oryn_v8_wait_environment_delete(environment: *mut c_void);
    fn oryn_v8_wait_environment_pending(environment: *mut c_void) -> bool;
    fn oryn_v8_wait_environment_poll(
        isolate: v8::UnsafeRawIsolatePtr,
        environment: *mut c_void,
    ) -> usize;
    fn oryn_v8_set_shared_wait_delegate(
        buffer: *const v8::SharedArrayBuffer,
        environment: *mut c_void,
        context: *mut c_void,
        begin: unsafe extern "C" fn(
            *mut c_void,
            v8::Local<v8::Context>,
            usize,
            bool,
            u64,
            i64,
            *mut i32,
        ) -> *mut c_void,
        poll: unsafe extern "C" fn(*mut c_void, u32) -> i32,
        admission: unsafe extern "C" fn(*mut c_void, u32) -> i32,
        cancel: unsafe extern "C" fn(*mut c_void),
        notify: unsafe extern "C" fn(*mut c_void, usize, u32) -> u32,
        release: unsafe extern "C" fn(*mut c_void),
    );
}
/// Not Send/Sync: persistent V8 handles are polled and retired only by their
/// owning native environment, before that isolate is disposed.
pub struct WaitEnvironment {
    pointer: NonNull<c_void>,
    _isolate: PhantomData<Rc<()>>,
}
impl WaitEnvironment {
    pub fn new() -> Option<Self> {
        NonNull::new(unsafe { oryn_v8_wait_environment_new() }).map(|pointer| Self {
            pointer,
            _isolate: PhantomData,
        })
    }
    pub(crate) fn native_pointer(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }
    pub fn poll(&self, scope: &mut v8::PinScope) -> usize {
        unsafe { oryn_v8_wait_environment_poll(scope.as_raw_isolate_ptr(), self.pointer.as_ptr()) }
    }
    pub fn retire(&self) {
        unsafe { oryn_v8_wait_environment_retire(self.pointer.as_ptr()) }
    }
    pub fn pending(&self) -> bool {
        unsafe { oryn_v8_wait_environment_pending(self.pointer.as_ptr()) }
    }
    pub fn attach(&self, buffer: &v8::SharedArrayBuffer, lease: Arc<MappingLease>) {
        let context = Box::into_raw(Box::new(lease)).cast();
        unsafe {
            oryn_v8_set_shared_wait_delegate(
                buffer,
                self.pointer.as_ptr(),
                context,
                begin_in_context,
                poll,
                admission,
                cancel,
                notify,
                release,
            )
        }
    }
}
impl Drop for WaitEnvironment {
    fn drop(&mut self) {
        unsafe { oryn_v8_wait_environment_delete(self.pointer.as_ptr()) }
    }
}
struct NativeWait {
    ticket: crate::shared_wait_custody::WaitTicket,
}
fn guarded<T>(fallback: T, operation: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).unwrap_or(fallback)
}
unsafe extern "C" fn begin_in_context(
    context: *mut c_void,
    current: v8::Local<v8::Context>,
    offset: usize,
    width64: bool,
    expected: u64,
    timeout: i64,
    initial: *mut i32,
) -> *mut c_void {
    guarded(std::ptr::null_mut(), || {
        if initial.is_null() {
            return std::ptr::null_mut();
        }
        let Some(state) = current.get_slot::<crate::v8_bindings::V8BindingState>() else {
            return std::ptr::null_mut();
        };
        let Some((transport, creator)) = state.native_shared_wait_authority() else {
            return std::ptr::null_mut();
        };
        let mapping = unsafe { &*context.cast::<Arc<MappingLease>>() }.clone();
        let acquire = creator != mapping.creator;
        let descriptor = mapping.descriptor;
        let Ok(ticket) = crate::shared_wait_custody::WaitTicket::begin(
            mapping,
            transport,
            creator,
            descriptor,
            acquire,
            offset as u64,
            width64,
            expected,
            u64::try_from(timeout).ok(),
        ) else {
            return std::ptr::null_mut();
        };
        // Admission is asynchronous even when the broker happens to be fast.
        // The engine's Admission loop processes interrupts and classifies from
        // the immutable compare/enqueue result, not a later terminal poll.
        unsafe {
            *initial = 5;
        }
        Box::into_raw(Box::new(NativeWait { ticket })).cast()
    })
}
fn state_code(result: Option<SharedWaitResult>) -> i32 {
    match result {
        None => 5,
        Some(SharedWaitResult::Pending(_)) => 0,
        Some(SharedWaitResult::Ok) => 1,
        Some(SharedWaitResult::NotEqual) => 2,
        Some(SharedWaitResult::TimedOut) => 3,
        Some(SharedWaitResult::Cancelled) => 4,
    }
}
unsafe extern "C" fn admission(context: *mut c_void, budget: u32) -> i32 {
    guarded(4, || {
        state_code(
            unsafe { &*context.cast::<NativeWait>() }
                .ticket
                .admission(std::time::Duration::from_millis(budget.min(50).into())),
        )
    })
}
unsafe extern "C" fn poll(context: *mut c_void, budget: u32) -> i32 {
    guarded(4, || {
        state_code(
            unsafe { &*context.cast::<NativeWait>() }
                .ticket
                .poll(std::time::Duration::from_millis(budget.min(50).into())),
        )
    })
}
unsafe extern "C" fn cancel(context: *mut c_void) {
    guarded((), || {
        drop(unsafe { Box::from_raw(context.cast::<NativeWait>()) })
    });
}
unsafe extern "C" fn release(context: *mut c_void) {
    guarded((), || {
        drop(unsafe { Box::from_raw(context.cast::<Arc<MappingLease>>()) });
    });
}
unsafe extern "C" fn notify(context: *mut c_void, offset: usize, count: u32) -> u32 {
    guarded(0, || {
        let lease = unsafe { &*context.cast::<Arc<MappingLease>>() };
        match lease.request(SharedMemoryRequest::Notify {
            token: lease.descriptor.token,
            offset: offset as u64,
            count: count.into(),
        }) {
            Ok(SharedMemoryResponse {
                outcome: SharedMemoryOutcome::Count(count),
                ..
            }) => u32::try_from(count).unwrap_or(u32::MAX),
            _ => 0,
        }
    })
}

/// Called only by V8 on the entered isolate thread. The returned handle is
/// copied immediately by C++; author properties never select the scheduler.
#[unsafe(no_mangle)]
unsafe extern "C" fn oryn_v8_current_wait_environment(
    context: v8::Local<v8::Context>,
) -> *mut c_void {
    guarded(std::ptr::null_mut(), || {
        let Some(state) = context.get_slot::<crate::v8_bindings::V8BindingState>() else {
            return std::ptr::null_mut();
        };
        if state.execution_retired() {
            return std::ptr::null_mut();
        }
        state.native_wait_environment_pointer()
    })
}
