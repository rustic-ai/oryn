//! Native platform objects. All grants and window/location state stay in the
//! BrowserContext backend; no author globals act as permission authority.
type ContainerRefresh = Rc<dyn Fn(&V8BindingState) -> Result<(), String>>;

type PermissionPolicy = Rc<dyn Fn(&str) -> bool>;

use super::*;
pub(super) mod abort_publication;
pub(super) mod focus_publication;
pub(super) mod publication;
use crate::platform_control::{Backend, PermissionDescriptor, PermissionState, Request, Response};

struct Permission {
    descriptor: PermissionDescriptor,
    state: PermissionState,
}
#[derive(Clone, Copy)]
struct PositionOptions {
    high_accuracy: bool,
    timeout: u32,
    maximum_age: u32,
}
impl Default for PositionOptions {
    fn default() -> Self {
        Self {
            high_accuracy: false,
            timeout: u32::MAX,
            maximum_age: 0,
        }
    }
}
#[derive(Clone)]
pub(super) struct CachedPosition {
    coordinates: crate::platform_control::Coordinates,
    timestamp: f64,
    acquired: std::time::Instant,
    high_accuracy: bool,
}
#[derive(Clone)]
struct PositionTask {
    success: v8::Global<v8::Function>,
    error: Option<v8::Global<v8::Function>>,
    watch: bool,
    revision: Option<u64>,
    options: PositionOptions,
    last: Option<
        Result<crate::platform_control::Coordinates, crate::platform_control::PositionError>,
    >,
    pending: Option<(
        Result<CachedPosition, crate::platform_control::PositionError>,
        u64,
    )>,
}
// A completed publication receipt, never a cached parent authorization/result.
// Weak identities do not prolong backend or transport cleanup ownership.
struct RootRegistrationReceipt {
    creator: crate::worker_wire::Creator,
    transport: std::sync::Weak<dyn crate::message_wire::MessageTransport>,
    backend: std::sync::Weak<dyn Backend>,
}
#[derive(Default)]
pub(super) struct State {
    root_registration: Option<RootRegistrationReceipt>,
    task_retry_at: Option<std::time::Instant>,
    task_retry_attempt: u8,
    unavailable_control_locales: BTreeSet<String>,
    backend: Option<std::sync::Arc<dyn Backend>>,
    permissions_snapshot:
        Option<Rc<dyn Fn() -> Result<crate::permissions_policy::Snapshot, String>>>,
    permissions_policy: Option<PermissionPolicy>,
    permissions_report_only: Option<PermissionPolicy>,
    container_refresh: Option<ContainerRefresh>,
    prototypes: BTreeMap<&'static str, v8::Weak<v8::Object>>,
    permissions: BTreeMap<u64, Permission>,
    objects: BTreeMap<u64, &'static str>,
    positions: BTreeMap<u32, PositionTask>,
    next_position: u32,
    cached: Option<CachedPosition>,
    errors: std::collections::VecDeque<(v8::Global<v8::Function>, v8::Global<v8::Object>)>,
    hidden: bool,
    backend_hidden: bool,
    lifecycle_hidden: bool,
    revision: Option<u64>,
    window_position: Option<(i32, i32)>,
    pub(super) csp_cursor: u64,
    storage_cursor: u64,
    pub(super) publications: std::collections::VecDeque<publication::Notification>,
    pub(super) publishing: bool,
    publication_bytes: usize,
    publication_retry_at: Option<std::time::Instant>,
    publication_retry_attempt: u8,
    aborts: std::collections::VecDeque<abort_publication::Batch>,
    abort_retry: Option<std::time::Instant>,
    abort_depth: usize,
}
pub(super) fn abort_dispatch_active(state: &V8BindingState) -> bool {
    state.native_platform.borrow().abort_depth > 0
}
pub(super) fn window_position(state: &Rc<V8BindingState>) -> Result<(i32, i32), V8BindingError> {
    if state.native_platform.borrow().backend.is_none() {
        return Ok((0, 0));
    }
    match request(state, Request::WindowPosition)? {
        Response::WindowPosition { x, y } => Ok((x, y)),
        _ => Err(failure("window position response mismatch")),
    }
}
pub(super) fn status_listener_active(state: &V8BindingState, id: u64) -> bool {
    state.native_platform.borrow().permissions.contains_key(&id)
        && (state
            .platform_listeners
            .borrow()
            .get(&(id, "change".into()))
            .is_some_and(|listeners| !listeners.is_empty())
            || state
                .platform_handlers
                .borrow()
                .contains_key(&(id, "change".into())))
}
pub(super) fn release_collected_status(state: &V8BindingState, id: u64) {
    let removed = {
        let mut platform = state.native_platform.borrow_mut();
        if platform.permissions.remove(&id).is_some() {
            platform.objects.remove(&id);
            true
        } else {
            false
        }
    };
    if removed {
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }
}
fn failure(message: impl Into<String>) -> V8BindingError {
    let message = message.into();
    let name = message.split(':').next().unwrap_or(&message);
    let member = if matches!(
        name,
        "TypeError"
            | "SecurityError"
            | "SyntaxError"
            | "InvalidStateError"
            | "NotAllowedError"
            | "NotFoundError"
            | "NotReadableError"
            | "QuotaExceededError"
    ) {
        name.to_owned()
    } else {
        message
    };
    V8BindingError::Unsupported {
        interface: "NativePlatform".into(),
        member,
    }
}
pub(super) fn has_backend(state: &V8BindingState) -> bool {
    state.native_platform.borrow().backend.is_some()
}
pub(super) fn initialize_prerender_hidden(state: &V8BindingState) {
    state.native_platform.borrow_mut().hidden = true;
}
pub(super) fn effective_hidden(state: &V8BindingState, backend: bool) -> bool {
    backend || state.native_platform.borrow().lifecycle_hidden || state.document_prerendering()
}
pub(super) fn document_window_focused(state: &V8BindingState) -> Result<bool, V8BindingError> {
    if state.execution_retired() || effective_hidden(state, false) {
        return Ok(false);
    }
    if !has_backend(state) {
        // A standalone JS realm has one implicit window, not a native page group.
        return Ok(!state.native_platform.borrow().backend_hidden);
    }
    match request(state, Request::DocumentFocus)? {
        Response::DocumentFocus { focused } => Ok(focused),
        Response::WindowSuspended | Response::WindowRetired => Ok(false),
        _ => Err(failure("document focus response mismatch")),
    }
}
pub(super) fn visibility_snapshot(
    state: &V8BindingState,
    lifecycle: Option<bool>,
) -> Result<(bool, bool), V8BindingError> {
    // A private prerender or departing document is hidden independently of
    // active backend authority; only a reveal needs a fresh platform sample.
    let forced = lifecycle.unwrap_or_else(|| state.native_platform.borrow().lifecycle_hidden)
        || state.document_prerendering();
    let backend = if forced {
        state.native_platform.borrow().backend_hidden
    } else if has_backend(state) {
        match request(state, Request::WindowState)? {
            Response::WindowState { hidden, .. } => hidden,
            Response::WindowSuspended | Response::WindowRetired => {
                return Err(failure("window visibility authority unavailable"));
            }
            _ => return Err(failure("window state response mismatch")),
        }
    } else {
        state.native_platform.borrow().backend_hidden
    };
    Ok((
        backend,
        backend
            || lifecycle.unwrap_or_else(|| state.native_platform.borrow().lifecycle_hidden)
            || state.document_prerendering(),
    ))
}
pub(super) fn rendering_visible(state: &V8BindingState) -> Result<bool, V8BindingError> {
    if state.document_prerendering() || state.native_platform.borrow().lifecycle_hidden {
        return Ok(false);
    }
    match visibility_snapshot(state, None) {
        Ok((_, hidden)) => Ok(!hidden),
        Err(_) => Ok(false),
    }
}
pub(super) fn request(
    state: &V8BindingState,
    request: Request,
) -> Result<Response, V8BindingError> {
    if state.execution_retired() {
        return Err(failure("InvalidStateError"));
    }
    if matches!(
        &request,
        Request::Permission { .. }
            | Request::Geolocation
            | Request::BluetoothDevices
            | Request::PromptBegin { .. }
            | Request::Media {
                request: crate::media_capture::Request::Capture { .. }
            }
    ) {
        state.refresh_native_container_policy()?;
    }
    let policy = state.native_platform.borrow().permissions_policy.clone();
    if let Some(policy) = policy {
        let allowed = |feature: &str| state.secure_context.get() && policy(feature);
        let report_only = state
            .native_platform
            .borrow()
            .permissions_report_only
            .clone();
        let report = |feature: &str| {
            if !policy(feature) {
                reporting::permission_native(state, feature, "enforce");
            }
            if report_only.as_ref().is_some_and(|check| !check(feature)) {
                reporting::permission_native(state, feature, "report");
            }
        };
        match &request {
            Request::Geolocation => report("geolocation"),
            Request::PromptBegin { kind } => {
                for feature in kind.names() {
                    report(feature)
                }
            }
            Request::Media {
                request: crate::media_capture::Request::Capture { constraints },
            } => {
                for (key, feature) in [("video", "camera"), ("audio", "microphone")] {
                    if constraints.get(key).is_some_and(|value| {
                        value == &serde_json::Value::Bool(true) || value.is_object()
                    }) {
                        report(feature);
                    }
                }
            }
            Request::BluetoothDevices
                if report_only
                    .as_ref()
                    .is_some_and(|check| !check("bluetooth")) =>
            {
                reporting::permission_native(state, "bluetooth", "report");
            }
            _ => {}
        }

        match &request {
            Request::Permission { descriptor } => {
                descriptor.key().map_err(failure)?;
                if matches!(
                    descriptor.name.as_str(),
                    "geolocation"
                        | "camera"
                        | "microphone"
                        | "midi"
                        | "display-capture"
                        | "screen-wake-lock"
                        | "local-fonts"
                        | "window-management"
                        | "accelerometer"
                        | "gyroscope"
                        | "magnetometer"
                        | "clipboard-read"
                        | "clipboard-write"
                ) && !allowed(&descriptor.name)
                {
                    return Ok(Response::Permission {
                        state: PermissionState::Denied,
                        revision: 0,
                    });
                }
            }
            Request::Geolocation if !allowed("geolocation") => {
                return Ok(Response::Geolocation {
                    position: Err(crate::platform_control::PositionError {
                        code: 1,
                        message: "Permissions Policy denied geolocation".into(),
                    }),
                    revision: 0,
                });
            }
            Request::BluetoothDevices if !allowed("bluetooth") => {
                return Ok(Response::Bluetooth {
                    devices: Vec::new(),
                    testing_enabled: false,
                    pending_responses: 0,
                    allowed: false,
                    reports: vec!["enforce".into()],
                });
            }
            Request::PromptBegin { kind }
                if kind.names().iter().any(|feature| !allowed(feature)) =>
            {
                return Err(failure("NotAllowedError"));
            }
            Request::Media {
                request: crate::media_capture::Request::Capture { constraints },
            } => {
                for (key, feature) in [("video", "camera"), ("audio", "microphone")] {
                    if constraints.get(key).is_some_and(|value| {
                        value == &serde_json::Value::Bool(true) || value.is_object()
                    }) && !allowed(feature)
                    {
                        return Err(failure("NotAllowedError"));
                    }
                }
            }
            _ => {}
        }
    }
    let backend = state
        .native_platform
        .borrow()
        .backend
        .clone()
        .ok_or_else(|| failure("native platform backend unavailable"))?;
    let creator = platform_creator(state, &backend)?;
    backend.request(creator, request).map_err(failure)
}

fn platform_creator(
    state: &V8BindingState,
    backend: &std::sync::Arc<dyn Backend>,
) -> Result<crate::worker_wire::Creator, V8BindingError> {
    let local = state.native_local_creator_identity()?;
    if state.document_prerendering()
        || !matches!(
            &local,
            crate::worker_wire::Creator::Window {
                target: crate::message_wire::WindowTarget::Root,
                ..
            }
        )
    {
        state.native_platform.borrow_mut().root_registration = None;
        return state.native_worker_creator();
    }
    let transport = state.message_transport.borrow().clone();
    if let Some(transport) = &transport {
        let platform = state.native_platform.borrow();
        if platform.root_registration.as_ref().is_some_and(|receipt| {
            receipt.creator == local
                && receipt
                    .transport
                    .ptr_eq(&std::sync::Arc::downgrade(transport))
                && receipt.backend.ptr_eq(&std::sync::Arc::downgrade(backend))
        }) {
            // The next backend request still validates this exact lifetime.
            // No WindowState, permission, event cursor or result is reused.
            return Ok(local);
        }
    }
    state.native_platform.borrow_mut().root_registration = None;
    let registered = state.native_worker_creator()?;
    if let Some(transport) = transport
        && registered == local
        && matches!(state.native_local_creator_identity(), Ok(current) if current == local)
        && !state.document_prerendering()
        && state
            .message_transport
            .borrow()
            .as_ref()
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, &transport))
        && state
            .native_platform
            .borrow()
            .backend
            .as_ref()
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, backend))
    {
        state.native_platform.borrow_mut().root_registration = Some(RootRegistrationReceipt {
            creator: registered.clone(),
            transport: std::sync::Arc::downgrade(&transport),
            backend: std::sync::Arc::downgrade(backend),
        });
    }
    Ok(registered)
}
fn publish_security(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    depth: usize,
) -> Result<(), V8BindingError> {
    if depth >= 256 || state.execution_retired() {
        return Err(failure("InvalidStateError"));
    }
    if matches!(
        state.native_worker_creator()?,
        crate::worker_wire::Creator::Window {
            target: crate::message_wire::WindowTarget::Frame(_),
            ..
        }
    ) {
        let global = state
            .global_object
            .to_local(scope)
            .ok_or_else(|| failure("InvalidStateError"))?;
        let parent = window_document::native_parent(scope, global);
        if parent.strict_equals(global.into()) {
            return Err(failure("SecurityError"));
        }
        let context = parent
            .get_creation_context(scope)
            .ok_or_else(|| failure("SecurityError"))?;
        let parent = context
            .get_slot::<V8BindingState>()
            .ok_or_else(|| failure("SecurityError"))?;
        publish_security(scope, &parent, depth + 1)?;
    }
    workers::publish_secure_context(state)
}
pub(super) fn publish_media_security(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    publish_security(scope, state, 0)
}
pub(super) fn prompt_request(
    state: &V8BindingState,
    request: Request,
) -> Result<Response, V8BindingError> {
    self::request(state, request)
}
pub(super) fn media_request(
    state: &V8BindingState,
    request: crate::media_capture::Request,
) -> Result<crate::media_capture::Response, V8BindingError> {
    match self::request(state, Request::Media { request })? {
        Response::Media { response } => Ok(response),
        _ => Err(failure("InvalidStateError")),
    }
}
pub(super) fn element_permission(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    name: &str,
) -> Result<PermissionState, V8BindingError> {
    publish_security(scope, state, 0)?;
    let descriptor = PermissionDescriptor {
        name: name.into(),
        sysex: None,
        pan_tilt_zoom: None,
        allow_without_gesture: None,
    };
    match request(state, Request::Permission { descriptor })? {
        Response::Permission { state, .. } => Ok(state),
        _ => Err(failure("permission response mismatch")),
    }
}
pub(super) type ElementLocation = Result<CachedPosition, crate::platform_control::PositionError>;
pub(super) fn acquire_element_location(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<ElementLocation, V8BindingError> {
    publish_security(scope, state, 0)?;
    match request(state, Request::Geolocation)? {
        Response::Geolocation { position, .. } => Ok(position
            .map(|coordinates| CachedPosition {
                coordinates,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as f64,
                acquired: std::time::Instant::now(),
                high_accuracy: false,
            })
            .map_err(|mut error| {
                if error.code == 1 {
                    error.message = "User denied Geolocation".into();
                }
                error
            })),
        _ => Err(failure("position response mismatch")),
    }
}
pub(super) fn publish_element_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    location: &ElementLocation,
) -> Result<(bool, v8::Local<'s, v8::Object>), V8BindingError> {
    match location {
        Ok(position) => position_object(scope, state, position).map(|value| (true, value)),
        Err(error) => position_error(scope, state, error).map(|value| (false, value)),
    }
}

fn state_for(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    binding_state_for_receiver(scope, object).ok_or_else(|| failure("Illegal invocation"))
}
fn object_id(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    kind: &str,
) -> Result<u64, V8BindingError> {
    let id = stateful_id(scope, object)?;
    if state.native_platform.borrow().objects.get(&id).copied() != Some(kind) {
        return Err(failure("Illegal invocation"));
    }
    Ok(id)
}
fn private<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, name).unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    kind: &'static str,
) -> Result<(u64, v8::Local<'s, v8::Object>), V8BindingError> {
    let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
    let object = new_platform_object(scope, state, "EventTarget", id)?;
    let prototype = state
        .native_platform
        .borrow()
        .prototypes
        .get(kind)
        .cloned()
        .ok_or_else(|| failure("prototype unavailable"))?;
    let prototype = prototype
        .to_local(scope)
        .ok_or_else(|| failure("prototype unavailable"))?;
    object.set_prototype(scope, prototype.into());
    state.native_platform.borrow_mut().objects.insert(id, kind);
    Ok((id, object))
}
fn getter(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    index: i32,
) -> Result<(), V8BindingError> {
    let data = v8::Integer::new(scope, index);
    let get = v8::Function::builder(read)
        .data(data.into())
        .build(scope)
        .ok_or_else(|| failure("getter allocation"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let name = v8::String::new(scope, name).unwrap();
    object.define_property(scope, name.into(), &descriptor);
    Ok(())
}
fn method(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    index: i32,
) -> Result<(), V8BindingError> {
    let length = match index {
        4 | 10 => 2,
        0 | 1 | 2 | 3 | 8 => 1,
        _ => 0,
    };
    let data = v8::Integer::new(scope, index);
    let method = v8::Function::builder(call)
        .data(data.into())
        .length(length)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or_else(|| failure("method allocation"))?;
    let name = v8::String::new(scope, name).unwrap();
    method.set_name(name);
    object.create_data_property(scope, name.into(), method.into());
    Ok(())
}
fn illegal(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    throw_type_error(scope, "Illegal constructor");
}
fn navigator_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(name) = args
        .data()
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let key = private(scope, &name);
    if let Some(value) = args
        .this()
        .get_private(scope, key)
        .filter(|value| value.is_object())
    {
        rv.set(value);
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}
fn geo_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: &str,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let key = private(scope, &format!("oryn.native.{kind}.data"));
    object
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError: incompatible geolocation receiver"))
}
fn geo_wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    kind: &str,
    values: &[v8::Local<'s, v8::Value>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let prototype = state
        .native_platform
        .borrow()
        .prototypes
        .get(kind)
        .cloned()
        .ok_or_else(|| failure("geolocation prototype unavailable"))?;
    let prototype = prototype
        .to_local(scope)
        .ok_or_else(|| failure("geolocation prototype unavailable"))?;
    let object = v8::Object::new(scope);
    object.set_prototype(scope, prototype.into());
    let data = v8::Array::new_with_elements(scope, values);
    let key = private(scope, &format!("oryn.native.{kind}.data"));
    object.set_private(scope, key, data.into());
    Ok(object)
}
const COORDINATE_FIELDS: [&str; 7] = [
    "accuracy",
    "latitude",
    "longitude",
    "altitude",
    "altitudeAccuracy",
    "heading",
    "speed",
];
fn geo_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    position: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let data = geo_data(
        scope,
        object,
        if position {
            "GeolocationPosition"
        } else {
            "GeolocationCoordinates"
        },
    )?;
    let result = v8::Object::new(scope);
    if position {
        let coords = data
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(|| failure("invalid native coordinates"))?;
        let coords = geo_json(scope, coords, false)?;
        let key = v8::String::new(scope, "coords").unwrap();
        result.create_data_property(scope, key.into(), coords.into());
        let timestamp = data
            .get_index(scope, 1)
            .ok_or(V8BindingError::JavaScriptException)?;
        let key = v8::String::new(scope, "timestamp").unwrap();
        result.create_data_property(scope, key.into(), timestamp);
    } else {
        for (index, name) in COORDINATE_FIELDS.iter().enumerate() {
            let value = data
                .get_index(scope, index as u32)
                .ok_or(V8BindingError::JavaScriptException)?;
            let key = v8::String::new(scope, name).unwrap();
            result.create_data_property(scope, key.into(), value);
        }
    }
    Ok(result)
}
fn geo_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    match geo_json(scope, args.this(), args.data().is_true()) {
        Ok(value) => rv.set(value.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn position_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    position: &CachedPosition,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let c = &position.coordinates;
    let heading = if c.speed == Some(0.) { None } else { c.heading };
    let values = [
        Some(c.accuracy),
        Some(c.latitude),
        Some(c.longitude),
        c.altitude,
        c.altitude_accuracy,
        heading,
        c.speed,
    ]
    .into_iter()
    .map(|value| -> v8::Local<v8::Value> {
        match value {
            Some(value) => v8::Number::new(scope, value).into(),
            None => v8::null(scope).into(),
        }
    })
    .collect::<Vec<_>>();
    let coords = geo_wrap(scope, state, "GeolocationCoordinates", &values)?;
    let timestamp = v8::Number::new(scope, position.timestamp);
    geo_wrap(
        scope,
        state,
        "GeolocationPosition",
        &[coords.into(), timestamp.into()],
    )
}
fn position_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    error: &crate::platform_control::PositionError,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let code = v8::Integer::new(scope, error.code as i32);
    let message = v8::String::new(scope, &error.message).unwrap();
    geo_wrap(
        scope,
        state,
        "GeolocationPositionError",
        &[code.into(), message.into()],
    )
}
fn invoke_position_callback(
    scope: &mut v8::PinScope,
    callback: v8::Local<v8::Function>,
    value: v8::Local<v8::Object>,
) {
    let context = callback.get_creation_context(scope);
    let (exception, message) = {
        v8::tc_scope!(let tc,scope);
        let receiver = v8::undefined(tc);
        callback.call(tc, receiver.into(), &[value.into()]);
        let exception = tc.exception();
        let message = tc.message();
        if !tc.is_execution_terminating() {
            tc.reset();
        }
        (exception, message)
    };
    if let Some(exception) = exception
        && !scope.is_execution_terminating()
    {
        if let Some(context) = context {
            let scope = &mut v8::ContextScope::new(scope, context);
            let _ = report_exception(scope, exception, message);
        } else {
            let _ = report_exception(scope, exception, message);
        }
    }
}
fn inactive_position_error(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    error: Option<v8::Local<v8::Function>>,
) -> Result<(), V8BindingError> {
    let Some(callback) = error else { return Ok(()) };
    let Some(context) = callback.get_creation_context(scope) else {
        return Ok(());
    };
    let Some(recipient) = context
        .get_slot::<V8BindingState>()
        .filter(|state| !state.execution_retired())
    else {
        return Ok(());
    };
    if recipient.native_platform.borrow().errors.len() >= 128 {
        return Err(failure("geolocation error task limit"));
    }
    let value = position_error(
        scope,
        state,
        &crate::platform_control::PositionError {
            code: 2,
            message: "Document is not fully active".into(),
        },
    )?;
    recipient.native_platform.borrow_mut().errors.push_back((
        v8::Global::new(scope, callback),
        v8::Global::new(scope, value),
    ));
    Ok(())
}
fn read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let index = args.data().int32_value(scope).unwrap_or(-1);
    if index >= 10 {
        let (kind, index) = if index < 20 {
            ("GeolocationCoordinates", index - 10)
        } else if index < 30 {
            ("GeolocationPosition", index - 20)
        } else {
            ("GeolocationPositionError", index - 30)
        };
        match geo_data(scope, args.this(), kind).and_then(|data| {
            data.get_index(scope, index as u32)
                .ok_or(V8BindingError::JavaScriptException)
        }) {
            Ok(value) => rv.set(value),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    let result = (|| {
        let state = state_for(scope, args.this())?;
        match args.data().int32_value(scope).unwrap_or(-1) {
            0 | 1 => {
                let id = object_id(scope, &state, args.this(), "PermissionStatus")?;
                let values = state.native_platform.borrow();
                let permission = values
                    .permissions
                    .get(&id)
                    .ok_or_else(|| failure("retired permission status"))?;
                v8_string(
                    scope,
                    if args.data().int32_value(scope) == Some(0) {
                        permission.state.as_str()
                    } else {
                        &permission.descriptor.name
                    },
                )
            }
            4 => {
                object_id(scope, &state, args.this(), "Bluetooth")?;
                let Response::Bluetooth {
                    testing_enabled, ..
                } = request(&state, Request::BluetoothTestingState)?
                else {
                    return Err(failure("Bluetooth state mismatch"));
                };
                if !testing_enabled {
                    return Ok(v8::undefined(scope).into());
                }
                let key = private(scope, "oryn.native.bluetooth.testing");
                if let Some(value) = args.this().get_private(scope, key)
                    && value.is_object()
                {
                    return Ok(value);
                }
                let (_, testing) = wrap(scope, &state, "BluetoothTesting")?;
                args.this().set_private(scope, key, testing.into());
                Ok(testing.into())
            }
            2 | 3 => {
                let node = resolve_receiver(scope, &state, args.this(), "Document")?;
                if node != state.document_root() {
                    return Err(failure("Illegal invocation"));
                }
                let hidden = if state.document_prerendering()
                    || state.native_platform.borrow().lifecycle_hidden
                {
                    true
                } else {
                    visibility_snapshot(&state, None)?.1
                };
                if args.data().int32_value(scope) == Some(2) {
                    v8_string(scope, if hidden { "hidden" } else { "visible" })
                } else {
                    Ok(v8::Boolean::new(scope, hidden).into())
                }
            }
            _ => Err(failure("unknown native property")),
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn onchange_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let state = state_for(scope, args.this())?;
        let id = object_id(scope, &state, args.this(), "PermissionStatus")?;
        Ok(platform_listener_graph::handler_get(
            scope, &state, id, "change",
        ))
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn onchange_set(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let result = (|| {
        let state = state_for(scope, args.this())?;
        let id = object_id(scope, &state, args.this(), "PermissionStatus")?;
        platform_listener_graph::handler_set(scope, &state, id, "change", args.get(0))
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}
fn descriptor(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<PermissionDescriptor, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| failure("TypeError: permission descriptor must be an object"))?;
    let name_key = v8::String::new(scope, "name").unwrap();
    let name = object
        .get(scope, name_key.into())
        .and_then(|value| value.to_string(scope))
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let boolean = |scope: &mut v8::PinScope, key: &str| -> Result<Option<bool>, V8BindingError> {
        let key = v8::String::new(scope, key).unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        Ok((!value.is_undefined()).then(|| value.boolean_value(scope)))
    };
    let mut descriptor = PermissionDescriptor {
        name,
        sysex: None,
        pan_tilt_zoom: None,
        allow_without_gesture: None,
    };
    descriptor.key().map_err(failure)?;
    match descriptor.name.as_str() {
        "midi" => descriptor.sysex = boolean(scope, "sysex")?,
        "camera" => descriptor.pan_tilt_zoom = boolean(scope, "panTiltZoom")?,
        "clipboard-read" | "clipboard-write" => {
            descriptor.allow_without_gesture = boolean(scope, "allowWithoutGesture")?
        }
        _ => {}
    }
    Ok(descriptor)
}
fn position_options(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<PositionOptions, V8BindingError> {
    let mut result = PositionOptions::default();
    if value.is_null_or_undefined() {
        return Ok(result);
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| failure("TypeError: options must be a dictionary"))?;
    // Web IDL dictionary conversion reads members in lexicographic order.
    for name in ["enableHighAccuracy", "maximumAge", "timeout"] {
        let key = v8::String::new(scope, name).unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if value.is_undefined() {
            continue;
        }
        if name == "enableHighAccuracy" {
            result.high_accuracy = value.boolean_value(scope);
            continue;
        }
        let number = value
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let clamped = if number.is_nan() || number <= 0. {
            0
        } else if number >= u32::MAX as f64 {
            u32::MAX
        } else {
            number.round_ties_even() as u32
        };
        if name == "maximumAge" {
            result.maximum_age = clamped
        } else {
            result.timeout = clamped
        }
    }
    Ok(result)
}
fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let index = args.data().int32_value(scope).unwrap_or(-1);
    if matches!(index, 5..=9) {
        v8::tc_scope!(let scope,scope);
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            throw_type_error(scope, "Promise allocation failed");
            return;
        };
        let promise = resolver.get_promise(scope);
        rv.set(promise.into());
        let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
            let state = state_for(scope, args.this())?;
            object_id(
                scope,
                &state,
                args.this(),
                if index == 6 {
                    "BluetoothTesting"
                } else {
                    "Bluetooth"
                },
            )?;
            publish_security(scope, &state, 0)?;
            let Response::Bluetooth {
                devices,
                testing_enabled,
                pending_responses,
                allowed,
                reports,
            } = request(
                &state,
                if index == 6 {
                    Request::BluetoothTestingState
                } else {
                    Request::BluetoothDevices
                },
            )?
            else {
                return Err(failure("Bluetooth response mismatch"));
            };
            for disposition in reports {
                reporting::permission(scope, &state, "bluetooth", &disposition)?;
            }
            if index == 7 {
                return Ok(v8::Boolean::new(scope, false).into());
            }
            if index != 6 && !allowed {
                return Err(failure("SecurityError"));
            }
            if matches!(index, 8 | 9) {
                let options = v8::Local::<v8::Object>::try_from(args.get(0))
                    .map_err(|_| failure("TypeError"))?;
                let name = v8::String::new(scope, "filters").unwrap();
                let filters = options
                    .get(scope, name.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
                if filters.is_undefined()
                    || v8::Local::<v8::Array>::try_from(filters)
                        .is_ok_and(|filters| filters.length() == 0)
                {
                    return Err(failure("TypeError"));
                }
                return Err(failure(
                    "NotFoundError: no native Bluetooth device has been selected",
                ));
            }
            if index == 6 {
                if !testing_enabled {
                    return Err(failure("NotAllowedError"));
                }
                return Ok(v8::Boolean::new(scope, pending_responses == 0).into());
            }
            if !devices.is_empty() {
                return Err(failure(
                    "NotSupportedError: native Bluetooth device wrapper unavailable",
                ));
            }
            Ok(v8::Array::new(scope, 0).into())
        })();
        match result {
            Ok(value) => {
                resolver.resolve(scope, value);
            }
            Err(error) => {
                if scope.is_execution_terminating() {
                    return;
                }
                let exception = scope.exception();
                scope.reset();
                if let Some(exception) = exception {
                    resolver.reject(scope, exception);
                    return;
                }
                let name = match &error {
                    V8BindingError::Unsupported { member, .. } => {
                        member.split(':').next().unwrap_or("InvalidStateError")
                    }
                    _ => "InvalidStateError",
                };
                if name == "TypeError" {
                    let text = v8::String::new(scope, "Bluetooth options require nonempty filters")
                        .unwrap();
                    let value = v8::Exception::type_error(scope, text);
                    resolver.reject(scope, value);
                } else if let Ok(value) = dom_exception_value(scope, name) {
                    resolver.reject(scope, value);
                }
            }
        }
        return;
    }
    if index == 0 {
        v8::tc_scope!(let scope,scope);
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            throw_type_error(scope, "Promise allocation failed");
            return;
        };
        let promise = resolver.get_promise(scope);
        rv.set(promise.into());
        let result = (|| {
            let state = state_for(scope, args.this())?;
            object_id(scope, &state, args.this(), "Permissions")?;
            platform_listener_graph::reclaim(&state);
            if state.native_platform.borrow().permissions.len() >= 1024 {
                return Err(failure("permission status limit"));
            }
            let descriptor = descriptor(scope, args.get(0))?;
            publish_security(scope, &state, 0)?;
            let Response::Permission {
                state: permission, ..
            } = request(
                &state,
                Request::Permission {
                    descriptor: descriptor.clone(),
                },
            )?
            else {
                return Err(failure("permission response mismatch"));
            };
            let (id, object) = wrap(scope, &state, "PermissionStatus")?;
            state.native_platform.borrow_mut().permissions.insert(
                id,
                Permission {
                    descriptor,
                    state: permission,
                },
            );
            Ok(object)
        })();
        match result {
            Ok(object) => {
                resolver.resolve(scope, object.into());
            }
            Err(error) => {
                if scope.is_execution_terminating() {
                    return;
                }
                let exception = scope.exception();
                scope.reset();
                let reason = exception.unwrap_or_else(|| {
                    let name = match &error {
                        V8BindingError::Unsupported { member, .. }
                            if matches!(
                                member.as_str(),
                                "SecurityError" | "InvalidStateError" | "NotAllowedError"
                            ) =>
                        {
                            Some(member.as_str())
                        }
                        _ => None,
                    };
                    if let Some(name) = name
                        && let Ok(value) = dom_exception_value(scope, name)
                    {
                        return value;
                    }
                    let text = v8::String::new(scope, &error.to_string()).unwrap();
                    v8::Exception::type_error(scope, text)
                });
                resolver.reject(scope, reason);
            }
        }
        return;
    }
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let state = state_for(scope, args.this())?;
        match index {
            1 | 2 => {
                object_id(scope, &state, args.this(), "Geolocation")?;
                let success = v8::Local::<v8::Function>::try_from(args.get(0))
                    .map_err(|_| failure("TypeError: location success callback is required"))?;
                let error = if args.get(1).is_null_or_undefined() {
                    None
                } else {
                    Some(
                        v8::Local::<v8::Function>::try_from(args.get(1)).map_err(|_| {
                            failure("TypeError: location error callback must be callable")
                        })?,
                    )
                };
                let options = position_options(scope, args.get(2))?;
                if state.execution_retired() {
                    inactive_position_error(scope, &state, error)?;
                    return Ok(if index == 2 {
                        Some(v8::Integer::new(scope, 0).into())
                    } else {
                        None
                    });
                }
                if state.native_platform.borrow().backend.is_none() {
                    return Err(failure("native geolocation backend unavailable"));
                }
                let mut platform = state.native_platform.borrow_mut();
                if platform.positions.len() >= 128 {
                    return Err(failure("geolocation callback limit"));
                }
                platform.next_position = platform
                    .next_position
                    .checked_add(1)
                    .filter(|value| *value <= i32::MAX as u32)
                    .ok_or_else(|| failure("geolocation identity exhausted"))?;
                let id = platform.next_position;
                platform.positions.insert(
                    id,
                    PositionTask {
                        success: v8::Global::new(scope, success),
                        error: error.map(|value| v8::Global::new(scope, value)),
                        watch: index == 2,
                        revision: None,
                        options,
                        last: None,
                        pending: None,
                    },
                );
                Ok(if index == 2 {
                    Some(v8::Integer::new_from_unsigned(scope, id).into())
                } else {
                    None
                })
            }
            3 => {
                object_id(scope, &state, args.this(), "Geolocation")?;
                if args.length() == 0 {
                    return Err(failure("TypeError: clearWatch requires a watch id"));
                }
                let id = args
                    .get(0)
                    .int32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if id > 0 {
                    state
                        .native_platform
                        .borrow_mut()
                        .positions
                        .remove(&(id as u32));
                }
                Ok(None)
            }
            4 | 10 => {
                let key = private(scope, "oryn.native.navigator.platform");
                if args
                    .this()
                    .get_private(scope, key)
                    .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
                    .map(|value| value.u64_value())
                    != Some((state.instance_id, true))
                {
                    return Err(failure("Illegal invocation"));
                }
                if args.length() < 2 {
                    return Err(failure(
                        "TypeError: registerProtocolHandler requires two arguments",
                    ));
                }
                let scheme = args
                    .get(0)
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
                let value = args
                    .get(1)
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
                let scheme = crate::platform_control::protocol_scheme(&scheme)
                    .map_err(|error| failure(&error))?;
                let base = Url::parse(&document_base_url(scope, &state, true)?)
                    .map_err(|_| failure("SyntaxError: invalid document base"))?;
                let encoding = hyperlinks::document_encoding(&state, state.document_root())?;
                let url = hyperlinks::parse(&value, Some(&base), encoding)
                    .map_err(|_| failure("SyntaxError: invalid protocol URL"))?
                    .to_string();
                publish_media_security(scope, &state)?;
                let label = scheme.clone();
                let handler = url.clone();
                let response = request(
                    &state,
                    if index == 4 {
                        Request::RegisterProtocol { scheme, url }
                    } else {
                        Request::UnregisterProtocol { scheme, url }
                    },
                )?;
                if index == 10 {
                    permission_prompt::cancel_protocol(&state, &label, &handler)?;
                }
                if let Response::Prompt(prompt) = response {
                    permission_prompt::open_protocol(scope, &state, prompt, label, handler)?;
                }
                Ok(None)
            }
            _ => Err(failure("unknown native platform method")),
        }
    })();
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or_else(|| failure("platform owner unavailable"))?;
    let global = context.global(scope);
    let name = v8::String::new(scope, "navigator").unwrap();
    let navigator = global
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| failure("Navigator unavailable"))?;
    let owner = v8::BigInt::new_from_u64(scope, state.instance_id);
    let key = private(scope, "oryn.native.navigator.platform");
    navigator.set_private(scope, key, owner.into());
    for kind in [
        "Permissions",
        "PermissionStatus",
        "Geolocation",
        "GeolocationPosition",
        "GeolocationCoordinates",
        "GeolocationPositionError",
        "Bluetooth",
        "BluetoothTesting",
    ] {
        let constructor =
            v8::Function::new(scope, illegal).ok_or_else(|| failure("constructor allocation"))?;
        let name = v8::String::new(scope, "prototype").unwrap();
        let prototype = constructor
            .get(scope, name.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(|| failure("prototype allocation"))?;
        if kind == "PermissionStatus" {
            let event = register_platform_object(&state, PlatformObjectRecord::EventTarget);
            let object = new_platform_object(scope, &state, "EventTarget", event)?;
            let parent = object
                .get_prototype(scope)
                .ok_or_else(|| failure("EventTarget prototype unavailable"))?;
            prototype.set_prototype(scope, parent);
            getter(scope, prototype, "state", 0)?;
            getter(scope, prototype, "name", 1)?;
            let get = v8::Function::new(scope, onchange_get).unwrap();
            let set = v8::Function::new(scope, onchange_set).unwrap();
            let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), set.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let key = v8::String::new(scope, "onchange").unwrap();
            prototype.define_property(scope, key.into(), &descriptor);
        } else if kind == "Bluetooth" {
            method(scope, prototype, "getDevices", 5)?;
            method(scope, prototype, "getAvailability", 7)?;
            method(scope, prototype, "requestDevice", 8)?;
            method(scope, prototype, "requestLEScan", 9)?;
            getter(scope, prototype, "test", 4)?;
        } else if kind == "BluetoothTesting" {
            method(scope, prototype, "allResponsesConsumed", 6)?;
        } else if kind == "Permissions" {
            method(scope, prototype, "query", 0)?;
        } else if kind == "Geolocation" {
            method(scope, prototype, "getCurrentPosition", 1)?;
            method(scope, prototype, "watchPosition", 2)?;
            method(scope, prototype, "clearWatch", 3)?;
        } else if kind == "GeolocationCoordinates" {
            for (index, name) in COORDINATE_FIELDS.iter().enumerate() {
                getter(scope, prototype, name, 10 + index as i32)?;
            }
        } else if kind == "GeolocationPosition" {
            getter(scope, prototype, "coords", 20)?;
            getter(scope, prototype, "timestamp", 21)?;
        } else {
            getter(scope, prototype, "code", 30)?;
            getter(scope, prototype, "message", 31)?;
            for (name, code) in [
                ("PERMISSION_DENIED", 1),
                ("POSITION_UNAVAILABLE", 2),
                ("TIMEOUT", 3),
            ] {
                let key = v8::String::new(scope, name).unwrap();
                let code = v8::Integer::new(scope, code);
                for object in [prototype, v8::Local::<v8::Object>::from(constructor)] {
                    object.define_own_property(
                        scope,
                        key.into(),
                        code.into(),
                        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                    );
                }
            }
        }
        if matches!(kind, "GeolocationPosition" | "GeolocationCoordinates") {
            let data = v8::Boolean::new(scope, kind == "GeolocationPosition");
            let function = v8::Function::builder(geo_to_json)
                .data(data.into())
                .build(scope)
                .unwrap();
            let key = v8::String::new(scope, "toJSON").unwrap();
            prototype.create_data_property(scope, key.into(), function.into());
        }
        let tag = v8::Symbol::get_to_string_tag(scope);
        let name = v8::String::new(scope, kind).unwrap();
        constructor.set_name(name);
        prototype.define_own_property(
            scope,
            tag.into(),
            name.into(),
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        );
        if state.secure_context.get()
            || !matches!(
                kind,
                "GeolocationPosition" | "GeolocationCoordinates" | "Bluetooth" | "BluetoothTesting"
            )
        {
            global.create_data_property(scope, name.into(), constructor.into());
        } else {
            global.delete(scope, name.into());
        }
        let retained = context_intrinsics::retain_object_for(
            scope,
            &state,
            &format!("native_platform-prototype-{kind}"),
            prototype,
        )?;
        state
            .native_platform
            .borrow_mut()
            .prototypes
            .insert(kind, retained);
    }
    for (name, kind) in [
        ("permissions", "Permissions"),
        ("geolocation", "Geolocation"),
        ("bluetooth", "Bluetooth"),
    ] {
        if kind == "Bluetooth" && !state.secure_context.get() {
            continue;
        }
        let (_, object) = wrap(scope, &state, kind)?;
        let key_name = format!("oryn.native.navigator.{name}");
        let key = private(scope, &key_name);
        navigator.set_private(scope, key, object.into());
        let data = v8::String::new(scope, &key_name).unwrap();
        let get = v8::Function::builder(navigator_get)
            .data(data.into())
            .build(scope)
            .unwrap();
        let undefined = v8::undefined(scope);
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), undefined.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        let name = v8::String::new(scope, name).unwrap();
        navigator.define_property(scope, name.into(), &descriptor);
    }
    if state.secure_context.get() {
        if state.binding_global == BindingGlobal::Window {
            super::encrypted_media::install(scope, context, navigator)?;
        }
        method(scope, navigator, "registerProtocolHandler", 4)?;
        method(scope, navigator, "unregisterProtocolHandler", 10)?;
    }
    let document = wrap_node(scope, &state, state.document_root())?;
    let prototype = document
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| failure("Document prototype unavailable"))?;
    getter(scope, prototype, "visibilityState", 2)?;
    getter(scope, prototype, "hidden", 3)?;
    Ok(())
}

// Admit a new platform task only in the document captured by this drain.
// Earlier callbacks may replace/park it without retiring its binding object.
pub(super) fn task_snapshot(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
) -> Option<(bool, u64)> {
    if scope.is_execution_terminating()
        || state.execution_retired()
        || state.execution_tasks_stopped()
        || state.document_generation() != generation
        || state.document_stream_epoch() != epoch
    {
        return None;
    }
    if state.native_platform.borrow().backend.is_none() {
        let platform = state.native_platform.borrow();
        return Some((platform.hidden, platform.revision.unwrap_or(0)));
    }
    match request(state, Request::WindowState) {
        Ok(Response::WindowState { hidden, revision }) => {
            let mut platform = state.native_platform.borrow_mut();
            platform.task_retry_at = None;
            platform.task_retry_attempt = 0;
            Some((hidden, revision))
        }
        Ok(Response::WindowRetired) => {
            state.retire_native_platform();
            None
        }
        _ => {
            let mut platform = state.native_platform.borrow_mut();
            let delay = (25u64 << platform.task_retry_attempt.min(6)).min(1000);
            platform.task_retry_attempt = platform.task_retry_attempt.saturating_add(1);
            platform.task_retry_at =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(delay));
            None
        }
    }
}
fn task_current(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
) -> bool {
    task_snapshot(scope, state, generation, epoch).is_some()
}

// All five polls perform only empty local bookkeeping when these exact
// queues are empty. The entry/abort guard remains immediately before this
// group. A retry, retained suffix or another editing owner keeps the full path.
fn poll_retained_script_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    if !global_events::has_pending_reports(state)
        && post_connection_scripts::queue_empty(state)
        && editing_commands::queue_empty(state)
        && state.range_clone_completions.borrow().is_empty()
        && form_activation::queue_empty(state)
    {
        return Ok(true);
    }
    *count += global_events::poll_reports(scope, state)?;
    *count += post_connection_scripts::poll(scope, state)?;
    *count += editing_commands::poll(scope, state)?;
    *count += range_contents_observable::poll_completions(scope, state)?;
    *count += form_activation::poll(scope, state)?;
    Ok(task_current(scope, state, generation, epoch))
}

fn poll_picture_in_picture_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    let requires_live_check = picture_in_picture::poll_requires_live_check(state);
    // Preserve even the empty poll's retry-clock bookkeeping.
    *count += picture_in_picture::poll(scope, state)?;
    Ok(!requires_live_check || task_current(scope, state, generation, epoch))
}

fn poll_joint_projection_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    match navigation_api::poll_joint_projection(scope, state)? {
        navigation_api::JointProjectionPoll::LocalOnly => Ok(true),
        navigation_api::JointProjectionPoll::CheckAuthority { completed } => {
            *count += completed;
            Ok(task_current(scope, state, generation, epoch))
        }
    }
}

// These four consecutive pollers have pure, local early returns. When all
// are guaranteed idle, there is no callback, mutation or publication between
// their intermediate authority checks. The caller retains the entry live guard; the active path retains its final
// guard. Never reuse authority across a potentially active stage or actor turn.
fn poll_document_task_group(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    if !document_refresh::ready(state)
        && submissions::queue_delay(state).is_none()
        && dialog::delay(state).is_none()
        && fullscreen::delay(state).is_none()
    {
        return Ok(true);
    }
    *count += document_refresh::poll(scope, state)?;
    if !task_current(scope, state, generation, epoch) {
        return Ok(false);
    }
    *count += submissions::poll_queues(scope, state)?;
    if !task_current(scope, state, generation, epoch) {
        return Ok(false);
    }
    *count += dialog::poll(scope, state)?;
    if !task_current(scope, state, generation, epoch) {
        return Ok(false);
    }
    *count += fullscreen::poll(scope, state)?;
    Ok(task_current(scope, state, generation, epoch))
}

// None means all native navigation queues are empty, not merely waiting for
// retry. The caller already checked current authority. An empty poll cannot run
// script or mutate state, so it does not need a second identical admission check.
fn poll_navigation_publications(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    if navigation_api::publication_delay(state).is_none() {
        return Ok(true);
    }
    *count += navigation_api::poll_publications(scope, state)?;
    Ok(task_current(scope, state, generation, epoch))
}
fn poll_navigation_history_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    // Re-read after joint projection: reconciliation can enqueue publications.
    if navigation_api::publication_delay(state).is_none() {
        return Ok(true);
    }
    *count += navigation_api::poll_joint_tasks(scope, state)?;
    if !task_current(scope, state, generation, epoch) {
        return Ok(false);
    }
    *count += navigation_api::poll_hash_tasks(scope, state)?;
    Ok(task_current(scope, state, generation, epoch))
}

// The caller has just checked live authority. An empty abort queue with no
// retry makes abort::poll assign None to an already-None retry and return. Keep
// the publishing barrier even on this exact empty path.
fn poll_abort_publications(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
) -> Result<bool, V8BindingError> {
    let idle = {
        let platform = state.native_platform.borrow();
        platform.aborts.is_empty() && platform.abort_retry.is_none()
    };
    if !idle {
        abort_publication::poll(scope, state)?;
    }
    let publishing = state.native_platform.borrow().publishing;
    Ok(!publishing && (idle || task_current(scope, state, generation, epoch)))
}
fn poll_top_layer_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    // delay(None) implies poll's own immediate return; a future retry is Some
    // and retains the original poll and live check. No count-based inference.
    if top_layer::delay(state).is_none() {
        return Ok(true);
    }
    *count += top_layer::poll(scope, state)?;
    Ok(task_current(scope, state, generation, epoch))
}

// The preceding history-stage guard remains live. Coalesce only a segment
// whose queues are exactly empty and whose retry state is already canonical.
// No callback, RPC, V8 allocation, or publication occurs on this branch. An
// active/retrying segment retains both original authority checks and ordering.
fn poll_publication_report_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    let publication_idle = {
        let platform = state.native_platform.borrow();
        platform.publications.is_empty()
            && !platform.publishing
            && platform.publication_retry_at.is_none()
            && platform.publication_retry_attempt == 0
    };
    if publication_idle && reporting::queues_empty(state) && fetch_later::records_empty(state) {
        return Ok(true);
    }
    *count += publication::drain(scope, state)?;
    if !state.native_platform.borrow().publications.is_empty()
        || !task_current(scope, state, generation, epoch)
    {
        return Ok(false);
    }
    *count += reporting::poll(scope, state)?;
    fetch_later::poll(scope, state)?;
    Ok(task_current(scope, state, generation, epoch))
}

// The caller just obtained hidden from a live task_snapshot. An unchanged
// visibility value performs only local reads; keep all pending-publication
// barriers and revalidate after every actual visibility publication/callback.
fn poll_visibility_publication(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    creator: &crate::worker_wire::Creator,
    hidden: bool,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    let changed = state.native_platform.borrow().hidden != effective_hidden(state, hidden);
    if changed {
        publication::enqueue(
            state,
            creator.clone(),
            publication::Payload::Visibility(hidden),
        )?;
        *count += publication::drain(scope, state)?;
    }
    if state.execution_retired()
        || scope.is_execution_terminating()
        || !state.native_platform.borrow().publications.is_empty()
    {
        return Ok(false);
    }
    Ok(!changed || task_current(scope, state, generation, epoch))
}

// An empty collection cannot issue a Permission RPC or dispatch a callback.
// The caller still obtains its unconditional fresh task_snapshot immediately
// after this group, before revision/geolocation work. Active status work keeps
// its original entry guard and every post-publication guard.
fn poll_permission_status_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    creator: &crate::worker_wire::Creator,
    count: &mut usize,
) -> Result<bool, V8BindingError> {
    if state.native_platform.borrow().permissions.is_empty() {
        return Ok(true);
    }
    if !task_current(scope, state, generation, epoch) {
        return Ok(false);
    }
    // Each PermissionStatus retains its published value. Do not suppress a
    // failed or partially published batch using a shared window revision.
    let statuses = state
        .native_platform
        .borrow()
        .permissions
        .iter()
        .map(|(id, value)| (*id, value.descriptor.clone(), value.state))
        .collect::<Vec<_>>();
    for (id, descriptor, old) in statuses {
        let Response::Permission { state: current, .. } =
            request(state, Request::Permission { descriptor })?
        else {
            return Err(failure("permission response mismatch"));
        };
        if old != current {
            publication::enqueue(
                state,
                creator.clone(),
                publication::Payload::Permission(id, current),
            )?;
            *count += publication::drain(scope, state)?;
            if state.execution_retired()
                || scope.is_execution_terminating()
                || !state.native_platform.borrow().publications.is_empty()
            {
                return Ok(false);
            }
            if !task_current(scope, state, generation, epoch) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// The caller supplies its immediately preceding post-StorageEvents snapshot.
// Reuse it only across an observed local-only permission/media segment; active
// work retains the original fresh check at each boundary, even if count is 0.
fn poll_permission_media_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    current: (bool, u64),
    count: &mut usize,
) -> Result<Option<(bool, u64)>, V8BindingError> {
    let (hidden, revision) = current;
    let (completed, permission_local) = permission_elements::poll_with_locality(
        scope,
        state,
        revision,
        effective_hidden(state, hidden),
    )?;
    *count += completed;
    let current = if permission_local {
        current
    } else {
        let Some(current) = task_snapshot(scope, state, generation, epoch) else {
            return Ok(None);
        };
        current
    };
    let media_local = native_media::poll_is_local(state);
    *count += native_media::poll(scope, state)?;
    if media_local {
        Ok(Some(current))
    } else {
        Ok(task_snapshot(scope, state, generation, epoch))
    }
}

pub fn poll(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> Result<usize, V8BindingError> {
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let retry_pending = state
        .native_platform
        .borrow()
        .task_retry_at
        .is_some_and(|deadline| deadline > std::time::Instant::now());
    if retry_pending || !task_current(scope, state, generation, epoch) {
        return Ok(0);
    }
    if !poll_abort_publications(scope, state, generation, epoch)? {
        return Ok(0);
    }
    let mut count = 0;
    if !poll_retained_script_tasks(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    if !poll_document_task_group(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    if !poll_picture_in_picture_tasks(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    if !poll_top_layer_tasks(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    count += interest::poll(scope, state)?;
    if !task_current(scope, state, generation, epoch) {
        return Ok(count);
    }
    if !poll_navigation_publications(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    if !poll_joint_projection_tasks(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    if !poll_navigation_history_tasks(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    if !poll_publication_report_tasks(scope, state, generation, epoch, &mut count)? {
        return Ok(count);
    }
    while let Some((callback, value)) = { state.native_platform.borrow_mut().errors.pop_front() } {
        let callback = v8::Local::new(scope, &callback);
        let value = v8::Local::new(scope, &value);
        invoke_position_callback(scope, callback, value);
        count += 1;
        if !task_current(scope, state, generation, epoch) {
            return Ok(count);
        }
    }
    count += media_resource::poll(scope, state)?;
    count += text_tracks::poll(scope, state)?;
    count += preload_resources::poll(scope, state)?;
    if state.native_platform.borrow().backend.is_none() {
        return Ok(count);
    }
    // Parked or not-yet-published documents have no current parent authority.
    // Leave their observations pending; never invent a visible state or grant.
    let response = match request(state, Request::WindowState) {
        Ok(response) => response,
        Err(_) => return Ok(count),
    };
    match response {
        Response::WindowState { .. } => (),
        Response::WindowSuspended | Response::WindowRetired => return Ok(count),
        _ => return Err(failure("window state response mismatch")),
    };
    let creator = state.native_worker_creator()?;
    let after = state.native_platform.borrow().csp_cursor;
    if let Response::CspEvents { events } = request(state, Request::CspEvents { after })? {
        let mut previous = after;
        for event in &events {
            if event.id <= previous {
                return Err(failure("CSP event cursor regressed"));
            }
            previous = event.id;
        }
        for event in events {
            publication::enqueue(state, creator.clone(), publication::Payload::Csp(event))?;
        }
        count += publication::drain(scope, state)?;
    } else {
        return Err(failure("CSP event response mismatch"));
    }
    if state.execution_retired()
        || scope.is_execution_terminating()
        || !state.native_platform.borrow().publications.is_empty()
    {
        return Ok(count);
    }
    if !task_current(scope, state, generation, epoch) {
        return Ok(count);
    }
    let after = state.native_platform.borrow().storage_cursor;
    if let Response::StorageEvents { events } = request(state, Request::StorageEvents { after })? {
        let mut previous = after;
        for event in &events {
            if event.id <= previous {
                return Err(failure("storage event cursor regressed"));
            }
            previous = event.id;
        }
        for event in events {
            publication::enqueue(state, creator.clone(), publication::Payload::Storage(event))?;
        }
        count += publication::drain(scope, state)?;
    } else {
        return Err(failure("storage event response mismatch"));
    }
    if state.execution_retired()
        || scope.is_execution_terminating()
        || !state.native_platform.borrow().publications.is_empty()
    {
        return Ok(count);
    }
    let Some(current) = task_snapshot(scope, state, generation, epoch) else {
        return Ok(count);
    };
    let Some((hidden, _)) =
        poll_permission_media_tasks(scope, state, generation, epoch, current, &mut count)?
    else {
        return Ok(count);
    };
    if !poll_visibility_publication(
        scope, state, generation, epoch, &creator, hidden, &mut count,
    )? {
        return Ok(count);
    }
    if (!state.native_platform.borrow().permissions.is_empty()
        || !state.native_platform.borrow().positions.is_empty())
        && publish_security(scope, state, 0).is_err()
    {
        return Ok(count);
    }
    if matches!(
        creator,
        crate::worker_wire::Creator::Window {
            target: crate::message_wire::WindowTarget::Root,
            ..
        }
    ) && let Ok(Response::WindowPosition { x, y }) = request(state, Request::WindowPosition)
    {
        let previous = state.native_platform.borrow().window_position;
        if previous.is_some_and(|previous| previous != (x, y)) {
            publication::enqueue(
                state,
                creator.clone(),
                publication::Payload::WindowMove(x, y),
            )?;
            count += publication::drain(scope, state)?;
            if state.execution_retired()
                || scope.is_execution_terminating()
                || !state.native_platform.borrow().publications.is_empty()
            {
                return Ok(count);
            }
        } else if previous.is_none() {
            state.native_platform.borrow_mut().window_position = Some((x, y));
        }
    }
    if !poll_permission_status_tasks(scope, state, generation, epoch, &creator, &mut count)? {
        return Ok(count);
    }
    let Some(current) = task_snapshot(scope, state, generation, epoch) else {
        return Ok(count);
    };
    let (_, revision) = current;
    state.native_platform.borrow_mut().revision = Some(revision);
    {
        let ids = state
            .native_platform
            .borrow()
            .positions
            .iter()
            .filter(|(_, task)| task.pending.is_some() || task.revision != Some(revision))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in ids {
            let Some(current) = task_snapshot(scope, state, generation, epoch) else {
                return Ok(count);
            };
            let (hidden, revision) = current;
            let Some(mut task) = state.native_platform.borrow().positions.get(&id).cloned() else {
                continue;
            };
            let (observation, observation_revision) = if let Some(pending) = task.pending.clone() {
                pending
            } else {
                let descriptor = PermissionDescriptor {
                    name: "geolocation".into(),
                    sysex: None,
                    pan_tilt_zoom: None,
                    allow_without_gesture: None,
                };
                let permission = match request(state, Request::Permission { descriptor }) {
                    Ok(Response::Permission { state, .. }) => state,
                    Ok(_) => return Err(failure("geolocation permission response mismatch")),
                    Err(_) => {
                        state
                            .native_platform
                            .borrow_mut()
                            .positions
                            .insert(id, task);
                        continue;
                    }
                };
                if effective_hidden(state, hidden) && permission == PermissionState::Granted {
                    task.revision = Some(revision);
                    state
                        .native_platform
                        .borrow_mut()
                        .positions
                        .insert(id, task);
                    continue;
                }
                let cached = state
                    .native_platform
                    .borrow()
                    .cached
                    .clone()
                    .filter(|value| {
                        task.last.is_none()
                            && task.options.maximum_age > 0
                            && value.high_accuracy == task.options.high_accuracy
                            && value.acquired.elapsed()
                                <= std::time::Duration::from_millis(task.options.maximum_age as u64)
                    });
                // Visibility and permission waiting do not consume acquisition time.
                // The parent provider currently returns a terminal observation in
                // its response; a zero timeout must not start that acquisition.
                let observation = if permission != PermissionState::Granted {
                    Err(crate::platform_control::PositionError {
                        code: 1,
                        message: "Geolocation permission is not granted".into(),
                    })
                } else if let Some(cached) = cached {
                    Ok(cached)
                } else if task.options.timeout == 0 {
                    Err(crate::platform_control::PositionError {
                        code: 3,
                        message: "Geolocation acquisition timed out".into(),
                    })
                } else {
                    let started = std::time::Instant::now();
                    let response = match request(state, Request::Geolocation) {
                        Ok(response) => response,
                        Err(_) => {
                            state
                                .native_platform
                                .borrow_mut()
                                .positions
                                .insert(id, task);
                            continue;
                        }
                    };
                    let Response::Geolocation { position, .. } = response else {
                        return Err(failure("position response mismatch"));
                    };
                    if started.elapsed()
                        > std::time::Duration::from_millis(task.options.timeout as u64)
                    {
                        Err(crate::platform_control::PositionError {
                            code: 3,
                            message: "Geolocation acquisition timed out".into(),
                        })
                    } else {
                        position.map(|coordinates| CachedPosition {
                            coordinates,
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as f64,
                            acquired: std::time::Instant::now(),
                            high_accuracy: task.options.high_accuracy,
                        })
                    }
                };
                // Acquisition has completed. Retain this exact observation before
                // any fallible V8 allocation; a retry must not query a new position.
                task.pending = Some((observation.clone(), revision));
                state
                    .native_platform
                    .borrow_mut()
                    .positions
                    .insert(id, task.clone());
                (observation, revision)
            };
            let last = observation
                .as_ref()
                .map(|value| value.coordinates.clone())
                .map_err(Clone::clone);
            let duplicate = task.last.as_ref() == Some(&last);
            let denied = matches!(&last,Err(error) if error.code==1);
            #[cfg(test)]
            if FAIL_POSITION_PUBLICATION.with(|flag| flag.replace(false)) {
                return Err(failure("injected position publication failure"));
            }
            let (callback, value) = match &observation {
                Ok(position) => {
                    state.native_platform.borrow_mut().cached = Some(position.clone());
                    (
                        Some(task.success.clone()),
                        position_object(scope, state, position)?,
                    )
                }
                Err(error) => (task.error.clone(), position_error(scope, state, error)?),
            };
            state.native_platform.borrow_mut().positions.remove(&id);
            task.last = Some(last);
            task.revision = Some(observation_revision);
            task.pending = None;
            if task.watch && !denied {
                state
                    .native_platform
                    .borrow_mut()
                    .positions
                    .insert(id, task);
            }
            if !duplicate && let Some(callback) = callback {
                let callback = v8::Local::new(scope, &callback);
                invoke_position_callback(scope, callback, value);
                count += 1;
            }
            if !task_current(scope, state, generation, epoch) {
                return Ok(count);
            }
        }
    }
    Ok(count)
}
impl V8BindingState {
    pub fn set_native_container_policy_refresh(&self, refresh: ContainerRefresh) {
        self.native_platform.borrow_mut().container_refresh = Some(refresh);
    }
    pub fn refresh_native_container_policy(&self) -> Result<(), V8BindingError> {
        let refresh = self.native_platform.borrow().container_refresh.clone();
        if let Some(refresh) = refresh {
            refresh(self).map_err(failure)?;
        }
        Ok(())
    }
    pub fn native_permissions_policy_allows(&self, feature: &str) -> bool {
        let policy = self.native_platform.borrow().permissions_policy.clone();
        policy.is_none_or(|policy| policy(feature))
    }
    pub fn native_permissions_report_only_allows(&self, feature: &str) -> bool {
        let policy = self
            .native_platform
            .borrow()
            .permissions_report_only
            .clone();
        policy.is_none_or(|policy| policy(feature))
    }
}
impl V8BindingState {
    pub fn set_native_permissions_report_only(&self, policy: PermissionPolicy) {
        self.native_platform.borrow_mut().permissions_report_only = Some(policy);
    }
}
impl V8BindingState {
    pub fn set_native_permissions_policy(&self, policy: PermissionPolicy) {
        self.native_platform.borrow_mut().permissions_policy = Some(policy);
    }
}
impl V8BindingState {
    pub fn set_native_platform_backend(&self, backend: std::sync::Arc<dyn Backend>) {
        let mut platform = self.native_platform.borrow_mut();
        platform.root_registration = None;
        platform.backend = Some(backend);
        platform.task_retry_at = None;
        platform.task_retry_attempt = 0;
        platform.publication_retry_at = None;
        platform.publication_retry_attempt = 0;
    }
}

impl V8BindingState {
    pub fn native_platform_ready(&self) -> bool {
        let platform = self.native_platform.borrow();
        if platform
            .task_retry_at
            .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return false;
        }
        !self.execution_retired()
            && !self.execution_tasks_stopped()
            && (!platform.aborts.is_empty()
                && platform
                    .abort_retry
                    .is_none_or(|deadline| deadline <= std::time::Instant::now())
                || range_contents_observable::completion_delay(self)
                    .is_some_and(|delay| delay.is_zero())
                || editing_commands::delay(self).is_some_and(|delay| delay.is_zero())
                || post_connection_scripts::delay(self).is_some_and(|delay| delay.is_zero())
                || global_events::report_delay(self).is_some_and(|delay| delay.is_zero())
                || form_activation::delay(self).is_some_and(|delay| delay.is_zero())
                || submissions::queue_delay(self).is_some_and(|delay| delay.is_zero())
                || dialog::delay(self).is_some_and(|delay| delay.is_zero())
                || fullscreen::delay(self).is_some_and(|delay| delay.is_zero())
                || picture_in_picture::delay(self).is_some_and(|delay| delay.is_zero())
                || top_layer::delay(self).is_some_and(|delay| delay.is_zero())
                || document_refresh::ready(self)
                || interest::ready(self)
                || navigation_api::publications_ready(self)
                || fetch_later::ready(self)
                || reporting::ready(self)
                || media_resource::ready(self)
                || text_tracks::ready(self)
                || preload_resources::ready(self)
                || !platform.publications.is_empty()
                    && platform
                        .publication_retry_at
                        .is_none_or(|deadline| deadline <= std::time::Instant::now())
                || !platform.errors.is_empty()
                || platform
                    .positions
                    .values()
                    .any(|task| task.pending.is_some() || task.revision.is_none())
                || (platform.backend.is_some()
                    && permission_elements::ready(self, platform.hidden)))
    }
    pub fn native_platform_delay(&self) -> Option<std::time::Duration> {
        if self.execution_tasks_stopped() {
            return None;
        }
        if let Some(deadline) = self
            .native_platform
            .borrow()
            .task_retry_at
            .filter(|deadline| *deadline > std::time::Instant::now())
        {
            return Some(deadline.saturating_duration_since(std::time::Instant::now()));
        }
        let retry = {
            let platform = self.native_platform.borrow();
            if platform.publications.is_empty() {
                None
            } else {
                platform
                    .publication_retry_at
                    .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()))
            }
        };
        permission_elements::delay(self)
            .into_iter()
            .chain(fetch_later::delay(self))
            .chain(native_media::delay(self))
            .chain(interest::delay(self))
            .chain(top_layer::delay(self))
            .chain(fullscreen::delay(self))
            .chain(dialog::delay(self))
            .chain(submissions::queue_delay(self))
            .chain(form_activation::delay(self))
            .chain(global_events::report_delay(self))
            .chain(post_connection_scripts::delay(self))
            .chain(editing_commands::delay(self))
            .chain(range_contents_observable::completion_delay(self))
            .chain(picture_in_picture::delay(self))
            .chain(retry)
            .chain(navigation_api::publication_delay(self))
            .chain(
                self.native_platform
                    .borrow()
                    .abort_retry
                    .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now())),
            )
            .min()
    }
    pub(super) fn retire_native_platform(&self) {
        let _ = preload_resources::clear(self);
        *self.reporting.borrow_mut() = Default::default();
        let mut platform = self.native_platform.borrow_mut();
        platform.aborts.clear();
        platform.abort_retry = None;
        platform.positions.clear();
        platform.errors.clear();
        for notification in platform.publications.drain(..) {
            notification.cancel(self);
        }
        platform.cached = None;
        platform.task_retry_at = None;
        platform.task_retry_attempt = 0;
        platform.publication_retry_at = None;
        platform.publication_retry_attempt = 0;
        permission_prompt::retire(self);
        permission_elements::retire(self);
        native_media::retire(self);
    }
}

#[cfg(test)]
thread_local! {static FAIL_POSITION_PUBLICATION:Cell<bool>=const{Cell::new(false)};}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Messages;
    impl crate::worker_wire::WorkerTransport for Messages {
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
    impl crate::message_wire::MessageTransport for Messages {
        fn request(
            &self,
            request: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            assert!(matches!(
                request,
                crate::message_wire::MessageRequest::RegisterRootDocument { .. }
                    | crate::message_wire::MessageRequest::RetireRootDocument { .. }
            ));
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    struct TestBackend(Mutex<(PermissionState, bool, u64)>);
    impl Backend for TestBackend {
        fn request(
            &self,
            _creator: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            let (permission, hidden, revision) = *self.0.lock().unwrap();
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
                Request::Permission { descriptor } => {
                    descriptor.key()?;
                    Response::Permission {
                        state: permission,
                        revision,
                    }
                }
                Request::PermissionsPolicySnapshot => {
                    Response::PermissionsPolicySnapshot(Default::default())
                }
                Request::WindowPosition => Response::WindowPosition { x: 0, y: 0 },
                Request::DocumentFocus => Response::DocumentFocus { focused: !hidden },
                Request::WindowState => Response::WindowState { hidden, revision },
                Request::Geolocation => Response::Geolocation {
                    position: if permission == PermissionState::Granted {
                        Ok(crate::platform_control::Coordinates {
                            latitude: 50.,
                            longitude: 0.,
                            accuracy: 100.,
                            altitude: None,
                            altitude_accuracy: None,
                            heading: None,
                            speed: None,
                        })
                    } else {
                        Err(crate::platform_control::PositionError {
                            code: 1,
                            message: "denied".into(),
                        })
                    },
                    revision,
                },
                Request::PromptBegin { .. }
                | Request::PromptAnswer { .. }
                | Request::PromptResolve { .. }
                | Request::PromptCancel { .. } => {
                    return Err("prompt fixture not configured".into());
                }
                Request::RegisterProtocol { .. } | Request::UnregisterProtocol { .. } => {
                    Response::ProtocolRegistration { accepted: true }
                }
                Request::Media { .. } => {
                    return Err("NotFoundError: test has no capture provider".into());
                }
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
    struct BluetoothPolicyBackend {
        enforce: bool,
        fallback: TestBackend,
    }
    impl Backend for BluetoothPolicyBackend {
        fn request(
            &self,
            creator: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            if matches!(&request, Request::BluetoothDevices) {
                return Ok(Response::Bluetooth {
                    devices: Vec::new(),
                    testing_enabled: false,
                    pending_responses: 0,
                    allowed: !self.enforce,
                    reports: vec![if self.enforce { "enforce" } else { "report" }.into()],
                });
            }
            self.fallback.request(creator, request)
        }
    }
    #[test]
    fn bluetooth_policy_outcomes_reach_native_reporting_observers_without_changing_promises() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        for enforce in [true, false] {
            state.set_native_platform_backend(Arc::new(BluetoothPolicyBackend {
                enforce,
                fallback: TestBackend(Mutex::new((PermissionState::Prompt, false, 0))),
            }));
            eval(
                scope,
                "globalThis.received=[];globalThis.observer=new ReportingObserver(batch=>received.push(...batch),{types:['permissions-policy-violation']});observer.observe();globalThis.outcome=null;navigator.bluetooth.getDevices().then(v=>outcome=v.length,e=>outcome=e.name)",
            );
            scope.perform_microtask_checkpoint();
            assert!(eval(scope, "received.length===0").is_true());
            reporting::poll(scope, &state).unwrap();
            assert!(eval(scope,if enforce{"outcome==='SecurityError'&&received.length===1&&received[0].body.disposition==='enforce'"}else{"outcome===0&&received.length===1&&received[0].body.disposition==='report'"}).is_true());
            assert!(
                eval(
                    scope,
                    "received[0].body.featureId==='bluetooth'&&navigator.bluetooth.test===undefined"
                )
                .is_true()
            );
            eval(scope, "observer.disconnect()");
        }
    }
    #[test]
    fn local_policy_denies_before_backend_but_keeps_cleanup_routable() {
        struct CleanupOnly;
        impl Backend for CleanupOnly {
            fn request(
                &self,
                _creator: crate::worker_wire::Creator,
                request: Request,
            ) -> Result<Response, String> {
                assert!(matches!(request, Request::WindowState));
                Ok(Response::WindowState {
                    hidden: false,
                    revision: 9,
                })
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        state.set_native_platform_backend(Arc::new(CleanupOnly));
        state.set_native_permissions_policy(Rc::new(|_| false));
        state.set_native_permissions_report_only(Rc::new(|_| false));
        eval(
            scope,
            "globalThis.policyObserver=new ReportingObserver(()=>{throw Error('native reports delivered synchronously')},{types:['permissions-policy-violation']});policyObserver.observe()",
        );
        assert!(matches!(
            request(&state, Request::Geolocation).unwrap(),
            Response::Geolocation {
                position: Err(crate::platform_control::PositionError { code: 1, .. }),
                ..
            }
        ));
        assert!(
            request(
                &state,
                Request::Media {
                    request: crate::media_capture::Request::Capture {
                        constraints: serde_json::json!({"video":true})
                    }
                }
            )
            .is_err()
        );
        assert!(matches!(
            request(&state, Request::WindowState).unwrap(),
            Response::WindowState { revision: 9, .. }
        ));
        assert!(eval(scope,"(()=>{const r=policyObserver.takeRecords();return r.length===4&&r[0].body.featureId==='geolocation'&&r[0].body.disposition==='enforce'&&r[1].body.disposition==='report'&&r[2].body.featureId==='camera'})()").is_true());
    }
    struct GroupBackend {
        inner: TestBackend,
        calls: Mutex<Vec<&'static str>>,
        retired: std::sync::atomic::AtomicBool,
    }
    impl Backend for GroupBackend {
        fn request(
            &self,
            creator: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            self.calls.lock().unwrap().push(match &request {
                Request::WindowState => "window",
                Request::CspEvents { .. } => "csp",
                Request::StorageEvents { .. } => "storage",
                _ => "other",
            });
            if matches!(&request, Request::WindowState) {
                if self.retired.load(std::sync::atomic::Ordering::SeqCst) {
                    return Ok(Response::WindowRetired);
                }
                if PARKED.with(Cell::get) {
                    return Ok(Response::WindowSuspended);
                }
            }
            self.inner.request(creator, request)
        }
    }
    thread_local! {
        static GROUP_TIMERS: RefCell<Vec<(u64, v8::Global<v8::Function>)>> = const { RefCell::new(Vec::new()) };
        static GROUP_PARK: Cell<bool> = const { Cell::new(false) };
        static GROUP_FAIL: Cell<bool> = const { Cell::new(false) };
    }
    fn group_schedule(
        scope: &mut v8::PinScope,
        _state: &V8BindingState,
        callback: v8::Local<v8::Function>,
        delay: u64,
    ) -> Result<(), V8BindingError> {
        if GROUP_FAIL.with(|value| value.replace(false)) {
            return Err(V8BindingError::Allocation(
                "group injected scheduler failure",
            ));
        }
        GROUP_TIMERS.with(|timers| {
            timers
                .borrow_mut()
                .push((delay, v8::Global::new(scope, callback)))
        });
        if GROUP_PARK.with(Cell::get) {
            PARKED.with(|value| value.set(true));
        }
        Ok(())
    }
    fn group_fixture(run: impl FnOnce(&mut v8::PinScope, &Rc<V8BindingState>, &Arc<GroupBackend>)) {
        super::super::tests::initialize_v8();
        PARKED.with(|value| value.set(false));
        GROUP_PARK.with(|value| value.set(false));
        GROUP_FAIL.with(|value| value.set(false));
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://group.test/start").unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        let backend = Arc::new(GroupBackend {
            inner: TestBackend(Mutex::new((PermissionState::Granted, false, 1))),
            calls: Mutex::new(Vec::new()),
            retired: std::sync::atomic::AtomicBool::new(false),
        });
        state.set_native_platform_backend(backend.clone());
        state.set_native_timeout_scheduler(group_schedule);
        run(scope, &state, &backend);
        GROUP_TIMERS.with(|timers| timers.borrow_mut().clear());
        PARKED.with(|value| value.set(false));
    }
    #[test]
    fn document_has_focus_uses_fresh_native_focus_and_rejects_inert_receivers() {
        struct FocusBackend(std::sync::atomic::AtomicUsize);
        impl Backend for FocusBackend {
            fn request(
                &self,
                _: crate::worker_wire::Creator,
                request: Request,
            ) -> Result<Response, String> {
                assert!(matches!(request, Request::DocumentFocus));
                Ok(match self.0.load(std::sync::atomic::Ordering::SeqCst) {
                    0 => Response::DocumentFocus { focused: true },
                    1 => Response::DocumentFocus { focused: false },
                    2 => Response::WindowSuspended,
                    _ => Response::WindowRetired,
                })
            }
        }
        group_fixture(|scope, state, _| {
            let backend = Arc::new(FocusBackend(std::sync::atomic::AtomicUsize::new(0)));
            state.set_native_platform_backend(backend.clone());
            assert!(eval(scope, "document.hasFocus()").is_true());
            for value in [1, 2, 3] {
                backend.0.store(value, std::sync::atomic::Ordering::SeqCst);
                assert!(eval(scope, "document.hasFocus()").is_false());
            }
            backend.0.store(0, std::sync::atomic::Ordering::SeqCst);
            assert!(eval(scope, "document.hasFocus()").is_true());
            assert!(eval(scope, "Document.prototype.hasFocus.call(document.implementation.createHTMLDocument('inert'))").is_false());
            state.retire_execution();
            assert!(eval(scope, "document.hasFocus()").is_false());
        });
    }
    #[test]
    fn idle_permission_media_remove_two_checks_and_refresh_on_next_turn() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let current = task_snapshot(scope, state, generation, epoch).unwrap();
            backend.calls.lock().unwrap().clear();
            permission_elements::poll_with_locality(scope, state, current.1, current.0).unwrap();
            assert!(task_current(scope, state, generation, epoch));
            native_media::poll(scope, state).unwrap();
            assert!(task_snapshot(scope, state, generation, epoch).is_some());
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 2]);
            backend.calls.lock().unwrap().clear();
            let before = std::time::Instant::now();
            let mut count = 0;
            assert_eq!(
                poll_permission_media_tasks(scope, state, generation, epoch, current, &mut count)
                    .unwrap(),
                Some(current)
            );
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            assert!(
                state
                    .native_media
                    .borrow()
                    .retried
                    .is_some_and(|time| time >= before)
            );
            assert!(!state.native_media.borrow().event_polling);
            poll(scope, state).unwrap();
            let calls = backend.calls.lock().unwrap().clone();
            assert!(
                calls.contains(&"window") && calls.contains(&"csp") && calls.contains(&"storage")
            );
            backend.calls.lock().unwrap().clear();
            backend
                .retired
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(poll(scope, state).unwrap(), 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
        });
    }
    #[test]
    fn idle_permission_media_still_discovers_new_connected_controls() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let current = task_snapshot(scope, state, generation, epoch).unwrap();
            let mut count = 0;
            assert!(
                poll_permission_media_tasks(scope, state, generation, epoch, current, &mut count)
                    .unwrap()
                    .is_some()
            );
            eval(
                scope,
                "globalThis.camera=document.createElement('camera');document.body.append(camera)",
            );
            backend.calls.lock().unwrap().clear();
            let (_, local) =
                permission_elements::poll_with_locality(scope, state, current.1, current.0)
                    .unwrap();
            assert!(
                !local,
                "a DOM mutation must be discovered even after an empty poll"
            );
            assert!(backend.calls.lock().unwrap().contains(&"window"));
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_permission_media_tasks(scope, state, generation, epoch, current, &mut count)
                    .unwrap()
                    .is_some()
            );
            assert!(
                backend.calls.lock().unwrap().contains(&"window"),
                "retained permission elements remain conservatively active"
            );
        });
    }
    struct PermissionMediaBackend {
        base: Arc<GroupBackend>,
        retire: bool,
    }
    impl Backend for PermissionMediaBackend {
        fn request(
            &self,
            creator: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            let response = match request {
                Request::PromptCancel { .. } => {
                    self.base.calls.lock().unwrap().push("cancel");
                    Response::PromptCanceled
                }
                Request::Media {
                    request: crate::media_capture::Request::AudioRender { .. },
                } => {
                    self.base.calls.lock().unwrap().push("media");
                    Response::Media {
                        response: crate::media_capture::Response::Stopped,
                    }
                }
                request => return self.base.request(creator, request),
            };
            *self.base.inner.0.lock().unwrap() = (PermissionState::Granted, true, 9);
            if self.retire {
                self.base
                    .retired
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(response)
        }
    }
    #[test]
    fn idle_permission_media_prompt_cancel_refreshes_before_media_even_with_zero_count() {
        for retire in [false, true] {
            group_fixture(|scope, state, backend| {
                let generation = state.document_generation();
                let epoch = state.document_stream_epoch();
                let current = task_snapshot(scope, state, generation, epoch).unwrap();
                state.set_native_platform_backend(Arc::new(PermissionMediaBackend {
                    base: backend.clone(),
                    retire,
                }));
                // A retained prompt whose native presentation is gone must still
                // cancel through the ordinary fixup path before returning.
                state.permission_elements.borrow_mut().prompt =
                    Some((state.document_root(), "held-prompt".into()));
                state.native_media.borrow_mut().retried = None;
                backend.calls.lock().unwrap().clear();
                let mut count = 0;
                let next = poll_permission_media_tasks(
                    scope, state, generation, epoch, current, &mut count,
                )
                .unwrap();
                assert_eq!(count, 0);
                assert!(state.permission_elements.borrow().prompt.is_none());
                assert_eq!(*backend.calls.lock().unwrap(), vec!["cancel", "window"]);
                if retire {
                    assert!(next.is_none());
                    assert!(state.native_media.borrow().retried.is_none());
                } else {
                    assert_eq!(next, Some((true, 9)));
                    assert!(state.native_media.borrow().retried.is_some());
                }
            });
        }
    }
    #[test]
    fn idle_permission_media_cleanup_rpc_refreshes_even_when_no_event_is_dispatched() {
        for retire in [false, true] {
            group_fixture(|scope, state, backend| {
                let generation = state.document_generation();
                let epoch = state.document_stream_epoch();
                let current = task_snapshot(scope, state, generation, epoch).unwrap();
                state.set_native_platform_backend(Arc::new(PermissionMediaBackend {
                    base: backend.clone(),
                    retire,
                }));
                state
                    .native_media
                    .borrow_mut()
                    .audio_cleanup
                    .insert(("owned-audio".into(), 71));
                assert!(!native_media::poll_is_local(state));
                backend.calls.lock().unwrap().clear();
                let mut count = 0;
                let next = poll_permission_media_tasks(
                    scope, state, generation, epoch, current, &mut count,
                )
                .unwrap();
                assert_eq!(count, 0);
                assert!(state.native_media.borrow().audio_cleanup.is_empty());
                assert_eq!(*backend.calls.lock().unwrap(), vec!["media", "window"]);
                assert_eq!(next, if retire { None } else { Some((true, 9)) });
            });
        }
    }
    #[test]
    fn idle_permission_media_reentrant_media_keeps_its_guard_and_fresh_snapshot() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let current = task_snapshot(scope, state, generation, epoch).unwrap();
            state.native_media.borrow_mut().event_polling = true;
            assert!(!native_media::poll_is_local(state));
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert_eq!(
                poll_permission_media_tasks(scope, state, generation, epoch, current, &mut count)
                    .unwrap(),
                Some(current)
            );
            assert_eq!(count, 0);
            assert!(state.native_media.borrow().event_polling);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
            state.native_media.borrow_mut().event_polling = false;
        });
    }

    fn major_idle_history_entry(id: &str) -> crate::navigation_history::NavigationHistorySnapshot {
        crate::navigation_history::NavigationHistorySnapshot {
            scroll: Default::default(),
            key: format!("major-key-{id}"),
            id: id.into(),
            url: format!("https://group.test/{id}"),
            document_id: "major-document".into(),
            state: None,
            history_state: None,
        }
    }

    #[test]
    fn major_idle_prefix_and_pip_keep_fresh_next_poll_authority() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            assert!(task_current(scope, state, generation, epoch));
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                poll_retained_script_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert!(
                poll_picture_in_picture_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            poll(scope, state).unwrap();
            let calls = backend.calls.lock().unwrap().clone();
            assert!(
                calls.contains(&"window") && calls.contains(&"csp") && calls.contains(&"storage")
            );
            backend.calls.lock().unwrap().clear();
            backend
                .retired
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(poll(scope, state).unwrap(), 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
        });
    }

    #[test]
    fn major_idle_report_suffix_retains_retry_callback_guard_and_no_replay() {
        group_fixture(|scope, state, backend| {
            fn park(_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
                PARKED.with(|flag| flag.set(true));
            }
            let callback = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkMajorReport").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), callback.into());
            eval(
                scope,
                "globalThis.majorReport=[];addEventListener('error',()=>majorReport.push('first'),{once:true});addEventListener('error',()=>{majorReport.push('tail');parkMajorReport()},{once:true});onerror=()=>{majorReport.push('handler');return true}",
            );
            let message = v8::String::new(scope, "retained major report").unwrap();
            let error = v8::Exception::type_error(scope, message);
            publication::fail_after_next_callback();
            assert!(global_events::report_exception(scope, error, None).is_err());
            assert!(global_events::has_pending_reports(state));
            assert!(eval(scope, "majorReport.join()==='first'").is_true());
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let mut count = 0;
            backend.calls.lock().unwrap().clear();
            // The exact unfinished error report remains queued, independently
            // of whether its retry deadline becomes due during this test.
            assert!(global_events::report_delay(state).is_some());
            global_events::retry_reports_now(state);
            backend.calls.lock().unwrap().clear();
            assert!(
                !poll_retained_script_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert!(eval(scope, "majorReport.filter(x=>x==='first').length===1&&majorReport.filter(x=>x==='tail').length===1").is_true());
            assert!(backend.calls.lock().unwrap().contains(&"window"));
            PARKED.with(|flag| flag.set(false));
            global_events::retry_reports_now(state);
            assert!(
                poll_retained_script_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert!(!global_events::has_pending_reports(state));
            assert!(eval(scope, "majorReport.join()==='first,tail,handler'").is_true());
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_retained_script_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert!(backend.calls.lock().unwrap().is_empty());
            assert!(eval(scope, "majorReport.join()==='first,tail,handler'").is_true());
        });
    }

    #[test]
    fn major_idle_projection_reconciles_canceled_receipt_without_query() {
        group_fixture(|scope, state, backend| {
            navigation_api::sync_history(
                scope,
                vec![major_idle_history_entry("a"), major_idle_history_entry("b")],
                1,
                "push",
            )
            .unwrap();
            let route = state.joint_history_route();
            let group = state.joint_history.borrow().clone();
            let plan = group.borrow().plan(-1).unwrap().unwrap();
            let receipt = group.borrow_mut().begin_execution(&plan).unwrap();
            assert!(group.borrow_mut().commit(&plan).unwrap());
            assert_eq!(group.borrow().selected(&route).unwrap().id, "a");
            receipt.cancel();
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                poll_joint_projection_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(group.borrow().selected(&route).unwrap().id, "b");
            assert_eq!(
                receipt.poll(),
                Some(Err(crate::joint_history::JointTraversalFailure::Canceled))
            );
            group.borrow().validate().unwrap();
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn major_idle_projection_dispose_keeps_callback_guard_and_no_replay() {
        group_fixture(|scope, state, backend| {
            fn park(_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
                PARKED.with(|flag| flag.set(true));
            }
            let callback = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkMajorDispose").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), callback.into());
            navigation_api::sync_history(
                scope,
                vec![major_idle_history_entry("a"), major_idle_history_entry("b")],
                0,
                "push",
            )
            .unwrap();
            eval(
                scope,
                "globalThis.majorDisposes=0;globalThis.majorDisposed=navigation.entries()[1];majorDisposed.addEventListener('dispose',()=>{majorDisposes++;parkMajorDispose()},{once:true})",
            );
            let route = state.joint_history_route();
            let group = state.joint_history.borrow().clone();
            group
                .borrow_mut()
                .attach("major-child".into(), major_idle_history_entry("child-a"))
                .unwrap();
            group
                .borrow_mut()
                .push("major-child", major_idle_history_entry("child-b"))
                .unwrap();
            assert!(!group.borrow().knows(&route, "b"));
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                !poll_joint_projection_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert_eq!(count, 1);
            assert!(backend.calls.lock().unwrap().contains(&"window"));
            assert!(eval(scope, "majorDisposes===1&&majorDisposed.index===-1").is_true());
            PARKED.with(|flag| flag.set(false));
            navigation_api::hold_publications_for_test(state, false);
            assert!(
                poll_navigation_publications(scope, state, generation, epoch, &mut count).unwrap()
            );
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_joint_projection_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert!(backend.calls.lock().unwrap().is_empty());
            assert!(eval(scope, "majorDisposes===1").is_true());
        });
    }

    #[test]
    fn major_idle_projection_pending_publication_keeps_authority_check() {
        group_fixture(|scope, state, backend| {
            navigation_api::hold_publications_for_test(state, true);
            let _receipt =
                navigation_api::start_native_document_lifecycle_publication(scope, false).unwrap();
            assert!(navigation_api::document_lifecycle_publication_pending(
                state
            ));
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                poll_joint_projection_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
            assert!(navigation_api::document_lifecycle_publication_pending(
                state
            ));
        });
    }

    #[test]
    fn remaining_idle_guards_elide_only_local_noops_and_query_again_next_poll() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let creator = state.native_worker_creator().unwrap();
            let (hidden, _) = task_snapshot(scope, state, generation, epoch).unwrap();
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                poll_visibility_publication(
                    scope, state, generation, epoch, &creator, hidden, &mut count
                )
                .unwrap()
            );
            assert!(
                poll_permission_status_tasks(scope, state, generation, epoch, &creator, &mut count)
                    .unwrap()
            );
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            // Neither the entry check nor independently polled remote queues
            // are removed, and no authorization survives the next poll.
            poll(scope, state).unwrap();
            let calls = backend.calls.lock().unwrap().clone();
            assert!(
                calls.contains(&"window") && calls.contains(&"csp") && calls.contains(&"storage")
            );
            backend.calls.lock().unwrap().clear();
            backend
                .retired
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(poll(scope, state).unwrap(), 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
        });
    }

    #[test]
    fn remaining_idle_visibility_preserves_pending_publication_barrier() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let creator = state.native_worker_creator().unwrap();
            let (hidden, _) = task_snapshot(scope, state, generation, epoch).unwrap();
            publication::enqueue(
                state,
                creator.clone(),
                publication::Payload::Visibility(!hidden),
            )
            .unwrap();
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                !poll_visibility_publication(
                    scope, state, generation, epoch, &creator, hidden, &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 0);
            assert!(!state.native_platform.borrow().publications.is_empty());
            assert!(backend.calls.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn remaining_idle_visibility_callback_parking_keeps_guard_and_no_replay() {
        group_fixture(|scope, state, backend| {
            fn park(_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
                PARKED.with(|flag| flag.set(true));
            }
            let callback = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkRemainingVisibility").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), callback.into());
            eval(
                scope,
                "globalThis.remainingVisibilityCalls=0;document.addEventListener('visibilitychange',()=>{remainingVisibilityCalls++;parkRemainingVisibility()})",
            );
            *backend.inner.0.lock().unwrap() = (PermissionState::Granted, true, 2);
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let creator = state.native_worker_creator().unwrap();
            let (hidden, _) = task_snapshot(scope, state, generation, epoch).unwrap();
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                !poll_visibility_publication(
                    scope, state, generation, epoch, &creator, hidden, &mut count
                )
                .unwrap()
            );
            assert!(eval(scope, "remainingVisibilityCalls===1").is_true());
            assert!(backend.calls.lock().unwrap().contains(&"window"));
            // Resume the already-owned publication suffix after unpark; do not
            // start another visibility transition or replay its callback.
            PARKED.with(|flag| flag.set(false));
            state.native_platform.borrow_mut().publication_retry_at = None;
            state.native_platform.borrow_mut().task_retry_at = None;
            count += publication::drain(scope, state).unwrap();
            assert!(state.native_platform.borrow().publications.is_empty());
            let (hidden, _) = task_snapshot(scope, state, generation, epoch).unwrap();
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_visibility_publication(
                    scope, state, generation, epoch, &creator, hidden, &mut count
                )
                .unwrap()
            );
            assert!(backend.calls.lock().unwrap().is_empty());
            assert!(eval(scope, "remainingVisibilityCalls===1").is_true());
        });
    }

    #[test]
    fn remaining_idle_permission_status_keeps_entry_and_callback_guards() {
        group_fixture(|scope, state, backend| {
            fn park(_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
                PARKED.with(|flag| flag.set(true));
            }
            let callback = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkRemainingPermission").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), callback.into());
            eval(
                scope,
                "globalThis.remainingPermissionCalls=0;navigator.permissions.query({name:'geolocation'}).then(status=>{globalThis.remainingStatus=status;status.onchange=()=>{remainingPermissionCalls++;parkRemainingPermission()}})",
            );
            scope.perform_microtask_checkpoint();
            assert_eq!(state.native_platform.borrow().permissions.len(), 1);
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let creator = state.native_worker_creator().unwrap();
            let mut count = 0;
            PARKED.with(|flag| flag.set(true));
            backend.calls.lock().unwrap().clear();
            assert!(
                !poll_permission_status_tasks(
                    scope, state, generation, epoch, &creator, &mut count
                )
                .unwrap()
            );
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
            assert!(eval(scope, "remainingPermissionCalls===0").is_true());
            PARKED.with(|flag| flag.set(false));
            *backend.inner.0.lock().unwrap() = (PermissionState::Denied, false, 2);
            backend.calls.lock().unwrap().clear();
            assert!(
                !poll_permission_status_tasks(
                    scope, state, generation, epoch, &creator, &mut count
                )
                .unwrap()
            );
            let calls = backend.calls.lock().unwrap().clone();
            assert_eq!(calls.first(), Some(&"window"));
            assert!(calls.iter().filter(|call| **call == "window").count() >= 2);
            assert!(
                eval(
                    scope,
                    "remainingPermissionCalls===1 && remainingStatus.state==='denied'"
                )
                .is_true()
            );
            PARKED.with(|flag| flag.set(false));
            state.native_platform.borrow_mut().publication_retry_at = None;
            state.native_platform.borrow_mut().task_retry_at = None;
            count += publication::drain(scope, state).unwrap();
            assert!(state.native_platform.borrow().publications.is_empty());
            assert!(
                poll_permission_status_tasks(scope, state, generation, epoch, &creator, &mut count)
                    .unwrap()
            );
            assert!(eval(scope, "remainingPermissionCalls===1").is_true());
        });
    }

    #[test]
    fn remaining_idle_status_group_keeps_fresh_query_after_position_rpc() {
        struct RetireAfterPosition {
            inner: Arc<GroupBackend>,
            position_seen: std::sync::atomic::AtomicBool,
        }
        impl Backend for RetireAfterPosition {
            fn request(
                &self,
                creator: crate::worker_wire::Creator,
                request: Request,
            ) -> Result<Response, String> {
                let position = matches!(&request, Request::WindowPosition);
                let response = self.inner.request(creator, request)?;
                if position {
                    self.position_seen
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    self.inner
                        .retired
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(response)
            }
        }
        group_fixture(|scope, state, backend| {
            let replacement = Arc::new(RetireAfterPosition {
                inner: backend.clone(),
                position_seen: std::sync::atomic::AtomicBool::new(false),
            });
            state.set_native_platform_backend(replacement.clone());
            assert!(state.native_platform.borrow().permissions.is_empty());
            assert!(state.native_platform.borrow().revision.is_none());
            backend.calls.lock().unwrap().clear();
            poll(scope, state).unwrap();
            assert!(
                replacement
                    .position_seen
                    .load(std::sync::atomic::Ordering::SeqCst)
            );
            assert_eq!(backend.calls.lock().unwrap().last(), Some(&"window"));
            assert!(
                state.native_platform.borrow().revision.is_none(),
                "the post-RPC retirement must stop revision/geolocation publication"
            );
        });
    }

    #[test]
    fn adjacent_idle_segments_remove_three_window_queries_without_caching_next_poll() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            let mut count = 0;
            // Previous idle boundaries: one post-document and two post-publication/report.
            assert!(poll_document_task_group(scope, state, generation, epoch, &mut count).unwrap());
            assert!(task_current(scope, state, generation, epoch));
            count += publication::drain(scope, state).unwrap();
            assert!(task_current(scope, state, generation, epoch));
            count += reporting::poll(scope, state).unwrap();
            fetch_later::poll(scope, state).unwrap();
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 3]);
            backend.calls.lock().unwrap().clear();
            assert!(poll_document_task_group(scope, state, generation, epoch, &mut count).unwrap());
            assert!(
                poll_publication_report_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            poll(scope, state).unwrap();
            let calls = backend.calls.lock().unwrap().clone();
            assert!(
                calls.contains(&"window") && calls.contains(&"csp") && calls.contains(&"storage")
            );
            backend.calls.lock().unwrap().clear();
            backend
                .retired
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(poll(scope, state).unwrap(), 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
        });
    }
    #[test]
    fn adjacent_publication_retry_keeps_original_clear_and_both_live_checks() {
        group_fixture(|scope, state, backend| {
            {
                let mut platform = state.native_platform.borrow_mut();
                platform.publication_retry_at =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
                platform.publication_retry_attempt = 2;
            }
            let mut count = 0;
            assert!(
                poll_publication_report_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            let platform = state.native_platform.borrow();
            assert!(platform.publication_retry_at.is_none());
            assert_eq!(platform.publication_retry_attempt, 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 2]);
            assert_eq!(count, 0);
        });
    }
    #[test]
    fn adjacent_reporting_callback_parking_stops_before_next_platform_stage() {
        group_fixture(|scope, state, backend| {
            fn park(
                _scope: &mut v8::PinScope,
                _args: v8::FunctionCallbackArguments,
                _rv: v8::ReturnValue,
            ) {
                PARKED.with(|value| value.set(true));
            }
            let callback = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkReportDocument").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), callback.into());
            eval(
                scope,
                "globalThis.reportCalls=0;globalThis.idleGuardObserver=new ReportingObserver(()=>{reportCalls++;parkReportDocument()});idleGuardObserver.observe()",
            );
            reporting::permission_native(state, "camera", "enforce");
            assert!(!reporting::queues_empty(state));
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                !poll_publication_report_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 1);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 2]);
            assert!(eval(scope, "reportCalls===1").is_true());
            assert!(state.native_platform.borrow().task_retry_at.is_some());
            PARKED.with(|value| value.set(false));
        });
    }

    #[test]
    fn adjacent_active_publication_keeps_guard_before_reporting_callback() {
        group_fixture(|scope, state, backend| {
            fn park(
                _scope: &mut v8::PinScope,
                _args: v8::FunctionCallbackArguments,
                _rv: v8::ReturnValue,
            ) {
                PARKED.with(|value| value.set(true));
            }
            let callback = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkVisibilityDocument").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), callback.into());
            eval(
                scope,
                "globalThis.visibilityCalls=0;globalThis.reportCalls=0;document.addEventListener('visibilitychange',()=>{visibilityCalls++;parkVisibilityDocument()});globalThis.pendingObserver=new ReportingObserver(()=>reportCalls++);pendingObserver.observe()",
            );
            reporting::permission_native(state, "camera", "enforce");
            let creator = state.native_worker_creator().unwrap();
            publication::enqueue(state, creator, publication::Payload::Visibility(true)).unwrap();
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                !poll_publication_report_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert!(eval(scope, "visibilityCalls===1 && reportCalls===0").is_true());
            assert!(!reporting::queues_empty(state));
            assert!(state.native_platform.borrow().task_retry_at.is_some());
            assert!(backend.calls.lock().unwrap().contains(&"window"));
            PARKED.with(|value| value.set(false));
        });
    }
    #[test]
    fn idle_document_group_elides_three_queries_but_full_poll_keeps_live_observations() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            // Execute the previous four-stage idle path as a counting reference.
            assert_eq!(document_refresh::poll(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(submissions::poll_queues(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(dialog::poll(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(fullscreen::poll(scope, state).unwrap(), 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 3]);
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(poll_document_task_group(scope, state, generation, epoch, &mut count).unwrap());
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            poll(scope, state).unwrap();
            let calls = backend.calls.lock().unwrap().clone();
            assert!(
                calls.contains(&"window") && calls.contains(&"csp") && calls.contains(&"storage")
            );
            *backend.inner.0.lock().unwrap() = (PermissionState::Granted, true, 2);
            poll(scope, state).unwrap();
            assert!(state.native_platform.borrow().hidden);
            backend.calls.lock().unwrap().clear();
            backend
                .retired
                .store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(poll(scope, state).unwrap(), 0);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
        });
    }
    #[test]
    fn active_document_group_revalidates_after_refresh_and_preserves_parked_retry() {
        group_fixture(|scope, state, backend| {
            state.document_completely_loaded.set(true);
            document_refresh::schedule(scope, state, "2;url=/next").unwrap();
            GROUP_PARK.with(|value| value.set(true));
            let mut count = 0;
            assert!(
                !poll_document_task_group(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 1);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
            assert!(state.native_platform.borrow().task_retry_at.is_some());
            GROUP_TIMERS.with(|timers| assert_eq!(timers.borrow()[0].0, 2000));
            assert_eq!(poll(scope, state).unwrap(), 0);
            GROUP_TIMERS.with(|timers| assert_eq!(timers.borrow().len(), 1));
            PARKED.with(|value| value.set(false));
            GROUP_PARK.with(|value| value.set(false));
            state.set_native_platform_backend(backend.clone());
            poll(scope, state).unwrap();
            GROUP_TIMERS.with(|timers| assert_eq!(timers.borrow().len(), 1));
        });
    }
    #[test]
    fn active_document_group_preserves_scheduler_error_and_retry_without_replay() {
        group_fixture(|scope, state, backend| {
            state.document_completely_loaded.set(true);
            document_refresh::schedule(scope, state, "2;url=/next").unwrap();
            GROUP_FAIL.with(|value| value.set(true));
            let mut count = 0;
            assert!(matches!(
                poll_document_task_group(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                ),
                Err(V8BindingError::Allocation(
                    "group injected scheduler failure"
                ))
            ));
            assert_eq!(count, 0);
            assert!(document_refresh::ready(state));
            assert!(backend.calls.lock().unwrap().is_empty());
            assert!(
                poll_document_task_group(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 1);
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 4]);
            assert!(!document_refresh::ready(state));
            assert!(
                poll_document_task_group(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            GROUP_TIMERS.with(|timers| assert_eq!(timers.borrow().len(), 1));
        });
    }

    #[test]
    fn empty_navigation_queues_elide_three_checks_without_skipping_projection_or_native_drain() {
        group_fixture(|scope, state, backend| {
            assert!(navigation_api::publication_delay(state).is_none());
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            assert_eq!(navigation_api::poll_publications(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(navigation_api::poll_joint_tasks(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(navigation_api::poll_hash_tasks(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 3]);
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                poll_navigation_publications(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert!(
                poll_navigation_history_tasks(scope, state, generation, epoch, &mut count).unwrap()
            );
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            // Native publication cleanup remains unconditional even if its queue is empty.
            state.native_platform.borrow_mut().publication_retry_at =
                Some(std::time::Instant::now());
            state.native_platform.borrow_mut().publication_retry_attempt = 3;
            poll(scope, state).unwrap();
            assert!(
                state
                    .native_platform
                    .borrow()
                    .publication_retry_at
                    .is_none()
            );
            assert_eq!(state.native_platform.borrow().publication_retry_attempt, 0);
            let calls = backend.calls.lock().unwrap();
            assert!(
                calls.contains(&"window") && calls.contains(&"csp") && calls.contains(&"storage")
            );
        });
    }
    #[test]
    fn queued_hash_tasks_keep_order_and_are_not_replayed_by_empty_fast_path() {
        group_fixture(|scope, state, backend| {
            state.commit_location("about:blank").unwrap();
            state.set_security_origin("null".into());
            navigation_api::sync_history(
                scope,
                vec![crate::navigation_history::NavigationHistorySnapshot {
                    scroll: Default::default(),
                    key: "key-opaque".into(),
                    id: "opaque".into(),
                    url: "about:blank".into(),
                    document_id: "retained-document".into(),
                    state: None,
                    history_state: None,
                }],
                0,
                "push",
            )
            .unwrap();
            let receipt =
                navigation_api::start_native_document_lifecycle_publication(scope, false).unwrap();
            assert!(navigation_api::resume_native_history_publication(scope, receipt).unwrap());
            assert!(eval(scope, "globalThis.hashes=[];onhashchange=e=>hashes.push(e.newURL);location.hash='#one';location.hash='#two';hashes.length===0").is_true());
            assert!(navigation_api::publication_delay(state).is_some());
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(
                poll_navigation_history_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 2);
            assert!(eval(scope, "hashes.join() === 'about:blank#one,about:blank#two'").is_true());
            assert!(backend.calls.lock().unwrap().contains(&"window"));
            assert!(navigation_api::publication_delay(state).is_none());
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_navigation_history_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert_eq!(count, 2);
            assert!(backend.calls.lock().unwrap().is_empty());
        });
    }
    #[test]
    fn active_navigation_publication_still_stops_after_callback_retires_document() {
        group_fixture(|scope, state, _backend| {
            fn retire(
                scope: &mut v8::PinScope,
                _args: v8::FunctionCallbackArguments,
                _rv: v8::ReturnValue,
            ) {
                scope
                    .get_current_context()
                    .get_slot::<V8BindingState>()
                    .expect("native publication callback binding state")
                    .retire_execution();
            }
            let function = v8::Function::new(scope, retire).unwrap();
            let key = v8::String::new(scope, "retireGroupDocument").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), function.into());
            eval(
                scope,
                "globalThis.lifecycle=[];onload=()=>{lifecycle.push('load');retireGroupDocument()};onpageshow=()=>lifecycle.push('pageshow')",
            );
            navigation_api::hold_publications_for_test(state, true);
            let _receipt =
                navigation_api::start_native_document_lifecycle_publication(scope, false).unwrap();
            assert!(navigation_api::document_lifecycle_publication_pending(
                state
            ));
            assert!(navigation_api::publication_delay(state).is_some());
            assert!(!state.execution_retired());
            assert!(eval(scope, "lifecycle.length===0").is_true());
            navigation_api::hold_publications_for_test(state, false);
            assert!(navigation_api::publications_ready(state));
            let mut count = 0;
            let result = poll_navigation_publications(
                scope,
                state,
                state.document_generation(),
                state.document_stream_epoch(),
                &mut count,
            );
            assert!(
                !matches!(result, Ok(true)),
                "retirement must stop the enclosing poll"
            );
            assert!(state.execution_retired());
            assert!(eval(scope, "lifecycle.join()==='load'").is_true());
        });
    }

    #[test]
    fn empty_abort_and_top_layer_elide_two_checks_but_keep_publishing_barrier() {
        group_fixture(|scope, state, backend| {
            let generation = state.document_generation();
            let epoch = state.document_stream_epoch();
            assert_eq!(abort_publication::poll(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(top_layer::poll(scope, state).unwrap(), 0);
            assert!(task_current(scope, state, generation, epoch));
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"; 2]);
            backend.calls.lock().unwrap().clear();
            let mut count = 0;
            assert!(poll_abort_publications(scope, state, generation, epoch).unwrap());
            assert!(poll_top_layer_tasks(scope, state, generation, epoch, &mut count).unwrap());
            assert_eq!(count, 0);
            assert!(backend.calls.lock().unwrap().is_empty());
            state.native_platform.borrow_mut().publishing = true;
            assert!(!poll_abort_publications(scope, state, generation, epoch).unwrap());
            state.native_platform.borrow_mut().publishing = false;
        });
    }
    #[test]
    fn pending_abort_keeps_retry_error_and_listener_cursor_without_replay() {
        group_fixture(|scope, state, backend| {
            eval(
                scope,
                "globalThis.aborter=new AbortController();globalThis.abortCalls=[];globalThis.abortEvent=null;aborter.signal.addEventListener('abort',e=>{abortEvent=e;abortCalls.push('first')},{once:true});aborter.signal.addEventListener('abort',e=>abortCalls.push(e===abortEvent?'second':'wrong'))",
            );
            let signal = v8::Local::<v8::Object>::try_from(eval(scope, "aborter.signal")).unwrap();
            let id = stateful_id(scope, signal).unwrap();
            let reason = v8::String::new(scope, "original").unwrap();
            publication::fail_after_next_callback();
            assert!(abort_publication::start(scope, state, id, reason.into()).is_err());
            assert!(abort_publication::pending(state));
            assert!(eval(scope, "abortCalls.join()==='first'").is_true());
            state.native_platform.borrow_mut().abort_retry =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_abort_publications(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch()
                )
                .unwrap()
            );
            assert_eq!(*backend.calls.lock().unwrap(), vec!["window"]);
            assert!(abort_publication::pending(state));
            assert!(eval(scope, "abortCalls.join()==='first'").is_true());
            state.native_platform.borrow_mut().abort_retry = None;
            assert!(
                poll_abort_publications(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch()
                )
                .unwrap()
            );
            assert!(!abort_publication::pending(state));
            assert!(
                eval(
                    scope,
                    "abortCalls.join()==='first,second'&&aborter.signal.reason==='original'"
                )
                .is_true()
            );
            backend.calls.lock().unwrap().clear();
            assert!(
                poll_abort_publications(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch()
                )
                .unwrap()
            );
            assert!(backend.calls.lock().unwrap().is_empty());
            assert!(eval(scope, "abortCalls.length===2").is_true());
        });
    }
    #[test]
    fn queued_top_layer_toggle_revalidates_after_callback_parks_document() {
        group_fixture(|scope, state, backend| {
            fn park(
                _scope: &mut v8::PinScope,
                _args: v8::FunctionCallbackArguments,
                _rv: v8::ReturnValue,
            ) {
                PARKED.with(|value| value.set(true));
            }
            let function = v8::Function::new(scope, park).unwrap();
            let key = v8::String::new(scope, "parkTopLayerDocument").unwrap();
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, key.into(), function.into());
            eval(
                scope,
                "globalThis.pop=document.createElement('div');pop.id='group-popover';pop.setAttribute('popover','manual');document.body.append(pop);globalThis.toggleCalls=0;pop.addEventListener('toggle',()=>{toggleCalls++;parkTopLayerDocument()})",
            );
            let target = state
                .dom
                .borrow()
                .get_element_by_id(state.document_root(), "group-popover")
                .unwrap()
                .unwrap();
            top_layer::toggle(scope, state, target, Some(true), None).unwrap();
            assert!(top_layer::delay(state).is_some());
            let mut count = 0;
            assert!(
                !poll_top_layer_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert!(eval(scope, "toggleCalls===1").is_true());
            assert!(state.native_platform.borrow().task_retry_at.is_some());
            PARKED.with(|value| value.set(false));
            state.set_native_platform_backend(backend.clone());
            assert!(
                poll_top_layer_tasks(
                    scope,
                    state,
                    state.document_generation(),
                    state.document_stream_epoch(),
                    &mut count
                )
                .unwrap()
            );
            assert!(eval(scope, "toggleCalls===1").is_true());
        });
    }

    thread_local! {static PARKED:Cell<bool>=const{Cell::new(false)};static MINIMIZED:Cell<bool>=const{Cell::new(false)};}
    struct ParkBackend(TestBackend);
    impl Backend for ParkBackend {
        fn request(
            &self,
            creator: crate::worker_wire::Creator,
            request: Request,
        ) -> Result<Response, String> {
            if matches!(request, Request::WindowState) && PARKED.with(Cell::get) {
                Ok(Response::WindowSuspended)
            } else if matches!(request, Request::WindowState) && MINIMIZED.with(Cell::get) {
                Ok(Response::WindowState {
                    hidden: true,
                    revision: 2,
                })
            } else {
                self.0.request(creator, request)
            }
        }
    }
    #[test]
    fn geolocation_callback_parking_preserves_later_task_without_busy_polling() {
        super::super::tests::initialize_v8();
        PARKED.with(|value| value.set(false));
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        let backend = Arc::new(ParkBackend(TestBackend(Mutex::new((
            PermissionState::Granted,
            false,
            1,
        )))));
        state.set_native_platform_backend(backend.clone());
        fn park(
            _scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            PARKED.with(|value| value.set(true));
        }
        let callback = v8::Function::new(scope, park).unwrap();
        let name = v8::String::new(scope, "parkNativeDocument").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into());
        eval(
            scope,
            "globalThis.deliveries=[];navigator.geolocation.getCurrentPosition(()=>{deliveries.push('first');parkNativeDocument()});navigator.geolocation.getCurrentPosition(()=>deliveries.push('second'))",
        );
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries.join()==='first'").is_true());
        assert_eq!(state.native_platform.borrow().positions.len(), 1);
        assert!(!state.native_platform_ready());
        assert!(state.native_platform_delay().unwrap() > std::time::Duration::ZERO);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert_eq!(state.native_platform.borrow().positions.len(), 1);
        PARKED.with(|value| value.set(false));
        state.set_native_platform_backend(backend);
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries.join()==='first,second'").is_true());
        assert!(state.native_platform.borrow().positions.is_empty());
    }
    #[test]
    fn minimize_in_first_location_callback_defers_later_location_until_restore() {
        super::super::tests::initialize_v8();
        PARKED.with(|value| value.set(false));
        MINIMIZED.with(|value| value.set(false));
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        let backend = Arc::new(ParkBackend(TestBackend(Mutex::new((
            PermissionState::Granted,
            false,
            1,
        )))));
        state.set_native_platform_backend(backend.clone());
        fn minimize(
            _scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            MINIMIZED.with(|value| value.set(true));
        }
        let callback = v8::Function::new(scope, minimize).unwrap();
        let name = v8::String::new(scope, "minimizeNativeWindow").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into());
        eval(
            scope,
            "globalThis.deliveries=[];navigator.geolocation.getCurrentPosition(()=>{deliveries.push('first');minimizeNativeWindow()});navigator.geolocation.getCurrentPosition(()=>deliveries.push('second'))",
        );
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries.join()==='first'").is_true());
        assert_eq!(state.native_platform.borrow().positions.len(), 1);
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries.join()==='first'&&document.hidden").is_true());
        MINIMIZED.with(|value| value.set(false));
        backend.0.0.lock().unwrap().2 = 3;
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "deliveries.join()==='first,second'&&!document.hidden"
            )
            .is_true()
        );
    }
    #[test]
    fn geolocation_publication_failure_retains_callback_until_exact_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        state.set_native_platform_backend(Arc::new(TestBackend(Mutex::new((
            PermissionState::Granted,
            false,
            1,
        )))));
        eval(
            scope,
            "globalThis.deliveries=0;navigator.geolocation.getCurrentPosition(()=>deliveries++,()=>deliveries+=100)",
        );
        FAIL_POSITION_PUBLICATION.with(|flag| flag.set(true));
        assert!(poll(scope, &state).is_err());
        assert_eq!(state.native_platform.borrow().positions.len(), 1);
        assert!(eval(scope, "deliveries===0").is_true());
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries===1").is_true());
        assert!(state.native_platform.borrow().positions.is_empty());
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries===1").is_true());
    }
    #[test]
    fn acquired_position_survives_allocation_retry_and_watch_observes_later_revision() {
        struct ChangingProvider {
            base: TestBackend,
            reads: Mutex<u64>,
        }
        impl Backend for ChangingProvider {
            fn request(
                &self,
                creator: crate::worker_wire::Creator,
                request: Request,
            ) -> Result<Response, String> {
                if matches!(&request, Request::Geolocation) {
                    let mut reads = self.reads.lock().unwrap();
                    *reads += 1;
                    return Ok(Response::Geolocation {
                        position: Ok(crate::platform_control::Coordinates {
                            latitude: 49. + *reads as f64,
                            longitude: 0.,
                            accuracy: 100.,
                            altitude: None,
                            altitude_accuracy: None,
                            heading: None,
                            speed: None,
                        }),
                        revision: self.base.0.lock().unwrap().2,
                    });
                }
                self.base.request(creator, request)
            }
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        let backend = Arc::new(ChangingProvider {
            base: TestBackend(Mutex::new((PermissionState::Granted, false, 1))),
            reads: Mutex::new(0),
        });
        state.set_native_platform_backend(backend.clone());
        eval(
            scope,
            "globalThis.positions=[];globalThis.watch=navigator.geolocation.watchPosition(p=>positions.push([p.coords.latitude,p.timestamp]))",
        );
        FAIL_POSITION_PUBLICATION.with(|flag| flag.set(true));
        assert!(poll(scope, &state).is_err());
        assert_eq!(*backend.reads.lock().unwrap(), 1);
        let timestamp = state
            .native_platform
            .borrow()
            .positions
            .values()
            .next()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .0
            .as_ref()
            .unwrap()
            .timestamp;
        backend.base.0.lock().unwrap().2 = 2;
        poll(scope, &state).unwrap();
        assert_eq!(*backend.reads.lock().unwrap(), 1);
        assert!(eval(scope, "positions.length===1&&positions[0][0]===50").is_true());
        assert_eq!(
            eval(scope, "positions[0][1]").number_value(scope).unwrap(),
            timestamp
        );
        poll(scope, &state).unwrap();
        assert_eq!(*backend.reads.lock().unwrap(), 2);
        assert!(eval(scope, "positions.length===2&&positions[1][0]===51").is_true());
        eval(scope, "navigator.geolocation.clearWatch(watch)");
        assert!(state.native_platform.borrow().positions.is_empty());
    }
    #[test]
    fn clear_watch_discards_acquired_unpublished_position() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        state.set_native_platform_backend(Arc::new(TestBackend(Mutex::new((
            PermissionState::Granted,
            false,
            1,
        )))));
        eval(
            scope,
            "globalThis.deliveries=0;globalThis.watch=navigator.geolocation.watchPosition(()=>deliveries++)",
        );
        FAIL_POSITION_PUBLICATION.with(|flag| flag.set(true));
        assert!(poll(scope, &state).is_err());
        eval(scope, "navigator.geolocation.clearWatch(watch)");
        poll(scope, &state).unwrap();
        assert!(eval(scope, "deliveries===0").is_true());
        assert!(state.native_platform.borrow().positions.is_empty());
    }
    #[test]
    fn permission_visibility_and_location_share_native_state_without_author_shims() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_message_transport(Arc::new(Messages));
        state.set_worker_transport(Arc::new(Messages), None, None);
        let backend = Arc::new(TestBackend(Mutex::new((PermissionState::Prompt, false, 0))));
        state.set_native_platform_backend(backend.clone());
        assert!(eval(scope,"typeof navigator.registerProtocolHandler==='function' && typeof navigator.unregisterProtocolHandler==='function' && navigator.unregisterProtocolHandler.length===2 && navigator.registerProtocolHandler.length===2").is_true());
        eval(
            scope,
            "document.documentElement.inert=true;document.documentElement.style.display='none'",
        );
        permission_prompt::open_protocol(
            scope,
            &state,
            crate::platform_control::PermissionPrompt {
                id: "native-test-protocol".into(),
                origin: "https://handler.test".into(),
                kind: crate::platform_control::PromptKind::Protocol,
                state: PermissionState::Prompt,
            },
            "web+demo".into(),
            "https://handler.test/?%s".into(),
        )
        .unwrap();
        permission_prompt::fixup(scope, &state).unwrap();
        assert!(
            state.permission_elements.borrow().prompt.is_some(),
            "author inert/display cannot dismiss browser registration UI"
        );
        permission_prompt::retire(&state);
        eval(
            scope,
            "document.documentElement.inert=false;document.documentElement.style.display=''",
        );

        assert!(eval(scope,"(()=>{for(const method of ['registerProtocolHandler','unregisterProtocolHandler']){let converted=false;try{navigator[method]('javascript',{toString(){converted=true;return 'http://['}})}catch(error){if(!converted||error.name!=='SecurityError')return false}}return true})()").is_true());

        eval(
            scope,
            "globalThis.changes=[];navigator.permissions.query({name:'geolocation'}).then(value=>{globalThis.permission=value;value.onchange=e=>changes.push([value.state,e.isTrusted]);});globalThis.visibility=[];document.addEventListener('visibilitychange',e=>visibility.push([document.hidden,e.isTrusted]));",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"permission instanceof PermissionStatus && permission instanceof EventTarget && permission.name==='geolocation' && permission.state==='prompt'").is_true());
        *backend.0.lock().unwrap() = (PermissionState::Granted, true, 1);
        poll(scope, &state).unwrap();
        eval(
            scope,
            "globalThis.rafRuns=0;requestAnimationFrame(()=>rafRuns++);Object.defineProperty(document,'hidden',{value:false,configurable:true})",
        );
        animation_frames::run(scope, &state, std::time::Instant::now()).unwrap();
        assert!(eval(scope, "rafRuns===0").is_true());
        assert!(
            !state.document_presentation_allowed(),
            "authored hidden override cannot admit a minimized scene"
        );
        eval(scope, "delete document.hidden");
        assert!(eval(scope,"permission.state==='granted' && changes[0][0]==='granted' && changes[0][1] && document.visibilityState==='hidden' && visibility[0][0] && visibility[0][1]").is_true());
        eval(
            scope,
            "globalThis.position=null;navigator.geolocation.getCurrentPosition(value=>position=value)",
        );
        poll(scope, &state).unwrap();
        assert!(!state.native_platform_ready());
        assert!(eval(scope, "position===null").is_true());
        *backend.0.lock().unwrap() = (PermissionState::Granted, false, 2);
        poll(scope, &state).unwrap();
        animation_frames::run(scope, &state, std::time::Instant::now()).unwrap();
        assert!(eval(scope, "rafRuns===1").is_true());
        assert!(
            state.document_presentation_allowed(),
            "native restoration admits the retained scene"
        );
        assert!(eval(scope,"position.coords.latitude===50 && position.coords.longitude===0 && position.coords.accuracy===100 && position.coords.altitude===null && position.timestamp>0").is_true());
        assert!(eval(scope,"position instanceof GeolocationPosition && position.coords instanceof GeolocationCoordinates && Object.prototype.toString.call(position)==='[object GeolocationPosition]' && position.coords===position.coords && !Object.hasOwn(position,'coords')").is_true());
        eval(
            scope,
            "position.coords.latitude=123;position.timestamp=0;globalThis.serialized=position.toJSON();globalThis.originalLatitude=Object.getOwnPropertyDescriptor(GeolocationCoordinates.prototype,'latitude');Object.defineProperty(GeolocationCoordinates.prototype,'latitude',{get(){throw new Error('author getter')}});globalThis.nativeJson=position.toJSON();Object.defineProperty(GeolocationCoordinates.prototype,'latitude',originalLatitude)",
        );
        assert!(eval(scope,"serialized.coords.latitude===50 && nativeJson.coords.latitude===50 && serialized.timestamp===position.timestamp && position.timestamp>0").is_true());
        assert!(eval(scope,"(()=>{try{GeolocationPosition.prototype.toJSON.call({coords:position.coords,timestamp:1});return false}catch(error){return error instanceof TypeError}})()").is_true());
        eval(
            scope,
            "globalThis.zero=null;navigator.geolocation.getCurrentPosition(()=>{throw new Error('zero timeout succeeded')},value=>zero=value,{timeout:-100,maximumAge:0})",
        );
        poll(scope, &state).unwrap();
        assert!(eval(scope,"zero instanceof GeolocationPositionError && zero.code===GeolocationPositionError.TIMEOUT && zero.TIMEOUT===3 && zero.PERMISSION_DENIED===1 && zero.POSITION_UNAVAILABLE===2").is_true());
        eval(
            scope,
            "globalThis.cached=null;navigator.geolocation.getCurrentPosition(value=>cached=value,null,{timeout:0,maximumAge:Infinity})",
        );
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "cached.timestamp===position.timestamp && cached.coords.latitude===50"
            )
            .is_true()
        );
        eval(
            scope,
            "globalThis.highAccuracy=null;navigator.geolocation.getCurrentPosition(()=>{throw new Error('wrong accuracy cache')},value=>highAccuracy=value,{enableHighAccuracy:true,timeout:0,maximumAge:Infinity})",
        );
        poll(scope, &state).unwrap();
        assert!(eval(scope, "highAccuracy.code===3").is_true());
        assert!(eval(scope,"(()=>{let order=[];navigator.geolocation.getCurrentPosition(()=>{},null,{get timeout(){order.push('timeout');return 0},get maximumAge(){order.push('maximumAge');return 0},get enableHighAccuracy(){order.push('enableHighAccuracy');return false}});return order.join(',')==='enableHighAccuracy,maximumAge,timeout'})()").is_true());
        poll(scope, &state).unwrap();
        assert!(eval(scope,"(()=>{const marker={};try{navigator.geolocation.getCurrentPosition(()=>{},null,{get timeout(){throw marker}});return false}catch(error){return error===marker}})()").is_true());
        eval(
            scope,
            "globalThis.geoErrors=0;globalThis.errorReported=false;addEventListener('error',event=>{if(event.error&&event.error.message==='geo callback'){errorReported=true;event.preventDefault()}});navigator.geolocation.getCurrentPosition(()=>{throw new Error('geo callback')});globalThis.watch=navigator.geolocation.watchPosition(()=>{},()=>geoErrors++,{timeout:0})",
        );
        poll(scope, &state).unwrap();
        assert!(eval(scope, "errorReported && geoErrors===1").is_true());
        *backend.0.lock().unwrap() = (PermissionState::Granted, false, 3);
        poll(scope, &state).unwrap();
        assert!(eval(scope, "geoErrors===1").is_true());
        eval(scope, "navigator.geolocation.clearWatch(watch)");
        eval(
            scope,
            "globalThis.rejected=false;navigator.permissions.query({name:'invented'}).catch(error=>rejected=error instanceof TypeError)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "rejected").is_true());
        eval(
            scope,
            "globalThis.descriptorError={marker:'original'};globalThis.preserved=false;navigator.permissions.query({get name(){throw descriptorError}}).catch(error=>preserved=error===descriptorError)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "preserved").is_true());
        eval(
            scope,
            "globalThis.ignored=false;navigator.permissions.query({name:'geolocation',get panTiltZoom(){throw descriptorError},get sysex(){throw descriptorError},get allowWithoutGesture(){throw descriptorError}}).then(()=>ignored=true)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "ignored").is_true());
        let retired_geo = eval(scope, "navigator.geolocation");
        state.retire_execution();
        assert_eq!(poll(scope, &state).unwrap(), 0);
        let caller = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, caller);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let recipient = install_generated_bindings(scope, caller, dom, RealmId(1), 1).unwrap();
        let key = v8::String::new(scope, "retiredGeo").unwrap();
        caller
            .global(scope)
            .create_data_property(scope, key.into(), retired_geo);
        assert!(eval(scope,"typeof GeolocationPosition==='undefined' && typeof GeolocationCoordinates==='undefined' && typeof GeolocationPositionError==='function'").is_true());
        eval(
            scope,
            "globalThis.inactive=null;globalThis.inactiveWatch=retiredGeo.watchPosition(()=>{throw new Error('inactive success')},value=>inactive=value)",
        );
        assert!(eval(scope, "inactive===null && inactiveWatch===0").is_true());
        assert!(recipient.native_platform_ready());
        poll(scope, &recipient).unwrap();
        assert!(
            eval(
                scope,
                "inactive.code===2 && inactive.POSITION_UNAVAILABLE===2"
            )
            .is_true()
        );
    }
}

// This can run before Window registration during initial setup. Cache only an
// authenticated successful response; later layout retries the initialization.
pub(super) fn ensure_ui_locale(state: &V8BindingState) {
    if state.native_platform.borrow().backend.is_none() {
        return;
    }
    if state.dom.borrow().native_ui_locale().is_none()
        && let Ok(Response::UiLocale { locale }) = request(state, Request::UiLocale)
    {
        let _ = state.dom.borrow_mut().set_native_ui_locale(&locale);
    }
    let missing = state.dom.borrow().missing_control_locales();
    for locale in missing {
        if state
            .native_platform
            .borrow()
            .unavailable_control_locales
            .contains(&locale)
        {
            continue;
        }
        if locale.len() > 128
            || !locale.split('-').all(|part| {
                !part.is_empty()
                    && part.len() <= 8
                    && part.bytes().all(|b| b.is_ascii_alphanumeric())
            })
        {
            continue;
        }
        if let Ok(Response::UiLocaleData { profile }) = request(
            state,
            Request::UiLocaleData {
                locale: locale.clone(),
            },
        ) {
            if let Some(profile) = profile {
                let _ = state
                    .dom
                    .borrow_mut()
                    .install_control_locale(&locale, *profile);
            } else {
                let mut platform = state.native_platform.borrow_mut();
                if platform.unavailable_control_locales.len() < 64 {
                    platform.unavailable_control_locales.insert(locale);
                }
            }
        }
    }
}

impl V8BindingState {
    pub fn set_native_permissions_policy_snapshot(
        &self,
        callback: Rc<dyn Fn() -> Result<crate::permissions_policy::Snapshot, String>>,
    ) {
        self.native_platform.borrow_mut().permissions_snapshot = Some(callback);
    }
    pub fn native_permissions_policy_snapshot(
        &self,
    ) -> Result<crate::permissions_policy::Snapshot, V8BindingError> {
        self.refresh_native_container_policy()?;
        let callback = self.native_platform.borrow().permissions_snapshot.clone();
        let snapshot = if let Some(callback) = callback {
            callback().map_err(failure)?
        } else if self.native_platform.borrow().backend.is_some() {
            match request(self, Request::PermissionsPolicySnapshot)? {
                Response::PermissionsPolicySnapshot(value) => value,
                _ => return Err(failure("policy snapshot response mismatch")),
            }
        } else {
            let origin = url::Url::parse(&self.request_initiator_url()?)
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_else(|_| "null".into());
            crate::permissions_policy::Snapshot {
                origin: origin.clone(),
                container: false,
                constraints: vec![crate::permissions_policy::Constraint::Header {
                    origin,
                    headers: Vec::new(),
                }],
            }
        };
        if !snapshot.validate() {
            return Err(failure("policy snapshot bounds"));
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod prototype_retention_tests {
    use super::*;
    #[test]
    fn native_prototypes_survive_public_replacement_and_release_retired_context() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, state, anchor) = {
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(991), 1).unwrap();
            let entries = state.native_platform.borrow().prototypes.clone();
            assert_eq!(entries.len(), 8);
            let global = context.global(scope);
            for (name, prototype) in entries {
                let key = v8::String::new(scope, name).unwrap();
                let replacement = v8::Object::new(scope);
                assert_eq!(
                    global.set(scope, key.into(), replacement.into()),
                    Some(true)
                );
                let (_, object) = wrap(scope, &state, name).unwrap();
                assert!(
                    object
                        .get_prototype(scope)
                        .unwrap()
                        .strict_equals(prototype.to_local(scope).unwrap().into())
                );
            }
            let anchor = v8::Object::new(scope);
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, context),
                state,
                v8::Global::new(scope, anchor),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles, &mut isolate);
            assert!(context.to_local(handles).is_some());
            assert!(
                state
                    .native_platform
                    .borrow()
                    .prototypes
                    .values()
                    .all(|value| value.to_local(handles).is_some())
            );
        }
        drop(anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles, &mut isolate);
        assert!(context.to_local(handles).is_none());
        assert_eq!(state.native_platform.borrow().prototypes.len(), 8);
        assert!(
            state
                .native_platform
                .borrow()
                .prototypes
                .values()
                .all(|value| value.to_local(handles).is_none())
        );
    }
}

#[cfg(test)]
mod root_registration_receipt_tests {
    use super::*;
    use crate::message_wire::{
        MessageHub, MessageRequest, MessageResponse, MessageTransport, WindowTarget,
    };
    use crate::worker_wire::Creator;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    #[derive(Default)]
    struct Authority {
        hub: Mutex<MessageHub>,
        registrations: AtomicUsize,
        queries: AtomicUsize,
        fail_registration: AtomicBool,
        hidden: AtomicBool,
    }
    impl MessageTransport for Authority {
        fn request(&self, request: MessageRequest) -> Result<MessageResponse, String> {
            if matches!(&request, MessageRequest::RegisterRootDocument { .. }) {
                self.registrations.fetch_add(1, Ordering::SeqCst);
                if self.fail_registration.swap(false, Ordering::SeqCst) {
                    return Err("injected registration failure".into());
                }
            }
            self.hub.lock().unwrap().handle(
                1,
                request,
                &BTreeMap::from([(1, "https://receipt.test".into())]),
            )
        }
    }
    impl Backend for Authority {
        fn request(&self, creator: Creator, request: Request) -> Result<Response, String> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            assert!(matches!(request, Request::WindowState));
            let Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } = creator
            else {
                return Err("Window required".into());
            };
            let hub = self.hub.lock().unwrap();
            if hub
                .root_document_authority(1, document_generation, stream_epoch)
                .is_err()
            {
                return Ok(
                    if hub
                        .retained_document_authority(1, &target, document_generation, stream_epoch)
                        .is_ok()
                    {
                        Response::WindowSuspended
                    } else {
                        Response::WindowRetired
                    },
                );
            }
            Ok(Response::WindowState {
                hidden: self.hidden.load(Ordering::SeqCst),
                revision: 1,
            })
        }
    }
    fn fixture(run: impl FnOnce(&Rc<V8BindingState>, &Arc<Authority>)) {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let authority = Arc::new(Authority::default());
        state.set_message_transport(authority.clone());
        state.set_native_platform_backend(authority.clone());
        run(&state, &authority);
    }
    #[test]
    fn repeated_platform_queries_reuse_registration_but_revalidate_parent_state() {
        fixture(|state, authority| {
            for _ in 0..32 {
                assert!(matches!(
                    request(state, Request::WindowState),
                    Ok(Response::WindowState { hidden: false, .. })
                ));
            }
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 1);
            assert_eq!(authority.queries.load(Ordering::SeqCst), 32);
            authority.hidden.store(true, Ordering::SeqCst);
            assert!(matches!(
                request(state, Request::WindowState),
                Ok(Response::WindowState { hidden: true, .. })
            ));
            MessageTransport::request(
                &**authority,
                MessageRequest::RetireRootDocument {
                    document_generation: state.document_generation(),
                    stream_epoch: state.document_authority_epoch(),
                },
            )
            .unwrap();
            assert!(matches!(
                request(state, Request::WindowState),
                Ok(Response::WindowRetired)
            ));
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 1);
            assert_eq!(authority.queries.load(Ordering::SeqCst), 34);
        });
    }
    #[test]
    fn failed_registration_retries_and_replaced_backend_transport_invalidate_receipt() {
        fixture(|state, authority| {
            authority.fail_registration.store(true, Ordering::SeqCst);
            assert!(request(state, Request::WindowState).is_err());
            assert_eq!(authority.queries.load(Ordering::SeqCst), 0);
            request(state, Request::WindowState).unwrap();
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 2);
            state.set_native_platform_backend(authority.clone());
            request(state, Request::WindowState).unwrap();
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 3);
            let replacement = Arc::new(Authority::default());
            state.set_message_transport(replacement.clone());
            request(state, Request::WindowState).unwrap();
            assert_eq!(replacement.registrations.load(Ordering::SeqCst), 1);
            // A successful receipt retains neither old transport nor backend.
            state.set_message_transport(authority.clone());
            let weak = Arc::downgrade(&replacement);
            drop(replacement);
            assert!(weak.upgrade().is_none());
            request(state, Request::WindowState).unwrap();
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 4);
        });
    }
    #[test]
    fn lifetime_prerender_and_park_transitions_do_not_reuse_wrong_registration() {
        fixture(|state, authority| {
            request(state, Request::WindowState).unwrap();
            state
                .document_generation
                .set(state.document_generation() + 1);
            request(state, Request::WindowState).unwrap();
            state
                .document_authority_epoch
                .set(state.document_authority_epoch() + 1);
            request(state, Request::WindowState).unwrap();
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 3);
            state
                .document_stream_epoch
                .set(state.document_stream_epoch() + 1);
            request(state, Request::WindowState).unwrap();
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 3);
            state.document_prerendering.set(true);
            request(state, Request::WindowState).unwrap();
            state.document_prerendering.set(false);
            request(state, Request::WindowState).unwrap();
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 4);
            MessageTransport::request(
                &**authority,
                MessageRequest::ParkDocument {
                    document_generation: state.document_generation(),
                    stream_epoch: state.document_authority_epoch(),
                },
            )
            .unwrap();
            assert!(matches!(
                request(state, Request::WindowState),
                Ok(Response::WindowSuspended)
            ));
            assert_eq!(authority.registrations.load(Ordering::SeqCst), 4);
            assert!(
                state
                    .native_platform
                    .borrow()
                    .root_registration
                    .as_ref()
                    .is_some_and(|receipt| matches!(
                        receipt.creator,
                        Creator::Window {
                            target: WindowTarget::Root,
                            ..
                        }
                    ))
            );
        });
    }
}
