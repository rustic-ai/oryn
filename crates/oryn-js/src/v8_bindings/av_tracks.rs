//! Native audio/video track lists reflect admitted decoder or MediaStream tracks.
use super::*;
#[derive(Clone)]
pub(super) struct Descriptor {
    pub key: String,
    pub video: bool,
    pub label: String,
    pub language: String,
}
pub(super) struct Track {
    pub media: NodeId,
    pub session: u64,
    pub descriptor: Descriptor,
    pub enabled: bool,
}
#[derive(Default)]
pub(super) struct State {
    lists: BTreeMap<(NodeId, bool), u64>,
    members: BTreeMap<(NodeId, bool), Vec<u64>>,
    records: BTreeMap<(NodeId, u64, String), u64>,
}
fn error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "AudioTrack".into(),
        member: "TypeError".into(),
    }
}
fn record(state: &V8BindingState, id: u64) -> Result<Rc<RefCell<Track>>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::MediaTrack(record) => Ok(record.clone()),
        _ => Err(error()),
    }
}
pub(super) fn refresh(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    media: NodeId,
) -> Result<(), V8BindingError> {
    let (session, descriptors) = media_element::track_descriptors(scope, state, media)?;
    let mut selected = false;
    let mut members = [Vec::new(), Vec::new()];
    for descriptor in descriptors {
        let key = (media, session, descriptor.key.clone());
        let existing = state.av_tracks.borrow().records.get(&key).copied();
        let video = descriptor.video;
        let id = if let Some(id) = existing {
            id
        } else {
            if state.av_tracks.borrow().records.len() >= 4096 {
                return Err(V8BindingError::Allocation("media track quota"));
            }
            let enabled = !video || !selected;
            let id = register_platform_object(
                state,
                PlatformObjectRecord::MediaTrack(Rc::new(RefCell::new(Track {
                    media,
                    session,
                    descriptor,
                    enabled,
                }))),
            );
            state.av_tracks.borrow_mut().records.insert(key, id);
            id
        };
        if video && record(state, id)?.borrow().enabled {
            selected = true;
        }
        members[usize::from(video)].push(id);
    }
    for video in [false, true] {
        let key = (media, video);
        let next = std::mem::take(&mut members[usize::from(video)]);
        let previous = state
            .av_tracks
            .borrow()
            .members
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let list = state.av_tracks.borrow().lists.get(&key).copied();
        if previous != next {
            state
                .av_tracks
                .borrow_mut()
                .members
                .insert(key, next.clone());
            if let Some(list) = list {
                for id in previous.iter().filter(|id| !next.contains(id)) {
                    text_tracks::queue_platform(state, list, "removetrack", Some(*id));
                }
                for id in next.iter().filter(|id| !previous.contains(id)) {
                    text_tracks::queue_platform(state, list, "addtrack", Some(*id));
                }
            }
        }
    }
    Ok(())
}
pub(super) fn node_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    media: NodeId,
    video: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    text_tracks::bind_owner(state);
    refresh(scope, state, media)?;
    let key = (media, video);
    let existing = state.av_tracks.borrow().lists.get(&key).copied();
    let id = if let Some(id) = existing {
        id
    } else {
        let id = register_platform_object(
            state,
            PlatformObjectRecord::MediaTrackList {
                media,
                video,
                _root: Box::new(submissions::RootedTarget::new(state, media)?),
            },
        );
        state.av_tracks.borrow_mut().lists.insert(key, id);
        id
    };
    text_tracks::object(scope, state, id)
}
pub(super) fn ids(state: &V8BindingState, id: u64) -> Result<Vec<u64>, V8BindingError> {
    let (media, video) = match &*platform_object(state, id)? {
        PlatformObjectRecord::MediaTrackList { media, video, .. } => (*media, *video),
        _ => return Err(error()),
    };
    Ok(state
        .av_tracks
        .borrow()
        .members
        .get(&(media, video))
        .cloned()
        .unwrap_or_default())
}
pub(super) fn refresh_list(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let media = match &*platform_object(state, id)? {
        PlatformObjectRecord::MediaTrackList { media, .. } => Some(*media),
        _ => None,
    };
    if let Some(media) = media {
        refresh(scope, state, media)?
    }
    Ok(())
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::MediaTrackList { .. }
    ) {
        refresh_list(scope, state, id)?;
        let ids = ids(state, id)?;
        return Ok(v8::Integer::new(
            scope,
            if member == "selectedIndex" {
                ids.iter()
                    .position(|id| record(state, *id).is_ok_and(|track| track.borrow().enabled))
                    .map_or(-1, |index| index as i32)
            } else {
                ids.len() as i32
            },
        )
        .into());
    }
    let value = record(state, id)?;
    let value = value.borrow();
    match member {
        "enabled" | "selected" => Ok(v8::Boolean::new(scope, value.enabled).into()),
        "id" => v8_string(scope, &value.descriptor.key),
        "kind" => v8_string(scope, "main"),
        "label" => v8_string(scope, &value.descriptor.label),
        "language" => v8_string(scope, &value.descriptor.language),
        _ => Err(error()),
    }
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let record_value = record(state, id)?;
    let enabled = value.boolean_value(scope);
    let (media, video, session, previous) = {
        let track = record_value.borrow();
        (
            track.media,
            track.descriptor.video,
            track.session,
            track.enabled,
        )
    };
    if previous == enabled {
        return Ok(());
    }
    let current = state
        .av_tracks
        .borrow()
        .members
        .get(&(media, video))
        .is_some_and(|members| members.contains(&id));
    if current {
        media_element::track_selection_changed(scope, state, media, video && !enabled)?;
    }
    if video && enabled && current {
        let ids = state
            .av_tracks
            .borrow()
            .members
            .get(&(media, video))
            .cloned()
            .unwrap_or_default();
        for other in ids {
            if let Ok(other) = record(state, other)
                && other.borrow().session == session
            {
                other.borrow_mut().enabled = false;
            }
        }
    }
    record_value.borrow_mut().enabled = enabled;
    if current && let Some(list) = state.av_tracks.borrow().lists.get(&(media, video)).copied() {
        text_tracks::queue_platform(state, list, "change", None);
    }
    Ok(())
}
pub(super) fn enabled(state: &V8BindingState, media: NodeId, key: &str, video: bool) -> bool {
    let ids = state
        .av_tracks
        .borrow()
        .members
        .get(&(media, video))
        .cloned();
    let Some(ids) = ids else { return true };
    ids.into_iter()
        .find_map(|id| {
            record(state, id).ok().and_then(|record| {
                let record = record.borrow();
                (record.descriptor.key == key).then_some(record.enabled)
            })
        })
        .unwrap_or(true)
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let key = args
        .get(0)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    refresh_list(scope, state, id)?;
    for id in ids(state, id)? {
        let value = record(state, id)?;
        let value = value.borrow();
        if value.descriptor.key == key {
            return new_platform_object(
                scope,
                state,
                if value.descriptor.video {
                    "VideoTrack"
                } else {
                    "AudioTrack"
                },
                id,
            )
            .map(|value| Some(value.into()));
        }
    }
    Ok(Some(v8::null(scope).into()))
}

pub(super) fn migrate(
    source: &Rc<V8BindingState>,
    destination: &Rc<V8BindingState>,
    nodes: &BTreeMap<NodeId, NodeId>,
) -> Result<(), V8BindingError> {
    if Rc::ptr_eq(source, destination) {
        return Ok(());
    }
    let lists = source
        .av_tracks
        .borrow()
        .lists
        .iter()
        .filter_map(|((node, video), id)| nodes.get(node).map(|new| ((*node, *video), *new, *id)))
        .collect::<Vec<_>>();
    for (key, media, old) in lists {
        let root = submissions::RootedTarget::new(destination, media)?;
        let id = register_platform_object(
            destination,
            PlatformObjectRecord::MediaTrackList {
                media,
                video: key.1,
                _root: Box::new(root),
            },
        );
        source.av_tracks.borrow_mut().lists.remove(&key);
        source.av_tracks.borrow_mut().members.remove(&key);
        destination
            .av_tracks
            .borrow_mut()
            .lists
            .insert((media, key.1), id);
        text_tracks::forward_platform(source, old, destination, id);
    }
    Ok(())
}

pub(super) fn prune(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) {
    let active = text_tracks::active_platforms(state);
    let lists = state
        .av_tracks
        .borrow()
        .lists
        .iter()
        .filter_map(|(key, id)| {
            if active.contains(id)
                || state
                    .platform_wrappers
                    .borrow()
                    .get(id)
                    .and_then(|wrapper| wrapper.to_local(scope))
                    .is_some()
            {
                return None;
            }
            let dom = state.dom.borrow();
            if dom.is_connected(key.0).unwrap_or(false) {
                return None;
            }
            let wrapper = state
                .wrappers
                .borrow()
                .lookup(dom.arena().id(), state.realm, key.0)
                .and_then(|id| {
                    state
                        .objects
                        .borrow()
                        .get(&id)
                        .and_then(|value| value.handle.to_local(scope))
                });
            wrapper.is_none().then_some((*key, *id))
        })
        .collect::<Vec<_>>();
    for (key, id) in lists {
        state.av_tracks.borrow_mut().lists.remove(&key);
        state.av_tracks.borrow_mut().members.remove(&key);
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }
    let live = state
        .av_tracks
        .borrow()
        .members
        .values()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    let records = state
        .av_tracks
        .borrow()
        .records
        .iter()
        .filter_map(|(key, id)| {
            (!live.contains(id)
                && !active.contains(id)
                && state
                    .platform_wrappers
                    .borrow()
                    .get(id)
                    .and_then(|wrapper| wrapper.to_local(scope))
                    .is_none())
            .then_some((key.clone(), *id))
        })
        .collect::<Vec<_>>();
    for (key, id) in records {
        state.av_tracks.borrow_mut().records.remove(&key);
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }
}

pub(super) fn association(state: &V8BindingState, id: u64) -> Option<(NodeId, Option<u64>)> {
    let record = record(state, id).ok()?;
    let record = record.borrow();
    let all = state.av_tracks.borrow();
    let key = (record.media, record.descriptor.video);
    all.members
        .get(&key)
        .is_some_and(|members| members.contains(&id))
        .then(|| (record.media, all.lists.get(&key).copied()))
}
