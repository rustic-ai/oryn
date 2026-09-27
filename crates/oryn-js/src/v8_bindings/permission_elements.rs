//! Native in-page permission consumers. Parent queries remain the only grant
//! authority; reflected attributes and author events never create permission.
use super::*;
use crate::platform_control::PermissionState;
/// Immutable native feature selection, installed before bindings for this realm.
/// Absent is the standard capture-producing usermedia interface.
#[derive(Clone, Copy, Debug, Default)]
pub struct UserMediaElementProfile {
    pub legacy: bool,
}
fn legacy(state: &V8BindingState, node: NodeId) -> bool {
    state.usermedia_legacy && kind(state, node) == Some("usermedia")
}
fn legacy_types(value: &str) -> Option<Vec<&str>> {
    let mut types = Vec::new();
    for token in value.split_ascii_whitespace() {
        if !matches!(token, "camera" | "microphone") || types.contains(&token) {
            return None;
        }
        types.push(token);
    }
    (!types.is_empty()).then_some(types)
}
pub(super) fn legacy_prompt_kind(
    state: &V8BindingState,
    node: NodeId,
) -> Option<crate::platform_control::PromptKind> {
    if !legacy(state, node) {
        return None;
    }
    let dom = state.dom.borrow();
    let types = legacy_types(
        dom.attribute_in_null_namespace(node, "type")
            .ok()
            .flatten()
            .unwrap_or(""),
    )?;
    Some(match types.as_slice() {
        ["camera"] => crate::platform_control::PromptKind::Camera,
        ["microphone"] => crate::platform_control::PromptKind::Microphone,
        _ => crate::platform_control::PromptKind::UserMedia,
    })
}
pub(super) fn install_legacy_statics(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    if !state.usermedia_legacy {
        return Ok(());
    }
    let key = v8::String::new(scope, "HTMLUserMediaElement")
        .ok_or(V8BindingError::Allocation("usermedia constructor"))?;
    let constructor = context
        .global(scope)
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or(V8BindingError::MissingInterface("HTMLUserMediaElement"))?;
    let function = v8::Function::builder(legacy_type_supported)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("usermedia type support"))?;
    let key = v8::String::new(scope, "isTypeSupported")
        .ok_or(V8BindingError::Allocation("usermedia type support name"))?;
    function.set_name(key);
    if constructor.define_own_property(
        scope,
        key.into(),
        function.into(),
        v8::PropertyAttribute::DONT_ENUM,
    ) != Some(true)
    {
        return Err(V8BindingError::Allocation(
            "usermedia type support publication",
        ));
    }
    Ok(())
}
fn legacy_type_supported(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut result: v8::ReturnValue,
) {
    if args.length() == 0 {
        if let Some(message) = v8::String::new(
            scope,
            "HTMLUserMediaElement.isTypeSupported requires a type",
        ) {
            let error = v8::Exception::type_error(scope, message);
            scope.throw_exception(error);
        }
        return;
    }
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    result.set_bool(legacy_types(&value.to_rust_string_lossy(scope)).is_some());
}
pub(super) fn set_type(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let original = wrap_node(scope, state, node)?;
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let value = rust_string_preserving_utf16(scope, value)?;
    let (owner, node) =
        node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
    if !legacy(&owner, node) {
        return Err(fail("TypeError"));
    }
    owner.dom.borrow_mut().set_attribute(node, "type", value)?;
    Ok(())
}

#[derive(Clone)]
struct CaptureFailure {
    message: String,
    canceled: bool,
    revision: u64,
}
struct ElementPublication {
    name: &'static str,
    stream: u64,
    authority: u64,
    activate: bool,
    notification: Option<native_platform::publication::Notification>,
}
struct Element {
    attached: std::time::Instant,
    initial: Option<PermissionState>,
    current: PermissionState,
    position: Option<v8::Global<v8::Object>>,
    stream: Option<v8::Global<v8::Object>>,
    track: Option<v8::Global<v8::Object>>,
    error: Option<v8::Global<v8::Value>>,
    constraints: serde_json::Value,
    operation: Option<(u64, u64, u64)>,
    prompt_result: Option<(crate::platform_control::PromptChoice, PermissionState)>,
    pending: Option<bool>,
    capture_from_prompt: bool,
    autolocated: bool,
    location: Option<(native_platform::ElementLocation, u64)>,
    captured: Option<v8::Global<v8::Object>>,
    capture_failure: Option<CaptureFailure>,
    publication: Option<ElementPublication>,
    revision: Option<u64>,
    valid: bool,
    reason: String,
    geometry: Option<(f64, f64, f64, f64)>,
    geometry_changed: Option<std::time::Instant>,
    visibility: Option<permission_visibility::Status>,
    visibility_changed: Option<std::time::Instant>,
}
impl Default for Element {
    fn default() -> Self {
        Self {
            attached: std::time::Instant::now(),
            initial: None,
            current: PermissionState::Prompt,
            position: None,
            stream: None,
            track: None,
            error: None,
            constraints: serde_json::json!({"audio":{},"video":{}}),
            operation: None,
            prompt_result: None,
            pending: None,
            capture_from_prompt: false,
            autolocated: false,
            location: None,
            captured: None,
            capture_failure: None,
            publication: None,
            revision: None,
            valid: false,
            reason: "unsuccessful_registration".into(),
            geometry: None,
            geometry_changed: None,
            visibility: None,
            visibility_changed: None,
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    pub(super) prompt: Option<(NodeId, String)>,
    pub(super) prompt_keys: BTreeSet<String>,
    pub(super) suppress_activation: Option<NodeId>,
    pub(super) prompt_pointer: Option<(i32, Option<String>)>,
    elements: BTreeMap<NodeId, Element>,
    publishing: bool,
    retry_at: Option<std::time::Instant>,
    cursor: Option<oryn_dom::MutationCursor>,
}
fn fail(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "PermissionElement".into(),
        member: name.into(),
    }
}
fn kind(state: &V8BindingState, node: NodeId) -> Option<&'static str> {
    let dom = state.dom.borrow();
    [
        "geolocation",
        "usermedia",
        "camera",
        "microphone",
        "install",
    ]
    .into_iter()
    .find(|tag| dom.is_element(node, "http://www.w3.org/1999/xhtml", tag))
}
fn ensure(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    if kind(state, node).is_none() {
        return Err(fail("TypeError"));
    }
    let mut registry = state.permission_elements.borrow_mut();
    if !registry.elements.contains_key(&node) && registry.elements.len() >= 4096 {
        return Err(fail("QuotaExceededError"));
    }
    let element = registry.elements.entry(node).or_default();
    if kind(state, node) == Some("camera") {
        element
            .constraints
            .as_object_mut()
            .unwrap()
            .insert("audio".into(), serde_json::Value::Bool(false));
    }
    if kind(state, node) == Some("microphone") {
        element
            .constraints
            .as_object_mut()
            .unwrap()
            .insert("video".into(), serde_json::Value::Bool(false));
    }
    Ok(())
}
pub(super) fn handles(interface: &str, member: &str) -> bool {
    matches!(
        interface,
        "HTMLGeolocationElement"
            | "HTMLUserMediaElement"
            | "HTMLCameraElement"
            | "HTMLMicrophoneElement"
            | "HTMLInstallElement"
    ) && !member.starts_with("on")
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    ensure(state, node)?;
    if member == "type" {
        return v8_string(
            scope,
            state
                .dom
                .borrow()
                .attribute_in_null_namespace(node, "type")?
                .unwrap_or(""),
        );
    }
    if matches!(member, "autolocate" | "watch") {
        return Ok(v8::Boolean::new(
            scope,
            state
                .dom
                .borrow()
                .attribute_in_null_namespace(node, member)?
                .is_some(),
        )
        .into());
    }
    if matches!(member, "isValid" | "invalidReason") {
        let reason = invalid_reason(state, node)?;
        return if member == "isValid" {
            Ok(v8::Boolean::new(scope, reason.is_empty()).into())
        } else {
            v8_string(scope, &reason)
        };
    }
    let registry = state.permission_elements.borrow();
    let value = &registry.elements[&node];
    match member {
        "position" => Ok(value
            .position
            .as_ref()
            .map(|value| v8::Local::new(scope, value).into())
            .unwrap_or_else(|| v8::null(scope).into())),
        "error" => Ok(value
            .error
            .as_ref()
            .map(|value| v8::Local::new(scope, value))
            .unwrap_or_else(|| v8::null(scope).into())),
        "track" => Ok(value
            .track
            .as_ref()
            .map(|value| v8::Local::new(scope, value).into())
            .unwrap_or_else(|| v8::null(scope).into())),
        "stream" => Ok(value
            .stream
            .as_ref()
            .map(|value| v8::Local::new(scope, value).into())
            .unwrap_or_else(|| v8::null(scope).into())),
        "permissionStatus" => v8_string(scope, value.current.as_str()),
        "initialPermissionStatus" => v8_string(
            scope,
            value.initial.unwrap_or(PermissionState::Prompt).as_str(),
        ),
        _ => Err(fail("unsupported permission element property")),
    }
}
pub(super) fn set(
    state: &V8BindingState,
    node: NodeId,
    name: &str,
    value: bool,
) -> Result<(), V8BindingError> {
    if value {
        state
            .dom
            .borrow_mut()
            .set_attribute_ns(node, None, name, "")?;
    } else {
        state
            .dom
            .borrow_mut()
            .remove_attribute_ns(node, None, name)?;
    }
    Ok(())
}
fn dictionary<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Local<'s, v8::Object>>, V8BindingError> {
    if value.is_null_or_undefined() {
        Ok(None)
    } else {
        v8::Local::<v8::Object>::try_from(value)
            .map(Some)
            .map_err(|_| fail("TypeError"))
    }
}
#[derive(Clone, Copy)]
enum ConstraintType {
    ULong,
    Double,
    Boolean,
    String,
    BooleanString,
}
fn primitive_constraint(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    kind: ConstraintType,
) -> Result<serde_json::Value, V8BindingError> {
    match kind {
        ConstraintType::Boolean => Ok(serde_json::Value::Bool(value.boolean_value(scope))),
        ConstraintType::BooleanString if value.is_boolean() => {
            Ok(serde_json::Value::Bool(value.boolean_value(scope)))
        }
        ConstraintType::String | ConstraintType::BooleanString => {
            let text = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            if text.len() > 4096 {
                return Err(fail("QuotaExceededError"));
            }
            Ok(serde_json::Value::String(text))
        }
        ConstraintType::ULong => {
            let number = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let value = if number.is_nan() || number <= 0. {
                0
            } else if number >= u32::MAX as f64 {
                u32::MAX
            } else {
                number.round_ties_even() as u32
            };
            Ok(serde_json::json!(value))
        }
        ConstraintType::Double => {
            let number = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            serde_json::Number::from_f64(number)
                .map(serde_json::Value::Number)
                .ok_or_else(|| fail("TypeError"))
        }
    }
}
fn string_sequence(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Option<serde_json::Value>, V8BindingError> {
    let key = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    if method.is_null_or_undefined() {
        return Ok(None);
    }
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| fail("TypeError"))?;
    let mut values = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        if values.len() >= 128 {
            return Err(fail("QuotaExceededError"));
        }
        values.push(primitive_constraint(scope, value, ConstraintType::String)?);
        Ok(())
    })?;
    Ok(Some(serde_json::Value::Array(values)))
}
fn constraint_value(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    kind: ConstraintType,
) -> Result<serde_json::Value, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(serde_json::json!({}));
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return primitive_constraint(scope, value, kind);
    };
    if matches!(kind, ConstraintType::String)
        && let Some(sequence) = string_sequence(scope, object)?
    {
        return Ok(sequence);
    }
    let mut result = serde_json::Map::new();
    // Dictionary inheritance visits range members before derived exact/ideal.
    let names: &[&str] = if matches!(kind, ConstraintType::ULong | ConstraintType::Double) {
        &["max", "min", "exact", "ideal"]
    } else {
        &["exact", "ideal"]
    };
    for name in names {
        let key = v8::String::new(scope, name).unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if value.is_undefined() {
            continue;
        }
        let converted = if matches!(kind, ConstraintType::String) {
            if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
                string_sequence(scope, object)?
                    .map(Ok)
                    .unwrap_or_else(|| primitive_constraint(scope, value, kind))?
            } else {
                primitive_constraint(scope, value, kind)?
            }
        } else {
            primitive_constraint(scope, value, kind)?
        };
        result.insert((*name).into(), converted);
    }
    Ok(serde_json::Value::Object(result))
}
fn track_constraint_set(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    advanced: bool,
) -> Result<serde_json::Value, V8BindingError> {
    let mut fields = serde_json::Map::new();
    let Some(object) = dictionary(value)? else {
        return Ok(serde_json::Value::Object(fields));
    };
    for (name, kind) in [
        ("aspectRatio", ConstraintType::Double),
        ("autoGainControl", ConstraintType::Boolean),
        ("backgroundBlur", ConstraintType::Boolean),
        ("channelCount", ConstraintType::ULong),
        ("deviceId", ConstraintType::String),
        ("echoCancellation", ConstraintType::BooleanString),
        ("facingMode", ConstraintType::String),
        ("frameRate", ConstraintType::Double),
        ("groupId", ConstraintType::String),
        ("height", ConstraintType::ULong),
        ("latency", ConstraintType::Double),
        ("noiseSuppression", ConstraintType::Boolean),
        ("resizeMode", ConstraintType::String),
        ("sampleRate", ConstraintType::ULong),
        ("sampleSize", ConstraintType::ULong),
        ("width", ConstraintType::ULong),
    ] {
        let key = v8::String::new(scope, name).unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_undefined() {
            fields.insert(name.into(), constraint_value(scope, value, kind)?);
        }
    }
    if advanced {
        let key = v8::String::new(scope, "advanced").unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_undefined() {
            let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| fail("TypeError"))?;
            let key = v8::Symbol::get_iterator(scope);
            let method = object
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            let method =
                v8::Local::<v8::Function>::try_from(method).map_err(|_| fail("TypeError"))?;
            let mut values = Vec::new();
            iterable_conversion::consume(scope, object, method, |scope, value| {
                if values.len() >= 128 {
                    return Err(fail("QuotaExceededError"));
                }
                values.push(track_constraint_set(scope, value, false)?);
                Ok(())
            })?;
            fields.insert("advanced".into(), serde_json::Value::Array(values));
        }
    }
    Ok(serde_json::Value::Object(fields))
}
pub(super) fn track_constraints(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<serde_json::Value, V8BindingError> {
    track_constraint_set(scope, value, true)
}
pub(super) fn constraints(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    ensure(state, node)?;
    if matches!(kind(state, node), Some("camera" | "microphone")) {
        let camera = kind(state, node) == Some("camera");
        let converted = track_constraints(scope, value)?;
        let mut video = serde_json::Map::new();
        let names: &[&str] = if camera {
            &[
                "width",
                "height",
                "aspectRatio",
                "frameRate",
                "facingMode",
                "resizeMode",
                "deviceId",
                "groupId",
            ]
        } else {
            &[
                "sampleRate",
                "sampleSize",
                "channelCount",
                "latency",
                "echoCancellation",
                "autoGainControl",
                "noiseSuppression",
                "deviceId",
                "groupId",
            ]
        };
        for &name in names {
            if let Some(value) = converted
                .get(name)
                .filter(|value| !value.is_object() && !value.is_array())
            {
                video.insert(name.into(), value.clone());
            }
        }
        state
            .permission_elements
            .borrow_mut()
            .elements
            .get_mut(&node)
            .unwrap()
            .constraints = if camera {
            serde_json::json!({"audio":false,"video":video})
        } else {
            serde_json::json!({"audio":video,"video":false})
        };
        return Ok(());
    }
    let mut result = serde_json::Map::new();
    let object = dictionary(value)?;
    for name in ["audio", "video"] {
        let fields = if let Some(object) = object {
            let key = v8::String::new(scope, name).unwrap();
            let value = object
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            track_constraints(scope, value)?
        } else {
            serde_json::json!({})
        };
        result.insert(name.into(), fields);
    }
    state
        .permission_elements
        .borrow_mut()
        .elements
        .get_mut(&node)
        .unwrap()
        .constraints = serde_json::Value::Object(result);
    Ok(())
}
fn invalid_reason(state: &V8BindingState, node: NodeId) -> Result<String, V8BindingError> {
    if state.execution_retired() || !state.dom.borrow().is_connected(node)? {
        return Ok("unsuccessful_registration".into());
    }
    if legacy(state, node) && legacy_prompt_kind(state, node).is_none() {
        return Ok("type_invalid".into());
    }
    if kind(state, node) == Some("install") {
        match native_platform::prompt_request(
            state,
            crate::platform_control::Request::InstallEligibility,
        )? {
            crate::platform_control::Response::InstallEligibility { reason }
                if !reason.is_empty() =>
            {
                return Ok(reason);
            }
            crate::platform_control::Response::InstallEligibility { .. } => {}
            _ => return Err(fail("InvalidStateError")),
        }
    }
    let (viewport, rect) = native_input_geometry(state, Some(node))?;
    let style = state.layout.borrow().styles.get(&node_key(node)).cloned();
    if let Some(style) = &style
        && !matches!(
            style
                .get("display")
                .map(String::as_str)
                .unwrap_or("inline-block"),
            "block" | "inline-block" | "flex" | "inline-flex"
        )
    {
        return Ok("style_invalid".into());
    }
    let Some(rect) = rect else {
        return Ok("intersection_out_of_viewport_or_clipped".into());
    };
    let geometry = (rect.x, rect.y, rect.width, rect.height);
    {
        let mut registry = state.permission_elements.borrow_mut();
        if let Some(element) = registry.elements.get_mut(&node)
            && element.geometry != Some(geometry)
        {
            element.geometry = Some(geometry);
            element.geometry_changed = Some(std::time::Instant::now());
        }
    }
    if state
        .permission_elements
        .borrow()
        .elements
        .get(&node)
        .is_none_or(|value| value.attached.elapsed() < std::time::Duration::from_millis(500))
    {
        return Ok("unsuccessful_registration".into());
    }
    if rect.width <= 0.
        || rect.height <= 0.
        || rect.x < 0.
        || rect.y < 0.
        || rect.x + rect.width > viewport.0
        || rect.y + rect.height > viewport.1
    {
        return Ok("intersection_out_of_viewport_or_clipped".into());
    }
    let visibility = permission_visibility::assess(state, node)?;
    {
        let mut registry = state.permission_elements.borrow_mut();
        if let Some(element) = registry.elements.get_mut(&node)
            && element.visibility != Some(visibility)
        {
            element.visibility_changed = Some(if element.visibility.is_none() {
                element
                    .geometry_changed
                    .unwrap_or_else(std::time::Instant::now)
            } else {
                std::time::Instant::now()
            });
            element.visibility = Some(visibility);
        }
    }
    match visibility {
        permission_visibility::Status::Visible => {}
        permission_visibility::Status::Clipped => {
            return Ok("intersection_out_of_viewport_or_clipped".into());
        }
        permission_visibility::Status::Occluded => {
            return Ok("intersection_occluded_or_distorted".into());
        }
        permission_visibility::Status::StyleInvalid => return Ok("style_invalid".into()),
    }
    if state
        .permission_elements
        .borrow()
        .elements
        .get(&node)
        .and_then(|value| {
            value
                .geometry_changed
                .into_iter()
                .chain(value.visibility_changed)
                .max()
        })
        .is_some_and(|time| time.elapsed() < std::time::Duration::from_millis(500))
    {
        return Ok("intersection_changed".into());
    }
    Ok(String::new())
}
pub(super) fn activate(
    state: &V8BindingState,
    node: NodeId,
    trusted: bool,
) -> Result<(), V8BindingError> {
    if trusted
        && state
            .permission_elements
            .borrow_mut()
            .suppress_activation
            .take()
            == Some(node)
    {
        return Ok(());
    }
    if kind(state, node).is_none() {
        return Ok(());
    }
    ensure(state, node)?;
    let identity = (
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    );
    let mut registry = state.permission_elements.borrow_mut();
    let element = registry.elements.get_mut(&node).unwrap();
    if element.operation.is_some_and(|old| old != identity) {
        native_media::mark_capture_cancellation(state, node);
        element.location = None;
        element.captured = None;
        element.capture_failure = None;
        element.prompt_result = None;
        element.autolocated = false;
    }
    element.capture_from_prompt = false;
    element.pending = Some(trusted);
    element.operation = Some(identity);
    Ok(())
}
fn dispatch_pending(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    if state.permission_elements.borrow().publishing {
        return Ok(());
    }
    let pending = state
        .permission_elements
        .borrow_mut()
        .elements
        .get_mut(&node)
        .and_then(|element| element.publication.take());
    let Some(mut pending) = pending else {
        return Ok(());
    };
    let current = !state.execution_retired()
        && state.document_stream_epoch() == pending.stream
        && state.document_authority_epoch() == pending.authority
        && state.dom.borrow().node_document(node).ok().flatten() == Some(state.document_root())
        && state.dom.borrow().is_connected(node).unwrap_or(false);
    if !current {
        if let Some(notification) = pending.notification {
            notification.cancel(state);
        }
        return Ok(());
    }
    state.permission_elements.borrow_mut().publishing = true;
    let result = (|| {
        if pending.notification.is_none() {
            let target = wrap_node(scope, state, node)?;
            let event = create_event_object(scope, state, pending.name, false, false)?;
            pending.notification = Some(
                native_platform::publication::pending_dom_dispatch(
                    scope,
                    state,
                    target,
                    pending.name,
                    event,
                )
                .for_parser_stream(pending.stream),
            );
        }
        let notification = pending.notification.as_mut().unwrap();
        if !notification.resume(scope, state)? {
            return Err(V8BindingError::Allocation(
                "permission element event suspended",
            ));
        }
        notification.finish(scope, state);
        if pending.activate
            && !state.execution_retired()
            && state.document_stream_epoch() == pending.stream
            && state.document_authority_epoch() == pending.authority
            && state.dom.borrow().node_document(node).ok().flatten() == Some(state.document_root())
            && state.dom.borrow().is_connected(node).unwrap_or(false)
        {
            activate(state, node, true)?;
        }
        Ok(())
    })();
    state.permission_elements.borrow_mut().publishing = false;
    if result.is_err() {
        state.permission_elements.borrow_mut().retry_at =
            std::time::Instant::now().checked_add(std::time::Duration::from_millis(25));
        if let Some(element) = state
            .permission_elements
            .borrow_mut()
            .elements
            .get_mut(&node)
        {
            element.publication = Some(pending);
        } else if let Some(notification) = pending.notification {
            notification.cancel(state);
        }
    }
    result
}
fn event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    name: &'static str,
) -> Result<(), V8BindingError> {
    event_with_activation(scope, state, node, name, false)
}
fn event_with_activation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    name: &'static str,
    activate: bool,
) -> Result<(), V8BindingError> {
    let mut registry = state.permission_elements.borrow_mut();
    let element = registry
        .elements
        .get_mut(&node)
        .ok_or_else(|| fail("event target unavailable"))?;
    if element.publication.is_some() {
        return Err(V8BindingError::Allocation(
            "permission element event pending",
        ));
    }
    element.publication = Some(ElementPublication {
        name,
        stream: state.document_stream_epoch(),
        authority: state.document_authority_epoch(),
        activate,
        notification: None,
    });
    drop(registry);
    dispatch_pending(scope, state, node)
}

pub(super) fn admit_prompt_result(
    state: &Rc<V8BindingState>,
    node: NodeId,
    choice: crate::platform_control::PromptChoice,
    permission: PermissionState,
) -> Result<(), V8BindingError> {
    if kind(state, node).is_none()
        || state.dom.borrow().node_document(node)? != Some(state.document_root())
        || !state.dom.borrow().is_connected(node)?
    {
        return Ok(());
    }
    ensure(state, node)?;
    let mut registry = state.permission_elements.borrow_mut();
    let element = registry.elements.get_mut(&node).unwrap();
    element.prompt_result = Some((choice, permission));
    element.operation = Some((
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    ));
    Ok(())
}
pub(super) fn resume_prompt_result(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    if !state
        .permission_elements
        .borrow()
        .elements
        .contains_key(&node)
    {
        return Ok(());
    }
    dispatch_pending(scope, state, node)?;
    publish_prompt_result(scope, state, node)
}
pub(super) fn prompt_result(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    choice: crate::platform_control::PromptChoice,
    permission: PermissionState,
) -> Result<(), V8BindingError> {
    admit_prompt_result(state, node, choice, permission)?;
    resume_prompt_result(scope, state, node)
}
fn publish_prompt_result(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let Some((choice, permission)) =
        state.permission_elements.borrow().elements[&node].prompt_result
    else {
        return Ok(());
    };
    if legacy(state, node) {
        {
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            element.prompt_result = None;
            element.pending = None;
            element.current = permission;
            element.stream = None;
            element.track = None;
            element.error = None;
        }
        event(
            scope,
            state,
            node,
            if choice == crate::platform_control::PromptChoice::Dismiss {
                "promptdismiss"
            } else {
                "promptaction"
            },
        )?;
    } else if kind(state, node) == Some("geolocation") {
        state
            .permission_elements
            .borrow_mut()
            .elements
            .get_mut(&node)
            .unwrap()
            .prompt_result = None;
        event_with_activation(
            scope,
            state,
            node,
            if choice == crate::platform_control::PromptChoice::Dismiss {
                "promptdismiss"
            } else {
                "promptaction"
            },
            choice != crate::platform_control::PromptChoice::Dismiss,
        )?;
    } else if permission == PermissionState::Granted
        && choice == crate::platform_control::PromptChoice::Allow
    {
        activate(state, node, true)?;
        let mut registry = state.permission_elements.borrow_mut();
        let element = registry.elements.get_mut(&node).unwrap();
        element.capture_from_prompt = true;
        element.prompt_result = None;
    } else {
        let error = dom_exception_value(scope, "NotAllowedError")?;
        {
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            element.prompt_result = None;
            element.error = Some(v8::Global::new(scope, error));
            element.stream = None;
            element.track = None;
        }
        event(scope, state, node, "cancel")?;
    }
    Ok(())
}

fn publish_capture_failure(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let Some(failure) = state.permission_elements.borrow().elements[&node]
        .capture_failure
        .clone()
    else {
        return Ok(());
    };
    native_media::cancel_capture(state, node)?;
    let name = failure
        .message
        .split(':')
        .next()
        .unwrap_or("NotReadableError");
    let error = if name == "OverconstrainedError" {
        native_media::overconstrained(
            scope,
            failure
                .message
                .split_once(':')
                .map(|(_, name)| name.trim())
                .unwrap_or(""),
            &failure.message,
        )?
    } else {
        dom_exception_value(scope, name)?
    };
    {
        let mut registry = state.permission_elements.borrow_mut();
        let element = registry.elements.get_mut(&node).unwrap();
        element.capture_failure = None;
        element.revision = Some(failure.revision);
        element.error = Some(v8::Global::new(scope, error));
        if failure.canceled {
            element.stream = None;
            element.track = None;
        }
    }
    event(
        scope,
        state,
        node,
        if failure.canceled { "cancel" } else { "error" },
    )
}
pub(super) fn poll_with_locality(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    revision: u64,
    hidden: bool,
) -> Result<(usize, bool), V8BindingError> {
    let mut local_only = false;
    let count = poll_inner(scope, state, revision, hidden, &mut local_only)?;
    Ok((count, local_only))
}

fn poll_inner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    _revision: u64,
    _hidden: bool,
    local_only: &mut bool,
) -> Result<usize, V8BindingError> {
    let local_candidate = {
        let registry = state.permission_elements.borrow();
        registry.elements.is_empty()
            && registry.prompt.is_none()
            && !registry.publishing
            && registry.retry_at.is_none()
    };
    let res = {
        let registry = state.permission_elements.borrow();
        registry.publishing
            || registry
                .retry_at
                .is_some_and(|deadline| deadline > std::time::Instant::now())
    };
    if res {
        return Ok(0);
    }
    state.permission_elements.borrow_mut().retry_at = None;
    let generation = state.document_generation();
    let stream = state.document_stream_epoch();
    let authority = state.document_authority_epoch();
    let current = |state: &V8BindingState| {
        !state.execution_retired()
            && !state.execution_tasks_stopped()
            && state.document_generation() == generation
            && state.document_stream_epoch() == stream
            && state.document_authority_epoch() == authority
    };
    let publications = state
        .permission_elements
        .borrow()
        .elements
        .iter()
        .filter_map(|(node, element)| element.publication.is_some().then_some(*node))
        .collect::<Vec<_>>();
    for node in publications {
        if native_platform::task_snapshot(scope, state, generation, stream).is_none() {
            return Ok(0);
        }
        dispatch_pending(scope, state, node)?;
        if !current(state) {
            return Ok(0);
        }
    }
    permission_prompt::fixup(state)?;
    let cursor = state.dom.borrow().mutation_cursor();
    {
        let dom = state.dom.borrow();
        state
            .permission_elements
            .borrow_mut()
            .elements
            .retain(|node, value| {
                let owned = dom.node_document(*node).ok().flatten() == Some(state.document_root());
                if !dom.is_connected(*node).unwrap_or(false) {
                    value.pending = None;
                    value.capture_from_prompt = false;
                    value.prompt_result = None;
                    value.location = None;
                    value.captured = None;
                    value.capture_failure = None;
                    value.operation = None;
                    value.autolocated = false;
                    value.attached = std::time::Instant::now();
                }
                owned
            });
    }
    let nodes = {
        let dom = state.dom.borrow();
        dom.arena()
            .iter()
            .map(|(node, _)| node)
            .filter(|node| {
                dom.is_connected(*node).unwrap_or(false)
                    && dom.node_document(*node).ok().flatten() == Some(state.document_root())
                    && (dom.is_element(*node, "http://www.w3.org/1999/xhtml", "geolocation")
                        || dom.is_element(*node, "http://www.w3.org/1999/xhtml", "usermedia")
                        || dom.is_element(*node, "http://www.w3.org/1999/xhtml", "camera")
                        || dom.is_element(*node, "http://www.w3.org/1999/xhtml", "microphone")
                        || dom.is_element(*node, "http://www.w3.org/1999/xhtml", "install"))
            })
            .collect::<Vec<_>>()
    };
    state.permission_elements.borrow_mut().cursor = Some(cursor);
    // Discovery is never skipped. Only the empty entry registry + absent
    // prompt + actual empty DOM scan proves no callback/RPC occurred above.
    *local_only = local_candidate && nodes.is_empty();
    let mut count = 0;
    for node in nodes {
        let Some(observed) = native_platform::task_snapshot(scope, state, generation, stream)
        else {
            return Ok(count);
        };
        let (hidden, revision) = observed;
        if !state.dom.borrow().is_connected(node).unwrap_or(false)
            || state.dom.borrow().node_document(node).ok().flatten() != Some(state.document_root())
        {
            continue;
        }
        ensure(state, node)?;
        if state.permission_elements.borrow().elements[&node]
            .operation
            .is_some_and(|operation| operation != (generation, stream, authority))
        {
            native_media::cancel_capture(state, node)?;
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            element.pending = None;
            element.capture_from_prompt = false;
            element.prompt_result = None;
            element.location = None;
            element.captured = None;
            element.capture_failure = None;
            element.operation = None;
            element.autolocated = false;
        }
        if state.permission_elements.borrow().elements[&node]
            .capture_failure
            .is_some()
        {
            publish_capture_failure(scope, state, node)?;
            count += 1;
            if !current(state) {
                return Ok(count);
            }
            continue;
        }
        if native_media::capture_canceling(state, node) {
            native_media::cancel_capture(state, node)?;
        }
        if state.permission_elements.borrow().elements[&node]
            .prompt_result
            .is_some()
        {
            publish_prompt_result(scope, state, node)?;
            if !current(state) {
                return Ok(count);
            }
            if !state.dom.borrow().is_connected(node).unwrap_or(false) {
                continue;
            }
        }
        let geo = kind(state, node) == Some("geolocation");
        let camera = kind(state, node) == Some("camera");
        let microphone = kind(state, node) == Some("microphone");
        let install = kind(state, node) == Some("install");
        let legacy_mode = legacy(state, node);
        let needed = {
            let registry = state.permission_elements.borrow();
            let element = &registry.elements[&node];
            legacy_mode || element.revision != Some(revision) || element.initial.is_none()
        };
        if needed {
            let permission = if legacy_mode {
                match legacy_prompt_kind(state, node) {
                    Some(crate::platform_control::PromptKind::Camera) => {
                        native_platform::element_permission(scope, state, "camera")?
                    }
                    Some(crate::platform_control::PromptKind::Microphone) => {
                        native_platform::element_permission(scope, state, "microphone")?
                    }
                    Some(_) => {
                        let camera = native_platform::element_permission(scope, state, "camera")?;
                        let microphone =
                            native_platform::element_permission(scope, state, "microphone")?;
                        if camera == PermissionState::Denied
                            || microphone == PermissionState::Denied
                        {
                            PermissionState::Denied
                        } else if camera == PermissionState::Granted
                            && microphone == PermissionState::Granted
                        {
                            PermissionState::Granted
                        } else {
                            PermissionState::Prompt
                        }
                    }
                    None => PermissionState::Prompt,
                }
            } else if install {
                PermissionState::Prompt
            } else if camera {
                native_platform::element_permission(scope, state, "camera")?
            } else if microphone {
                native_platform::element_permission(scope, state, "microphone")?
            } else if geo {
                native_platform::element_permission(scope, state, "geolocation")?
            } else {
                let camera = native_platform::element_permission(scope, state, "camera")?;
                let microphone = native_platform::element_permission(scope, state, "microphone")?;
                if camera == PermissionState::Denied || microphone == PermissionState::Denied {
                    PermissionState::Denied
                } else if camera == PermissionState::Granted
                    && microphone == PermissionState::Granted
                {
                    PermissionState::Granted
                } else {
                    PermissionState::Prompt
                }
            };
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            element.initial.get_or_insert(permission);
            if element.current != permission && permission == PermissionState::Granted {
                element.autolocated = false;
            }
            element.current = permission;
            drop(registry);
            if !install {
                state
                    .dom
                    .borrow_mut()
                    .set_permission_granted(node, permission == PermissionState::Granted)?;
            }
        }
        let (autolocate, watch) = {
            let dom = state.dom.borrow();
            (
                dom.attribute_in_null_namespace(node, "autolocate")?
                    .is_some(),
                dom.attribute_in_null_namespace(node, "watch")?.is_some(),
            )
        };
        let (pending, automatic) = {
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            let automatic = geo
                && autolocate
                && (!element.autolocated || (watch && element.revision != Some(revision)));
            if !autolocate {
                element.autolocated = false;
            }
            let pending = if hidden { None } else { element.pending };
            (pending, automatic && !hidden)
        };
        let reason =
            invalid_reason(state, node).unwrap_or_else(|_| "unsuccessful_registration".into());
        let valid = reason.is_empty();
        let changed = {
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            let changed = element.reason != reason;
            element.valid = valid;
            element.reason = reason;
            changed
        };
        if changed && (geo || install || legacy_mode) {
            event(scope, state, node, "validationstatuschange")?;
            count += 1;
            if !current(state) {
                return Ok(count);
            }
            if !state.dom.borrow().is_connected(node).unwrap_or(false) {
                continue;
            }
        }
        if legacy_mode {
            if pending == Some(true)
                && valid
                && state.permission_elements.borrow().elements[&node].current
                    == PermissionState::Prompt
            {
                state
                    .permission_elements
                    .borrow_mut()
                    .elements
                    .get_mut(&node)
                    .unwrap()
                    .pending = None;
                if let Err(error) = permission_prompt::open(scope, state, node, false) {
                    state
                        .permission_elements
                        .borrow_mut()
                        .elements
                        .get_mut(&node)
                        .unwrap()
                        .pending = Some(true);
                    return Err(error);
                }
            } else {
                state
                    .permission_elements
                    .borrow_mut()
                    .elements
                    .get_mut(&node)
                    .unwrap()
                    .pending = None;
            }
            state
                .permission_elements
                .borrow_mut()
                .elements
                .get_mut(&node)
                .unwrap()
                .revision = Some(revision);
            continue;
        }
        if install {
            state
                .permission_elements
                .borrow_mut()
                .elements
                .get_mut(&node)
                .unwrap()
                .revision = Some(revision);
            continue;
        }
        if pending == Some(true)
            && valid
            && state.permission_elements.borrow().elements[&node].current == PermissionState::Prompt
        {
            state
                .permission_elements
                .borrow_mut()
                .elements
                .get_mut(&node)
                .unwrap()
                .pending = None;
            if let Err(error) = permission_prompt::open(scope, state, node, geo) {
                if let Some(element) = state
                    .permission_elements
                    .borrow_mut()
                    .elements
                    .get_mut(&node)
                {
                    element.pending = Some(true);
                }
                return Err(error);
            }
            count += 1;
            if !current(state) {
                return Ok(count);
            }
            continue;
        }
        let continuing = native_media::capture_pending(state, node)
            || state.permission_elements.borrow().elements[&node]
                .captured
                .is_some();
        if pending.is_none()
            && !automatic
            && !continuing
            && state.permission_elements.borrow().elements[&node]
                .location
                .is_none()
        {
            state
                .permission_elements
                .borrow_mut()
                .elements
                .get_mut(&node)
                .unwrap()
                .revision = Some(revision);
            continue;
        }
        if geo {
            if pending == Some(false) || pending == Some(true) && !valid {
                let mut registry = state.permission_elements.borrow_mut();
                let element = registry.elements.get_mut(&node).unwrap();
                element.pending = None;
                element.revision = Some(revision);
                continue;
            }
            if state.permission_elements.borrow().elements[&node]
                .location
                .is_none()
            {
                let location = native_platform::acquire_element_location(scope, state)?;
                let mut registry = state.permission_elements.borrow_mut();
                let element = registry.elements.get_mut(&node).unwrap();
                element.location = Some((location, revision));
                element.operation = Some((generation, stream, authority));
            }
            let (location, location_revision) = state.permission_elements.borrow().elements[&node]
                .location
                .clone()
                .unwrap();
            #[cfg(test)]
            if FAIL_LOCATION_PUBLICATION.with(|flag| flag.replace(false)) {
                return Err(V8BindingError::Allocation(
                    "injected element location publication failure",
                ));
            }
            let (success, value) =
                native_platform::publish_element_location(scope, state, &location)?;
            {
                let mut registry = state.permission_elements.borrow_mut();
                let element = registry.elements.get_mut(&node).unwrap();
                element.autolocated = true;
                element.location = None;
                element.pending = None;
                element.revision = Some(location_revision);
                if success {
                    element.position = Some(v8::Global::new(scope, value));
                    element.error = None;
                } else {
                    element.position = None;
                    element.error =
                        Some(v8::Global::new(scope, v8::Local::<v8::Value>::from(value)));
                }
            }
            event(scope, state, node, "location")?;
            count += 1;
        } else {
            state
                .permission_elements
                .borrow_mut()
                .elements
                .get_mut(&node)
                .unwrap()
                .operation = Some((generation, stream, authority));
            let permission = state.permission_elements.borrow().elements[&node].current;
            {
                let mut registry = state.permission_elements.borrow_mut();
                let element = registry.elements.get_mut(&node).unwrap();
                // A native grant changes the private label and may resize the
                // control. Keep the admitted request pending until it is stable;
                // do not turn the browser's own resize into a capture error.
                // Occlusion, clipping and all authority checks still apply.
                if element.capture_from_prompt
                    && pending == Some(true)
                    && element.reason == "intersection_changed"
                {
                    continue;
                }
                element.capture_from_prompt = false;
            }
            let result = if pending == Some(false) || !valid {
                Err(fail("InvalidStateError"))
            } else if permission != PermissionState::Granted {
                Err(fail("NotAllowedError"))
            } else {
                let captured = state.permission_elements.borrow().elements[&node]
                    .captured
                    .clone();
                if let Some(captured) = captured {
                    Ok(Some(v8::Local::new(scope, &captured)))
                } else {
                    let constraints = state.permission_elements.borrow().elements[&node]
                        .constraints
                        .clone();
                    native_media::capture(scope, state, node, constraints)
                }
            };
            match result {
                Ok(None) => {
                    state
                        .permission_elements
                        .borrow_mut()
                        .elements
                        .get_mut(&node)
                        .unwrap()
                        .pending = None;
                    continue;
                }
                Ok(Some(stream)) => {
                    state
                        .permission_elements
                        .borrow_mut()
                        .elements
                        .get_mut(&node)
                        .unwrap()
                        .captured = Some(v8::Global::new(scope, stream));
                    let track = if camera || microphone {
                        Some(native_media::single_track(
                            scope,
                            state,
                            stream,
                            if camera {
                                crate::media_capture::Kind::Video
                            } else {
                                crate::media_capture::Kind::Audio
                            },
                        )?)
                    } else {
                        None
                    };
                    {
                        let mut registry = state.permission_elements.borrow_mut();
                        let element = registry.elements.get_mut(&node).unwrap();
                        element.pending = None;
                        element.captured = None;
                        element.revision = Some(revision);
                        element.stream = Some(v8::Global::new(scope, stream));
                        element.track = track.map(|track| v8::Global::new(scope, track));
                        element.error = None;
                    }
                    event(
                        scope,
                        state,
                        node,
                        if camera || microphone {
                            "track"
                        } else {
                            "stream"
                        },
                    )?;
                }
                Err(error @ V8BindingError::Allocation(_)) => return Err(error),
                Err(error) => {
                    let message = match &error {
                        V8BindingError::Unsupported { member, .. } => member.clone(),
                        _ => "NotReadableError".into(),
                    };
                    let canceled = (pending == Some(true) || continuing)
                        && valid
                        && message.split(':').next() == Some("NotAllowedError");
                    {
                        let mut registry = state.permission_elements.borrow_mut();
                        let element = registry.elements.get_mut(&node).unwrap();
                        element.pending = None;
                        element.captured = None;
                        element.capture_failure = Some(CaptureFailure {
                            message,
                            canceled,
                            revision,
                        });
                    }
                    publish_capture_failure(scope, state, node)?;
                }
            }
            count += 1;
        }
        if !current(state) {
            break;
        }
    }
    Ok(count)
}
pub(super) fn retire(state: &V8BindingState) {
    let old = std::mem::take(&mut *state.permission_elements.borrow_mut());
    for element in old.elements.into_values() {
        if let Some(publication) = element.publication
            && let Some(notification) = publication.notification
        {
            notification.cancel(state);
        }
    }
}
pub(super) fn ready(state: &V8BindingState, hidden: bool) -> bool {
    let registry = state.permission_elements.borrow();
    if registry
        .retry_at
        .is_some_and(|deadline| deadline > std::time::Instant::now())
    {
        return false;
    }
    let dom = state.dom.borrow();
    registry.elements.values().any(|element| {
        element.publication.is_some()
            || element.location.is_some()
            || element.prompt_result.is_some()
            || element.capture_failure.is_some()
    }) || registry.cursor != Some(dom.mutation_cursor())
        || (!hidden
            && registry.elements.iter().any(|(node, value)| {
                value.pending.is_some()
                    && dom.is_connected(*node).unwrap_or(false)
                    && dom.node_document(*node).ok().flatten() == Some(state.document_root())
            }))
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_retired() {
        return None;
    }
    if let Some(deadline) = state.permission_elements.borrow().retry_at {
        return Some(deadline.saturating_duration_since(std::time::Instant::now()));
    }
    let dom = state.dom.borrow();
    state
        .permission_elements
        .borrow()
        .elements
        .iter()
        .filter(|(node, _)| {
            dom.is_connected(**node).unwrap_or(false)
                && dom.node_document(**node).ok().flatten() == Some(state.document_root())
        })
        .flat_map(|(_, value)| {
            [
                std::time::Duration::from_millis(500).checked_sub(value.attached.elapsed()),
                value.geometry_changed.and_then(|time| {
                    std::time::Duration::from_millis(500).checked_sub(time.elapsed())
                }),
                value.visibility_changed.and_then(|time| {
                    std::time::Duration::from_millis(500).checked_sub(time.elapsed())
                }),
            ]
            .into_iter()
            .flatten()
        })
        .min()
}

#[cfg(test)]
thread_local! {static FAIL_LOCATION_PUBLICATION:Cell<bool>=const{Cell::new(false)};}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_control::{Backend, Coordinates, PositionError, Request, Response};
    use std::sync::{Arc, Mutex};
    struct Transport;
    impl crate::message_wire::MessageTransport for Transport {
        fn request(
            &self,
            _request: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    impl crate::worker_wire::WorkerTransport for Transport {
        fn request(
            &self,
            request: crate::worker_wire::AgentServiceRequest,
        ) -> Result<crate::worker_wire::AgentServiceResponse, crate::worker_wire::WorkerError>
        {
            assert!(matches!(
                request,
                crate::worker_wire::AgentServiceRequest::PublishSecureContext { .. }
            ));
            Ok(crate::worker_wire::AgentServiceResponse::Done)
        }
    }
    struct Platform(Mutex<(bool, u64)>);
    impl Backend for Platform {
        fn request(
            &self,
            _creator: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            let (granted, revision) = *self.0.lock().unwrap();
            Ok(match request {
                Request::BluetoothDevices | Request::BluetoothTestingState => Response::Bluetooth {
                    devices: Vec::new(),
                    testing_enabled: false,
                    pending_responses: 0,
                    allowed: true,
                    reports: Vec::new(),
                },
                Request::InstallEligibility => Response::InstallEligibility {
                    reason: String::new(),
                },
                Request::UiLocaleData { .. } => Response::UiLocaleData { profile: None },
                Request::CspEvents { .. } => Response::CspEvents { events: Vec::new() },
                Request::StorageEvents { .. } => Response::StorageEvents { events: Vec::new() },
                Request::StorageAccess { .. } => Response::StorageAccess { allowed: true },
                Request::Storage { .. } => unreachable!(),
                Request::UiLocale => Response::UiLocale {
                    locale: "en-US".into(),
                },
                Request::Permission { .. } => Response::Permission {
                    state: if granted {
                        PermissionState::Granted
                    } else {
                        PermissionState::Denied
                    },
                    revision,
                },
                Request::PermissionsPolicySnapshot => {
                    Response::PermissionsPolicySnapshot(Default::default())
                }
                Request::WindowPosition => Response::WindowPosition { x: 0, y: 0 },
                Request::DocumentFocus => Response::DocumentFocus { focused: true },
                Request::WindowState => Response::WindowState {
                    hidden: false,
                    revision,
                },
                Request::Geolocation => Response::Geolocation {
                    position: if granted {
                        Ok(Coordinates {
                            latitude: 4.,
                            longitude: 2.,
                            accuracy: 1.,
                            altitude: None,
                            altitude_accuracy: None,
                            heading: None,
                            speed: None,
                        })
                    } else {
                        Err(PositionError {
                            code: 1,
                            message: "denied".into(),
                        })
                    },
                    revision,
                },
                Request::PromptBegin { .. }
                | Request::PromptAnswer { .. }
                | Request::PromptResolve { .. }
                | Request::PromptCancel { .. }
                | Request::RegisterProtocol { .. }
                | Request::UnregisterProtocol { .. }
                | Request::Media { .. } => unreachable!(),
            })
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn legacy_profile_is_frozen_per_realm_and_standard_exposure_is_unchanged() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        for legacy in [true, false] {
            let context = v8::Context::new(scope, Default::default());
            context.set_slot(Rc::new(UserMediaElementProfile { legacy }));
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<usermedia id=u type=camera></usermedia>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(
                scope,
                context,
                dom,
                RealmId(if legacy { 91 } else { 92 }),
                1,
            )
            .unwrap();
            eval(scope, "globalThis.u=document.getElementById('u')");
            if legacy {
                assert!(eval(scope,"HTMLUserMediaElement.isTypeSupported('camera')&&HTMLUserMediaElement.isTypeSupported('microphone')&&!HTMLUserMediaElement.isTypeSupported('invalid')&&u.type==='camera'&&'isValid' in u&&'invalidReason' in u&&u.stream===null&&u.error===null").is_true());
                assert!(
                    eval(
                        scope,
                        "u.type='invalid';!u.isValid&&u.invalidReason==='type_invalid'"
                    )
                    .is_true()
                );
                assert!(eval(scope,"let reason={};try{u.type={toString(){throw reason}}}catch(e){if(e!==reason)throw Error('conversion')}u.type==='invalid'").is_true());
            } else {
                assert!(eval(scope,"typeof HTMLUserMediaElement.isTypeSupported==='undefined'&&!('type' in u)&&!('isValid' in u)&&!('invalidReason' in u)&&typeof u.setConstraints==='function'").is_true());
            }
            context.set_slot(Rc::new(UserMediaElementProfile { legacy: !legacy }));
            assert_eq!(state.usermedia_legacy, legacy);
        }
    }
    #[test]
    fn legacy_granted_activation_validates_without_standard_capture_or_events() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(UserMediaElementProfile { legacy: true }));
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><usermedia id=u type=camera></usermedia></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(93), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        state.set_native_platform_backend(Arc::new(Platform(Mutex::new((true, 1)))));
        let node = state.dom.borrow().find_by_id("u").unwrap();
        ensure(&state, node).unwrap();
        {
            let mut layout = state.layout.borrow_mut();
            layout.viewport = Some((300., 200.));
            layout.geometry.insert(
                node_key(node),
                BindingGeometry::Available(BindingRect {
                    x: 10.,
                    y: 10.,
                    width: 100.,
                    height: 30.,
                }),
            );
            layout.styles.insert(
                node_key(node),
                Arc::new(BTreeMap::from([("display".into(), "inline-block".into())])),
            );
        }
        let old = std::time::Instant::now() - std::time::Duration::from_secs(1);
        state
            .permission_elements
            .borrow_mut()
            .elements
            .get_mut(&node)
            .unwrap()
            .attached = old;
        let _ = invalid_reason(&state, node);
        {
            let mut registry = state.permission_elements.borrow_mut();
            let element = registry.elements.get_mut(&node).unwrap();
            element.geometry_changed = Some(old);
            element.visibility_changed = Some(old);
        }
        eval(
            scope,
            "globalThis.u=document.getElementById('u');globalThis.captureEvents=[];u.onstream=e=>captureEvents.push('stream');u.onerror=e=>captureEvents.push('error');u.oncancel=e=>captureEvents.push('cancel')",
        );
        assert!(invalid_reason(&state, node).unwrap().is_empty());
        activate(&state, node, true).unwrap();
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope,"u.isValid&&u.invalidReason===''&&u.stream===null&&u.error===null&&captureEvents.length===0").is_true());
        assert!(
            state.permission_elements.borrow().elements[&node]
                .pending
                .is_none()
        );
        assert_eq!(
            legacy_prompt_kind(&state, node),
            Some(crate::platform_control::PromptKind::Camera)
        );
        eval(scope, "u.type='microphone'");
        assert_eq!(
            legacy_prompt_kind(&state, node),
            Some(crate::platform_control::PromptKind::Microphone)
        );
        activate(&state, node, false).unwrap();
        native_platform::poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "u.stream===null&&u.error===null&&captureEvents.length===0"
            )
            .is_true()
        );
    }
    #[test]
    fn usermedia_script_focus_requires_native_activation_and_pointer_focus_remains_allowed() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<span tabindex=0 id=s>focus</span><usermedia tabindex=0 id=u></usermedia><camera tabindex=0 id=c></camera><install tabindex=0 id=i></install>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        struct Activation {
            creator: crate::worker_wire::Creator,
            active: Cell<bool>,
            sticky: Cell<bool>,
        }
        impl crate::user_activation::LocalActivationBackend for Activation {
            fn notify(&self, creator: crate::worker_wire::Creator) -> Result<(), String> {
                if creator != self.creator {
                    return Err("foreign test activation creator".into());
                }
                self.active.set(true);
                self.sticky.set(true);
                Ok(())
            }
            fn sticky(&self, creator: crate::worker_wire::Creator) -> Result<bool, String> {
                if creator != self.creator {
                    return Err("foreign test activation creator".into());
                }
                Ok(self.sticky.get())
            }
            fn transient(
                &self,
                creator: crate::worker_wire::Creator,
                consume: bool,
            ) -> Result<bool, String> {
                if creator != self.creator {
                    return Err("foreign test activation creator".into());
                }
                Ok(if consume {
                    self.active.replace(false)
                } else {
                    self.active.get()
                })
            }
        }
        state.set_local_activation_backend(Rc::new(Activation {
            creator: state.native_local_creator_identity().unwrap(),
            active: Cell::new(false),
            sticky: Cell::new(false),
        }));
        assert!(eval(scope,"globalThis.s=document.getElementById('s');globalThis.u=document.getElementById('u');s.focus();u.focus();document.activeElement===s").is_true());
        for id in ["c", "i"] {
            assert!(eval(scope,&format!("s.focus();document.getElementById('{id}').focus();document.activeElement===s")).is_true());
            state.notify_user_activation();
            assert!(eval(scope,&format!("document.getElementById('{id}').focus();document.activeElement===document.getElementById('{id}')")).is_true());
            state.consume_transient_user_activation();
            let node = state.dom.borrow().find_by_id(id).unwrap();
            eval(scope, "s.focus()");
            focus_html_element(scope, &state, node, true).unwrap();
            assert!(
                eval(
                    scope,
                    &format!("document.activeElement===document.getElementById('{id}')")
                )
                .is_true()
            );
        }
        state.notify_user_activation();
        assert!(eval(scope, "u.focus();document.activeElement===u").is_true());
        state.consume_transient_user_activation();
        eval(scope, "s.focus()");
        let node = state
            .dom
            .borrow()
            .query_selector(state.document_root(), "#u")
            .unwrap()
            .unwrap();
        focus_html_element(scope, &state, node, true).unwrap();
        assert!(eval(scope, "document.activeElement===u").is_true());
    }
    #[test]
    fn geolocation_element_retries_same_observation_and_event_listener_prefix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<geolocation id=g></geolocation>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        let platform = Arc::new(Platform(Mutex::new((true, 1))));
        state.set_native_platform_backend(platform.clone());
        native_platform::poll(scope, &state).unwrap();
        eval(
            scope,
            "globalThis.g=document.getElementById('g');globalThis.seen=[];g.addEventListener('location',e=>{globalThis.firstEvent=e;seen.push('once')},{once:true});g.onlocation=e=>seen.push(e===firstEvent&&e.isTrusted?'handler':'wrong');g.autolocate=true",
        );
        FAIL_LOCATION_PUBLICATION.with(|flag| flag.set(true));
        assert!(native_platform::poll(scope, &state).is_err());
        let node = state.dom.borrow().find_by_id("g").unwrap();
        assert!(
            state.permission_elements.borrow().elements[&node]
                .location
                .is_some()
        );
        // A later provider error cannot replace the result already acquired.
        *platform.0.lock().unwrap() = (false, 2);
        native_platform::publication::fail_after_next_callback();
        assert!(native_platform::poll(scope, &state).is_err());
        assert!(
            eval(
                scope,
                "seen.join()==='once'&&g.position.coords.latitude===4&&g.error===null"
            )
            .is_true()
        );
        state.permission_elements.borrow_mut().retry_at = None;
        native_platform::poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "seen.join()==='once,handler'&&g.position.coords.latitude===4"
            )
            .is_true()
        );
        assert!(
            state.permission_elements.borrow().elements[&node]
                .publication
                .is_none()
        );
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope, "seen.length===2").is_true());
    }
    #[test]
    fn capture_cancel_intent_survives_grant_restore_and_preserves_fresh_activation() {
        struct Cancellations {
            calls: Mutex<Vec<&'static str>>,
            base: Platform,
        }
        impl Backend for Cancellations {
            fn request(
                &self,
                creator: crate::worker_wire::Creator,
                request: Request,
            ) -> Result<Response, String> {
                use crate::media_capture::{Request as MediaRequest, Response as MediaResponse};
                if let Request::Media { request } = &request {
                    let mut calls = self.calls.lock().unwrap();
                    let response = match request {
                        MediaRequest::Capture { .. } => {
                            calls.push("capture");
                            MediaResponse::Pending {
                                id: format!(
                                    "capture-{}",
                                    calls.iter().filter(|call| **call == "capture").count()
                                ),
                            }
                        }
                        MediaRequest::PollCapture { .. } => {
                            calls.push("poll");
                            return Err("canceled operation must not resume".into());
                        }
                        MediaRequest::CancelCapture { id } => {
                            assert_eq!(id, "capture-1");
                            calls.push("cancel");
                            if calls.iter().filter(|call| **call == "cancel").count() == 1 {
                                return Err("cancellation ACK lost".into());
                            }
                            MediaResponse::Stopped
                        }
                        _ => return Err("unexpected media request".into()),
                    };
                    return Ok(Response::Media { response });
                }
                self.base.request(creator, request)
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<usermedia id=u></usermedia>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        let backend = Arc::new(Cancellations {
            calls: Mutex::new(Vec::new()),
            base: Platform(Mutex::new((true, 1))),
        });
        state.set_native_platform_backend(backend.clone());
        let node = state.dom.borrow().find_by_id("u").unwrap();
        ensure(&state, node).unwrap();
        assert!(
            native_media::capture(scope, &state, node, serde_json::json!({"video":true}))
                .unwrap()
                .is_none()
        );
        state
            .permission_elements
            .borrow_mut()
            .elements
            .get_mut(&node)
            .unwrap()
            .capture_failure = Some(CaptureFailure {
            message: "NotAllowedError".into(),
            canceled: true,
            revision: 1,
        });
        eval(
            scope,
            "globalThis.cancels=0;document.getElementById('u').oncancel=()=>cancels++",
        );
        assert!(publish_capture_failure(scope, &state, node).is_err());
        assert!(native_media::capture_canceling(&state, node));
        // Even a direct capture attempt under the restored grant only retries
        // cancellation; it cannot issue PollCapture or a replacement Capture.
        assert!(
            native_media::capture(scope, &state, node, serde_json::json!({"video":true})).is_err()
        );
        assert!(!native_media::capture_canceling(&state, node));
        assert_eq!(
            *backend.calls.lock().unwrap(),
            vec!["capture", "cancel", "cancel"]
        );
        activate(&state, node, true).unwrap();
        publish_capture_failure(scope, &state, node).unwrap();
        assert_eq!(
            state.permission_elements.borrow().elements[&node].pending,
            Some(true)
        );
        assert!(eval(scope, "cancels===1").is_true());
        assert!(
            native_media::capture(scope, &state, node, serde_json::json!({"video":true}))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            *backend.calls.lock().unwrap(),
            vec!["capture", "cancel", "cancel", "capture"]
        );
    }
    #[test]
    fn old_stream_acquisition_and_activation_do_not_publish_into_reused_element() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<geolocation id=g autolocate></geolocation>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        state.set_native_platform_backend(Arc::new(Platform(Mutex::new((true, 1)))));
        eval(
            scope,
            "globalThis.g=document.getElementById('g');globalThis.locations=0;g.onlocation=()=>locations++",
        );
        FAIL_LOCATION_PUBLICATION.with(|flag| flag.set(true));
        assert!(native_platform::poll(scope, &state).is_err());
        let node = state.dom.borrow().find_by_id("g").unwrap();
        assert!(
            state.permission_elements.borrow().elements[&node]
                .location
                .is_some()
        );
        activate(&state, node, true).unwrap();
        state.advance_document_stream_epoch().unwrap();
        eval(scope, "g.autolocate=false");
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope, "locations===0&&g.position===null").is_true());
        let registry = state.permission_elements.borrow();
        assert!(registry.elements[&node].location.is_none());
        assert!(registry.elements[&node].pending.is_none());
    }
    #[test]
    fn prompt_activation_follows_retained_delivery_and_retirement_clears_event_flags() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<geolocation id=g></geolocation>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = state.dom.borrow().find_by_id("g").unwrap();
        ensure(&state, node).unwrap();
        eval(
            scope,
            "globalThis.g=document.getElementById('g');globalThis.seen=0;g.addEventListener('promptaction',()=>seen++,{once:true})",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(
            prompt_result(
                scope,
                &state,
                node,
                crate::platform_control::PromptChoice::Allow,
                PermissionState::Granted
            )
            .is_err()
        );
        assert!(
            state.permission_elements.borrow().elements[&node]
                .pending
                .is_none()
        );
        dispatch_pending(scope, &state, node).unwrap();
        assert_eq!(
            state.permission_elements.borrow().elements[&node].pending,
            Some(true)
        );
        assert!(eval(scope, "seen===1").is_true());
        eval(scope, "g.addEventListener('location',()=>seen++)");
        native_platform::publication::fail_after_next_callback();
        assert!(event(scope, &state, node, "location").is_err());
        assert!(
            state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
        retire(&state);
        assert!(
            !state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
        assert!(state.permission_elements.borrow().elements.is_empty());
    }
    #[test]
    fn microphone_constraints_and_errors_use_single_audio_consumer() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<microphone id=m></microphone><camera id=c></camera>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        let platform = Arc::new(Platform(Mutex::new((false, 0))));
        state.set_native_platform_backend(platform.clone());
        eval(
            scope,
            "globalThis.m=document.getElementById('m');globalThis.events=[];m.onerror=e=>events.push(['error',m.error.name,e.isTrusted]);m.oncancel=e=>events.push(['cancel',m.error.name,e.isTrusted]);m.setConstraints({sampleSize:16,echoCancellation:{exact:true},width:1280,frameRate:{ideal:60}})",
        );
        let node = state.dom.borrow().find_by_id("m").unwrap();
        assert_eq!(
            state.permission_elements.borrow().elements[&node].constraints,
            serde_json::json!({"audio":{"sampleSize":16},"video":false})
        );
        assert!(eval(scope,"m instanceof HTMLMicrophoneElement && m.track===null && m.error===null && !(m instanceof HTMLCameraElement)").is_true());
        assert!(eval(scope,"(()=>{const marker={};try{m.setConstraints({get sampleSize(){throw marker}});return false}catch(error){return error===marker}})()").is_true());
        assert_eq!(
            state.permission_elements.borrow().elements[&node].constraints,
            serde_json::json!({"audio":{"sampleSize":16},"video":false})
        );
        eval(scope, "m.click()");
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope,"events.length===1&&events[0][0]==='error'&&events[0][1]==='InvalidStateError'&&events[0][2]&&m.track===null").is_true());
        *platform.0.lock().unwrap() = (true, 1);
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope, "m.matches(':granted')").is_true());
        state
            .permission_elements
            .borrow_mut()
            .elements
            .get_mut(&node)
            .unwrap()
            .capture_failure = Some(CaptureFailure {
            message: "NotAllowedError".into(),
            canceled: true,
            revision: 2,
        });
        publish_capture_failure(scope, &state, node).unwrap();
        assert!(eval(scope,"events.length===2&&events[1][0]==='cancel'&&events[1][1]==='NotAllowedError'&&events[1][2]&&m.track===null").is_true());
    }
    #[test]
    fn parser_elements_consume_native_grants_and_untrusted_activation_never_captures() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<geolocation id=g autolocate></geolocation><usermedia id=u></usermedia><div id=d granted></div>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Transport));
        state.set_worker_transport(Arc::new(Transport), None, None);
        let platform = Arc::new(Platform(Mutex::new((false, 0))));
        state.set_native_platform_backend(platform.clone());
        eval(
            scope,
            "globalThis.g=document.getElementById('g');globalThis.u=document.getElementById('u');globalThis.locations=[];g.onlocation=e=>locations.push(e.isTrusted);globalThis.errors=[];u.onerror=e=>errors.push([e.isTrusted,u.error.name]);u.setConstraints({video:{width:1280,frameRate:{exact:30}},audio:{echoCancellation:{ideal:true}}})",
        );
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope,"g instanceof HTMLGeolocationElement && u instanceof HTMLUserMediaElement && g.autolocate && g.position===null && g.error.code===1 && g.error.message==='User denied Geolocation' && locations[0] && !g.matches(':granted') && !document.getElementById('d').matches(':granted')").is_true());
        *platform.0.lock().unwrap() = (true, 1);
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope,"g.matches(':granted') && u.matches(':granted') && g.permissionStatus==='granted' && g.initialPermissionStatus==='denied' && g.position instanceof GeolocationPosition && g.position.coords.latitude===4 && g.error===null").is_true());
        eval(scope, "u.click()");
        native_platform::poll(scope, &state).unwrap();
        assert!(eval(scope,"u.stream===null && u.error instanceof DOMException && errors.length===1 && errors[0][0] && errors[0][1]==='InvalidStateError'").is_true());
        assert!(eval(scope,"(()=>{const marker={};try{u.setConstraints({get audio(){throw marker}});return false}catch(error){return error===marker}})()").is_true());
        let node = state
            .dom
            .borrow()
            .query_selector(state.document_root(), "#u")
            .unwrap()
            .unwrap();
        assert_eq!(
            state.permission_elements.borrow().elements[&node].constraints["video"]["width"],
            1280
        );
        *platform.0.lock().unwrap() = (false, 2);
        native_platform::poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "!g.matches(':granted') && !u.matches(':granted') && g.permissionStatus==='denied'"
            )
            .is_true()
        );
        eval(
            scope,
            "globalThis.cancel=false;u.oncancel=()=>cancel=true;u.dispatchEvent(new Event('cancel'))",
        );
        assert!(eval(scope, "cancel").is_true());
        state.retire_execution();
        assert!(state.permission_elements.borrow().elements.is_empty());
    }
    #[test]
    fn permission_locality_preserves_retry_and_publishing_fences_and_cursor_scan() {
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
        assert_eq!(
            poll_with_locality(scope, &state, 0, false).unwrap(),
            (0, true)
        );
        assert!(state.permission_elements.borrow().cursor.is_some());
        state.permission_elements.borrow_mut().publishing = true;
        assert_eq!(
            poll_with_locality(scope, &state, 0, false).unwrap(),
            (0, false)
        );
        assert!(state.permission_elements.borrow().publishing);
        state.permission_elements.borrow_mut().publishing = false;
        let later = std::time::Instant::now() + std::time::Duration::from_secs(60);
        state.permission_elements.borrow_mut().retry_at = Some(later);
        assert_eq!(
            poll_with_locality(scope, &state, 0, false).unwrap(),
            (0, false)
        );
        assert_eq!(state.permission_elements.borrow().retry_at, Some(later));
        state.permission_elements.borrow_mut().retry_at = Some(std::time::Instant::now());
        assert_eq!(
            poll_with_locality(scope, &state, 0, false).unwrap(),
            (0, false)
        );
        assert!(state.permission_elements.borrow().retry_at.is_none());
        assert_eq!(
            poll_with_locality(scope, &state, 0, false).unwrap(),
            (0, true)
        );
    }
}
