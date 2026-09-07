//! Scoped access to two public V8 context APIs missing from rusty_v8 150.4.0.
use std::rc::Rc;

unsafe extern "C" {
    fn oryn_v8_incumbent_context(isolate: v8::UnsafeRawIsolatePtr) -> *const v8::Context;
    fn oryn_v8_detach_global(context: *const v8::Context);
}

pub(crate) fn incumbent_slot<T: 'static>(scope: &mut v8::PinScope) -> Option<Rc<T>> {
    // SAFETY: The isolate is entered and the scope is live. V8 allocates the
    // returned local handle in this scope, using its standard public Local<T>
    // representation. Read the Rust-owned slot immediately; neither a raw
    // pointer nor a borrowed Context escapes this call. The cloned Rc owns T.
    unsafe { oryn_v8_incumbent_context(scope.as_raw_isolate_ptr()).as_ref() }
        .and_then(v8::Context::get_slot::<T>)
}

pub(crate) fn detach_global(scope: &mut v8::PinScope<'_, '_, ()>, context: v8::Local<v8::Context>) {
    // SAFETY: Rebind into the caller's live handle scope so the context remains
    // rooted for the complete native call. Context's public API permits internal
    // mutation through the opaque handle, as do rusty_v8's other context methods.
    let context = v8::Local::new(scope, context);
    unsafe { oryn_v8_detach_global(&*context) }
}
