//! Scoped access to public V8 context APIs missing from rusty_v8 150.4.0.
use std::rc::Rc;

unsafe extern "C" {
    fn oryn_v8_incumbent_context(isolate: v8::UnsafeRawIsolatePtr) -> *const v8::Context;
    fn oryn_v8_detach_global(context: *const v8::Context);
    fn oryn_v8_can_access_window(
        isolate: v8::UnsafeRawIsolatePtr,
        receiver: *const v8::Object,
    ) -> bool;
}

pub fn incumbent_slot<T: 'static>(scope: &mut v8::PinScope) -> Option<Rc<T>> {
    // SAFETY: The isolate is entered and the scope is live. V8 allocates the
    // returned local handle in this scope, using its standard public Local<T>
    // representation. Read the Rust-owned slot immediately; neither a raw
    // pointer nor a borrowed Context escapes this call. The cloned Rc owns T.
    unsafe { oryn_v8_incumbent_context(scope.as_raw_isolate_ptr()).as_ref() }
        .and_then(v8::Context::get_slot::<T>)
}

pub fn detach_global(scope: &mut v8::PinScope<'_, '_, ()>, context: v8::Local<v8::Context>) {
    // SAFETY: Rebind into the caller's live handle scope so the context remains
    // rooted for the complete native call. Context's public API permits internal
    // mutation through the opaque handle, as do rusty_v8's other context methods.
    let context = v8::Local::new(scope, context);
    unsafe { oryn_v8_detach_global(&*context) }
}

/// Check the calling realm against an actual native Window global proxy.
/// Ordinary DOM objects and author-created proxies are not Window receivers.
pub(crate) fn can_access_window(scope: &mut v8::PinScope, receiver: v8::Local<v8::Object>) -> bool {
    // SAFETY: Both the receiver and the incumbent context are rooted in the
    // active isolate's live handle scope. The C bridge only compares native
    // context/global identities and security tokens; it executes no page code.
    unsafe { oryn_v8_can_access_window(scope.as_raw_isolate_ptr(), &*receiver) }
}
