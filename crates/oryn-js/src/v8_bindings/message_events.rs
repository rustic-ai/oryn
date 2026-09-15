//! MessageEvent payloads are traced by V8, including foreign Window/port edges.
use super::*;

const FIELDS: [&str; 5] = ["data", "origin", "lastEventId", "source", "ports"];

fn string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
    usv: bool,
) -> Option<v8::Local<'s, v8::String>> {
    let value = if value.is_undefined() {
        v8::String::new(scope, "")?
    } else {
        value.to_string(scope)?
    };
    if usv {
        let text = value.to_rust_string_lossy(scope);
        v8::String::new(scope, &text)
    } else {
        Some(value)
    }
}

fn is_port(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    if object.internal_field_count() == 0
        || !object
            .get_internal_field(scope, 0)
            .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
            .is_some_and(|v| v.is_number())
    {
        return false;
    }
    let Some(state) = binding_state_for_receiver(scope, object) else {
        return false;
    };
    let Ok(id) = stateful_id(scope, object) else {
        return false;
    };
    let same_wrapper = state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|w| w.to_local(scope))
        .is_some_and(|w| w.strict_equals(object.into()));
    same_wrapper
        && matches!(
            state.platform_objects.borrow().get(&id),
            Some(PlatformObjectRecord::MessagePort(_) | PlatformObjectRecord::DetachedMessagePort)
        )
}

fn source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    if value.is_null_or_undefined() {
        return Some(v8::null(scope).into());
    }
    if is_port(scope, value) {
        return Some(value);
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(context) = object.get_creation_context(scope)
        && context.global(scope).strict_equals(value)
        && context
            .get_slot::<V8BindingState>()
            .is_some_and(|state| state.binding_global == BindingGlobal::Window)
    {
        return Some(value);
    }
    // ServiceWorker has no native wrapper yet. Author-defined lookalikes are
    // not platform objects and must not pass the MessageEventSource union.
    throw_type_error(
        scope,
        "MessageEvent source requires a WindowProxy, MessagePort, or ServiceWorker",
    );
    None
}

fn ports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
) -> Option<v8::Local<'s, v8::Array>> {
    let result = v8::Array::new(scope, 0);
    if !value.is_undefined() {
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            throw_type_error(scope, "MessageEvent ports requires an iterable object");
            return None;
        };
        let key = v8::Symbol::get_iterator(scope);
        let method = object.get(scope, key.into())?;
        let Ok(method) = v8::Local::<v8::Function>::try_from(method) else {
            throw_type_error(scope, "MessageEvent ports requires an iterable object");
            return None;
        };
        let iterator = method.call(scope, object.into(), &[])?;
        let Ok(iterator) = v8::Local::<v8::Object>::try_from(iterator) else {
            throw_type_error(scope, "iterator must return an object");
            return None;
        };
        let key = v8::String::new(scope, "next")?;
        let next = iterator.get(scope, key.into())?;
        let Ok(next) = v8::Local::<v8::Function>::try_from(next) else {
            throw_type_error(scope, "iterator next must be callable");
            return None;
        };
        loop {
            let step = next.call(scope, iterator.into(), &[])?;
            let Ok(step) = v8::Local::<v8::Object>::try_from(step) else {
                throw_type_error(scope, "iterator result must be an object");
                return None;
            };
            let key = v8::String::new(scope, "done")?;
            if step.get(scope, key.into())?.boolean_value(scope) {
                break;
            }
            let key = v8::String::new(scope, "value")?;
            let value = step.get(scope, key.into())?;
            if !is_port(scope, value) {
                // IteratorClose preserves the original conversion exception,
                // including when getting/calling return itself throws.
                {
                    v8::tc_scope!(let tc, scope);
                    let key = v8::String::new(tc, "return")?;
                    if let Some(close) = iterator
                        .get(tc, key.into())
                        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
                    {
                        close.call(tc, iterator.into(), &[]);
                    }
                }
                throw_type_error(scope, "MessageEvent ports contains a non-MessagePort value");
                return None;
            }
            let index = v8::String::new(scope, &result.length().to_string())?;
            result.create_data_property(scope, index.into(), value)?;
        }
    }
    result.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    Some(result)
}

fn payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let expected = state
        .message_event_brand
        .borrow()
        .as_ref()
        .map(|v| v8::Local::new(scope, v));
    let brand = (object.internal_field_count() >= 4)
        .then(|| object.get_internal_field(scope, 3))
        .flatten()
        .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok());
    if !expected
        .zip(brand)
        .is_some_and(|(a, b)| b.strict_equals(a.into()))
    {
        throw_type_error(scope, "MessageEvent receiver required");
        return None;
    }
    object
        .get_internal_field(scope, 2)
        .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
}

pub(super) fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let fields = payload(scope, state, object)?;
    fields.get_index(scope, FIELDS.iter().position(|n| *n == name)? as u32)
}

pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<()> {
    if args.length() == 0 {
        throw_type_error(scope, "MessageEvent constructor requires a type");
        return None;
    }
    let event_type = args.get(0).to_string(scope)?;
    let event_type = rust_string_preserving_utf16(scope, event_type).ok()?;
    let init = args.get(1);
    let init = if init.is_null_or_undefined() {
        None
    } else if init.is_object() {
        Some(init.to_object(scope)?)
    } else {
        throw_type_error(
            scope,
            "MessageEventInit must be an object, null, or undefined",
        );
        return None;
    };
    let bubbles = event_init_member(scope, init, "bubbles")?.boolean_value(scope);
    let cancelable = event_init_member(scope, init, "cancelable")?.boolean_value(scope);
    let composed = event_init_member(scope, init, "composed")?.boolean_value(scope);
    let data = event_init_member(scope, init, "data")?;
    let data = if data.is_undefined() {
        v8::null(scope).into()
    } else {
        data
    };
    let last = event_init_member(scope, init, "lastEventId")?;
    let last = string(scope, last, false)?;
    let origin = event_init_member(scope, init, "origin")?;
    let origin = string(scope, origin, true)?;
    let list = event_init_member(scope, init, "ports")?;
    let list = ports(scope, list)?;
    let sender = event_init_member(scope, init, "source")?;
    let sender = source(scope, sender)?;
    let fields = v8::Array::new_with_elements(
        scope,
        &[data, origin.into(), last.into(), sender, list.into()],
    );
    let id = state.next_event_id.get();
    state.next_event_id.set(id.saturating_add(1));
    state.events.borrow_mut().insert(
        id,
        EventRecord {
            time_stamp: state.performance_now(),
            event_type,
            target: None,
            current_target: None,
            platform_target: None,
            platform_path: Vec::new(),
            platform_current_target: None,
            bubbles,
            cancelable,
            composed,
            is_trusted: false,
            default_prevented: false,
            propagation_stopped: false,
            immediate_stopped: false,
            in_passive_listener: false,
            dispatching: false,
            initialized: true,
            phase: 0,
        },
    );
    args.this()
        .set_internal_field(0, v8::Number::new(scope, id as f64).into());
    tag_event_object(scope, state, args.this());
    install_fields(scope, state, args.this(), fields);
    Some(())
}

fn install_fields(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    fields: v8::Local<v8::Array>,
) {
    let brand = {
        let mut brand = state.message_event_brand.borrow_mut();
        if brand.is_none() {
            let symbol = v8::Symbol::new(scope, None);
            *brand = Some(v8::Global::new(scope, symbol));
        }
        v8::Local::new(scope, brand.as_ref().unwrap())
    };
    object.set_internal_field(2, fields.into());
    object.set_internal_field(3, brand.into());
}

pub(super) fn delivery<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    data: v8::Local<'s, v8::Value>,
    transferred: &[v8::Local<'s, v8::Object>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let object =
        create_event_object_with_interface(scope, state, "MessageEvent", "message", false, false)?;
    initialize_empty(scope, state, object).ok_or(V8BindingError::Allocation("message payload"))?;
    let fields = object
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("message fields"))?;
    fields.set_index(scope, 0, data);
    let values = transferred
        .iter()
        .map(|port| (*port).into())
        .collect::<Vec<_>>();
    let ports = v8::Array::new_with_elements(scope, &values);
    ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    fields.set_index(scope, 4, ports.into());
    let id = event_id(scope, object)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
    }
    Ok(object)
}

pub(super) fn initialize_empty(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Option<()> {
    let empty = v8::String::new(scope, "")?;
    let null = v8::null(scope);
    let undefined = v8::undefined(scope);
    let list = ports(scope, undefined.into())?;
    let fields = v8::Array::new_with_elements(
        scope,
        &[
            null.into(),
            empty.into(),
            empty.into(),
            null.into(),
            list.into(),
        ],
    );
    install_fields(scope, state, object, fields);
    Some(())
}

pub(super) fn reinitialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<()> {
    payload(scope, state, args.this())?;
    let event_type = args.get(0).to_string(scope)?;
    let event_type = rust_string_preserving_utf16(scope, event_type).ok()?;
    let bubbles = args.get(1).boolean_value(scope);
    let cancelable = args.get(2).boolean_value(scope);
    let data = args.get(3);
    let data = if data.is_undefined() {
        v8::null(scope).into()
    } else {
        data
    };
    let origin = string(scope, args.get(4), true)?;
    let last = string(scope, args.get(5), false)?;
    let sender = source(scope, args.get(6))?;
    let list = ports(scope, args.get(7))?;
    let id = event_id(scope, args.this()).ok()?;
    let mut events = state.events.borrow_mut();
    let event = events.get_mut(&id)?;
    if event.dispatching {
        return Some(());
    }
    let fields = v8::Array::new_with_elements(
        scope,
        &[data, origin.into(), last.into(), sender, list.into()],
    );
    args.this().set_internal_field(2, fields.into());
    let empty = v8::undefined(scope);
    args.this().set_internal_field(4, empty.into());
    args.this().set_internal_field(5, empty.into());
    event.event_type = event_type;
    event.bubbles = bubbles;
    event.cancelable = cancelable;
    event.default_prevented = false;
    event.propagation_stopped = false;
    event.immediate_stopped = false;
    event.is_trusted = false;
    event.target = None;
    event.platform_target = None;
    event.initialized = true;
    Some(())
}
