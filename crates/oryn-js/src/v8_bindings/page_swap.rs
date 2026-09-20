//! Page transition payloads live on native event wrappers, never page properties.
use super::*;

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "PageSwapEvent".into(),
        member: "TypeError: incompatible transition object".into(),
    }
}

pub(super) fn activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let value =
        event_init_member(scope, init, "activation").ok_or(V8BindingError::JavaScriptException)?;
    let value = if value.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        value
    };
    navigation_api::validate_activation(scope, value)?;
    Ok(value)
}

pub(super) fn view_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let value = event_init_member(scope, init, "viewTransition")
        .ok_or(V8BindingError::JavaScriptException)?;
    if value.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    // A transition must be supplied by the native rendering owner. No public
    // constructor/prototype or author-defined instanceof hook is authority.
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    if object.is_proxy() {
        return Err(invalid());
    }
    let key = transition_brand(scope)?;
    if !object
        .get_private(scope, key)
        .is_some_and(|value| value.is_true())
    {
        return Err(invalid());
    }
    Ok(value)
}

fn transition_brand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.native.view-transition")
        .ok_or(V8BindingError::Allocation("transition brand"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

pub(super) fn attach(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    interface: &str,
    activation: v8::Local<v8::Value>,
    transition: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let brand = v8_string(scope, interface)?;
    // Create populated elements directly so inherited Array index setters are
    // never called while creating a trusted browser event.
    let payload = v8::Array::new_with_elements(scope, &[brand, activation, transition]);
    let null = v8::null(scope);
    if payload.set_prototype(scope, null.into()) != Some(true) {
        return Err(V8BindingError::Allocation("transition event payload"));
    }
    event.set_internal_field(2, payload.into());
    Ok(())
}

pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if event.is_proxy() {
        return Err(invalid());
    }
    event_id(scope, event).map_err(|_| invalid())?;
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let brand = payload.get_index(scope, 0).ok_or_else(invalid)?;
    if !brand.is_string() || brand.to_rust_string_lossy(scope) != interface {
        return Err(invalid());
    }
    let index = match (interface, member) {
        ("PageSwapEvent", "activation") => 1,
        ("PageSwapEvent" | "PageRevealEvent", "viewTransition") => 2,
        _ => return Err(invalid()),
    };
    payload.get_index(scope, index).ok_or_else(invalid)
}

pub(super) fn prepare_reveal(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<native_platform::publication::Notification, V8BindingError> {
    let event = create_event_object_with_interface(
        scope,
        state,
        "PageRevealEvent",
        "pagereveal",
        false,
        false,
    )?;
    let null = v8::null(scope);
    attach(scope, event, "PageRevealEvent", null.into(), null.into())?;
    let id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&id)
        .ok_or_else(invalid)?
        .is_trusted = true;
    Ok(native_platform::publication::pending_platform_dispatch(
        scope,
        state,
        global_events::GLOBAL_TARGET,
        "pagereveal",
        event,
    )
    .for_parser_stream(state.document_stream_epoch()))
}

pub(super) fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    activation: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired transition document"));
    }
    let (interface, event_type) = ("PageSwapEvent", "pageswap");
    let event =
        create_event_object_with_interface(scope, state, interface, event_type, false, false)?;
    let transition = v8::null(scope);
    attach(scope, event, interface, activation, transition.into())?;
    let id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&id)
        .ok_or_else(invalid)?
        .is_trusted = true;
    global_events::dispatch(scope, state, event).map(|_| ())
}

/// A native navigation owner retains this before entering departing-document
/// callbacks. It is tied to that exact Document, not a reusable Window shell.
pub struct NativePageSwap {
    owner: RcWeak<V8BindingState>,
    generation: u64,
    stream: u64,
    epoch: u64,
    event: Option<native_platform::publication::Notification>,
    complete: bool,
    canceled: bool,
}
impl std::fmt::Debug for NativePageSwap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativePageSwap")
            .field("generation", &self.generation)
            .field("stream", &self.stream)
            .field("complete", &self.complete)
            .field("canceled", &self.canceled)
            .finish()
    }
}
impl NativePageSwap {
    pub fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        url: &str,
        kind: &str,
        disclose: bool,
    ) -> Result<Self, V8BindingError> {
        if state.execution_retired() || state.execution_suspended() {
            return Err(invalid());
        }
        let activation = if disclose {
            navigation_api::activation_for_swap(scope, url, kind)?.into()
        } else {
            v8::null(scope).into()
        };
        let event = create_event_object_with_interface(
            scope,
            state,
            "PageSwapEvent",
            "pageswap",
            false,
            false,
        )?;
        let null = v8::null(scope);
        attach(scope, event, "PageSwapEvent", activation, null.into())?;
        let receipt = Self::from_event(scope, state, "pageswap", event)?;
        navigation_api::consume_swap_destination(state);
        Ok(receipt)
    }
    pub fn page_transition(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        name: &str,
        persisted: bool,
    ) -> Result<Self, V8BindingError> {
        if !matches!(name, "pageshow" | "pagehide")
            || state.execution_retired()
            || state.execution_suspended()
        {
            return Err(invalid());
        }
        let event = create_event_object_with_interface(
            scope,
            state,
            "PageTransitionEvent",
            name,
            false,
            false,
        )?;
        event.set_internal_field(2, v8::Boolean::new(scope, persisted).into());
        Self::from_event(scope, state, name, event)
    }
    fn from_event(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        name: &str,
        event: v8::Local<v8::Object>,
    ) -> Result<Self, V8BindingError> {
        let id = event_id(scope, event)?;
        state
            .events
            .borrow_mut()
            .get_mut(&id)
            .ok_or_else(invalid)?
            .is_trusted = true;
        Ok(Self {
            owner: Rc::downgrade(state),
            generation: state.document_generation(),
            stream: state.document_stream_epoch(),
            epoch: state.document_authority_epoch(),
            event: Some(
                native_platform::publication::pending_platform_dispatch(
                    scope,
                    state,
                    global_events::GLOBAL_TARGET,
                    name,
                    event,
                )
                .for_parser_stream(state.document_stream_epoch()),
            ),
            complete: false,
            canceled: false,
        })
    }
    pub fn owner(&self) -> Option<Rc<V8BindingState>> {
        self.owner.upgrade()
    }
    fn current(&self, state: &V8BindingState) -> bool {
        !state.execution_retired()
            && !state.execution_suspended()
            && state.document_generation() == self.generation
            && state.document_stream_epoch() == self.stream
            && state.document_authority_epoch() == self.epoch
    }
    pub fn retry_safe(&self) -> bool {
        !self.complete && !self.canceled && self.owner().is_some_and(|state| self.current(&state))
    }
    pub fn canceled(&self) -> bool {
        self.canceled
    }
    pub fn cancel(&mut self) {
        if self.complete {
            return;
        }
        if let Some(state) = self.owner()
            && let Some(event) = self.event.take()
        {
            event.cancel(&state);
        }
        self.canceled = true;
        self.complete = true;
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.complete {
            return Ok(true);
        }
        let Some(state) = self.owner().filter(|state| self.current(state)) else {
            self.cancel();
            return Ok(true);
        };
        let done = self
            .event
            .as_mut()
            .expect("live page transition event")
            .resume(scope, &state)?;
        if !self.current(&state) {
            self.cancel();
            return Ok(true);
        }
        if !done {
            return Ok(false);
        }
        if let Some(event) = self.event.take() {
            event.finish(scope, &state);
        }
        self.complete = true;
        Ok(true)
    }
}
impl Drop for NativePageSwap {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod retained_tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> bool {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    }
    #[test]
    fn swap_and_hide_keep_exact_callback_suffix_and_do_not_replay() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];addEventListener('pageswap',e=>calls.push('swap:'+e.isTrusted+':'+(e.activation===null)));addEventListener('pageswap',()=>calls.push('suffix'));onpagehide=e=>calls.push('hide:'+e.persisted);true",
        );
        let mut swap =
            NativePageSwap::new(scope, &state, "https://other.test/", "push", false).unwrap();
        let mut hide = NativePageSwap::page_transition(scope, &state, "pagehide", false).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(swap.poll(scope).is_err());
        assert!(swap.retry_safe());
        assert!(eval(scope, "calls.join()==='swap:true:true'"));
        assert!(swap.poll(scope).unwrap());
        assert!(swap.poll(scope).unwrap());
        assert!(hide.poll(scope).unwrap());
        assert!(eval(
            scope,
            "calls.join()==='swap:true:true,suffix,hide:false'"
        ));
        assert!(!swap.canceled());
    }
    #[test]
    fn stream_replacement_cancels_retained_departure() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];addEventListener('pageswap',()=>{calls.push('first');document.open()});addEventListener('pageswap',()=>calls.push('suffix'));true",
        );
        let mut swap =
            NativePageSwap::new(scope, &state, "https://other.test/", "push", false).unwrap();
        assert!(swap.poll(scope).unwrap());
        assert!(swap.canceled());
        assert!(eval(scope, "calls.join()==='first'"));
    }
}
