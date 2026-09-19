//! Native immutable touch snapshots. Platform-object identity, rather than
//! author-visible properties, authenticates Touch and TouchList receivers.
use super::*;

pub(super) struct TouchRecord {
    pub identifier: i32,
    pub target: v8::Global<v8::Value>,
    pub coordinates: [f64; 10],
}
const FIELDS: [&str; 10] = [
    "clientX",
    "clientY",
    "screenX",
    "screenY",
    "pageX",
    "pageY",
    "radiusX",
    "radiusY",
    "rotationAngle",
    "force",
];
fn invalid() -> V8BindingError {
    V8BindingError::Allocation("Touch receiver or dictionary")
}

pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let init = args.get(0);
    let mut coordinates = [0.; 10];
    let mut identifier = None;
    let mut target = None;
    // Web IDL dictionary conversion visits members in lexicographic order.
    for name in [
        "clientX",
        "clientY",
        "force",
        "identifier",
        "pageX",
        "pageY",
        "radiusX",
        "radiusY",
        "rotationAngle",
        "screenX",
        "screenY",
        "target",
    ] {
        let value = checked_dictionary_member(scope, init, name)?;
        if name == "identifier" {
            identifier = Some(
                value
                    .ok_or_else(invalid)?
                    .int32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?,
            );
        } else if name == "target" {
            let value = value.ok_or_else(invalid)?;
            if value.is_null_or_undefined() {
                return Err(invalid());
            }
            let value = ui_event_nullable_target(scope, state, value, false).ok_or_else(invalid)?;
            target = Some(v8::Global::new(scope, value));
        } else if let Some(value) = value {
            let mut value = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let index = FIELDS.iter().position(|field| *field == name).unwrap();
            if index >= 6 {
                value = (value as f32) as f64;
            }
            if !value.is_finite() {
                return Err(invalid());
            }
            coordinates[index] = value;
        }
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::Touch(TouchRecord {
            identifier: identifier.ok_or_else(invalid)?,
            target: target.ok_or_else(invalid)?,
            coordinates,
        }),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}

pub(super) fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    record: TouchRecord,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let id = register_platform_object(state, PlatformObjectRecord::Touch(record));
    new_platform_object(scope, state, "Touch", id)
}

pub(super) fn list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    values: &[v8::Local<v8::Object>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let mut entries = Vec::with_capacity(values.len());
    for value in values {
        let owner = binding_state_for_receiver(scope, *value).ok_or_else(invalid)?;
        let id = stateful_id(scope, *value)?;
        if !matches!(
            &*platform_object(&owner, id)?,
            PlatformObjectRecord::Touch(_)
        ) {
            return Err(invalid());
        }
        entries.push(v8::Global::new(scope, *value));
    }
    let id = register_platform_object(state, PlatformObjectRecord::TouchList(entries));
    new_platform_object(scope, state, "TouchList", id)
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let id = stateful_id(scope, object)?;
    match &*platform_object(&owner, id)? {
        PlatformObjectRecord::Touch(record) if interface == "Touch" => {
            if member == "identifier" {
                return Ok(v8::Integer::new(scope, record.identifier).into());
            }
            if member == "target" {
                return Ok(v8::Local::new(scope, &record.target));
            }
            let index = FIELDS
                .iter()
                .position(|field| *field == member)
                .ok_or_else(invalid)?;
            Ok(v8::Number::new(scope, record.coordinates[index]).into())
        }
        PlatformObjectRecord::TouchList(entries)
            if interface == "TouchList" && member == "length" =>
        {
            Ok(v8::Integer::new_from_unsigned(scope, entries.len() as u32).into())
        }
        _ => Err(invalid()),
    }
}

pub(super) fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    index: u32,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let id = stateful_id(scope, object)?;
    match &*platform_object(&owner, id)? {
        PlatformObjectRecord::TouchList(entries) => Ok(entries
            .get(index as usize)
            .map(|value| v8::Local::new(scope, value).into())
            .unwrap_or_else(|| v8::null(scope).into())),
        _ => Err(invalid()),
    }
}

pub(super) fn attach_lists(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event: v8::Local<v8::Object>,
    touches: &[v8::Local<v8::Object>],
    target_touches: &[v8::Local<v8::Object>],
    changed: &[v8::Local<v8::Object>],
) -> Result<(), V8BindingError> {
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let data = payload
        .get_index(scope, 4)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(invalid)?;
    for (name, values) in [
        ("touches", touches),
        ("targetTouches", target_touches),
        ("changedTouches", changed),
    ] {
        let value = list(scope, state, values)?;
        let key = v8::String::new(scope, name).ok_or_else(invalid)?;
        if data.create_data_property(scope, key.into(), value.into()) != Some(true) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
) -> Result<Vec<v8::Local<'s, v8::Object>>, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    let symbol = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, symbol.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| invalid())?;
    let mut entries = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
        let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
        let id = stateful_id(scope, object)?;
        if !matches!(
            &*platform_object(&owner, id)?,
            PlatformObjectRecord::Touch(_)
        ) {
            return Err(invalid());
        }
        entries.push(v8::Global::new(scope, object));
        Ok(())
    })?;
    Ok(entries
        .iter()
        .map(|value| v8::Local::new(scope, value))
        .collect())
}

pub(super) fn construct_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    initialize_ui_event(scope, state, "TouchEvent", args)
        .ok_or(V8BindingError::JavaScriptException)?;
    let init = args.get(1);
    let data = v8::Object::new(scope);
    let null = v8::null(scope);
    if data.set_prototype(scope, null.into()) != Some(true) {
        return Err(invalid());
    }
    for name in [
        "altKey",
        "ctrlKey",
        "metaKey",
        "modifierAltGraph",
        "modifierCapsLock",
        "shiftKey",
    ] {
        let value = checked_dictionary_member(scope, init, name)?
            .is_some_and(|value| value.boolean_value(scope));
        let value = v8::Boolean::new(scope, value);
        let key = v8::String::new(scope, name).ok_or_else(invalid)?;
        if data.create_data_property(scope, key.into(), value.into()) != Some(true) {
            return Err(invalid());
        }
    }
    let mut lists = Vec::new();
    for name in ["changedTouches", "targetTouches", "touches"] {
        let values = if let Some(value) = checked_dictionary_member(scope, init, name)? {
            sequence(scope, value)?
        } else {
            Vec::new()
        };
        lists.push((name, list(scope, state, &values)?));
    }
    for (name, value) in lists {
        let key = v8::String::new(scope, name).ok_or_else(invalid)?;
        if data.create_data_property(scope, key.into(), value.into()) != Some(true) {
            return Err(invalid());
        }
    }
    let payload = args
        .this()
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let null = v8::null(scope);
    if payload.set_prototype(scope, null.into()) != Some(true) {
        return Err(invalid());
    }
    let kind = v8::String::new(scope, "TouchEvent").ok_or_else(invalid)?;
    if payload.set_index(scope, 4, data.into()) != Some(true)
        || payload.set_index(scope, 5, kind.into()) != Some(true)
    {
        return Err(invalid());
    }
    Ok(())
}

fn indexed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    index: u32,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let value = item(scope, object, index)?;
    Ok((!value.is_null()).then_some(value))
}
fn indexed_get(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match indexed(scope, args.holder(), index) {
        Ok(Some(value)) => {
            rv.set(value);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn indexed_query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match indexed(scope, args.holder(), index) {
        Ok(Some(_)) => {
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn indexed_enumerate(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match get(scope, args.holder(), "TouchList", "length") {
        Ok(length) => {
            let length = length.uint32_value(scope).unwrap_or(0);
            let values = (0..length)
                .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &values));
        }
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn indexed_set(
    scope: &mut v8::PinScope,
    _index: u32,
    _value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "TouchList has no indexed setter");
    }
    v8::Intercepted::kYes
}
fn indexed_define(
    scope: &mut v8::PinScope,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "TouchList indices cannot be defined");
    }
    v8::Intercepted::kYes
}
fn indexed_delete(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match indexed(scope, args.holder(), index) {
        Ok(value) => {
            let allowed = value.is_none();
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(scope, "TouchList index cannot be deleted");
            }
        }
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}
pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_get)
            .query(indexed_query)
            .enumerator(indexed_enumerate)
            .setter(indexed_set)
            .definer(indexed_define)
            .deleter(indexed_delete),
    );
}

fn path_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.native.touch.dispatch.path").ok_or_else(invalid)?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn event_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    if payload.length() < 6 {
        return None;
    }
    if payload.get_index(scope, 5)?.to_rust_string_lossy(scope) != "TouchEvent" {
        return None;
    }
    payload
        .get_index(scope, 4)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
/// Snapshot every listener's touch targets before invoking any author callback.
/// A listener moving a host cannot expose a formerly closed descendant later.
pub(super) fn capture_path(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event: v8::Local<v8::Object>,
    target: NodeId,
    ancestors: &[NodeId],
    window: bool,
) -> Result<(), V8BindingError> {
    let Some(data) = event_data(scope, event) else {
        return Ok(());
    };
    let owner = binding_state_for_receiver(scope, event).ok_or_else(invalid)?;
    let mut originals = Vec::new();
    for name in ["touches", "targetTouches", "changedTouches"] {
        let key = v8::String::new(scope, name).ok_or_else(invalid)?;
        let list = data
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(invalid)?;
        let list_owner = binding_state_for_receiver(scope, list).ok_or_else(invalid)?;
        let id = stateful_id(scope, list)?;
        let entries = match &*platform_object(&list_owner, id)? {
            PlatformObjectRecord::TouchList(entries) => entries
                .iter()
                .map(|value| v8::Local::new(scope, value))
                .collect::<Vec<_>>(),
            _ => return Err(invalid()),
        };
        originals.push(entries);
    }
    let mut rows = Vec::new();
    for observer in std::iter::once(Some(target))
        .chain(ancestors.iter().copied().map(Some))
        .chain(window.then_some(None))
    {
        // The same Touch can occur in all three lists; preserve that identity
        // within each observer's snapshot when retargeting creates a new object.
        let mut cache: BTreeMap<(u64, u64), v8::Local<v8::Object>> = BTreeMap::new();
        let mut row = Vec::new();
        for original in &originals {
            let mut adjusted = Vec::new();
            for touch in original {
                let touch_owner = binding_state_for_receiver(scope, *touch).ok_or_else(invalid)?;
                let id = stateful_id(scope, *touch)?;
                let identity = (Rc::as_ptr(&touch_owner) as usize as u64, id);
                if let Some(value) = cache.get(&identity) {
                    adjusted.push(*value);
                    continue;
                }
                let (identifier, coordinates, original_target) =
                    match &*platform_object(&touch_owner, id)? {
                        PlatformObjectRecord::Touch(record) => (
                            record.identifier,
                            record.coordinates,
                            v8::Local::new(scope, &record.target),
                        ),
                        _ => return Err(invalid()),
                    };
                let mut adjusted_target = original_target;
                if let Ok(object) = v8::Local::<v8::Object>::try_from(original_target) {
                    if let Some((node, _, _)) = embedded_node_identity(scope, object) {
                        let node_owner =
                            binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
                        let against = if Rc::ptr_eq(&node_owner.dom, &state.dom) {
                            observer
                        } else {
                            None
                        };
                        let node = node_owner.dom.borrow().retarget_node(node, against)?;
                        adjusted_target = wrap_node(scope, &node_owner, node)?.into();
                    }
                }
                let value = if adjusted_target.strict_equals(original_target) {
                    *touch
                } else {
                    snapshot(
                        scope,
                        &owner,
                        TouchRecord {
                            identifier,
                            coordinates,
                            target: v8::Global::new(scope, adjusted_target),
                        },
                    )?
                };
                cache.insert(identity, value);
                adjusted.push(value);
            }
            row.push(list(scope, &owner, &adjusted)?.into());
        }
        rows.push(v8::Array::new_with_elements(scope, &row).into());
    }
    let path = v8::Array::new_with_elements(scope, &rows);
    let key = path_key(scope)?;
    if event.set_private(scope, key, path.into()) != Some(true) {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn apply_path(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    index: u32,
) -> Result<(), V8BindingError> {
    let key = path_key(scope)?;
    let Some(path) = event
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Ok(());
    };
    let row = path
        .get_index(scope, index)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let data = event_data(scope, event).ok_or_else(invalid)?;
    for (index, name) in ["touches", "targetTouches", "changedTouches"]
        .iter()
        .enumerate()
    {
        let value = row.get_index(scope, index as u32).ok_or_else(invalid)?;
        let key = v8::String::new(scope, name).ok_or_else(invalid)?;
        if data.create_data_property(scope, key.into(), value) != Some(true) {
            return Err(invalid());
        }
    }
    Ok(())
}
pub(super) fn clear_path(scope: &mut v8::PinScope, event: v8::Local<v8::Object>) {
    if let Ok(key) = path_key(scope) {
        let empty = v8::undefined(scope);
        event.set_private(scope, key, empty.into());
    }
}
pub(super) fn clear_targets(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    if event_data(scope, event).is_some() {
        attach_lists(scope, state, event, &[], &[], &[])?;
    }
    Ok(())
}
