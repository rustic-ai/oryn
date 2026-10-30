//! V8 modal prompt installation and trusted beforeunload dispatch.
use super::*;
pub use crate::prompts::*;

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    for (name, kind) in [("alert", 0), ("confirm", 1), ("prompt", 2)] {
        let data = v8::Integer::new(scope, kind);
        let function = v8::Function::builder(dialog)
            .data(data.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("prompt function"))?;
        let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("prompt name"))?;
        function.set_name(key);
        if context
            .global(scope)
            .create_data_property(scope, key.into(), function.into()) != Some(true) {
            return Err(V8BindingError::Allocation("prompt exposure"));
        }
    }
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("beforeunload owner"))?;
    if generated_interface("BeforeUnloadEvent").is_some() {
        let constructor = global_property(scope, "BeforeUnloadEvent")?
            .to_object(scope)
            .ok_or(V8BindingError::MissingInterface("BeforeUnloadEvent"))?;
        let key = v8::String::new(scope, "prototype").unwrap();
        let prototype = constructor
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or(V8BindingError::Allocation("BeforeUnloadEvent prototype"))?;
        let retained = context_intrinsics::retain_object_for(
            scope,
            &state,
            "beforeunload-prototype",
            prototype,
        )?;
        *state.before_unload_prototype.borrow_mut() = Some(retained);
        return Ok(());
    }
    // The source-generation gate defers registry regeneration. This bootstrap
    // fallback disappears from the active path once the updated IDL is generated.
    let constructor = v8::Function::new(scope, illegal_before_unload)
        .ok_or(V8BindingError::Allocation("BeforeUnloadEvent"))?;
    let prototype_key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, prototype_key.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("BeforeUnloadEvent prototype"))?;
    let event_constructor = global_property(scope, "Event")?
        .to_object(scope)
        .ok_or(V8BindingError::MissingInterface("Event"))?;
    let event_prototype = event_constructor
        .get(scope, prototype_key.into())
        .ok_or(V8BindingError::Allocation("Event prototype"))?;
    prototype.set_prototype(scope, event_prototype);
    let getter = v8::Function::new(scope, return_value_get)
        .ok_or(V8BindingError::Allocation("beforeunload return getter"))?;
    let setter = v8::Function::new(scope, return_value_set)
        .ok_or(V8BindingError::Allocation("beforeunload return setter"))?;
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let property = v8::String::new(scope, "returnValue").unwrap();
    prototype.define_property(scope, property.into(), &descriptor);
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "BeforeUnloadEvent").unwrap();
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
    let retained =
        context_intrinsics::retain_object_for(scope, &state, "beforeunload-prototype", prototype)?;
    *state.before_unload_prototype.borrow_mut() = Some(retained);
    context
        .global(scope)
        .create_data_property(scope, name.into(), constructor.into());
    Ok(())
}
fn illegal_before_unload(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    throw_type_error(scope, "Illegal constructor");
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    #[test]
    fn modal_bindings_validate_native_receivers_before_author_conversion() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"(() => {
            const throwsType = fn => { try { fn(); return false; } catch(e) { return e instanceof TypeError; } };
            let converted = 0;
            const value = {toString() { ++converted; return 'message'; }};
            for (const name of ['alert', 'confirm', 'prompt']) {
                const d = Object.getOwnPropertyDescriptor(globalThis, name);
                const f = d.value;
                if (!d.writable || !d.enumerable || !d.configurable ||
                    f.name !== name || f.length !== 0 ||
                    !throwsType(() => Reflect.construct(f, [])) ||
                    !throwsType(() => f.call({}, value)) || converted !== 0 ||
                    !throwsType(() => f.call(globalThis, Symbol('message')))) return false;
            }
            return converted === 0;
        })()"#).unwrap();
        assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().is_true());
    }
}

/// One beforeunload event and its legacy return conversion keep their callback
/// cursor until the caller finishes or cancels the navigation admission.
pub struct BeforeUnloadReceipt {
    state: Rc<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    activation: bool,
    notification: Option<native_platform::publication::Notification>,
    event_id: Option<u64>,
    phase: u8,
    result: Option<bool>,
    dispatch_owner: Option<Rc<()>>,
    unload_guard: Option<UnloadGuard>,
    prompt_shown: bool,
}
impl BeforeUnloadReceipt {
    pub fn new(state: &Rc<V8BindingState>, activation: bool) -> Result<Self, V8BindingError> {
        if state.execution_retired() || state.execution_suspended() {
            return Err(V8BindingError::Allocation("inactive beforeunload owner"));
        }
        // A replacement stream may admit a new navigation while an older
        // receipt is still retained by its caller. Only its own token may
        // release the new dispatch guard.
        let identity = (
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        let stale = state.before_unload_owner.borrow().as_ref().is_some_and(
            |(_, generation, stream, authority)| (*generation, *stream, *authority) != identity,
        );
        if stale {
            state.before_unload_owner.borrow_mut().take();
            state.before_unload_dispatching.set(false);
        }
        let nested = state.before_unload_dispatching.replace(true);
        let dispatch_owner = (!nested).then(|| Rc::new(()));
        if let Some(owner) = dispatch_owner.as_ref() {
            *state.before_unload_owner.borrow_mut() =
                Some((owner.clone(), identity.0, identity.1, identity.2));
        }
        Ok(Self {
            state: state.clone(),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            activation,
            notification: None,
            event_id: None,
            phase: 0,
            result: nested.then_some(true),
            dispatch_owner,
            unload_guard: None,
            prompt_shown: false,
        })
    }
    pub fn owner(&self) -> Rc<V8BindingState> {
        self.state.clone()
    }
    pub fn prompt_shown(&self) -> bool {
        self.prompt_shown
    }
    fn current(&self) -> bool {
        !self.state.execution_retired()
            && self.identity
                == (
                    self.state.document_root(),
                    self.state.document_generation(),
                    self.state.document_stream_epoch(),
                    self.state.document_authority_epoch(),
                )
    }
    pub fn retry_safe(&self) -> bool {
        self.result.is_none() && self.current()
    }
    fn complete(&mut self, result: bool) {
        if let Some(notification) = self.notification.take() {
            notification.cancel(&self.state);
        }
        if let Some(owner) = self.dispatch_owner.take() {
            let matches = self
                .state
                .before_unload_owner
                .borrow()
                .as_ref()
                .is_some_and(|(current, _, _, _)| Rc::ptr_eq(current, &owner));
            if matches {
                self.state.before_unload_owner.borrow_mut().take();
                self.state.before_unload_dispatching.set(false);
            }
        }
        self.unload_guard.take();
        self.result = Some(result);
    }
    pub fn cancel(&mut self) {
        if self.result.is_none() {
            self.complete(false)
        }
    }
    pub fn poll(&mut self, scope: &mut v8::PinScope) -> Result<Option<bool>, V8BindingError> {
        if self.result.is_some() {
            return Ok(self.result);
        }
        if !self.current() {
            self.complete(false);
            return Ok(self.result);
        }
        if self.state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(None);
        }
        let state = self.state.clone();
        if self.unload_guard.is_none() {
            self.unload_guard = Some(state.begin_unload());
        }
        if self.phase == 0 {
            let interface = if generated_interface("BeforeUnloadEvent").is_some() {
                "BeforeUnloadEvent"
            } else {
                "Event"
            };
            let event = create_event_object_with_interface(
                scope,
                &state,
                interface,
                "beforeunload",
                false,
                true,
            )?;
            if let Some(prototype) = state.before_unload_prototype.borrow().as_ref() {
                let prototype = prototype
                    .to_local(scope)
                    .ok_or(V8BindingError::Allocation("beforeunload prototype"))?;
                if event.set_prototype(scope, prototype.into()) != Some(true) {
                    return Err(V8BindingError::Allocation("beforeunload prototype"));
                }
            }
            let id = event_id(scope, event)?;
            {
                let mut events = state.events.borrow_mut();
                let record = events
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("beforeunload event"))?;
                record.is_trusted = true;
                record.before_unload_return_value = Some(String::new());
            }
            self.notification = Some(
                native_platform::publication::pending_platform_dispatch(
                    scope,
                    &state,
                    global_events::GLOBAL_TARGET,
                    "beforeunload",
                    event,
                )
                .for_parser_stream(self.identity.2),
            );
            self.event_id = Some(id);
            self.phase = 1;
        }
        if self.phase == 1 {
            let notification = self.notification.as_mut().unwrap();
            if !notification.resume(scope, &state)? {
                return Ok(None);
            }
            notification.finish(scope, &state);
            self.notification = None;
            self.phase = 2;
        }
        if !self.current() {
            self.complete(false);
            return Ok(self.result);
        }
        if state.execution_suspended() || scope.is_execution_terminating() {
            return Ok(None);
        }
        let cancelled = state
            .events
            .borrow()
            .get(&self.event_id.unwrap())
            .is_some_and(|record| {
                record.default_prevented
                    || record
                        .before_unload_return_value
                        .as_ref()
                        .is_some_and(|value| !value.is_empty())
            });
        if !cancelled
            || !self.activation
            || !state.has_sticky_user_activation()
            || !state.modals_allowed.get()
        {
            self.complete(true);
            return Ok(self.result);
        }
        // A failed prompt transaction is terminal: retrying could show a second
        // prompt after a lost response. Only the event prefix is retryable.
        self.prompt_shown = true;
        let response = state.prompt_controller.show(
            state.realm,
            self.identity.1,
            PromptKind::BeforeUnload,
            String::new(),
            String::new(),
        );
        match response {
            Ok(response) => {
                let allowed = self.current()
                    && !state.execution_suspended()
                    && matches!(response, PromptResponse::Accepted(_));
                self.complete(allowed);
                Ok(self.result)
            }
            Err(error) => {
                self.complete(false);
                Err(V8BindingError::Unsupported {
                    interface: "Window".into(),
                    member: format!("beforeunload prompt: {error:?}"),
                })
            }
        }
    }
}
impl Drop for BeforeUnloadReceipt {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn dispatch_before_unload(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    has_sticky_activation: bool,
) -> Result<bool, V8BindingError> {
    let mut receipt = BeforeUnloadReceipt::new(state, has_sticky_activation)?;
    // This synchronous compatibility caller cannot retain a parked operation.
    // Deny admission and cancel it; resumable host callers keep the receipt.
    Ok(receipt.poll(scope)?.unwrap_or(false))
}

// HTML simple-dialog eligibility uses the actual top-level Window, never the
// replaceable author-facing `top` property. Opaque origins compare by native
// identity, so a sandboxed document cannot gain permission by spoofing a URL.
fn simple_dialogs_allowed(scope: &mut v8::PinScope, state: &V8BindingState) -> bool {
    if !state.modals_allowed() {
        return false;
    }
    let Some(window) = state.global_object.to_local(scope) else {
        return false;
    };
    let top = window_document::native_top(scope, window);
    if top.strict_equals(window.into()) {
        return true;
    }
    let Ok(Some(_)) = native_window_identity(scope, top.into()) else {
        return false;
    };
    let Some(owner_context) = window.get_creation_context(scope) else {
        return false;
    };
    let Some(top_context) = top.get_creation_context(scope) else {
        return false;
    };
    let Some(owner) = owner_context.get_slot::<crate::realm::RealmDescriptor>() else {
        return false;
    };
    let Some(top) = top_context.get_slot::<crate::realm::RealmDescriptor>() else {
        return false;
    };
    owner.origin == top.origin
}

fn dialog(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "prompt requires a Window receiver");
        return;
    };
    if state.binding_global != BindingGlobal::Window || !global_events::is_global(scope, &state, args.this()) {
        throw_type_error(scope, "prompt requires a Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, &state, args.this()) { return; }
    if state.execution_retired() {
        throw_type_error(scope, "prompt owner is retired");
        return;
    }
    let identity = (state.document_generation(), state.document_stream_epoch(), state.document_authority_epoch());
    let kind = match args.data().int32_value(scope) {
        Some(0) => PromptKind::Alert,
        Some(1) => PromptKind::Confirm,
        _ => PromptKind::Prompt,
    };
    // Convert before publishing; conversion may execute author code/reenter dialogs.
    let text = if args.length() == 0 {
        String::new()
    } else {
        let Some(value) = args.get(0).to_string(scope) else {
            return;
        };
        match rust_string_preserving_utf16(scope, value) {
            Ok(value) => value,
            Err(error) => {
                throw_binding_error(scope, &error);
                return;
            }
        }
    };
    let default = if kind == PromptKind::Prompt && args.length() > 1 && !args.get(1).is_undefined()
    {
        let Some(value) = args.get(1).to_string(scope) else {
            return;
        };
        match rust_string_preserving_utf16(scope, value) {
            Ok(value) => value,
            Err(error) => {
                throw_binding_error(scope, &error);
                return;
            }
        }
    } else {
        String::new()
    };
    if state.execution_retired() || identity != (state.document_generation(), state.document_stream_epoch(), state.document_authority_epoch()) {
        throw_type_error(scope, "prompt owner changed during conversion");
        return;
    }
    if !simple_dialogs_allowed(scope, &state) {
        match kind {
            PromptKind::Confirm => rv.set(v8::Boolean::new(scope, false).into()),
            PromptKind::Prompt => rv.set(v8::null(scope).into()),
            _ => {}
        }
        return;
    }
    let response = state.prompt_controller.show(
        state.realm,
        state.document_generation.get(),
        kind,
        text,
        default,
    );
    match response {
        Ok(response) => match kind {
            PromptKind::Alert => {}
            PromptKind::Confirm => rv.set(
                v8::Boolean::new(scope, matches!(response, PromptResponse::Accepted(_))).into(),
            ),
            PromptKind::Prompt => match response {
                PromptResponse::Accepted(text) => match v8_string(scope, &text) {
                    Ok(value) => rv.set(value),
                    Err(error) => throw_binding_error(scope, &error),
                },
                PromptResponse::Dismissed => rv.set(v8::null(scope).into()),
            },
            PromptKind::BeforeUnload => {}
        },
        Err(error) => throw_type_error(scope, &format!("native prompt failed: {error:?}")),
    }
}

pub(super) fn return_value_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "BeforeUnloadEvent receiver is invalid");
        return;
    };
    let value = event_id(scope, args.this()).ok().and_then(|id| {
        state
            .events
            .borrow()
            .get(&id)
            .and_then(|record| record.before_unload_return_value.clone())
    });
    match value {
        Some(value) => match v8_string(scope, &value) {
            Ok(value) => rv.set(value),
            Err(error) => throw_binding_error(scope, &error),
        },
        None => throw_type_error(scope, "BeforeUnloadEvent receiver is invalid"),
    }
}
pub(super) fn return_value_set(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "BeforeUnloadEvent receiver is invalid");
        return;
    };
    let Some(id) = event_id(scope, args.this()).ok().filter(|id| {
        state
            .events
            .borrow()
            .get(id)
            .is_some_and(|record| record.before_unload_return_value.is_some())
    }) else {
        throw_type_error(scope, "BeforeUnloadEvent receiver is invalid");
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    match rust_string_preserving_utf16(scope, value) {
        Ok(value) => {
            if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                record.before_unload_return_value = Some(value);
            }
        }
        Err(error) => throw_binding_error(scope, &error),
    }
}
