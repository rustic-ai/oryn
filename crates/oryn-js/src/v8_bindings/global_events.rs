//! Native global event dispatch and callback reporting.
type EventCallback<'s> = (v8::Local<'s, v8::Function>, v8::Local<'s, v8::Value>);

use super::*;
pub(super) const GLOBAL_TARGET: u64 = 0;
const WINDOW_HANDLER_TYPES: &[&str] = &[
    "move",
    "securitypolicyviolation",
    "error",
    "message",
    "messageerror",
    "load",
    "pageshow",
    "pagehide",
    "pageswap",
    "pagereveal",
    "popstate",
    "hashchange",
    "storage",
    "beforeunload",
    "unload",
    "beforeprint",
    "afterprint",
    "dragstart",
    "drag",
    "dragenter",
    "dragleave",
    "dragover",
    "drop",
    "dragend",
    "copy",
    "cut",
    "paste",
    "animationstart",
    "animationend",
    "animationiteration",
    "animationcancel",
    "transitionrun",
    "transitionstart",
    "transitionend",
    "transitioncancel",
    "webkitAnimationStart",
    "webkitAnimationEnd",
    "webkitAnimationIteration",
    "webkitTransitionEnd",
];
const WORKER_HANDLER_TYPES: &[&str] = &[
    "securitypolicyviolation",
    "error",
    "message",
    "messageerror",
    "connect",
    "unhandledrejection",
    "rejectionhandled",
    "install",
    "activate",
    "fetch",
    "push",
    "sync",
];

fn native_handler_types(global: BindingGlobal) -> &'static [&'static str] {
    if global == BindingGlobal::Worker {
        WORKER_HANDLER_TYPES
    } else {
        WINDOW_HANDLER_TYPES
    }
}

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
pub(super) fn check_window_access(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    receiver: v8::Local<v8::Object>,
) -> bool {
    if state.binding_global == BindingGlobal::Worker
        || crate::v8_context_bridge::can_access_window(scope, receiver)
    {
        return true;
    }
    throw_binding_error(
        scope,
        &V8BindingError::Unsupported {
            interface: "Window".into(),
            member: "SecurityError".into(),
        },
    );
    false
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
    // Window inherits these operations through its native prototype chain.
    // Copying them onto the global changes Web IDL placement and shadows
    // later prototype updates. Keep the separate Worker bootstrap unchanged.
    if context
        .get_slot::<V8BindingState>()
        .is_some_and(|state| state.binding_global == BindingGlobal::Worker)
    {
        for name in ["addEventListener", "removeEventListener", "dispatchEvent"] {
            let key = v8::String::new(scope, name).unwrap();
            let method = prototype.get(scope, key.into()).unwrap();
            context.global(scope).set(scope, key.into(), method);
        }
    }
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("global handler owner"))?;
    let types = native_handler_types(state.binding_global);
    for event_type in types {
        let data = v8::String::new(scope, event_type).unwrap();
        let getter = v8::Function::builder(global_handler_get)
            .data(data.into())
            .build(scope)
            .unwrap();
        let setter = v8::Function::builder(global_handler_set)
            .data(data.into())
            .build(scope)
            .unwrap();
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
        descriptor.set_configurable(true);
        descriptor.set_enumerable(true);
        let name = v8::String::new(scope, &web_animations::handler_property(event_type)).unwrap();
        context
            .global(scope)
            .define_property(scope, name.into(), &descriptor);
    }
    Ok(())
}
fn global_handler_get(
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
    if !check_window_access(scope, &state, args.this()) {
        return;
    }
    let event_type = args.data().to_rust_string_lossy(scope);
    rv.set(handler_value(scope, &state, &event_type));
}

pub(super) fn handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    event_type: &str,
) -> v8::Local<'s, v8::Value> {
    let source = state.global_handler_sources.borrow_mut().remove(event_type);
    if let Some(source) = source {
        return compile_content_handler(scope, state, event_type, &source);
    }
    let handler = state
        .platform_listeners
        .borrow()
        .get(&(GLOBAL_TARGET, event_type.to_owned()))
        .and_then(|list| list.iter().find(|entry| entry.global_handler))
        .map(|entry| entry.callback.clone());
    match handler {
        Some(handler) => handler.value(scope),
        None => v8::null(scope).into(),
    }
}

fn compile_content_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    event_type: &str,
    source: &str,
) -> v8::Local<'s, v8::Value> {
    let Some(context) = state
        .global_object
        .to_local(scope)
        .and_then(|global| global.get_creation_context(scope))
    else {
        return v8::null(scope).into();
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let (function, exception, message) = {
        v8::tc_scope!(let tc, scope);
        let arguments: &[&str] = if event_type == "error" {
            &["event", "source", "lineno", "colno", "error"]
        } else {
            &["event"]
        };
        let function = compile_inline_event_handler_with_arguments(tc, source, arguments).ok();
        let exception = tc.exception();
        let message = tc.message();
        if !tc.is_execution_terminating() {
            tc.reset();
        }
        (function, exception, message)
    };
    let value: v8::Local<v8::Value> = match function {
        Some(function) => function.into(),
        None => v8::null(scope).into(),
    };
    // Compilation cannot call author code. Store the result before reporting
    // an error: an error handler can replace/remove this slot reentrantly.
    if let Some(handler) = state
        .platform_listeners
        .borrow_mut()
        .get_mut(&(GLOBAL_TARGET, event_type.to_owned()))
        .and_then(|list| list.iter_mut().find(|entry| entry.global_handler))
    {
        handler.callback = platform_listener_graph::Callback::new(scope, value);
    }
    if let Err(error) = platform_listener_graph::refresh(scope, state, GLOBAL_TARGET) {
        throw_binding_error(scope, &error);
        return v8::null(scope).into();
    }
    if let Some(exception) = exception
        && !scope.is_execution_terminating()
    {
        let _ = report_exception(scope, exception, message);
    }
    value
}

pub(super) fn set_content_handler(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    event_type: &str,
    source: Option<&str>,
) {
    let value = match source {
        Some(_) => v8::Object::new(scope).into(),
        None => v8::null(scope).into(),
    };
    set_handler_value(scope, state, event_type, value);
    if let Some(source) = source {
        state
            .global_handler_sources
            .borrow_mut()
            .insert(event_type.to_owned(), source.to_owned());
    }
}

fn global_handler_set(
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
    if !check_window_access(scope, &state, args.this()) {
        return;
    }
    let event_type = args.data().to_rust_string_lossy(scope);
    let value = args.get(0);
    set_handler_value(scope, &state, &event_type, value);
}

pub(super) fn set_handler_value(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    event_type: &str,
    value: v8::Local<v8::Value>,
) {
    state.global_handler_sources.borrow_mut().remove(event_type);
    {
        let mut listeners = state.platform_listeners.borrow_mut();
        let list = listeners
            .entry((GLOBAL_TARGET, event_type.into()))
            .or_default();
        if !value.is_object() {
            list.retain(|entry| !entry.global_handler);
        } else if let Some(handler) = list.iter_mut().find(|entry| entry.global_handler) {
            handler.callback = platform_listener_graph::Callback::new(scope, value);
        } else {
            let registration_id = state.next_event_listener_id.get();
            state.next_event_listener_id.set(registration_id + 1);
            list.push(PlatformEventListener {
                registration_id,
                global_handler: true,
                callback: platform_listener_graph::Callback::new(scope, value),
                capture: false,
                once: false,
                passive: false,
                signal: None,
            });
        }
    }
    if let Err(error) = platform_listener_graph::refresh(scope, state, GLOBAL_TARGET) {
        throw_binding_error(scope, &error);
    }
}
pub(super) fn dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
) -> Result<bool, V8BindingError> {
    dispatch_with_document_target(scope, state, object, None)
}

pub(super) fn dispatch_with_document_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    document_target: Option<NodeId>,
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
        event.target = document_target;
        event.current_target = None;
        event.platform_target = document_target.is_none().then_some(GLOBAL_TARGET);
    }
    let target = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("global target"))?;
    let exposed_target: v8::Local<v8::Value> = if let Some(document) = document_target {
        wrap_node(scope, state, document)?.into()
    } else {
        target.into()
    };
    object.set_internal_field(4, exposed_target.into());
    set_event_path(scope, object, &[target.into()])?;
    let result = (|| {
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
    })();
    clear_event_path(scope, object);
    result
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
    let event_type =
        web_animations::invocation_type(scope, state, target, &event_state, id, &event_type)?;
    let _event_type = web_animations::InvocationTypeGuard::new(&event_state, id, &event_type);
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
        let callback = if current.global_handler {
            handler_value(scope, state, &event_type)
        } else {
            current.callback.value(scope)
        };
        if listener
            .signal
            .as_ref()
            .is_some_and(node_event_listeners::aborted)
        {
            continue;
        }
        if listener.once
            && let Some(list) = state.platform_listeners.borrow_mut().get_mut(&key)
        {
            list.retain(|entry| entry.registration_id != listener.registration_id);
        }
        if listener.once {
            platform_listener_graph::refresh(scope, state, GLOBAL_TARGET)?;
        }
        event_state
            .events
            .borrow_mut()
            .get_mut(&id)
            .unwrap()
            .in_passive_listener = listener.passive;
        if listener.global_handler {
            if callback.is_function() {
                let is_error = event_type == "error"
                    && event_state
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
                if let Some(value) = invoke(scope, callback, target.into(), &arguments) {
                    if event_type == "beforeunload" && !value.is_null_or_undefined() {
                        // HTML's special BeforeUnloadEvent handler converts its
                        // non-null return to DOMString before touching event state.
                        let (converted, exception, message) = {
                            v8::tc_scope!(let tc,scope);
                            let converted = value
                                .to_string(tc)
                                .and_then(|value| rust_string_preserving_utf16(tc, value).ok());
                            let exception = tc.exception();
                            let message = tc.message();
                            if !tc.is_execution_terminating() {
                                tc.reset();
                            }
                            (converted, exception, message)
                        };
                        if let Some(exception) = exception
                            && !scope.is_execution_terminating()
                        {
                            let _ = report_exception(scope, exception, message);
                        }
                        if let Some(value) = converted {
                            let mut events = event_state.events.borrow_mut();
                            if let Some(event) = events.get_mut(&id)
                                && let Some(old) = event.before_unload_return_value.as_mut()
                            {
                                if old.is_empty() {
                                    *old = value;
                                }
                                event.default_prevented = true;
                            }
                        }
                    } else if (is_error && value.is_true())
                        || (event_type != "beforeunload" && !is_error && value.is_false())
                    {
                        let mut events = event_state.events.borrow_mut();
                        let event = events.get_mut(&id).unwrap();
                        if event.cancelable {
                            event.default_prevented = true;
                        }
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
    if !native_handler_types(state.binding_global).contains(&event_type.as_str())
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
/// Resolve EventListener's author handleEvent getter separately from calling
/// its method, allowing native retained dispatch to park between those steps.
#[cfg(test)]
pub(super) fn resolve_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: v8::Local<'s, v8::Value>,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<Option<EventCallback<'s>>, V8BindingError> {
    resolve_listener_capturing(scope, listener, receiver).map(|(callback, _)| callback)
}

/// Callback-interface resolution can itself throw before invocation. Retain
/// that event-dispatch outcome separately from reporting the exception so IDB
/// request notification can run its specified abort algorithm afterward.
pub(super) fn resolve_listener_capturing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: v8::Local<'s, v8::Value>,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<(Option<EventCallback<'s>>, bool), V8BindingError> {
    if let Ok(function) = v8::Local::<v8::Function>::try_from(listener) {
        return Ok((Some((function, receiver)), false));
    }
    let object = listener
        .to_object(scope)
        .ok_or(V8BindingError::Allocation("listener object"))?;
    let context = object
        .get_creation_context(scope)
        .ok_or(V8BindingError::Allocation("listener context"))?;
    let (result, exception, message) = {
        v8::tc_scope!(let tc,scope);
        let key = v8::String::new(tc, "handleEvent")
            .ok_or(V8BindingError::Allocation("listener method name"))?;
        let function = object
            .get(tc, key.into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        if function.is_none() && !tc.has_caught() && !tc.is_execution_terminating() {
            throw_type_error(tc, "EventListener.handleEvent is not callable");
        }
        let exception = tc.exception();
        let message = tc.message();
        if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
        } else {
            tc.reset();
        }
        (function, exception, message)
    };
    let threw = exception.is_some() && !scope.is_execution_terminating();
    if let Some(exception) = exception
        && !scope.is_execution_terminating()
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        let _ = report_exception(scope, exception, message);
    }
    Ok((
        result
            .filter(|_| !scope.is_execution_terminating())
            .map(|function| (function, object.into())),
        threw,
    ))
}

pub(super) fn invoke<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: v8::Local<'s, v8::Value>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let mut context = listener.to_object(scope)?.get_creation_context(scope)?;
    if let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() {
        state.commit_parser_author_effect();
    }
    if let Some(state) = context.get_slot::<V8BindingState>() {
        state.commit_parser_author_effect();
    }
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
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        if function.is_none() && !tc.has_caught() && !tc.is_execution_terminating() {
            throw_type_error(tc, "EventListener.handleEvent is not callable");
        }
        let result = if tc.has_terminated() || tc.is_execution_terminating() {
            None
        } else {
            function.and_then(|function| function.call(tc, receiver, arguments))
        };
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        let result = result.filter(|_| !tc.has_terminated() && !tc.is_execution_terminating());
        let exception = tc.exception();
        let message = tc.message();
        // Propagate the materialized termination exception. Merely requesting
        // another interrupt here would poison the next callback after recovery.
        if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
        } else {
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
pub fn report_exception(
    scope: &mut v8::PinScope,
    exception: v8::Local<v8::Value>,
    message: Option<v8::Local<v8::Message>>,
) -> Result<bool, V8BindingError> {
    report_script_exception(scope, exception, message, false)
}

/// Report an exception, sanitizing opaque classic-script errors before event delivery.
#[path = "global_error_reporting.rs"]
mod retained_reports;
#[cfg(test)]
pub(super) use retained_reports::retry_now as retry_reports_now;
pub(super) use retained_reports::{
    Registry as ReportRegistry, delay as report_delay, has_pending as has_pending_reports,
    poll as poll_reports, set_location as set_exception_location,
};

pub fn report_script_exception(
    scope: &mut v8::PinScope,
    exception: v8::Local<v8::Value>,
    message: Option<v8::Local<v8::Message>>,
    muted: bool,
) -> Result<bool, V8BindingError> {
    retained_reports::report(scope, exception, message, muted)
}

fn forward_worker_error(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    exception: v8::Local<v8::Value>,
    message: Option<v8::Local<v8::Message>>,
    muted: bool,
) -> Result<(), V8BindingError> {
    let Some(reporter) = state.worker_error_reporter.borrow().clone() else {
        return Ok(());
    };
    let message = message.unwrap_or_else(|| v8::Exception::create_message(scope, exception));
    let (text, filename, line, column) = if muted {
        (
            "Script error.".encode_utf16().collect(),
            String::new(),
            0,
            0,
        )
    } else {
        let text = message.get(scope);
        let mut units = vec![0; text.length()];
        text.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
        let filename = message
            .get_script_resource_name(scope)
            .filter(|value| !value.is_undefined())
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        let line = message.get_line_number(scope).unwrap_or(0) as u32;
        let column = if line == 0 {
            0
        } else {
            message.get_start_column() as u32 + 1
        };
        (units, filename, line, column)
    };
    reporter(crate::worker_wire::AgentEvent::Error {
        message: text,
        filename,
        line,
        column,
    })
    .map_err(|_| V8BindingError::Allocation("worker error publication"))
}

#[cfg(test)]
mod listener_termination_tests {
    use super::*;

    #[test]
    fn native_listener_calls_preserve_termination_and_report_ordinary_errors() {
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
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let halt = v8::Function::new(scope, halt).unwrap();
        let key = v8::String::new(scope, "halt").unwrap();
        context.global(scope).set(scope, key.into(), halt.into());
        super::super::test_eval::eval(
            scope,
            "globalThis.calls=0;globalThis.errors=0;addEventListener('error',e=>{errors++;e.preventDefault()})",
        );
        // Invoke from native depth zero, rather than under Script::run where
        // V8 automatically forwards termination when unwinding a TryCatch.
        for (source, resolve) in [
            ("(()=>{calls++;halt()})", false),
            ("({get handleEvent(){calls++;halt()}})", false),
            ("({get handleEvent(){calls++;halt()}})", true),
        ] {
            let listener = super::super::test_eval::eval(scope, source);
            let receiver = context.global(scope).into();
            if resolve {
                assert!(
                    resolve_listener(scope, listener, receiver)
                        .unwrap()
                        .is_none()
                );
            } else {
                assert!(invoke(scope, listener, receiver, &[]).is_none());
            }
            assert!(scope.is_execution_terminating());
            scope.cancel_terminate_execution();
            assert!(
                super::super::test_eval::eval(
                    scope,
                    "globalThis.resumptions=(globalThis.resumptions||0)+1;true"
                )
                .is_true(),
                "first unrelated evaluation after cancellation must execute"
            );
        }
        assert!(
            super::super::test_eval::eval(scope, "calls===3&&errors===0&&resumptions===3")
                .is_true()
        );
        for (source, resolve) in [
            ("(()=>{throw Error('callback')})", false),
            ("({get handleEvent(){throw Error('getter')}})", false),
            (
                "({get handleEvent(){throw Error('resolved getter')}})",
                true,
            ),
        ] {
            let listener = super::super::test_eval::eval(scope, source);
            let receiver = context.global(scope).into();
            if resolve {
                assert!(
                    resolve_listener(scope, listener, receiver)
                        .unwrap()
                        .is_none()
                );
            } else {
                assert!(invoke(scope, listener, receiver, &[]).is_none());
            }
            assert!(!scope.is_execution_terminating());
        }
        assert!(super::super::test_eval::eval(scope, "errors===3&&calls===3").is_true());
    }
}
