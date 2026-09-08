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

/// Public V8 interceptor callbacks used only when a context access check fails.
/// The function pointers use exactly the callback ABI exposed by rusty_v8.
#[repr(C)]
#[derive(Default)]
pub struct CrossContextHandlers {
    pub named_getter: Option<v8::NamedPropertyGetterCallback>,
    pub named_setter: Option<v8::NamedPropertySetterCallback>,
    pub named_query: Option<v8::NamedPropertyQueryCallback>,
    pub named_deleter: Option<v8::NamedPropertyDeleterCallback>,
    pub named_enumerator: Option<v8::NamedPropertyEnumeratorCallback>,
    pub named_definer: Option<v8::NamedPropertyDefinerCallback>,
    pub named_descriptor: Option<v8::NamedPropertyDescriptorCallback>,
    pub indexed_getter: Option<v8::IndexedPropertyGetterCallback>,
    pub indexed_setter: Option<v8::IndexedPropertySetterCallback>,
    pub indexed_query: Option<v8::IndexedPropertyQueryCallback>,
    pub indexed_deleter: Option<v8::IndexedPropertyDeleterCallback>,
    pub indexed_enumerator: Option<v8::IndexedPropertyEnumeratorCallback>,
    pub indexed_definer: Option<v8::IndexedPropertyDefinerCallback>,
    pub indexed_descriptor: Option<v8::IndexedPropertyDescriptorCallback>,
}

unsafe extern "C" {
    fn oryn_v8_set_cross_context_handlers(
        template: *const v8::ObjectTemplate,
        handlers: *const CrossContextHandlers,
    );
}

/// Install token-based access checks and native cross-context interceptors.
/// Configure the template before creating any contexts from it. The handlers
/// must enforce the host interface's allowed cross-origin property surface.
pub fn set_cross_context_handlers(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<v8::ObjectTemplate>,
    handlers: &CrossContextHandlers,
) {
    let template = v8::Local::new(scope, template);
    // SAFETY: The template is rooted in this isolate and the repr(C) record
    // contains only C callback pointers. V8 copies the pointers synchronously;
    // it never retains a pointer to the Rust configuration record.
    unsafe { oryn_v8_set_cross_context_handlers(&*template, handlers) }
}

unsafe extern "C" {
    fn oryn_v8_set_window_failed_access_callback(isolate: v8::UnsafeRawIsolatePtr);
}
/// Configure native Window errors for operations bypassing interceptors.
pub(crate) fn set_window_failed_access_callback(scope: &mut v8::PinScope<'_, '_, ()>) {
    // SAFETY: The isolate is entered. The fixed C++ callback forwards rooted
    // current-context and target handles to the native Window binding callback.
    unsafe { oryn_v8_set_window_failed_access_callback(scope.as_raw_isolate_ptr()) }
}

unsafe extern "C" {
    fn oryn_v8_set_window_index_handlers(
        template: *const v8::ObjectTemplate,
        handlers: *const CrossContextHandlers,
    );
}
/// Install normal Window indexed handlers with false-returning write hooks.
pub(crate) fn set_window_index_handlers(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<v8::ObjectTemplate>,
    handlers: &CrossContextHandlers,
) {
    let template = v8::Local::new(scope, template);
    // SAFETY: The template is rooted; V8 synchronously copies the public C
    // callback pointers. The C++ adapter supplies the modern boolean write ABI.
    unsafe { oryn_v8_set_window_index_handlers(&*template, handlers) }
}

unsafe extern "C" {
    fn oryn_v8_configure_html_all_collection(
        template: *const v8::ObjectTemplate,
        callback: v8::FunctionCallback,
    );
}

/// Configure HTMLAllCollection's native call behavior and [[IsHTMLDDA]] flag.
/// Call before instantiating the template; the callback must implement the
/// collection's legacy call algorithm and validate its native owner.
pub fn configure_html_all_collection(
    template: v8::Local<v8::ObjectTemplate>,
    callback: v8::FunctionCallback,
) {
    // SAFETY: The local roots the template, and FunctionCallback has the V8
    // callback ABI. V8 retains the function pointer, not a Rust reference.
    unsafe { oryn_v8_configure_html_all_collection(&*template, callback) }
}
