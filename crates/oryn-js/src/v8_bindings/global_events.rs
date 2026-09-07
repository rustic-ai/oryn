//! Isolated prototype: native global event dispatch and callback reporting.
use super::*;
pub(super) const GLOBAL_TARGET: u64 = 0;

pub(super) fn is_global(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> bool {
    state
        .global_object
        .to_local(scope)
        .is_some_and(|global| global.strict_equals(object.into()))
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let constructor = global_property(scope, "EventTarget")?
        .to_object(scope)
        .unwrap();
    let key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, key.into())
        .unwrap()
        .to_object(scope)
        .unwrap();
    for name in ["addEventListener", "removeEventListener", "dispatchEvent"] {
        let key = v8::String::new(scope, name).unwrap();
        let method = prototype.get(scope, key.into()).unwrap();
        context.global(scope).set(scope, key.into(), method);
    }
    let getter = v8::Function::new(scope, error_handler_get).unwrap();
    let setter = v8::Function::new(scope, error_handler_set).unwrap();
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
    descriptor.set_configurable(true);
    descriptor.set_enumerable(true);
    let name = v8::String::new(scope, "onerror").unwrap();
    context
        .global(scope)
        .define_property(scope, name.into(), &descriptor);
    Ok(())
}
fn error_handler_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        return;
    };
    if !is_global(scope, &state, args.this()) {
        throw_type_error(scope, "global event handler receiver is invalid");
        return;
    }
    let handler = state
        .platform_listeners
        .borrow()
        .get(&(GLOBAL_TARGET, "error".into()))
        .and_then(|list| list.iter().find(|entry| entry.global_handler))
        .map(|entry| entry.callback.clone());
    rv.set(handler.as_ref().map_or_else(
        || v8::null(scope).into(),
        |handler| v8::Local::new(scope, handler),
    ));
}
fn error_handler_set(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        return;
    };
    if !is_global(scope, &state, args.this()) {
        throw_type_error(scope, "global event handler receiver is invalid");
        return;
    }
    let value = args.get(0);
    let mut listeners = state.platform_listeners.borrow_mut();
    let list = listeners
        .entry((GLOBAL_TARGET, "error".into()))
        .or_default();
    if !value.is_object() {
        list.retain(|entry| !entry.global_handler);
        return;
    }
    if let Some(handler) = list.iter_mut().find(|entry| entry.global_handler) {
        handler.callback = v8::Global::new(scope, value);
        return;
    }
    let registration_id = state.next_event_listener_id.get();
    state.next_event_listener_id.set(registration_id + 1);
    list.push(PlatformEventListener {
        registration_id,
        global_handler: true,
        callback: v8::Global::new(scope, value),
        capture: false,
        once: false,
        passive: false,
        signal_id: None,
    });
}
pub(super) fn dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
) -> Result<bool, V8BindingError> {
    let event_state = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("event owner"))?;
    let id = event_id(scope, object)?;
    let _guard = begin_event_dispatch(&event_state, id)?;
    {
        let mut events = event_state.events.borrow_mut();
        let event = events
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("event"))?;
        event.target = None;
        event.current_target = None;
        event.platform_target = Some(GLOBAL_TARGET);
    }
    let target = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("global target"))?;
    object.set_internal_field(4, target.into());
    phase(scope, state, object, true, 2)?;
    if !event_state
        .events
        .borrow()
        .get(&id)
        .unwrap()
        .immediate_stopped
    {
        phase(scope, state, object, false, 2)?;
    }
    Ok(!event_state
        .events
        .borrow()
        .get(&id)
        .unwrap()
        .default_prevented)
}
pub(super) fn phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    capture: bool,
    phase: u16,
) -> Result<(), V8BindingError> {
    let event_state = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("event owner"))?;
    let id = event_id(scope, object)?;
    let event_type = {
        let mut events = event_state.events.borrow_mut();
        let event = events
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("event"))?;
        event.current_target = None;
        event.platform_current_target = Some(GLOBAL_TARGET);
        event.phase = phase;
        event.event_type.clone()
    };
    let target = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("global"))?;
    object.set_internal_field(5, target.into());
    let key = (GLOBAL_TARGET, event_type.clone());
    let snapshot = state
        .platform_listeners
        .borrow()
        .get(&key)
        .cloned()
        .unwrap_or_default();
    for listener in snapshot {
        if scope.is_execution_terminating()
            || event_state
                .events
                .borrow()
                .get(&id)
                .unwrap()
                .immediate_stopped
        {
            break;
        }
        if listener.capture != capture {
            continue;
        }
        let current = state
            .platform_listeners
            .borrow()
            .get(&key)
            .and_then(|list| {
                list.iter()
                    .find(|entry| entry.registration_id == listener.registration_id)
            })
            .cloned();
        let Some(current) = current else {
            continue;
        };
        let callback = v8::Local::new(scope, &current.callback);
        if listener.signal_id.is_some_and(|signal| matches!(platform_object(state, signal).as_deref(), Ok(PlatformObjectRecord::AbortSignal(signal)) if signal.aborted)) { continue; }
        if listener.once
            && let Some(list) = state.platform_listeners.borrow_mut().get_mut(&key)
        {
            list.retain(|entry| entry.registration_id != listener.registration_id);
        }
        event_state
            .events
            .borrow_mut()
            .get_mut(&id)
            .unwrap()
            .in_passive_listener = listener.passive;
        if listener.global_handler {
            if callback.is_function() {
                let is_error = event_state
                    .events
                    .borrow()
                    .get(&id)
                    .unwrap()
                    .platform_target
                    == Some(GLOBAL_TARGET)
                    && error_event_property(scope, &event_state, object, "error").is_ok();
                let arguments = if is_error {
                    ["message", "filename", "lineno", "colno", "error"]
                        .into_iter()
                        .map(|name| error_event_property(scope, &event_state, object, name))
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    vec![object.into()]
                };
                if let Some(value) = invoke(scope, callback, target.into(), &arguments)
                    && ((is_error && value.is_true()) || (!is_error && value.is_false()))
                {
                    let mut events = event_state.events.borrow_mut();
                    let event = events.get_mut(&id).unwrap();
                    if event.cancelable {
                        event.default_prevented = true;
                    }
                }
            }
        } else {
            invoke(scope, callback, target.into(), &[object.into()]);
        }

        event_state
            .events
            .borrow_mut()
            .get_mut(&id)
            .unwrap()
            .in_passive_listener = false;
    }
    if event_type != "error"
        && !capture
        && !scope.is_execution_terminating()
        && !event_state
            .events
            .borrow()
            .get(&id)
            .unwrap()
            .immediate_stopped
    {
        let property = v8::String::new(scope, &format!("on{event_type}")).unwrap();
        if let Some(handler) = target
            .get(scope, property.into())
            .filter(|value| value.is_function())
        {
            let error_event = event_type == "error"
                && error_event_property(scope, &event_state, object, "error").is_ok();
            let arguments = if error_event {
                ["message", "filename", "lineno", "colno", "error"]
                    .into_iter()
                    .map(|name| error_event_property(scope, &event_state, object, name))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![object.into()]
            };
            if let Some(value) = invoke(scope, handler, target.into(), &arguments)
                && ((error_event && value.is_true()) || (!error_event && value.is_false()))
            {
                let mut events = event_state.events.borrow_mut();
                let event = events.get_mut(&id).unwrap();
                if event.cancelable {
                    event.default_prevented = true;
                }
            }
        }
    }
    if let Some(event) = event_state.events.borrow_mut().get_mut(&id) {
        event.platform_current_target = None;
        event.phase = 0;
    }
    let empty = v8::undefined(scope);
    object.set_internal_field(5, empty.into());
    Ok(())
}
pub(super) fn invoke<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: v8::Local<'s, v8::Value>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let mut context = listener.to_object(scope)?.get_creation_context(scope)?;
    let (result, exception, message) = {
        v8::tc_scope!(let tc, scope);
        let (function, receiver) =
            if let Ok(function) = v8::Local::<v8::Function>::try_from(listener) {
                (Some(function), receiver)
            } else {
                let object = listener.to_object(tc)?;
                let key = v8::String::new(tc, "handleEvent").unwrap();
                let function = object
                    .get(tc, key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
                (function, object.into())
            };
        if let Some(function) = function
            && let Some(creation) = function.get_creation_context(tc)
        {
            context = creation;
        }
        if function.is_none() && !tc.has_caught() {
            throw_type_error(tc, "EventListener.handleEvent is not callable");
        }
        let result = function.and_then(|function| function.call(tc, receiver, arguments));
        let exception = tc.exception();
        let message = tc.message();
        if !tc.is_execution_terminating() {
            tc.reset();
        }
        (result, exception, message)
    };
    if let Some(exception) = exception
        && !scope.is_execution_terminating()
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        let _ = report_exception(scope, exception, message);
    }
    result
}
pub(super) fn report_exception(
    scope: &mut v8::PinScope,
    exception: v8::Local<v8::Value>,
    message: Option<v8::Local<v8::Message>>,
) -> Result<bool, V8BindingError> {
    if scope.is_execution_terminating() {
        return Ok(false);
    }
    let state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("exception realm"))?;
    if state.reporting_exception.replace(true) {
        return Ok(false);
    }
    struct Guard<'a>(&'a Cell<bool>);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.set(false);
        }
    }
    let _guard = Guard(&state.reporting_exception);
    let message = message.unwrap_or_else(|| v8::Exception::create_message(scope, exception));
    let text = message.get(scope);
    let filename = message
        .get_script_resource_name(scope)
        .filter(|v| !v.is_undefined())
        .and_then(|v| v.to_string(scope))
        .unwrap_or_else(|| v8::String::new(scope, "").unwrap());
    let line = message.get_line_number(scope).unwrap_or(0) as u32;
    let column = if line == 0 {
        0
    } else {
        message.get_start_column() as u32 + 1
    };
    let event =
        create_event_object_with_interface(scope, &state, "ErrorEvent", "error", false, true)?;
    let data = v8::Array::new(scope, ERROR_EVENT_FIELDS.len() as i32);
    data.set_index(scope, 0, text.into());
    data.set_index(scope, 1, filename.into());
    let line = v8::Integer::new_from_unsigned(scope, line);
    data.set_index(scope, 2, line.into());
    let column = v8::Integer::new_from_unsigned(scope, column);
    data.set_index(scope, 3, column.into());
    data.set_index(scope, 4, exception);
    let brand = {
        let mut brand = state.error_event_brand.borrow_mut();
        if brand.is_none() {
            let symbol = v8::Symbol::new(scope, None);
            *brand = Some(v8::Global::new(scope, symbol));
        }
        v8::Local::new(scope, brand.as_ref().unwrap())
    };
    event.set_internal_field(2, data.into());
    event.set_internal_field(3, brand.into());
    let id = event_id(scope, event)?;
    state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
    dispatch(scope, &state, event).map(|accepted| !accepted)
}
