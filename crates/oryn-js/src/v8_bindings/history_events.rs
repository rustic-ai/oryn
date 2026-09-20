//! Typed history event payloads retained independently of public properties.
use super::*;

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Event".into(),
        member: "TypeError: incompatible history event receiver".into(),
    }
}

pub(super) fn payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface: &str,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let (first, second) = if interface == "HashChangeEvent" {
        let mut read = |name| -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
            let value =
                event_init_member(scope, init, name).ok_or(V8BindingError::JavaScriptException)?;
            if value.is_undefined() {
                return v8_string(scope, "");
            }
            let value = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let value = value.to_rust_string_lossy(scope);
            v8_string(scope, &value)
        };
        // Flattened dictionary conversion is lexical, independent of IDL order.
        let new_url = read("newURL")?;
        let old_url = read("oldURL")?;
        (old_url, new_url)
    } else {
        let transition = event_init_member(scope, init, "hasUAVisualTransition")
            .ok_or(V8BindingError::JavaScriptException)?;
        let transition = transition.boolean_value(scope);
        let transition = v8::Boolean::new(scope, transition);
        let state =
            event_init_member(scope, init, "state").ok_or(V8BindingError::JavaScriptException)?;
        let state = if state.is_undefined() {
            v8::null(scope).into()
        } else {
            state
        };
        (state, transition.into())
    };
    make_payload(scope, interface, first, second)
}

fn make_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface: &str,
    first: v8::Local<'s, v8::Value>,
    second: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let brand = v8_string(scope, interface)?;
    let payload = v8::Array::new_with_elements(scope, &[brand, first, second]);
    let null = v8::null(scope);
    if payload.set_prototype(scope, null.into()) != Some(true) {
        return Err(invalid());
    }
    Ok(payload)
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
    if !payload
        .get_index(scope, 0)
        .is_some_and(|value| value.is_string() && value.to_rust_string_lossy(scope) == interface)
    {
        return Err(invalid());
    }
    let index = match (interface, member) {
        ("HashChangeEvent", "oldURL") | ("PopStateEvent", "state") => 1,
        ("HashChangeEvent", "newURL") | ("PopStateEvent", "hasUAVisualTransition") => 2,
        _ => return Err(invalid()),
    };
    payload.get_index(scope, index).ok_or_else(invalid)
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    event_type: &str,
    properties: &[(&str, v8::Local<'s, v8::Value>)],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let property = |name| {
        properties
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let (interface, first, second) = match event_type {
        "popstate" => (
            "PopStateEvent",
            property("state").unwrap_or_else(|| v8::null(scope).into()),
            v8::Boolean::new(scope, false).into(),
        ),
        "hashchange" => (
            "HashChangeEvent",
            property("oldURL").ok_or_else(invalid)?,
            property("newURL").ok_or_else(invalid)?,
        ),
        _ => return Err(invalid()),
    };
    let event =
        create_event_object_with_interface(scope, state, interface, event_type, false, false)?;
    let payload = make_payload(scope, interface, first, second)?;
    event.set_internal_field(2, payload.into());
    let id = event_id(scope, event)?;
    state
        .events
        .borrow_mut()
        .get_mut(&id)
        .ok_or_else(invalid)?
        .is_trusted = true;
    Ok(event)
}
