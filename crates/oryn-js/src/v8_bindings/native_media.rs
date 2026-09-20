//! Native stream and track wrappers backed by parent capture custody. Native
//! track identities are never supplied by author object properties.
use super::*;
use crate::media_capture::{Kind, Request, Response, Stream, Track};
struct Captured {
    stream: Stream,
    wrappers: Vec<v8::Global<v8::Object>>,
}
#[derive(Default)]
pub(super) struct State {
    pub(super) events: VecDeque<media_element::EventTask>,
    pub(super) event_polling: bool,
    pub(super) resources: media_resource::State,
    muted: BTreeSet<u64>,
    pub(super) audio_cleanup: BTreeSet<(String, u64)>,
    captures: BTreeMap<NodeId, String>,
    canceling: BTreeSet<NodeId>,
    captured: BTreeMap<NodeId, Captured>,
    pub(super) playback: BTreeMap<NodeId, media_element::Playback>,
    tracks: BTreeMap<u64, Track>,
    streams: BTreeMap<u64, String>,
    prototypes: BTreeMap<&'static str, v8::Weak<v8::Object>>,
    pending: BTreeMap<String, bool>,
    pub(super) retried: Option<std::time::Instant>,
}
fn failure(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "MediaStream".into(),
        member: name.split(':').next().unwrap_or(name).into(),
    }
}
fn request(state: &V8BindingState, request: Request) -> Result<Response, V8BindingError> {
    native_platform::media_request(state, request)
}
fn release(state: &V8BindingState, id: String, remove: bool) {
    let remove = remove
        || state
            .native_media
            .borrow()
            .pending
            .get(&id)
            .copied()
            .unwrap_or(false);
    let command = if remove {
        Request::Release { id: id.clone() }
    } else {
        Request::Stop { id: id.clone() }
    };
    match request(state, command) {
        Ok(Response::Stopped) => {
            state.native_media.borrow_mut().pending.remove(&id);
        }
        _ => {
            state.native_media.borrow_mut().pending.insert(id, remove);
        }
    }
}
fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    track: bool,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    let state = binding_state_for_receiver(scope, object).ok_or_else(|| failure("TypeError"))?;
    let id = stateful_id(scope, object)?;
    let known = if track {
        state.native_media.borrow().tracks.contains_key(&id)
    } else {
        state.native_media.borrow().streams.contains_key(&id)
    };
    if !known {
        return Err(failure("TypeError"));
    }
    Ok((state, id))
}
pub(super) fn stream_tracks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Result<Vec<Track>, V8BindingError> {
    let (owner_state, _) = owner(scope, object, false)?;
    if !Rc::ptr_eq(state, &owner_state) {
        return Err(failure("TypeError"));
    }
    let values = tracks(scope, object)?;
    let mut result = Vec::new();
    for index in 0..values.length() {
        let object = values
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(|| failure("TypeError"))?;
        let (owner_state, id) = owner(scope, object, true)?;
        result.push(owner_state.native_media.borrow().tracks[&id].clone());
    }
    Ok(result)
}
pub(super) fn single_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    stream: v8::Local<v8::Object>,
    kind: Kind,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let (owner_state, _) = owner(scope, stream, false)?;
    if !Rc::ptr_eq(state, &owner_state) {
        return Err(failure("TypeError"));
    }
    let values = tracks(scope, stream)?;
    if values.length() != 1 {
        return Err(failure("InvalidStateError"));
    }
    let object = values
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError"))?;
    let (owner_state, id) = owner(scope, object, true)?;
    if owner_state.native_media.borrow().tracks[&id].kind != kind {
        return Err(failure("InvalidStateError"));
    }
    Ok(object)
}
fn tracks_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.native.stream.tracks").unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn tracks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let key = tracks_key(scope);
    stream
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError"))
}
fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    kind: &'static str,
) -> Result<(u64, v8::Local<'s, v8::Object>), V8BindingError> {
    let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
    let object = new_platform_object(scope, state, "EventTarget", id)?;
    let prototype = state
        .native_media
        .borrow()
        .prototypes
        .get(kind)
        .cloned()
        .ok_or_else(|| failure("native media prototype missing"))?;
    let prototype = prototype
        .to_local(scope)
        .ok_or_else(|| failure("native media prototype missing"))?;
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return Err(V8BindingError::Allocation(
            "native media prototype publication",
        ));
    }
    Ok((id, object))
}
fn track_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    track: Track,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if state.native_media.borrow().tracks.len() >= 1024 {
        return Err(failure("QuotaExceededError"));
    }
    let (id, object) = wrap(scope, state, "MediaStreamTrack")?;
    state.native_media.borrow_mut().tracks.insert(id, track);
    Ok(object)
}
fn stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: String,
    values: &[v8::Local<'s, v8::Value>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if state.native_media.borrow().streams.len() >= 1024 || values.len() > 1024 {
        return Err(failure("QuotaExceededError"));
    }
    let (target, object) = wrap(scope, state, "MediaStream")?;
    let array = v8::Array::new_with_elements(scope, values);
    let key = tracks_key(scope);
    #[cfg(test)]
    if FAIL_STREAM_PUBLICATION.with(|flag| flag.replace(false)) {
        return Err(V8BindingError::Allocation(
            "injected stream backing publication failure",
        ));
    }
    if object.set_private(scope, key, array.into()) != Some(true) {
        return Err(V8BindingError::Allocation(
            "native stream backing publication",
        ));
    }
    state.native_media.borrow_mut().streams.insert(target, id);
    Ok(object)
}
pub(super) fn capture_pending(state: &V8BindingState, node: NodeId) -> bool {
    let media = state.native_media.borrow();
    media.captures.contains_key(&node) || media.captured.contains_key(&node)
}
pub(super) fn mark_capture_cancellation(state: &V8BindingState, node: NodeId) {
    let mut media = state.native_media.borrow_mut();
    if media.captures.contains_key(&node) || media.captured.contains_key(&node) {
        media.canceling.insert(node);
    }
}
pub(super) fn capture_canceling(state: &V8BindingState, node: NodeId) -> bool {
    state.native_media.borrow().canceling.contains(&node)
}
pub(super) fn cancel_capture(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    state.native_media.borrow_mut().canceling.insert(node);
    let id = state.native_media.borrow().captures.get(&node).cloned();
    if let Some(id) = id {
        // Transport success alone does not acknowledge cancellation. Keep the
        // exact pending identity until the parent confirms terminal cleanup.
        if !matches!(
            request(state, Request::CancelCapture { id })?,
            Response::Stopped
        ) {
            return Err(failure("InvalidStateError"));
        }
        state.native_media.borrow_mut().captures.remove(&node);
    }
    let captured = state.native_media.borrow_mut().captured.remove(&node);
    if let Some(captured) = captured {
        for track in captured.stream.tracks {
            release(state, track.id.clone(), true);
            let mut media = state.native_media.borrow_mut();
            let ids = media
                .tracks
                .iter()
                .filter_map(|(id, value)| (value.id == track.id).then_some(*id))
                .collect::<Vec<_>>();
            for id in ids {
                media.tracks.remove(&id);
                media.muted.remove(&id);
            }
        }
    }
    state.native_media.borrow_mut().canceling.remove(&node);
    Ok(())
}
pub(super) fn capture<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    constraints: serde_json::Value,
) -> Result<Option<v8::Local<'s, v8::Object>>, V8BindingError> {
    if capture_canceling(state, node) {
        cancel_capture(state, node)?;
        return Err(V8BindingError::Allocation(
            "capture cancellation completed before new admission",
        ));
    }
    if state.native_media.borrow().pending.len() >= 1024 {
        return Err(failure("QuotaExceededError"));
    }
    if !state.native_media.borrow().captured.contains_key(&node) {
        let pending = state.native_media.borrow().captures.get(&node).cloned();
        if pending.is_none() {
            native_platform::publish_media_security(scope, state)?;
        }
        let command = pending
            .map(|id| Request::PollCapture { id })
            .unwrap_or(Request::Capture { constraints });
        let response = request(state, command)?;
        let stream = match response {
            Response::Pending { id } => {
                state.native_media.borrow_mut().captures.insert(node, id);
                return Ok(None);
            }
            Response::Captured(stream) => stream,
            _ => return Err(failure("InvalidStateError")),
        };
        let mut media = state.native_media.borrow_mut();
        media.captures.remove(&node);
        media.captured.insert(
            node,
            Captured {
                stream,
                wrappers: Vec::new(),
            },
        );
    }
    // Native capture is accepted exactly once. Partial wrapper construction
    // retains the same track IDs and all completed wrappers until publication.
    loop {
        let next = {
            let media = state.native_media.borrow();
            let captured = &media.captured[&node];
            captured.stream.tracks.get(captured.wrappers.len()).cloned()
        };
        let Some(track) = next else { break };
        if state.native_media.borrow().tracks.len() >= 1024 {
            return Err(V8BindingError::Allocation(
                "capture track publication limit",
            ));
        }
        let wrapper = track_object(scope, state, track)?;
        state
            .native_media
            .borrow_mut()
            .captured
            .get_mut(&node)
            .unwrap()
            .wrappers
            .push(v8::Global::new(scope, wrapper));
        #[cfg(test)]
        if FAIL_CAPTURE_PUBLICATION.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "injected capture publication failure",
            ));
        }
    }
    let (id, values) = {
        let media = state.native_media.borrow();
        let captured = &media.captured[&node];
        (
            captured.stream.id.clone(),
            captured
                .wrappers
                .iter()
                .map(|wrapper| v8::Local::new(scope, wrapper).into())
                .collect::<Vec<_>>(),
        )
    };
    if state.native_media.borrow().streams.len() >= 1024 {
        return Err(V8BindingError::Allocation(
            "capture stream publication limit",
        ));
    }
    let stream = stream_object(scope, state, id, &values)?;
    state.native_media.borrow_mut().captured.remove(&node);
    Ok(Some(stream))
}
#[cfg(test)]
thread_local! {static FAIL_CAPTURE_PUBLICATION:Cell<bool>=const{Cell::new(false)};static FAIL_STREAM_PUBLICATION:Cell<bool>=const{Cell::new(false)};}

fn json_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &serde_json::Value,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match value {
        serde_json::Value::Null => Ok(v8::null(scope).into()),
        serde_json::Value::Bool(value) => Ok(v8::Boolean::new(scope, *value).into()),
        serde_json::Value::Number(value) => {
            Ok(v8::Number::new(scope, value.as_f64().ok_or_else(|| failure("TypeError"))?).into())
        }
        serde_json::Value::String(value) => v8_string(scope, value),
        serde_json::Value::Array(values) => {
            let values = values
                .iter()
                .map(|value| json_value(scope, value))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v8::Array::new_with_elements(scope, &values).into())
        }
        serde_json::Value::Object(values) => {
            let object = v8::Object::new(scope);
            for (name, value) in values {
                let value = json_value(scope, value)?;
                let key = v8::String::new(scope, name).unwrap();
                object.create_data_property(scope, key.into(), value);
            }
            Ok(object.into())
        }
    }
}
fn read(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let index = args.data().int32_value(scope).unwrap_or(-1);
    let result = (|| {
        let (state, id) = owner(scope, args.this(), index >= 10)?;
        if index >= 10 {
            let media = state.native_media.borrow();
            let track = &media.tracks[&id];
            return match index {
                10 => v8_string(scope, &track.id),
                11 => v8_string(scope, track.kind.as_str()),
                12 => v8_string(scope, &track.label),
                13 => Ok(v8::Boolean::new(scope, track.enabled).into()),
                14 => Ok(
                    v8::Boolean::new(scope, state.native_media.borrow().muted.contains(&id)).into(),
                ),
                15 => v8_string(scope, if track.ended { "ended" } else { "live" }),
                _ => Err(failure("TypeError")),
            };
        }
        if index == 0 {
            return v8_string(scope, &state.native_media.borrow().streams[&id]);
        }
        let tracks = tracks(scope, args.this())?;
        let mut active = false;
        for index in 0..tracks.length() {
            let value = tracks
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .ok_or_else(|| failure("InvalidStateError"))?;
            let (owner, id) = owner(scope, value, true)?;
            active |= !owner.native_media.borrow().tracks[&id].ended;
        }
        Ok(v8::Boolean::new(scope, active).into())
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn enabled(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| {
        let (state, id) = owner(scope, args.this(), true)?;
        let enabled = args.get(0).boolean_value(scope);
        let track = state.native_media.borrow().tracks[&id].clone();
        if !track.ended {
            if let Response::Track(track) = request(
                &state,
                Request::Enable {
                    id: track.id,
                    enabled,
                },
            )? {
                state.native_media.borrow_mut().tracks.insert(id, track);
            }
        } else {
            state
                .native_media
                .borrow_mut()
                .tracks
                .get_mut(&id)
                .unwrap()
                .enabled = enabled;
        }
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}
fn handler_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.data().to_rust_string_lossy(scope);
    let result = (|| {
        let (state, id) = owner(
            scope,
            args.this(),
            name != "addtrack" && name != "removetrack",
        )?;
        Ok(platform_listener_graph::handler_get(
            scope, &state, id, &name,
        ))
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn handler_set(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let name = args.data().to_rust_string_lossy(scope);
    let result = (|| {
        let (state, id) = owner(
            scope,
            args.this(),
            name != "addtrack" && name != "removetrack",
        )?;
        platform_listener_graph::handler_set(scope, &state, id, &name, args.get(0))
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}
fn clone_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let (state, id) = owner(scope, object, true)?;
    if state.native_media.borrow().pending.len() >= 1024 {
        return Err(failure("QuotaExceededError"));
    }
    let id = state.native_media.borrow().tracks[&id].id.clone();
    let Response::Track(track) = request(&state, Request::Clone { id })? else {
        return Err(failure("InvalidStateError"));
    };
    let native_id = track.id.clone();
    let result = track_object(scope, &state, track);
    if result.is_err() {
        release(&state, native_id, true);
    }
    result
}
fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let index = args.data().int32_value(scope).unwrap_or(-1);
    if index == 15 {
        v8::tc_scope!(let scope,scope);
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            throw_type_error(scope, "Promise allocation failed");
            return;
        };
        let promise = resolver.get_promise(scope);
        rv.set(promise.into());
        let result = (|| {
            let (state, id) = owner(scope, args.this(), true)?;
            let constraints = permission_elements::track_constraints(scope, args.get(0))?;
            let native_id = state.native_media.borrow().tracks[&id].id.clone();
            let Response::Track(track) = request(
                &state,
                Request::ApplyConstraints {
                    id: native_id,
                    constraints,
                },
            )?
            else {
                return Err(failure("InvalidStateError"));
            };
            state.native_media.borrow_mut().tracks.insert(id, track);
            Ok(())
        })();
        match result {
            Ok(()) => {
                let value = v8::undefined(scope);
                resolver.resolve(scope, value.into());
            }
            Err(error) => {
                if scope.is_execution_terminating() {
                    return;
                }
                let thrown = scope.exception();
                scope.reset();
                let reason = if let Some(thrown) = thrown {
                    thrown
                } else {
                    let message = match &error {
                        V8BindingError::Unsupported { member, .. } => member.as_str(),
                        _ => "TypeError",
                    };
                    if message.starts_with("OverconstrainedError:") {
                        let constraint = message
                            .split_once(':')
                            .map(|(_, name)| name.trim())
                            .unwrap_or("");
                        overconstrained(scope, constraint, message).unwrap_or_else(|_| {
                            let message = v8::String::new(scope, message).unwrap();
                            v8::Exception::type_error(scope, message)
                        })
                    } else if message.ends_with("Error") && message != "TypeError" {
                        dom_exception_value(scope, message).unwrap_or_else(|_| {
                            let message = v8::String::new(scope, message).unwrap();
                            v8::Exception::type_error(scope, message)
                        })
                    } else {
                        let message = v8::String::new(scope, message).unwrap();
                        v8::Exception::type_error(scope, message)
                    }
                };
                resolver.reject(scope, reason);
            }
        }
        return;
    }
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let (state, id) = owner(scope, args.this(), index >= 10)?;
        if index == 3 && args.length() == 0 {
            return Err(failure("TypeError"));
        }
        if index >= 10 {
            let track = state.native_media.borrow().tracks[&id].clone();
            return match index {
                10 => {
                    if !track.ended {
                        release(&state, track.id, false);
                        state
                            .native_media
                            .borrow_mut()
                            .tracks
                            .get_mut(&id)
                            .unwrap()
                            .ended = true;
                    }
                    Ok(None)
                }
                11 => clone_track(scope, args.this()).map(|value| Some(value.into())),
                12 => json_value(scope, &track.settings).map(Some),
                13 => json_value(scope, &track.constraints).map(Some),
                14 => {
                    let Response::Capabilities(value) =
                        request(&state, Request::Capabilities { id: track.id })?
                    else {
                        return Err(failure("InvalidStateError"));
                    };
                    json_value(scope, &value).map(Some)
                }
                _ => Err(failure("TypeError")),
            };
        }
        // DOMString conversion can synchronously change stream membership. Capture
        // the native track set only after that authored conversion has completed.
        let wanted = if index == 3 {
            Some(
                args.get(0)
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope),
            )
        } else {
            None
        };
        let array = tracks(scope, args.this())?;
        let mut values = Vec::new();
        for index in 0..array.length() {
            values.push(
                array
                    .get_index(scope, index)
                    .ok_or(V8BindingError::JavaScriptException)?,
            );
        }
        match index {
            0..=3 => {
                let mut selected = Vec::new();
                for value in values {
                    let object = v8::Local::<v8::Object>::try_from(value)
                        .map_err(|_| failure("TypeError"))?;
                    let (owner, id) = owner(scope, object, true)?;
                    let media = owner.native_media.borrow();
                    let track = &media.tracks[&id];
                    if index == 0
                        || (index == 1 && track.kind == Kind::Audio)
                        || (index == 2 && track.kind == Kind::Video)
                        || wanted.as_deref() == Some(track.id.as_str())
                    {
                        selected.push(value);
                    }
                }
                if index == 3 {
                    Ok(Some(
                        selected
                            .first()
                            .copied()
                            .unwrap_or_else(|| v8::null(scope).into()),
                    ))
                } else {
                    Ok(Some(v8::Array::new_with_elements(scope, &selected).into()))
                }
            }
            4 | 5 => {
                let object = v8::Local::<v8::Object>::try_from(args.get(0))
                    .map_err(|_| failure("TypeError"))?;
                owner(scope, object, true)?;
                let exists = values
                    .iter()
                    .any(|value| value.strict_equals(object.into()));
                if index == 4 && !exists {
                    values.push(object.into());
                }
                if index == 5 {
                    values.retain(|value| !value.strict_equals(object.into()));
                }
                if values.len() > 1024 {
                    return Err(failure("QuotaExceededError"));
                }
                let array = v8::Array::new_with_elements(scope, &values);
                let key = tracks_key(scope);
                args.this().set_private(scope, key, array.into());
                Ok(None)
            }
            6 => {
                let mut clones: Vec<v8::Local<v8::Value>> = Vec::new();
                let result = (|| {
                    for value in values {
                        let object = v8::Local::<v8::Object>::try_from(value)
                            .map_err(|_| failure("TypeError"))?;
                        clones.push(clone_track(scope, object)?.into());
                    }
                    let fresh = format!(
                        "oryn-stream-{}-{}",
                        state.instance_id,
                        reserve_platform_object_id(&state)
                    );
                    stream_object(scope, &state, fresh, &clones).map(|value| Some(value.into()))
                })();
                if result.is_err() {
                    for value in clones {
                        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
                            && let Ok((owner, id)) = owner(scope, object, true)
                        {
                            let native_id = owner.native_media.borrow().tracks[&id].id.clone();
                            release(&owner, native_id, true);
                            owner.native_media.borrow_mut().tracks.remove(&id);
                        }
                    }
                }
                result
            }
            _ => Err(failure("TypeError")),
        }
    })();
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<v8::Value>,
    kind: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let target_object = target
        .try_cast::<v8::Object>()
        .map_err(|_| failure("TypeError"))?;
    let key = v8_string(scope, "prototype")?;
    let prototype = target_object
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if prototype.is_object() {
        return Ok(prototype);
    }
    let realm = crate::v8_context_bridge::constructor_realm::<V8BindingState>(target)
        .ok_or_else(|| failure("TypeError"))?;
    let prototype = realm
        .native_media
        .borrow()
        .prototypes
        .get(kind)
        .cloned()
        .ok_or_else(|| failure("InvalidStateError"))?;
    Ok(prototype
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?
        .into())
}
fn construct(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if !args.is_construct_call() || args.data().is_true() {
        throw_type_error(scope, "Illegal constructor");
        return;
    }
    let result = (|| {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let mut values = Vec::new();
        if args.length() > 0 {
            let value =
                v8::Local::<v8::Object>::try_from(args.get(0)).map_err(|_| failure("TypeError"))?;
            if owner(scope, value, false).is_ok() {
                let array = tracks(scope, value)?;
                for i in 0..array.length() {
                    values.push(
                        array
                            .get_index(scope, i)
                            .ok_or(V8BindingError::JavaScriptException)?,
                    );
                }
            } else {
                let key = v8::Symbol::get_iterator(scope);
                let method = value
                    .get(scope, key.into())
                    .ok_or(V8BindingError::JavaScriptException)?
                    .try_cast::<v8::Function>()
                    .map_err(|_| failure("TypeError"))?;
                iterable_conversion::consume(scope, value, method, |scope, value| {
                    if values.len() >= 1024 {
                        return Err(failure("QuotaExceededError"));
                    }
                    let object = v8::Local::<v8::Object>::try_from(value)
                        .map_err(|_| failure("TypeError"))?;
                    owner(scope, object, true)?;
                    if !values.iter().any(|old| old.strict_equals(value)) {
                        values.push(value);
                    }
                    Ok(())
                })?;
            }
        }
        let prototype = constructor_prototype(scope, args.new_target(), "MediaStream")?;
        let id = format!(
            "oryn-stream-{}-{}",
            state.instance_id,
            reserve_platform_object_id(&state)
        );
        let object = stream_object(scope, &state, id, &values)?;
        if object.set_prototype(scope, prototype) != Some(true) {
            return Err(V8BindingError::Allocation(
                "MediaStream constructor prototype",
            ));
        }
        Ok(object)
    })();
    match result {
        Ok(value) => rv.set(value.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn constraint_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.native.overconstrained.constraint")
        .ok_or(V8BindingError::Allocation("constraint key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn constraint_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let object = args.this();
        if object.is_proxy() {
            return Err(failure("TypeError"));
        }
        let key = constraint_key(scope)?;
        let value = object
            .get_private(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_string() {
            return Err(failure("TypeError"));
        }
        Ok(value)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn overconstrained<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constraint: &str,
    message: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let prototype = state
        .native_media
        .borrow()
        .prototypes
        .get("OverconstrainedError")
        .cloned()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let prototype = prototype
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?;
    let id = register_platform_object(
        &state,
        PlatformObjectRecord::DomException {
            name: "OverconstrainedError".into(),
            message: message.into(),
            code: 0,
        },
    );
    let object = new_platform_object(scope, &state, "DOMException", id)?;
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return Err(V8BindingError::Allocation("constraint prototype"));
    }
    let key = constraint_key(scope)?;
    let constraint = v8_string(scope, constraint)?;
    if object.set_private(scope, key, constraint) != Some(true) {
        return Err(V8BindingError::Allocation("constraint backing"));
    }
    Ok(object.into())
}
fn construct_error(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if !args.is_construct_call() || args.length() == 0 {
        throw_type_error(scope, "OverconstrainedError requires a constraint");
        return;
    }
    let result = (|| {
        let constraint = required_string(scope, &args, 0)?;
        let message = if args.get(1).is_undefined() {
            String::new()
        } else {
            required_string(scope, &args, 1)?
        };
        let prototype = constructor_prototype(scope, args.new_target(), "OverconstrainedError")?;
        let value = overconstrained(scope, &constraint, &message)?;
        let object = value
            .try_cast::<v8::Object>()
            .map_err(|_| failure("InvalidStateError"))?;
        if object.set_prototype(scope, prototype) != Some(true) {
            return Err(V8BindingError::Allocation(
                "OverconstrainedError constructor prototype",
            ));
        }
        Ok(value)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let global = context.global(scope);
    for kind in ["MediaStream", "MediaStreamTrack"] {
        let track = kind == "MediaStreamTrack";
        let data = v8::Boolean::new(scope, track);
        let template = v8::FunctionTemplate::builder(construct)
            .data(data.into())
            .build(scope);
        crate::v8_context_bridge::defer_html_constructor_prototype(template);
        let constructor = template.get_function(scope).unwrap();
        let key = v8::String::new(scope, "prototype").unwrap();
        let prototype = constructor
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap();
        let event = register_platform_object(&state, PlatformObjectRecord::EventTarget);
        let object = new_platform_object(scope, &state, "EventTarget", event)?;
        let parent = object.get_prototype(scope).unwrap();
        prototype.set_prototype(scope, parent);
        let properties: &[(&str, i32)] = if track {
            &[
                ("id", 10),
                ("kind", 11),
                ("label", 12),
                ("enabled", 13),
                ("muted", 14),
                ("readyState", 15),
            ]
        } else {
            &[("id", 0), ("active", 1)]
        };
        for (name, index) in properties {
            let data = v8::Integer::new(scope, *index);
            let get = v8::Function::builder(read)
                .data(data.into())
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .unwrap();
            let get_name = v8::String::new(scope, &format!("get {name}")).unwrap();
            get.set_name(get_name);
            let set = if *name == "enabled" {
                let set = v8::Function::builder(enabled)
                    .length(1)
                    .constructor_behavior(v8::ConstructorBehavior::Throw)
                    .build(scope)
                    .unwrap();
                let name = v8::String::new(scope, &format!("set {name}")).unwrap();
                set.set_name(name);
                set.into()
            } else {
                v8::undefined(scope).into()
            };
            let mut property = v8::PropertyDescriptor::new_from_get_set(get.into(), set);
            property.set_enumerable(true);
            property.set_configurable(true);
            let key = v8::String::new(scope, name).unwrap();
            prototype.define_property(scope, key.into(), &property);
        }
        let methods: &[(&str, i32)] = if track {
            &[
                ("stop", 10),
                ("clone", 11),
                ("getSettings", 12),
                ("getConstraints", 13),
                ("getCapabilities", 14),
                ("applyConstraints", 15),
            ]
        } else {
            &[
                ("getTracks", 0),
                ("getAudioTracks", 1),
                ("getVideoTracks", 2),
                ("getTrackById", 3),
                ("addTrack", 4),
                ("removeTrack", 5),
                ("clone", 6),
            ]
        };
        for (name, index) in methods {
            let data = v8::Integer::new(scope, *index);
            let length = if matches!(*index, 3..=5) { 1 } else { 0 };
            let function = v8::Function::builder(call)
                .data(data.into())
                .length(length)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .unwrap();
            let key = v8::String::new(scope, name).unwrap();
            function.set_name(key);
            prototype.create_data_property(scope, key.into(), function.into());
        }
        for name in if track {
            vec!["ended", "mute", "unmute"]
        } else {
            vec!["addtrack", "removetrack"]
        } {
            let data = v8::String::new(scope, name).unwrap();
            let get = v8::Function::builder(handler_get)
                .data(data.into())
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .unwrap();
            let set = v8::Function::builder(handler_set)
                .data(data.into())
                .length(1)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .unwrap();
            let getter_name = v8::String::new(scope, &format!("get on{name}")).unwrap();
            get.set_name(getter_name);
            let setter_name = v8::String::new(scope, &format!("set on{name}")).unwrap();
            set.set_name(setter_name);
            let mut property = v8::PropertyDescriptor::new_from_get_set(get.into(), set.into());
            property.set_enumerable(true);
            property.set_configurable(true);
            let key = v8::String::new(scope, &format!("on{name}")).unwrap();
            prototype.define_property(scope, key.into(), &property);
        }
        let name = v8::String::new(scope, kind).unwrap();
        constructor.set_name(name);
        let tag = v8::Symbol::get_to_string_tag(scope);
        prototype.define_own_property(
            scope,
            tag.into(),
            name.into(),
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        );
        global.create_data_property(scope, name.into(), constructor.into());
        let retained = context_intrinsics::retain_object_for(
            scope,
            &state,
            &format!("native_media-prototype-{kind}"),
            prototype,
        )?;
        state
            .native_media
            .borrow_mut()
            .prototypes
            .insert(kind, retained);
    }
    let template = v8::FunctionTemplate::builder(construct_error)
        .length(1)
        .build(scope);
    crate::v8_context_bridge::defer_html_constructor_prototype(template);
    let constructor = template.get_function(scope).unwrap();
    let key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap();
    let error = dom_exception_value(scope, "OverconstrainedError")?;
    let error = v8::Local::<v8::Object>::try_from(error).unwrap();
    if let Some(parent) = error.get_prototype(scope) {
        prototype.set_prototype(scope, parent);
    }
    let name = v8::String::new(scope, "OverconstrainedError").unwrap();
    let tag = v8::Symbol::get_to_string_tag(scope);
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
    constructor.set_name(name);
    let getter = v8::Function::builder(constraint_get)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("constraint getter"))?;
    let getter_name = v8::String::new(scope, "get constraint").unwrap();
    getter.set_name(getter_name);
    let undefined = v8::undefined(scope);
    let mut property = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    property.set_enumerable(true);
    property.set_configurable(true);
    let key = v8::String::new(scope, "constraint").unwrap();
    if prototype.define_property(scope, key.into(), &property) != Some(true) {
        return Err(V8BindingError::Allocation("constraint descriptor"));
    }
    global.create_data_property(scope, name.into(), constructor.into());
    let retained = context_intrinsics::retain_object_for(
        scope,
        &state,
        "native_media-prototype-OverconstrainedError",
        prototype,
    )?;
    state
        .native_media
        .borrow_mut()
        .prototypes
        .insert("OverconstrainedError", retained);
    Ok(())
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let captures = {
        let media = state.native_media.borrow();
        media
            .captures
            .keys()
            .chain(media.captured.keys())
            .chain(media.canceling.iter())
            .copied()
            .collect::<BTreeSet<_>>()
    };
    for node in captures {
        let dom = state.dom.borrow();
        let current = dom.is_connected(node).unwrap_or(false)
            && dom.node_document(node).ok().flatten() == Some(state.document_root());
        drop(dom);
        if !current || capture_canceling(state, node) {
            cancel_capture(state, node)?;
        }
    }
    let pending = state.native_media.borrow().pending.clone();
    for (id, remove) in pending {
        release(state, id, remove);
    }
    state.native_media.borrow_mut().retried = Some(std::time::Instant::now());
    let tracks = state
        .native_media
        .borrow()
        .tracks
        .iter()
        .map(|(id, track)| (*id, track.clone()))
        .collect::<Vec<_>>();
    let mut count = 0;
    for (id, old) in tracks {
        if state
            .platform_wrappers
            .borrow()
            .get(&id)
            .and_then(|value| value.to_local(scope))
            .is_none()
        {
            release(state, old.id, true);
            state.native_media.borrow_mut().tracks.remove(&id);
            state.native_media.borrow_mut().muted.remove(&id);
            continue;
        }
        if old.ended {
            continue;
        }
        let next = match request(state, Request::Inspect { id: old.id.clone() }) {
            Ok(Response::Track(track)) => track,
            Err(_) => {
                if state.native_media.borrow_mut().muted.insert(id) {
                    dispatch_platform_event(scope, state, id, "mute", None)?;
                    count += 1;
                }
                continue;
            }
            _ => return Err(failure("InvalidStateError")),
        };
        let was_muted = state.native_media.borrow_mut().muted.remove(&id);
        if was_muted && !next.ended {
            dispatch_platform_event(scope, state, id, "unmute", None)?;
            count += 1;
            if state.execution_retired() {
                break;
            }
        }
        let ended = next.ended;
        state.native_media.borrow_mut().tracks.insert(id, next);
        if ended {
            dispatch_platform_event(scope, state, id, "ended", None)?;
            count += 1;
        }
        if state.execution_retired() {
            break;
        }
    }
    let ids = state
        .native_media
        .borrow()
        .streams
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in ids {
        if state
            .platform_wrappers
            .borrow()
            .get(&id)
            .and_then(|value| value.to_local(scope))
            .is_none()
        {
            state.native_media.borrow_mut().streams.remove(&id);
        }
    }
    count += media_element::poll(scope, state)?;
    Ok(count)
}
pub(super) fn retire(state: &V8BindingState) {
    let events = std::mem::take(&mut state.native_media.borrow_mut().events);
    drop(events);
    let _ = media_resource::clear(state);
    let mut media = state.native_media.borrow_mut();
    for track in media.tracks.values_mut() {
        track.ended = true;
    }
    media.pending.clear();
    media.captures.clear();
    media.captured.clear();
    media.canceling.clear();
    media.muted.clear();
    media.audio_cleanup.clear();
    let nodes = media.playback.keys().copied().collect::<Vec<_>>();
    media.playback.clear();
    drop(media);
    for node in nodes {
        state.decoded_images.borrow_mut().remove(&node);
        let _ = state.dom.borrow_mut().set_media_frame(node, None);
    }
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    let media = state.native_media.borrow();
    if state.execution_retired() {
        return None;
    }
    let fast = !media.events.is_empty()
        || !media.captures.is_empty()
        || !media.captured.is_empty()
        || !media.canceling.is_empty()
        || media.playback.values().any(media_element::is_playing);
    if !fast
        && !media.resources.has_work()
        && media.pending.is_empty()
        && media.audio_cleanup.is_empty()
        && !media.tracks.values().any(|track| !track.ended)
    {
        return None;
    }
    let interval = std::time::Duration::from_millis(if fast { 16 } else { 100 });
    Some(
        media
            .retried
            .map(|time| interval.saturating_sub(time.elapsed()))
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn single_audio_capture_preserves_native_track_identity_and_rejects_video() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        let audio = track_object(
            scope,
            &state,
            Track {
                id: "microphone-audio".into(),
                kind: Kind::Audio,
                label: "microphone".into(),
                enabled: true,
                ended: false,
                settings: serde_json::json!({"sampleSize":16}),
                constraints: serde_json::json!({"sampleSize":16}),
            },
        )
        .unwrap();
        let stream =
            stream_object(scope, &state, "microphone-stream".into(), &[audio.into()]).unwrap();
        assert!(
            single_track(scope, &state, stream, Kind::Audio)
                .unwrap()
                .strict_equals(audio.into())
        );
        assert!(single_track(scope, &state, stream, Kind::Video).is_err());
        let empty = stream_object(scope, &state, "empty-stream".into(), &[]).unwrap();
        assert!(single_track(scope, &state, empty, Kind::Audio).is_err());
        let duplicate = stream_object(
            scope,
            &state,
            "duplicate-stream".into(),
            &[audio.into(), audio.into()],
        )
        .unwrap();
        assert!(single_track(scope, &state, duplicate, Kind::Audio).is_err());
    }
    #[test]
    fn constructor_prototype_conversion_order_subclasses_and_foreign_fallback() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(210), 1).unwrap()
        };
        let (b, foreign) = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(211), 1).unwrap();
            let function = eval(
                scope,
                "(function(){function Foreign(){};Foreign.prototype=1;return Foreign})()",
            );
            (state, v8::Global::new(scope, function))
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let key = v8::String::new(scope, "Foreign").unwrap();
        let foreign = v8::Local::new(scope, &foreign);
        first
            .global(scope)
            .create_data_property(scope, key.into(), foreign);
        for (name, kind) in [
            ("foreignMediaPrototype", "MediaStream"),
            ("foreignErrorPrototype", "OverconstrainedError"),
        ] {
            let prototype = b.native_media.borrow().prototypes[kind].clone();
            let prototype = prototype.to_local(scope).unwrap();
            let key = v8::String::new(scope, name).unwrap();
            first
                .global(scope)
                .create_data_property(scope, key.into(), prototype.into());
        }
        assert!(eval(scope,r#"(()=>{
          const check=(value,message)=>{if(!value)throw Error(message)};
          class StreamSubclass extends MediaStream{};class ErrorSubclass extends OverconstrainedError{};
          check(new StreamSubclass() instanceof StreamSubclass,'stream subclass');check(new ErrorSubclass('x') instanceof ErrorSubclass,'error subclass');
          check(Object.getPrototypeOf(Reflect.construct(MediaStream,[],Foreign))===foreignMediaPrototype,'foreign stream fallback');
          check(Object.getPrototypeOf(Reflect.construct(OverconstrainedError,['x'],Foreign))===foreignErrorPrototype,'foreign error fallback');
          let order=[];const prototype={};const target=new Proxy(function(){},{get(t,key,receiver){if(key==='prototype'){order.push('prototype');return prototype}return Reflect.get(t,key,receiver)}});
          const input={[Symbol.iterator](){order.push('iterator');return [][Symbol.iterator]()}};
          check(Object.getPrototypeOf(Reflect.construct(MediaStream,[input],target))===prototype&&order.join(',')==='iterator,prototype','stream conversion before one prototype lookup');
          order=[];Reflect.construct(OverconstrainedError,[{toString(){order.push('constraint');return 'x'}},{toString(){order.push('message');return 'm'}}],target);check(order.join(',')==='constraint,message,prototype','error conversion order');
          order=[];let illegal=false;try{Reflect.construct(MediaStreamTrack,[],target)}catch(error){illegal=error instanceof TypeError}check(illegal&&order.length===0,'illegal constructor before prototype getter');
          for(const objectPrototype of [true,false]){let proxy,revoke;({proxy,revoke}=Proxy.revocable(function(){},{get(t,key,receiver){if(key==='prototype'){revoke();return objectPrototype?prototype:0}return Reflect.get(t,key,receiver)}}));let result,error;try{result=Reflect.construct(MediaStream,[],proxy)}catch(value){error=value}check(objectPrototype?Object.getPrototypeOf(result)===prototype:error instanceof TypeError,'revocation only requires realm for fallback');}
          return true;
        })()"#).is_true());
        let _ = a;
    }
    #[test]
    fn pending_capture_cancellation_retains_identity_until_terminal_ack() {
        #[derive(Default)]
        struct CancellationFixture(Mutex<Vec<String>>);
        impl crate::platform_control::Backend for CancellationFixture {
            fn request(
                &self,
                creator: crate::worker_wire::Creator,
                request: crate::platform_control::Request,
            ) -> Result<crate::platform_control::Response, String> {
                if creator
                    != (crate::worker_wire::Creator::Window {
                        target: crate::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    })
                {
                    return Err("foreign media fixture creator".into());
                }
                let crate::platform_control::Request::Media {
                    request: Request::CancelCapture { id },
                } = request
                else {
                    return Err("unexpected request during cancellation".into());
                };
                let mut calls = self.0.lock().unwrap();
                calls.push(id);
                match calls.len() {
                    1 => Err("AbortError: interrupted cancellation acknowledgement".into()),
                    2 => Ok(crate::platform_control::Response::Media {
                        response: Response::Pending {
                            id: "unrelated-response".into(),
                        },
                    }),
                    _ => Ok(crate::platform_control::Response::Media {
                        response: Response::Stopped,
                    }),
                }
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<usermedia id=u></usermedia>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let messages = media_creator_messages(&state);
        let fixture = Arc::new(CancellationFixture::default());
        state.set_native_platform_backend(fixture.clone());
        let node = state.dom.borrow().find_by_id("u").unwrap();
        state
            .native_media
            .borrow_mut()
            .captures
            .insert(node, "original-pending-id".into());
        for _ in 0..2 {
            assert!(cancel_capture(&state, node).is_err());
            assert_eq!(
                state
                    .native_media
                    .borrow()
                    .captures
                    .get(&node)
                    .map(String::as_str),
                Some("original-pending-id")
            );
            assert!(capture_pending(&state, node));
        }
        cancel_capture(&state, node).unwrap();
        assert!(!capture_pending(&state, node));
        cancel_capture(&state, node).unwrap();
        assert_eq!(
            *fixture.0.lock().unwrap(),
            vec!["original-pending-id".to_string(); 3]
        );
        messages.assert_lifecycle(0);
    }
    fn media_creator_messages(
        state: &V8BindingState,
    ) -> Arc<native_user_activation::test_transport::Messages> {
        let messages = Arc::new(native_user_activation::test_transport::Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        // Exercise the same parent admission as media_request before injecting
        // media-specific transport failures; do not fabricate a creator token.
        state.native_worker_creator().unwrap();
        messages.assert_lifecycle(0);
        messages
    }
    #[derive(Default)]
    struct CaptureFixture {
        inspect_failure: Mutex<bool>,
        sample_backpressure: Mutex<bool>,
        sample_success: Mutex<bool>,
        tracks: Mutex<BTreeMap<String, Track>>,
        clones: Mutex<usize>,
        fail_second_clone: Mutex<bool>,
        fail_release: Mutex<bool>,
    }
    impl crate::platform_control::Backend for CaptureFixture {
        fn request(
            &self,
            creator: crate::worker_wire::Creator,
            request: crate::platform_control::Request,
        ) -> Result<crate::platform_control::Response, String> {
            if creator
                != (crate::worker_wire::Creator::Window {
                    target: crate::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                })
            {
                return Err("foreign media fixture creator".into());
            }
            let crate::platform_control::Request::Media { request } = request else {
                return Err("unexpected fixture request".into());
            };
            let mut tracks = self.tracks.lock().unwrap();
            let response = match request {
                Request::Clone { id } => {
                    let mut count = self.clones.lock().unwrap();
                    *count += 1;
                    if *count == 2 && *self.fail_second_clone.lock().unwrap() {
                        return Err("NotReadableError: injected second allocation failure".into());
                    }
                    let mut track = tracks.get(&id).ok_or("unknown fixture track")?.clone();
                    track.id = format!("clone-{count}");
                    tracks.insert(track.id.clone(), track.clone());
                    Response::Track(track)
                }
                Request::Release { id } => {
                    if *self.fail_release.lock().unwrap() {
                        return Err("injected transport failure".into());
                    }
                    tracks.remove(&id);
                    Response::Stopped
                }
                Request::Stop { id } => {
                    tracks.get_mut(&id).ok_or("unknown fixture track")?.ended = true;
                    Response::Stopped
                }
                Request::Inspect { id } => {
                    if *self.inspect_failure.lock().unwrap() {
                        return Err(
                            "NotReadableError: injected observation transport failure".into()
                        );
                    }
                    Response::Track(tracks.get(&id).ok_or("unknown fixture track")?.clone())
                }
                Request::Enable { id, enabled } => {
                    let track = tracks.get_mut(&id).ok_or("unknown fixture track")?;
                    track.enabled = enabled;
                    Response::Track(track.clone())
                }
                Request::Capabilities { id } => {
                    if !tracks.contains_key(&id) {
                        return Err("unknown fixture track".into());
                    }
                    Response::Capabilities(serde_json::json!({"width":{"min":1,"max":1920}}))
                }
                Request::ApplyConstraints { id, constraints } => {
                    let track = tracks.get_mut(&id).ok_or("unknown fixture track")?;
                    if constraints
                        .get("width")
                        .and_then(|v| v.get("exact"))
                        .and_then(serde_json::Value::as_u64)
                        == Some(9000)
                    {
                        return Err("OverconstrainedError: width".into());
                    }
                    track.constraints = constraints;
                    Response::Track(track.clone())
                }
                Request::Sample { .. } => {
                    if *self.sample_backpressure.lock().unwrap() {
                        return Ok(crate::platform_control::Response::Media {
                            response: Response::Sample(
                                serde_json::json!({"kind":"video","sequence":1,"timestamp":0.1,"backpressure":true,"rendered":false}),
                            ),
                        });
                    }
                    if !*self.sample_success.lock().unwrap() {
                        return Err("NotReadableError: injected sample failure".into());
                    }
                    Response::Sample(
                        serde_json::json!({"kind":"video","sequence":1,"timestamp":0.1,"width":1,"height":1,"data":BASE64.encode([12u8,23,34,255]),"ended":false,"dropped":0}),
                    )
                }
                Request::PollCapture { .. }
                | Request::CancelCapture { .. }
                | Request::AudioRender { .. } => {
                    return Err("sample delivery uses parent production provider".into());
                }
                Request::Capture { .. } => {
                    return Err("capture is covered through the parent production route".into());
                }
                Request::UrlOpen { .. }
                | Request::UrlSample { .. }
                | Request::UrlRelease { .. }
                | Request::UrlCanPlay { .. }
                | Request::UrlPictureInPicture { .. }
                | Request::UrlPictureInPictureState { .. } => {
                    return Err("URL media has a separate decoder fixture".into());
                }
            };
            Ok(crate::platform_control::Response::Media { response })
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
    fn playback_sample_failure_settles_promises_and_same_stream_replacement_retires_old_callbacks()
    {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><video id=player></video></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let messages = media_creator_messages(&state);
        let fixture = Arc::new(CaptureFixture::default());
        state.set_native_platform_backend(fixture.clone());
        let track = Track {
            id: "video".into(),
            kind: Kind::Video,
            label: "fixture".into(),
            enabled: true,
            ended: false,
            settings: serde_json::json!({}),
            constraints: serde_json::json!({}),
        };
        fixture
            .tracks
            .lock()
            .unwrap()
            .insert(track.id.clone(), track.clone());
        let track = track_object(scope, &state, track).unwrap();
        let stream = stream_object(scope, &state, "stream".into(), &[track.into()]).unwrap();
        let key = v8::String::new(scope, "stream").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), stream.into());
        eval(
            scope,
            "globalThis.player=document.getElementById('player');globalThis.errors=[];player.onerror=e=>errors.push([e.isTrusted,player.error.code]);player.srcObject=stream;globalThis.result='pending';player.play().then(()=>result='resolved',e=>result=e.name)",
        );
        assert!(media_element::poll(scope, &state).unwrap() > 0);
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"result==='NotReadableError'&&player.paused&&player.error instanceof MediaError&&errors.length===1&&errors[0][0]&&errors[0][1]===3&&document.body.contains(player)").is_true());
        *fixture.sample_backpressure.lock().unwrap() = true;
        eval(
            scope,
            "player.srcObject=stream;globalThis.pressured='pending';player.play().then(()=>pressured='resolved',e=>pressured=e.name)",
        );
        media_element::poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(
            eval(
                scope,
                "pressured==='pending'&&player.error===null&&!player.paused"
            )
            .is_true()
        );
        *fixture.sample_backpressure.lock().unwrap() = false;
        *fixture.sample_success.lock().unwrap() = true;
        media_element::poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "pressured==='resolved'&&player.readyState===4").is_true());
        let resources = super::super::native_drag_image::snapshot_native_paint_resources(
            &state,
            state.document_root(),
        )
        .unwrap();
        let player = state.dom.borrow().find_by_id("player").unwrap();
        let resource = resources
            .iter()
            .find(|resource| resource.node == player)
            .unwrap();
        assert_eq!((resource.width, resource.height), (1, 1));
        assert_eq!(
            image::load_from_memory(&resource.png)
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0,
            [12, 23, 34, 255]
        );
        assert_eq!(
            state
                .dom
                .borrow()
                .arena()
                .get(player)
                .unwrap()
                .media_frame
                .as_ref(),
            Some(&resource.png)
        );
        *fixture.sample_success.lock().unwrap() = true;
        eval(
            scope,
            "player.muted=true;player.volume=0.25;player.srcObject=stream;globalThis.seen=[];player.onloadedmetadata=()=>{seen.push('metadata');player.srcObject=stream};player.onloadeddata=()=>seen.push('data');globalThis.replaced='pending';player.play().then(()=>replaced='resolved',e=>replaced=e.name)",
        );
        media_element::poll(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"seen.join(',')==='metadata'&&replaced==='AbortError'&&player.srcObject===stream&&player.readyState===0&&player.muted&&player.volume===0.25&&player.error===null").is_true());
        eval(
            scope,
            "globalThis.trackEvents=[];stream.getVideoTracks()[0].onmute=()=>trackEvents.push('mute');stream.getVideoTracks()[0].onunmute=()=>trackEvents.push('unmute')",
        );
        *fixture.inspect_failure.lock().unwrap() = true;
        poll(scope, &state).unwrap();
        assert!(eval(scope,"stream.getVideoTracks()[0].muted&&stream.getVideoTracks()[0].readyState==='live'&&trackEvents.join(',')==='mute'").is_true());
        *fixture.inspect_failure.lock().unwrap() = false;
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "!stream.getVideoTracks()[0].muted&&trackEvents.join(',')==='mute,unmute'"
            )
            .is_true()
        );
        messages.assert_lifecycle(0);
    }
    #[test]
    fn accepted_capture_retains_native_ids_and_partial_wrappers_until_publication() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<usermedia id=u></usermedia>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let messages = media_creator_messages(&state);
        let fixture = Arc::new(CaptureFixture::default());
        state.set_native_platform_backend(fixture.clone());
        let node = state.dom.borrow().find_by_id("u").unwrap();
        let tracks = [(Kind::Audio, "audio"), (Kind::Video, "video")]
            .into_iter()
            .map(|(kind, id)| Track {
                id: id.into(),
                kind,
                label: "fixture".into(),
                enabled: true,
                ended: false,
                settings: serde_json::json!({}),
                constraints: serde_json::json!({}),
            })
            .collect::<Vec<_>>();
        for track in &tracks {
            fixture
                .tracks
                .lock()
                .unwrap()
                .insert(track.id.clone(), track.clone());
        }
        state.native_media.borrow_mut().captured.insert(
            node,
            Captured {
                stream: Stream {
                    id: "accepted-stream".into(),
                    tracks: tracks.clone(),
                },
                wrappers: Vec::new(),
            },
        );
        FAIL_CAPTURE_PUBLICATION.with(|flag| flag.set(true));
        assert!(capture(scope, &state, node, serde_json::json!({})).is_err());
        assert!(capture_pending(&state, node));
        assert_eq!(fixture.tracks.lock().unwrap().len(), 2);
        let first = state.native_media.borrow().captured[&node].wrappers[0].clone();
        FAIL_STREAM_PUBLICATION.with(|flag| flag.set(true));
        assert!(capture(scope, &state, node, serde_json::json!({})).is_err());
        assert!(capture_pending(&state, node));
        assert_eq!(
            state.native_media.borrow().captured[&node].wrappers.len(),
            2
        );
        assert!(state.native_media.borrow().streams.is_empty());
        assert_eq!(fixture.tracks.lock().unwrap().len(), 2);
        let stream = capture(scope, &state, node, serde_json::json!({}))
            .unwrap()
            .unwrap();
        assert!(!capture_pending(&state, node));
        let values = super::tracks(scope, stream).unwrap();
        let published = values.get_index(scope, 0).unwrap();
        assert!(published.strict_equals(v8::Local::new(scope, &first).into()));
        assert_eq!(values.length(), 2);
        // Cancellation after accepted capture preserves failed native release
        // obligations without keeping obsolete wrapper-construction custody.
        state.native_media.borrow_mut().captured.insert(
            node,
            Captured {
                stream: Stream {
                    id: "abandoned-stream".into(),
                    tracks,
                },
                wrappers: Vec::new(),
            },
        );
        *fixture.fail_release.lock().unwrap() = true;
        cancel_capture(&state, node).unwrap();
        assert!(!capture_pending(&state, node));
        assert_eq!(state.native_media.borrow().pending.len(), 2);
        *fixture.fail_release.lock().unwrap() = false;
        let releases = state.native_media.borrow().pending.clone();
        for (id, remove) in releases {
            release(&state, id, remove);
        }
        assert!(fixture.tracks.lock().unwrap().is_empty());
        assert!(state.native_media.borrow().pending.is_empty());
        messages.assert_lifecycle(0);
    }
    #[test]
    fn native_track_identity_iterator_close_clone_rollback_and_cleanup_retry() {
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
        let messages = media_creator_messages(&state);
        let fixture = Arc::new(CaptureFixture::default());
        state.set_native_platform_backend(fixture.clone());
        let mut values = Vec::new();
        for (kind, id) in [(Kind::Audio, "audio"), (Kind::Video, "video")] {
            let track = Track {
                id: id.into(),
                kind,
                label: "explicit unit fixture".into(),
                enabled: true,
                ended: false,
                settings: serde_json::json!({"width":640}),
                constraints: serde_json::json!({}),
            };
            fixture
                .tracks
                .lock()
                .unwrap()
                .insert(id.into(), track.clone());
            values.push(track_object(scope, &state, track).unwrap().into());
        }
        let stream = stream_object(scope, &state, "fixture-stream".into(), &values).unwrap();
        let key = v8::String::new(scope, "stream").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), stream.into());
        assert!(eval(scope,"stream instanceof MediaStream && stream instanceof EventTarget && stream.getTracks().every(t=>t instanceof MediaStreamTrack) && stream.getAudioTracks()[0]===stream.getTrackById('audio') && stream.active").is_true());
        assert!(eval(scope,r#"(()=>{
          if(new MediaStream().getTracks().length!==0)return false;
          for(const value of [undefined,null,1,'text',{}]){let rejected=false;try{new MediaStream(value)}catch(error){rejected=error instanceof TypeError}if(!rejected)return false;}
          const marker={};let exact=false;try{new MediaStream({get [Symbol.iterator](){throw marker}})}catch(error){exact=error===marker}if(!exact)return false;
          Object.defineProperty(stream,Symbol.iterator,{configurable:true,get(){throw marker}});
          let copy;try{copy=new MediaStream(stream)}finally{delete stream[Symbol.iterator]}
          const original=stream.getTracks();return copy!==stream&&copy.getTracks().every((track,index)=>track===original[index])&&copy.getTracks().length===original.length;
        })()"#).is_true());
        assert!(eval(scope,"(()=>{for(const proto of [MediaStream.prototype,MediaStreamTrack.prototype])for(const name of Object.keys(proto)){const d=Object.getOwnPropertyDescriptor(proto,name);if(typeof d.value==='function'){if(d.value.name!==name||Object.hasOwn(d.value,'prototype')||d.value.length!==(['getTrackById','addTrack','removeTrack'].includes(name)?1:0))return false}}let missing=false;try{stream.getTrackById()}catch(error){missing=error instanceof TypeError}return missing&&stream.getTrackById(undefined)===null})()").is_true());
        assert!(eval(scope,"(()=>{const track=stream.getVideoTracks()[0];let calls=0;const removed=stream.getTrackById({toString(){calls++;stream.removeTrack(track);return 'video'}});const added=stream.getTrackById({toString(){calls++;stream.addTrack(track);return 'video'}});const marker={};let exact=false;try{stream.getTrackById({toString(){stream.removeTrack(track);throw marker}})}catch(error){exact=error===marker}const absent=stream.getTrackById('video')===null;stream.addTrack(track);return removed===null&&added===track&&calls===2&&exact&&absent})()").is_true());
        assert!(eval(scope,"(()=>{let closed=false;const iterator={next(){return {value:{id:'audio'},done:false}},return(){closed=true;return {done:true}}};try{new MediaStream({[Symbol.iterator](){return iterator}})}catch(e){return e instanceof TypeError && closed}return false})()").is_true());
        assert!(eval(scope,"(()=>{let failed=false;try{MediaStreamTrack.prototype.stop.call({id:'audio'})}catch(e){failed=e instanceof TypeError}const a=stream.getAudioTracks()[0];Object.defineProperty(a,'id',{get(){throw Error('author getter must not be custody')}});a.enabled=false;const settings=a.getSettings();settings.width=1;return failed && !a.enabled && a.getSettings().width===640})()").is_true());
        assert!(eval(scope,"stream.getVideoTracks()[0].getCapabilities().width.max===1920 && new OverconstrainedError('width','bad size') instanceof DOMException").is_true());
        assert!(eval(scope,r#"(()=>{
          for(const proto of [MediaStream.prototype,MediaStreamTrack.prototype])for(const name of Object.keys(proto)){
            const d=Object.getOwnPropertyDescriptor(proto,name);if(!d.get)continue;
            if(d.get.name!=='get '+name||d.get.length!==0||Object.hasOwn(d.get,'prototype')||!d.enumerable||!d.configurable)return false;
            if(d.set&&(d.set.name!=='set '+name||d.set.length!==1||Object.hasOwn(d.set,'prototype')))return false;
          }
          const d=Object.getOwnPropertyDescriptor(OverconstrainedError.prototype,'constraint');
          if(!d||d.set!==undefined||!d.enumerable||!d.configurable||d.get.name!=='get constraint'||d.get.length!==0||Object.hasOwn(d.get,'prototype')||OverconstrainedError.length!==1)return false;
          let order=[];const e=new OverconstrainedError({toString(){order.push('constraint');return '\ud800'}},{toString(){order.push('message');return '\udc00'}});
          if(e.constraint!=='\ud800'||e.message!=='\udc00'||Object.hasOwn(e,'constraint')||order.join(',')!=='constraint,message')return false;
          Object.defineProperty(e,'constraint',{value:'authored'});if(d.get.call(e)!=='\ud800')return false;
          for(const invalid of [{},Object.create(e),new DOMException('','OverconstrainedError'),new Proxy(e,{})]){let rejected=false;try{d.get.call(invalid)}catch(error){rejected=error instanceof TypeError}if(!rejected)return false;}
          const marker={};let caught=false;try{new OverconstrainedError({toString(){throw marker}},{toString(){throw Error('message must not convert')}})}catch(error){caught=error===marker}
          return caught&&new OverconstrainedError(undefined).constraint==='undefined';
        })()"#).is_true());
        eval(
            scope,
            "globalThis.marker={};globalThis.original=false;stream.getVideoTracks()[0].applyConstraints({get width(){throw marker}}).catch(e=>original=e===marker)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "original").is_true());
        eval(
            scope,
            "globalThis.over=false;stream.getVideoTracks()[0].applyConstraints({width:{exact:9000}}).catch(e=>over=e instanceof OverconstrainedError && e instanceof DOMException && e.constraint==='width')",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "over").is_true());
        eval(
            scope,
            "globalThis.applied=false;globalThis.order=[];stream.getVideoTracks()[0].applyConstraints({width:{get max(){order.push('max');return 1920},get min(){order.push('min');return 1},get exact(){order.push('exact');return 2.5},get ideal(){order.push('ideal');return 3.5}},deviceId:{exact:new Set(['video'])},advanced:[{width:100}]}).then(()=>applied=true)",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"applied && order.join(',')==='max,min,exact,ideal' && stream.getVideoTracks()[0].getConstraints().width.exact===2 && stream.getVideoTracks()[0].getConstraints().width.ideal===4 && stream.getVideoTracks()[0].getConstraints().deviceId.exact[0]==='video' && stream.getVideoTracks()[0].getConstraints().advanced[0].width===100").is_true());
        *fixture.fail_second_clone.lock().unwrap() = true;
        assert!(
            eval(
                scope,
                "(()=>{try{stream.clone();return false}catch(e){return true}})()"
            )
            .is_true()
        );
        assert_eq!(
            fixture.tracks.lock().unwrap().len(),
            2,
            "failed stream clone releases its first native allocation"
        );
        *fixture.fail_release.lock().unwrap() = true;
        release(&state, "audio".into(), true);
        assert_eq!(
            state.native_media.borrow().pending.get("audio"),
            Some(&true)
        );
        assert!(delay(&state).is_some());
        *fixture.fail_release.lock().unwrap() = false;
        // Retry custody without inspecting the deliberately released wrapper.
        let pending = state.native_media.borrow().pending.clone();
        for (id, remove) in pending {
            release(&state, id, remove);
        }
        assert!(state.native_media.borrow().pending.is_empty());
        assert!(!fixture.tracks.lock().unwrap().contains_key("audio"));
        eval(scope, "stream.getVideoTracks()[0].stop()");
        assert!(fixture.tracks.lock().unwrap()["video"].ended);
        retire(&state);
        assert!(
            eval(
                scope,
                "!stream.active && stream.getTracks().every(t=>t.readyState==='ended')"
            )
            .is_true()
        );
        messages.assert_lifecycle(0);
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
            let state = install_generated_bindings(scope, context, dom, RealmId(992), 1).unwrap();
            let entries = state.native_media.borrow().prototypes.clone();
            assert_eq!(entries.len(), 3);
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
                    .native_media
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
        assert_eq!(state.native_media.borrow().prototypes.len(), 3);
        assert!(
            state
                .native_media
                .borrow()
                .prototypes
                .values()
                .all(|value| value.to_local(handles).is_none())
        );
    }
}
