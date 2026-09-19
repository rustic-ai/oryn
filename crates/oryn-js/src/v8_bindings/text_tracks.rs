//! Native text-track identity, cue lists, resource custody and cue events.
use super::native_platform::publication::Notification;
use super::*;
#[derive(Default)]
pub(super) struct State {
    region_refs: BTreeMap<u64, usize>,
    self_owner: std::rc::Weak<V8BindingState>,
    forward: BTreeMap<u64, (std::rc::Weak<V8BindingState>, u64)>,
    canonical: BTreeMap<u64, (std::rc::Weak<V8BindingState>, u64)>,
    nodes: BTreeMap<NodeId, u64>,
    tracks: BTreeMap<u64, Track>,
    lists: BTreeMap<(NodeId, u8), u64>,
    events: VecDeque<(Target, &'static str)>,
    delivery: Option<Notification>,
    delivery_targets: BTreeSet<u64>,
    dirty: bool,
    cursor: Option<u64>,
    polling: bool,
}
pub(super) struct Track {
    _node: Option<submissions::RootedTarget>,
    _media: Option<submissions::RootedTarget>,
    node: Option<NodeId>,
    media: Option<NodeId>,
    kind: String,
    label: String,
    language: String,
    mode: String,
    cues: Vec<u64>,
    active: Vec<u64>,
    cues_list: Option<u64>,
    active_list: Option<u64>,
    key: Option<Key>,
    generation: u64,
    ready: u16,
    last_sample: Option<(u64, u64, f64)>,
}
#[derive(Clone, PartialEq, Eq)]
struct Key {
    media: NodeId,
    url: String,
    cors: Option<String>,
}
#[derive(Clone)]
enum Target {
    Node(NodeId),
    Platform(u64),
    TrackEvent(u64, std::rc::Weak<V8BindingState>, u64),
}
#[derive(Clone, Copy)]
pub(super) enum List {
    Tracks(NodeId),
    Cues(u64, bool),
}
pub(super) struct Cue {
    data: oryn_dom::webvtt::Cue,
    track: Option<u64>,
    track_owner: Option<std::rc::Weak<V8BindingState>>,
    canonical: Option<(std::rc::Weak<V8BindingState>, u64)>,
    pause: bool,
    options: BTreeMap<String, serde_json::Value>,
    region: Option<(std::rc::Weak<V8BindingState>, u64)>,
}
impl Drop for Cue {
    fn drop(&mut self) {
        if let Some((owner, id)) = self.region.take() {
            if let Some(owner) = owner.upgrade() {
                let mut all = owner.text_tracks.borrow_mut();
                if let Some(count) = all.region_refs.get_mut(&id) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        all.region_refs.remove(&id);
                    }
                }
            }
        }
    }
}
pub(super) struct Region {
    id: String,
    values: BTreeMap<String, f64>,
    scroll: String,
}
pub(super) struct Pending {
    track: u64,
    generation: u64,
    document_generation: u64,
    epoch: u64,
    stream: Option<u64>,
    bytes: Vec<u8>,
}
impl Pending {
    pub(super) fn initiator(&self) -> &str {
        "track"
    }
}
fn fail(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "TextTrack".into(),
        member: name.into(),
    }
}
fn track<'a>(
    state: &'a V8BindingState,
    id: u64,
) -> Result<std::cell::Ref<'a, Track>, V8BindingError> {
    std::cell::Ref::filter_map(state.text_tracks.borrow(), |s| s.tracks.get(&id))
        .map_err(|_| fail("InvalidStateError"))
}
fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    binding_state_for_receiver(scope, object).ok_or_else(|| fail("TypeError"))
}
pub(super) fn bind_owner(state: &Rc<V8BindingState>) {
    state.text_tracks.borrow_mut().self_owner = Rc::downgrade(state);
}
pub(super) fn forward_platform(
    source: &Rc<V8BindingState>,
    old: u64,
    destination: &Rc<V8BindingState>,
    new: u64,
) {
    let canonical = source
        .text_tracks
        .borrow()
        .canonical
        .get(&old)
        .cloned()
        .unwrap_or_else(|| (Rc::downgrade(source), old));
    if let Some(owner) = canonical.0.upgrade() {
        owner
            .text_tracks
            .borrow_mut()
            .forward
            .insert(canonical.1, (Rc::downgrade(destination), new));
    }
    source
        .text_tracks
        .borrow_mut()
        .forward
        .insert(old, (Rc::downgrade(destination), new));
    destination
        .text_tracks
        .borrow_mut()
        .canonical
        .insert(new, canonical);
}
pub(super) fn resolve(
    mut state: Rc<V8BindingState>,
    mut id: u64,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    for _ in 0..4096 {
        let next = state.text_tracks.borrow().forward.get(&id).cloned();
        let Some((owner, next)) = next else {
            return Ok((state, id));
        };
        state = owner.upgrade().ok_or_else(|| fail("InvalidStateError"))?;
        id = next;
    }
    Err(fail("InvalidStateError"))
}
fn cue(state: &V8BindingState, id: u64) -> Result<Rc<RefCell<Cue>>, V8BindingError> {
    match &*platform_object(state, id)? {
        PlatformObjectRecord::TextTrackCue(value) => Ok(value.clone()),
        _ => Err(fail("TypeError")),
    }
}
fn cue_record(data: oryn_dom::webvtt::Cue) -> Cue {
    let mut options = BTreeMap::new();
    for (k, v) in [
        ("vertical", serde_json::json!("")),
        ("snapToLines", serde_json::json!(true)),
        ("line", serde_json::json!("auto")),
        ("lineAlign", serde_json::json!("start")),
        ("position", serde_json::json!("auto")),
        ("positionAlign", serde_json::json!("auto")),
        ("size", serde_json::json!(100)),
        ("align", serde_json::json!("center")),
    ] {
        options.insert(k.into(), v);
    }
    for (k, v) in &data.settings {
        match k.as_str() {
            "line" | "position" => {
                let mut parts = v.split(',');
                let position = parts.next().unwrap();
                let value = if position == "auto" {
                    serde_json::json!("auto")
                } else {
                    serde_json::json!(position.trim_end_matches('%').parse::<f64>().unwrap_or(0.))
                };
                options.insert(k.clone(), value);
                if k == "line" && position.ends_with('%') {
                    options.insert("snapToLines".into(), serde_json::json!(false));
                }
                if let Some(align) = parts.next() {
                    options.insert(format!("{k}Align"), serde_json::json!(align));
                }
            }
            "size" => {
                options.insert(
                    k.clone(),
                    serde_json::json!(v.trim_end_matches('%').parse::<f64>().unwrap_or(100.)),
                );
            }
            "vertical" | "align" => {
                options.insert(k.clone(), serde_json::json!(v));
            }
            _ => {}
        }
    }
    Cue {
        data,
        track: None,
        track_owner: None,
        canonical: None,
        pause: false,
        options,
        region: None,
    }
}
fn cue_capacity(state: &V8BindingState, count: usize, bytes: usize) -> Result<(), V8BindingError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut retained = 0usize;
    for record in state.platform_objects.borrow().values() {
        if let PlatformObjectRecord::TextTrackCue(cue) = record {
            if seen.insert(Rc::as_ptr(cue) as usize) {
                let cue = cue.borrow();
                retained = retained
                    .saturating_add(cue.data.text.len())
                    .saturating_add(cue.data.id.len());
            }
        }
    }
    if seen.len().saturating_add(count) > oryn_dom::webvtt::MAX_CUES
        || retained.saturating_add(bytes) > 32 * 1024 * 1024
    {
        Err(fail("QuotaExceededError"))
    } else {
        Ok(())
    }
}
fn new_track(
    state: &V8BindingState,
    node: Option<NodeId>,
    media: Option<NodeId>,
    kind: String,
    label: String,
    language: String,
    mode: &str,
) -> Result<u64, V8BindingError> {
    if state.text_tracks.borrow().tracks.len() >= 4096 {
        return Err(fail("QuotaExceededError"));
    }
    let node_root = node
        .map(|node| submissions::RootedTarget::new(state, node))
        .transpose()?;
    let media_root = media
        .map(|node| submissions::RootedTarget::new(state, node))
        .transpose()?;
    let id = register_platform_object(state, PlatformObjectRecord::TextTrack);
    state.text_tracks.borrow_mut().tracks.insert(
        id,
        Track {
            _node: node_root,
            _media: media_root,
            node,
            media,
            kind,
            label,
            language,
            mode: mode.into(),
            cues: Vec::new(),
            active: Vec::new(),
            cues_list: None,
            active_list: None,
            key: None,
            generation: 0,
            ready: 0,
            last_sample: None,
        },
    );
    Ok(id)
}
fn ensure(state: &V8BindingState, node: NodeId) -> Result<u64, V8BindingError> {
    if let Some(id) = state.text_tracks.borrow().nodes.get(&node) {
        return Ok(*id);
    }
    let (kind, label, language, media, default) = {
        let dom = state.dom.borrow();
        let attr = |name| {
            dom.attribute_in_null_namespace(node, name)
                .ok()
                .flatten()
                .unwrap_or("")
                .to_owned()
        };
        let kind = dom
            .attribute_in_null_namespace(node, "kind")?
            .map(|value| value.to_ascii_lowercase());
        let media = dom.parent(node)?.filter(|parent| {
            dom.is_element(*parent, "http://www.w3.org/1999/xhtml", "video")
                || dom.is_element(*parent, "http://www.w3.org/1999/xhtml", "audio")
        });
        (
            match kind {
                Some(kind)
                    if matches!(
                        kind.as_str(),
                        "subtitles" | "captions" | "descriptions" | "chapters" | "metadata"
                    ) =>
                {
                    kind
                }
                Some(_) => "metadata".into(),
                None => "subtitles".into(),
            },
            attr("label"),
            attr("srclang"),
            media,
            dom.attribute_in_null_namespace(node, "default")?.is_some(),
        )
    };
    let id = new_track(
        state,
        Some(node),
        media,
        kind,
        label,
        language,
        if default { "showing" } else { "disabled" },
    )?;
    let mut all = state.text_tracks.borrow_mut();
    all.nodes.insert(node, id);
    all.dirty = true;
    drop(all);
    notify_list(state, media, "addtrack", id);
    Ok(id)
}
fn list(state: &V8BindingState, kind: List) -> u64 {
    register_platform_object(state, PlatformObjectRecord::TextTrackList(kind))
}
fn ids(state: &V8BindingState, id: u64) -> Result<Vec<u64>, V8BindingError> {
    if matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::MediaTrackList { .. }
    ) {
        return av_tracks::ids(state, id);
    }
    let kind = match &*platform_object(state, id)? {
        PlatformObjectRecord::TextTrackList(kind) => *kind,
        _ => return Err(fail("TypeError")),
    };
    match kind {
        List::Tracks(media) => {
            let children = state.dom.borrow().children(media)?.to_vec();
            for child in &children {
                if state
                    .dom
                    .borrow()
                    .is_element(*child, "http://www.w3.org/1999/xhtml", "track")
                {
                    ensure(state, *child)?;
                }
            }
            // Element tracks follow current child order, even before the next loading task.
            let dom = state.dom.borrow();
            let all = state.text_tracks.borrow();
            let mut result = Vec::new();
            for node in dom.children(media)? {
                if let Some(id) = all.nodes.get(node) {
                    result.push(*id)
                }
            }
            result.extend(
                all.tracks
                    .iter()
                    .filter(|(_, track)| track.node.is_none() && track.media == Some(media))
                    .map(|(id, _)| *id),
            );
            Ok(result)
        }
        List::Cues(track_id, active) => {
            let t = track(state, track_id)?;
            let mut result = if t.mode == "disabled" {
                Vec::new()
            } else if active {
                t.active.clone()
            } else {
                t.cues.clone()
            };
            drop(t);
            // Stable ties retain cue insertion order. Mutating cue times immediately reorders a live list.
            result.sort_by(|a, b| {
                let a = cue(state, *a).expect("retained cue");
                let b = cue(state, *b).expect("retained cue");
                let a = a.borrow();
                let b = b.borrow();
                a.data
                    .start
                    .total_cmp(&b.data.start)
                    .then_with(|| b.data.end.total_cmp(&a.data.end))
            });
            Ok(result)
        }
    }
}
pub(super) fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let canonical = state.text_tracks.borrow().canonical.get(&id).cloned();
    if let Some((owner, original)) = canonical {
        if let Some(owner) = owner.upgrade() {
            let interface = platform_interface_for_id(&owner, original)
                .ok_or_else(|| fail("InvalidStateError"))?;
            return Ok(new_platform_object(scope, &owner, interface, original)?.into());
        }
    }
    if let Ok(value) = cue(state, id) {
        let canonical = value.borrow().canonical.clone();
        if let Some((owner, original)) = canonical {
            if let Some(owner) = owner.upgrade() {
                return Ok(new_platform_object(scope, &owner, "VTTCue", original)?.into());
            }
        }
    }
    let interface = platform_interface_for_id(state, id).ok_or_else(|| fail("TypeError"))?;
    Ok(new_platform_object(scope, state, interface, id)?.into())
}
pub(super) fn node_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    state.text_tracks.borrow_mut().self_owner = Rc::downgrade(state);
    if member == "textTracks" {
        discover(scope, state)?;
        let existing = state.text_tracks.borrow().lists.get(&(node, 0)).copied();
        let id = if let Some(id) = existing {
            id
        } else {
            let id = list(state, List::Tracks(node));
            state.text_tracks.borrow_mut().lists.insert((node, 0), id);
            id
        };
        let value = object(scope, state, id)?;
        refresh_gc_edges(scope, state)?;
        return Ok(value);
    }
    let id = ensure(state, node)?;
    if member == "readyState" {
        return Ok(v8::Integer::new(scope, track(state, id)?.ready as i32).into());
    }
    let value = object(scope, state, id)?;
    refresh_gc_edges(scope, state)?;
    Ok(value)
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object_value: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let state = owner(scope, object_value)?;
    let id = stateful_id(scope, object_value)?;
    let (state, id) = resolve(state, id)?;
    if matches!(
        &*platform_object(&state, id)?,
        PlatformObjectRecord::TextTrack
    ) {
        if matches!(member, "cues" | "activeCues") {
            let active = member == "activeCues";
            let t = track(&state, id)?;
            if t.mode == "disabled" {
                return Ok(v8::null(scope).into());
            }
            let existing = if active { t.active_list } else { t.cues_list };
            drop(t);
            let list_id = existing.unwrap_or_else(|| {
                let value = list(&state, List::Cues(id, active));
                let mut all = state.text_tracks.borrow_mut();
                let t = all.tracks.get_mut(&id).unwrap();
                if active {
                    t.active_list = Some(value)
                } else {
                    t.cues_list = Some(value)
                }
                value
            });
            return object(scope, &state, list_id);
        }
        let t = track(&state, id)?;
        let attribute = |name: &str| {
            t.node.and_then(|node| {
                state
                    .dom
                    .borrow()
                    .attribute_in_null_namespace(node, name)
                    .ok()
                    .flatten()
                    .map(str::to_owned)
            })
        };
        let value = match member {
            "kind" => attribute("kind")
                .map(|kind| {
                    let kind = kind.to_ascii_lowercase();
                    if matches!(
                        kind.as_str(),
                        "subtitles" | "captions" | "descriptions" | "chapters" | "metadata"
                    ) {
                        kind
                    } else {
                        "metadata".into()
                    }
                })
                .unwrap_or_else(|| {
                    if t.node.is_some() {
                        "subtitles".into()
                    } else {
                        t.kind.clone()
                    }
                }),
            "label" => {
                if t.node.is_some() {
                    attribute("label").unwrap_or_default()
                } else {
                    t.label.clone()
                }
            }
            "language" => {
                if t.node.is_some() {
                    attribute("srclang").unwrap_or_default()
                } else {
                    t.language.clone()
                }
            }
            "mode" => t.mode.clone(),
            "inBandMetadataTrackDispatchType" => String::new(),
            "id" => t
                .node
                .and_then(|node| {
                    state
                        .dom
                        .borrow()
                        .attribute_in_null_namespace(node, "id")
                        .ok()
                        .flatten()
                        .map(str::to_owned)
                })
                .unwrap_or_default(),
            _ => return Err(fail("TypeError")),
        };
        return v8_string(scope, &value);
    }
    if matches!(
        &*platform_object(&state, id)?,
        PlatformObjectRecord::TextTrackList(_)
    ) {
        return Ok(v8::Integer::new_from_unsigned(scope, ids(&state, id)?.len() as u32).into());
    }
    if let PlatformObjectRecord::VttRegion(region) = &*platform_object(&state, id)? {
        let region = region.borrow();
        return if member == "id" {
            v8_string(scope, &region.id)
        } else if member == "scroll" {
            v8_string(scope, &region.scroll)
        } else {
            Ok(v8::Number::new(
                scope,
                *region.values.get(member).ok_or_else(|| fail("TypeError"))?,
            )
            .into())
        };
    }
    let cue = cue(&state, id)?;
    let cue = cue.borrow();
    match member {
        "track" => {
            let track_owner = cue
                .track_owner
                .as_ref()
                .and_then(std::rc::Weak::upgrade)
                .unwrap_or_else(|| state.clone());
            cue.track
                .map(|id| object(scope, &track_owner, id))
                .unwrap_or_else(|| Ok(v8::null(scope).into()))
        }
        "region" => {
            if let Some((owner, id)) = &cue.region {
                if let Some(owner) = owner.upgrade() {
                    object(scope, &owner, *id)
                } else {
                    Ok(v8::null(scope).into())
                }
            } else {
                Ok(v8::null(scope).into())
            }
        }
        "id" => v8_string(scope, &cue.data.id),
        "text" => v8_string(scope, &cue.data.text),
        "startTime" => Ok(v8::Number::new(scope, cue.data.start).into()),
        "endTime" => Ok(v8::Number::new(scope, cue.data.end).into()),
        "pauseOnExit" => Ok(v8::Boolean::new(scope, cue.pause).into()),
        _ => match cue.options.get(member) {
            Some(serde_json::Value::String(value)) => v8_string(scope, value),
            Some(serde_json::Value::Bool(value)) => Ok(v8::Boolean::new(scope, *value).into()),
            Some(value) if value.is_number() => {
                Ok(v8::Number::new(scope, value.as_f64().unwrap()).into())
            }
            _ => Err(fail("TypeError")),
        },
    }
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let state = owner(scope, object)?;
    let id = stateful_id(scope, object)?;
    let (state, id) = resolve(state, id)?;
    if matches!(
        &*platform_object(&state, id)?,
        PlatformObjectRecord::TextTrack
    ) {
        let mode = value
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        if !matches!(mode.as_str(), "disabled" | "hidden" | "showing") {
            return Ok(());
        }
        let mut all = state.text_tracks.borrow_mut();
        let t = all
            .tracks
            .get_mut(&id)
            .ok_or_else(|| fail("InvalidStateError"))?;
        if t.mode != mode {
            t.mode = mode;
            let media = t.media;
            all.dirty = true;
            drop(all);
            notify_list(&state, media, "change", id);
        }
        return Ok(());
    }
    if let PlatformObjectRecord::VttRegion(region) = &*platform_object(&state, id)? {
        let region = region.clone();
        if member == "id" {
            let text = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            if text.len() > 4096 {
                return Err(fail("QuotaExceededError"));
            }
            region.borrow_mut().id = text;
        } else if member == "lines" {
            let number = value
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            region
                .borrow_mut()
                .values
                .insert(member.into(), f64::from(number));
        } else if member == "scroll" {
            let text = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            if !matches!(text.as_str(), "" | "up") {
                return Ok(());
            }
            region.borrow_mut().scroll = text;
        } else {
            let number = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            if !number.is_finite() {
                return Err(fail("TypeError"));
            }
            if number < 0. || member != "lines" && number > 100. {
                return Err(fail("IndexSizeError"));
            }
            region.borrow_mut().values.insert(member.into(), number);
        }
        return Ok(());
    }
    let cue = cue(&state, id)?;
    match member {
        "id" | "text" => {
            let text = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            if text.len() > oryn_dom::webvtt::MAX_BYTES {
                return Err(fail("QuotaExceededError"));
            }
            let previous = {
                let cue = cue.borrow();
                if member == "id" {
                    cue.data.id.len()
                } else {
                    cue.data.text.len()
                }
            };
            cue_capacity(&state, 0, text.len().saturating_sub(previous))?;
            if member == "id" {
                cue.borrow_mut().data.id = text
            } else {
                cue.borrow_mut().data.text = text
            }
        }
        "startTime" | "endTime" => {
            let number = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            if member == "startTime" && !number.is_finite()
                || number.is_nan()
                || number == f64::NEG_INFINITY
            {
                return Err(fail("TypeError"));
            }
            if member == "startTime" {
                cue.borrow_mut().data.start = number
            } else {
                cue.borrow_mut().data.end = number
            }
        }
        "pauseOnExit" => cue.borrow_mut().pause = value.boolean_value(scope),
        "snapToLines" => {
            cue.borrow_mut()
                .options
                .insert(member.into(), serde_json::json!(value.boolean_value(scope)));
        }
        "region" => {
            let region = if value.is_null() {
                None
            } else {
                let object =
                    v8::Local::<v8::Object>::try_from(value).map_err(|_| fail("TypeError"))?;
                let region_owner = owner(scope, object)?;
                let region = stateful_id(scope, object)?;
                if !matches!(
                    &*platform_object(&region_owner, region)?,
                    PlatformObjectRecord::VttRegion(_)
                ) {
                    return Err(fail("TypeError"));
                }
                Some((Rc::downgrade(&region_owner), region))
            };
            if let Some((owner, id)) = &region {
                if let Some(owner) = owner.upgrade() {
                    let mut all = owner.text_tracks.borrow_mut();
                    *all.region_refs.entry(*id).or_default() += 1;
                }
            }
            let old = std::mem::replace(&mut cue.borrow_mut().region, region);
            if let Some((owner, id)) = old {
                if let Some(owner) = owner.upgrade() {
                    let mut all = owner.text_tracks.borrow_mut();
                    if let Some(count) = all.region_refs.get_mut(&id) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            all.region_refs.remove(&id);
                        }
                    }
                }
            }
        }
        "line" | "position" | "size" => {
            if value.is_string() && value.to_rust_string_lossy(scope) == "auto" && member != "size"
            {
                cue.borrow_mut()
                    .options
                    .insert(member.into(), serde_json::json!("auto"));
            } else {
                let number = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !number.is_finite() {
                    return Err(fail("TypeError"));
                }
                if member != "line" && !(0.0..=100.0).contains(&number) {
                    return Err(fail("IndexSizeError"));
                }
                cue.borrow_mut()
                    .options
                    .insert(member.into(), serde_json::json!(number));
            }
        }
        _ => {
            let text = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            let valid = match member {
                "vertical" => matches!(text.as_str(), "" | "rl" | "lr"),
                "align" => matches!(text.as_str(), "start" | "center" | "end" | "left" | "right"),
                "lineAlign" => matches!(text.as_str(), "start" | "center" | "end"),
                "positionAlign" => matches!(
                    text.as_str(),
                    "auto" | "line-left" | "center" | "line-right"
                ),
                _ => false,
            };
            if !valid {
                return Ok(());
            }
            cue.borrow_mut()
                .options
                .insert(member.into(), serde_json::json!(text));
        }
    }
    if let Some(owner) = cue
        .borrow()
        .track_owner
        .as_ref()
        .and_then(std::rc::Weak::upgrade)
    {
        owner.text_tracks.borrow_mut().dirty = true;
    }
    state.text_tracks.borrow_mut().dirty = true;
    refresh_gc_edges(scope, &state)?;
    platform_listener_graph::reconcile_activity(scope, &state);
    Ok(())
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    name: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    bind_owner(state);
    platform_listener_graph::reclaim(state);
    let record = if name == "VTTRegion" {
        if state
            .platform_objects
            .borrow()
            .values()
            .filter(|record| matches!(record, PlatformObjectRecord::VttRegion(_)))
            .count()
            >= 4096
        {
            return Err(fail("QuotaExceededError"));
        }
        PlatformObjectRecord::VttRegion(Rc::new(RefCell::new(Region {
            id: String::new(),
            values: [
                ("width", 100.),
                ("lines", 3.),
                ("regionAnchorX", 0.),
                ("regionAnchorY", 100.),
                ("viewportAnchorX", 0.),
                ("viewportAnchorY", 100.),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
            scroll: String::new(),
        })))
    } else if name == "VTTCue" {
        if args.length() < 3 {
            return Err(fail("TypeError"));
        }
        let start = args
            .get(0)
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let end = args
            .get(1)
            .number_value(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let text = args
            .get(2)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        if !start.is_finite() || end.is_nan() || end == f64::NEG_INFINITY {
            return Err(fail("TypeError"));
        }
        if text.len() > oryn_dom::webvtt::MAX_BYTES {
            return Err(fail("QuotaExceededError"));
        }
        {
            cue_capacity(state, 1, text.len())?;
            PlatformObjectRecord::TextTrackCue(Rc::new(RefCell::new(cue_record(
                oryn_dom::webvtt::Cue {
                    id: String::new(),
                    start,
                    end,
                    text,
                    settings: BTreeMap::new(),
                },
            ))))
        }
    } else {
        return Err(fail("TypeError"));
    };
    let id = register_platform_object(state, record);
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn discover(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<(), V8BindingError> {
    let nodes = {
        let dom = state.dom.borrow();
        dom.arena()
            .iter()
            .filter_map(|(id, _)| {
                (dom.is_element(id, "http://www.w3.org/1999/xhtml", "track")
                    && dom.node_document(id).ok().flatten() == Some(state.document_root()))
                .then_some(id)
            })
            .collect::<Vec<_>>()
    };
    for node in nodes {
        let admitted = {
            let dom = state.dom.borrow();
            let live = |scope: &mut v8::PinScope, node| {
                state
                    .wrappers
                    .borrow()
                    .lookup(dom.arena().id(), state.realm, node)
                    .and_then(|id| {
                        state
                            .objects
                            .borrow()
                            .get(&id)
                            .and_then(|object| object.handle.to_local(scope))
                    })
                    .is_some()
            };
            dom.is_connected(node).unwrap_or(false)
                || live(scope, node)
                || dom
                    .parent(node)
                    .ok()
                    .flatten()
                    .is_some_and(|parent| live(scope, parent))
        };
        if admitted {
            ensure(state, node)?;
        }
    }
    Ok(())
}
fn selection(state: &V8BindingState, node: NodeId) -> Result<Option<Key>, V8BindingError> {
    let dom = state.dom.borrow();
    if dom
        .node_document(node)?
        .is_none_or(|document| document != state.document_root())
    {
        return Ok(None);
    }
    let Some(media) = dom.parent(node)?.filter(|parent| {
        dom.is_element(*parent, "http://www.w3.org/1999/xhtml", "audio")
            || dom.is_element(*parent, "http://www.w3.org/1999/xhtml", "video")
    }) else {
        return Ok(None);
    };
    let Some(source) = dom
        .attribute_in_null_namespace(node, "src")?
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
    else {
        return Ok(None);
    };
    let cors = dom
        .attribute_in_null_namespace(media, "crossorigin")?
        .map(str::to_owned);
    drop(dom);
    Ok(Some(Key {
        media,
        url: state.native_document_resource_url(&source)?.to_string(),
        cors,
    }))
}
fn cancel(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    let operations = state
        .pending_platform_operations
        .borrow()
        .iter()
        .filter_map(|(operation, pending)| {
            matches!(pending,PendingPlatformOperation::TextTrack(p)if p.track==id)
                .then_some(*operation)
        })
        .collect::<Vec<_>>();
    for operation in operations {
        let pending = state
            .pending_platform_operations
            .borrow_mut()
            .remove(&operation);
        state
            .cancelled_platform_operations
            .borrow_mut()
            .insert(operation);
        if let Some(host) = state.platform_host.borrow().clone() {
            host.cancel(operation);
        }
        if let Some(PendingPlatformOperation::TextTrack(p)) = pending {
            if let Some(stream_id) = p.stream {
                submit_platform_operation(
                    state,
                    PlatformHostRequest::HttpAbort { stream_id },
                    PendingPlatformOperation::WebSocketCommand,
                )?;
            }
        }
    }
    Ok(())
}
fn current(state: &V8BindingState, p: &Pending) -> bool {
    !state.execution_retired()
        && !state.document_load_stopped()
        && state.document_generation() == p.document_generation
        && state.document_stream_epoch() == p.epoch
        && state
            .text_tracks
            .borrow()
            .tracks
            .get(&p.track)
            .is_some_and(|track| {
                track.generation == p.generation
                    && track.mode != "disabled"
                    && track
                        .node
                        .is_some_and(|node| selection(state, node).ok().flatten() == track.key)
            })
}
fn terminal(state: &V8BindingState, id: u64, success: bool) {
    let node = {
        let mut all = state.text_tracks.borrow_mut();
        let Some(track) = all.tracks.get_mut(&id) else {
            return;
        };
        track.ready = if success { 2 } else { 3 };
        track.node
    };
    if let Some(node) = node {
        state
            .text_tracks
            .borrow_mut()
            .events
            .push_back((Target::Node(node), if success { "load" } else { "error" }));
    }
    state.text_tracks.borrow_mut().dirty = true;
}
fn abort(state: &V8BindingState, p: &mut Pending) -> Result<(), V8BindingError> {
    if let Some(stream_id) = p.stream.take() {
        submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        )?;
    }
    Ok(())
}
pub(super) fn complete(
    state: &V8BindingState,
    mut pending: Pending,
    result: Result<PlatformHostResult, V8BindingError>,
) -> Result<(), V8BindingError> {
    if !current(state, &pending) {
        if let Ok(PlatformHostResult::HttpStarted { stream_id, .. })
        | Ok(PlatformHostResult::HttpChunk {
            stream_id,
            done: false,
            ..
        }) = result
        {
            pending.stream = Some(stream_id)
        }
        abort(state, &mut pending)?;
        return Ok(());
    }
    match result {
        Ok(PlatformHostResult::HttpStarted {
            stream_id, status, ..
        }) => {
            pending.stream = Some(stream_id);
            if !(200..300).contains(&status) {
                abort(state, &mut pending)?;
                terminal(state, pending.track, false);
                return Ok(());
            }
        }
        Ok(PlatformHostResult::HttpChunk {
            encoded_body_size: _,
            stream_id,
            bytes,
            done,
        }) => {
            if pending.stream != Some(stream_id)
                || bytes.len() > oryn_dom::webvtt::MAX_BYTES.saturating_sub(pending.bytes.len())
            {
                abort(state, &mut pending)?;
                terminal(state, pending.track, false);
                return Ok(());
            }
            let retained = state
                .pending_platform_operations
                .borrow()
                .values()
                .filter_map(|operation| {
                    if let PendingPlatformOperation::TextTrack(p) = operation {
                        Some(p.bytes.len())
                    } else {
                        None
                    }
                })
                .sum::<usize>();
            if retained
                .saturating_add(pending.bytes.len())
                .saturating_add(bytes.len())
                > 32 * 1024 * 1024
            {
                abort(state, &mut pending)?;
                terminal(state, pending.track, false);
                return Ok(());
            }
            pending.bytes.extend_from_slice(&bytes);
            if done {
                pending.stream = None;
                match oryn_dom::webvtt::parse(&pending.bytes) {
                    Ok(document) => {
                        let bytes = document.cues.iter().fold(0usize, |sum, cue| {
                            sum.saturating_add(cue.id.len())
                                .saturating_add(cue.text.len())
                        });
                        if cue_capacity(state, document.cues.len(), bytes).is_err() {
                            terminal(state, pending.track, false);
                            return Ok(());
                        }
                        let ids = document
                            .cues
                            .into_iter()
                            .map(|data| {
                                let mut record = cue_record(data);
                                record.track = Some(pending.track);
                                register_platform_object(
                                    state,
                                    PlatformObjectRecord::TextTrackCue(Rc::new(RefCell::new(
                                        record,
                                    ))),
                                )
                            })
                            .collect::<Vec<_>>();
                        if let Some(track) = state
                            .text_tracks
                            .borrow_mut()
                            .tracks
                            .get_mut(&pending.track)
                        {
                            track.cues = ids;
                        }
                        terminal(state, pending.track, true);
                    }
                    Err(_) => terminal(state, pending.track, false),
                }
                return Ok(());
            }
        }
        _ => {
            abort(state, &mut pending)?;
            terminal(state, pending.track, false);
            return Ok(());
        }
    }
    let stream_id = pending.stream.ok_or_else(|| fail("InvalidStateError"))?;
    let id = pending.track;
    if let Err(error) = submit_platform_operation(
        state,
        PlatformHostRequest::HttpRead {
            stream_id,
            max_bytes: 64 * 1024,
        },
        PendingPlatformOperation::TextTrack(pending),
    ) {
        let _ = submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        );
        terminal(state, id, false);
        return Err(error);
    }
    Ok(())
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    let all = state.text_tracks.borrow();
    !state.execution_retired()
        && !state.document_load_stopped()
        && (all.dirty
            || all.delivery.is_some()
            || !all.events.is_empty()
            || all.cursor != Some(state.dom.borrow().mutation_cursor().0))
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    state.text_tracks.borrow_mut().self_owner = Rc::downgrade(state);
    if state.execution_retired()
        || state.execution_suspended()
        || state.text_tracks.borrow().polling
    {
        return Ok(0);
    }
    state.text_tracks.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.text_tracks.borrow_mut().polling = false;
        }
    }
    let _guard = Guard(state);
    // Drain a retained notification batch before sampling another one. Producer
    // counts are bounded by the track/cue quotas; authored callbacks cannot grow
    // an unbounded succession of undelivered timeline batches.
    prune_detached(scope, state)?;
    av_tracks::prune(scope, state);
    let admit_batch = state.text_tracks.borrow().events.len() < 128
        && state.text_tracks.borrow().delivery.is_none();
    if !state.document_load_stopped() && admit_batch {
        discover(scope, state)?;
        let ids = state
            .text_tracks
            .borrow()
            .tracks
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            let (node, mode, old, ready) = {
                let t = track(state, id)?;
                (t.node, t.mode.clone(), t.key.clone(), t.ready)
            };
            let Some(node) = node else { continue };
            let media = {
                let dom = state.dom.borrow();
                dom.parent(node)?.filter(|parent| {
                    dom.is_element(*parent, "http://www.w3.org/1999/xhtml", "video")
                        || dom.is_element(*parent, "http://www.w3.org/1999/xhtml", "audio")
                })
            };
            let old_media = track(state, id)?.media;
            if old_media != media {
                let root = media
                    .map(|node| submissions::RootedTarget::new(state, node))
                    .transpose()?;
                {
                    let mut all = state.text_tracks.borrow_mut();
                    let t = all.tracks.get_mut(&id).unwrap();
                    t.media = media;
                    t._media = root;
                    t.last_sample = None;
                }
                notify_list(state, old_media, "removetrack", id);
                notify_list(state, media, "addtrack", id);
            }
            let selected = match selection(state, node) {
                Ok(value) => value,
                Err(_) => {
                    if track(state, id)?.ready != 3 {
                        cancel(state, id)?;
                        terminal(state, id, false);
                    }
                    continue;
                }
            };
            let changed = old != selected;
            if changed || mode == "disabled" && ready == 1 {
                cancel(state, id)?;
                let mut all = state.text_tracks.borrow_mut();
                let t = all.tracks.get_mut(&id).unwrap();
                t.generation = t
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| fail("QuotaExceededError"))?;
                t.ready = 0;
                t.key = selected.clone();
                if changed {
                    for cue_id in &t.cues {
                        if let Ok(cue) = cue(state, *cue_id) {
                            {
                                let mut cue = cue.borrow_mut();
                                cue.track = None;
                                cue.track_owner = None;
                            }
                        }
                    }
                    t.cues.clear();
                    t.active.clear();
                    t.last_sample = None;
                }
            }
            if mode == "disabled" {
                continue;
            }
            let Some(key) = selected else { continue };
            if track(state, id)?.ready != 0 {
                continue;
            }
            let (generation, epoch, document_generation) = {
                let mut all = state.text_tracks.borrow_mut();
                let t = all.tracks.get_mut(&id).unwrap();
                t.ready = 1;
                (
                    t.generation,
                    state.document_stream_epoch(),
                    state.document_generation(),
                )
            };
            let request = PlatformHttpRequest {
                upgrade_mixed_image: false,
                fetch: Some(crate::platform::FetchOptions {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    mode: if key.cors.is_some() {
                        "cors"
                    } else {
                        "same-origin"
                    }
                    .into(),
                    redirect: "follow".into(),
                    integrity: String::new(),
                }),
                use_cors_preflight: false,
                method: "GET".into(),
                url: key.url.clone(),
                headers: vec![("accept".into(), "text/vtt,*/*;q=0.1".into())],
                body: Vec::new(),
                credentials: if key
                    .cors
                    .as_ref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"))
                {
                    "include"
                } else {
                    "same-origin"
                }
                .into(),
                initiator_url: Some(state.request_initiator_url()?),
                referrer_policy: Some(state.document_referrer_policy()),
            };
            if let Err(error) = submit_platform_operation(
                state,
                PlatformHostRequest::MediaStart {
                    request,
                    cors_enabled: true,
                    destination: "track".into(),
                },
                PendingPlatformOperation::TextTrack(Pending {
                    track: id,
                    generation,
                    document_generation,
                    epoch,
                    stream: None,
                    bytes: Vec::new(),
                }),
            ) {
                terminal(state, id, false);
                return Err(error);
            }
        }
        state.text_tracks.borrow_mut().cursor = Some(state.dom.borrow().mutation_cursor().0);
        state.text_tracks.borrow_mut().dirty = false;
    }
    if admit_batch {
        update_active(scope, state)?;
    }
    let mut count = 0;
    for _ in 0..128 {
        if state.execution_suspended() || state.execution_retired() {
            break;
        }
        let retained = state.text_tracks.borrow_mut().delivery.take();
        let mut notification = if let Some(retained) = retained {
            retained
        } else {
            let Some((target, name)) = state.text_tracks.borrow().events.front().cloned() else {
                break;
            };
            state.text_tracks.borrow_mut().delivery_targets = match &target {
                Target::Platform(id) | Target::TrackEvent(id, ..) => BTreeSet::from([*id]),
                Target::Node(_) => BTreeSet::new(),
            };
            let event = create_event_object_with_interface(
                scope,
                state,
                if matches!(&target, Target::TrackEvent(..)) {
                    "TrackEvent"
                } else {
                    "Event"
                },
                name,
                false,
                false,
            )?;
            if let Target::TrackEvent(_, ref owner, track) = target {
                let owner = owner.upgrade().ok_or_else(|| fail("InvalidStateError"))?;
                let value = object(scope, &owner, track)?;
                set_event_track(scope, event, value)?;
            }
            let notification = match target {
                Target::Node(node) => {
                    let target = wrap_node(scope, state, node)?;
                    native_platform::publication::pending_dom_dispatch(
                        scope, state, target, name, event,
                    )
                }
                Target::Platform(id) | Target::TrackEvent(id, _, _) => {
                    native_platform::publication::pending_platform_dispatch(
                        scope, state, id, name, event,
                    )
                }
            };
            state.text_tracks.borrow_mut().events.pop_front();
            notification
        };
        match notification.resume(scope, state) {
            Ok(true) => {
                notification.finish(scope, state);
                state.text_tracks.borrow_mut().delivery_targets.clear();
                count += 1;
            }
            Ok(false) => {
                state.text_tracks.borrow_mut().delivery = Some(notification);
                break;
            }
            Err(error) => {
                state.text_tracks.borrow_mut().delivery = Some(notification);
                return Err(error);
            }
        }
    }
    refresh_gc_edges(scope, state)?;
    platform_listener_graph::reconcile_activity(scope, state);
    Ok(count)
}
fn cue_transitions(
    cues: &[(u64, f64, f64)],
    previous: &[u64],
    time: f64,
    normal_since: Option<f64>,
    enabled: bool,
) -> (Vec<u64>, Vec<(u64, &'static str)>) {
    let mut ordered = cues.to_vec();
    ordered.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.2.total_cmp(&a.2)));
    let active = ordered
        .iter()
        .filter(|(_, start, end)| enabled && *start <= time && time < *end)
        .map(|(id, _, _)| *id)
        .collect::<Vec<_>>();
    let mut events = Vec::new();
    for (order, (id, start, end)) in ordered.iter().enumerate() {
        let was = previous.contains(id);
        let now = active.contains(id);
        let skipped = enabled && normal_since.is_some_and(|last| last < *start && *end <= time);
        if (!was && now) || skipped {
            events.push((*start, order, 0, *id, "enter"));
        }
        if (was && !now) || skipped {
            events.push((*end, order, 1, *id, "exit"));
        }
    }
    events.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    (
        active,
        events
            .into_iter()
            .map(|(_, _, _, id, name)| (id, name))
            .collect(),
    )
}
fn update_active(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let tracks = state
        .text_tracks
        .borrow()
        .tracks
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in tracks {
        let (media, mode, cues, previous, last) = {
            let t = track(state, id)?;
            (
                t.media,
                t.mode.clone(),
                t.cues.clone(),
                t.active.clone(),
                t.last_sample,
            )
        };
        let Some(media) = media else { continue };
        if state.dom.borrow().node_document(media)? != Some(state.document_root()) {
            let mut all = state.text_tracks.borrow_mut();
            let t = all.tracks.get_mut(&id).unwrap();
            t.active.clear();
            t.last_sample = None;
            continue;
        }
        let sample = media_element::text_track_clock(state, media)?;
        let normal_since = last
            .filter(|last| last.0 == sample.0 && last.1 == sample.1 && last.2 <= sample.2)
            .map(|last| last.2);
        let values = cues
            .iter()
            .map(|id| {
                let cue = cue(state, *id)?;
                let cue = cue.borrow();
                Ok((*id, cue.data.start, cue.data.end))
            })
            .collect::<Result<Vec<_>, V8BindingError>>()?;
        let (active, events) = cue_transitions(
            &values,
            &previous,
            sample.2,
            normal_since,
            mode != "disabled",
        );
        {
            let mut all = state.text_tracks.borrow_mut();
            let t = all.tracks.get_mut(&id).unwrap();
            t.last_sample = Some(sample);
            t.active = active.clone();
        }
        if active == previous && events.is_empty() {
            continue;
        }
        for (cue_id, name) in events {
            let value = cue(state, cue_id)?;
            if name == "exit" && normal_since.is_some() && value.borrow().pause {
                media_element::call(scope, state, media, "pause")?;
            }
            let canonical = value.borrow().canonical.clone();
            if let Some((owner, original)) = canonical {
                if let Some(owner) = owner.upgrade() {
                    owner
                        .text_tracks
                        .borrow_mut()
                        .events
                        .push_back((Target::Platform(original), name));
                    continue;
                }
            }
            state
                .text_tracks
                .borrow_mut()
                .events
                .push_back((Target::Platform(cue_id), name));
        }
        let node = state.text_tracks.borrow().tracks[&id].node;
        queue_platform(state, id, "cuechange", None);
        if let Some(node) = node {
            state
                .text_tracks
                .borrow_mut()
                .events
                .push_back((Target::Node(node), "cuechange"));
        }
    }
    Ok(())
}
/// Cancel under the original request authority before DOM ownership changes.
pub(super) fn cancel_nodes(
    state: &Rc<V8BindingState>,
    nodes: &BTreeSet<NodeId>,
) -> Result<(), V8BindingError> {
    let ids = state
        .text_tracks
        .borrow()
        .tracks
        .iter()
        .filter(|(_, track)| {
            track.node.is_some_and(|node| nodes.contains(&node))
                || track.media.is_some_and(|node| nodes.contains(&node))
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in ids {
        cancel(state, id)?;
        let mut all = state.text_tracks.borrow_mut();
        let track = all.tracks.get_mut(&id).unwrap();
        if track.ready == 1 {
            track.ready = 0;
        }
        track.key = None;
        track.last_sample = None;
    }
    state
        .text_tracks
        .borrow_mut()
        .events
        .retain(|(target, _)| !matches!(target,Target::Node(node)if nodes.contains(node)));
    Ok(())
}
/// Platform wrappers remain in their original realm. Native forwarding records
/// move behavioral ownership without changing the wrapper or its listeners.
pub(super) fn migrate(
    source: &Rc<V8BindingState>,
    destination: &Rc<V8BindingState>,
    nodes: &BTreeMap<NodeId, NodeId>,
) -> Result<(), V8BindingError> {
    destination.text_tracks.borrow_mut().self_owner = Rc::downgrade(destination);
    if Rc::ptr_eq(source, destination) {
        return Ok(());
    }
    let selected = source
        .text_tracks
        .borrow()
        .tracks
        .iter()
        .filter(|(_, track)| {
            track.node.is_some_and(|node| nodes.contains_key(&node))
                || track.node.is_none() && track.media.is_some_and(|node| nodes.contains_key(&node))
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    if destination
        .text_tracks
        .borrow()
        .tracks
        .len()
        .saturating_add(selected.len())
        > 4096
    {
        return Err(fail("QuotaExceededError"));
    }
    for old in selected {
        let (node, media) = {
            let all = source.text_tracks.borrow();
            let track = &all.tracks[&old];
            (
                track.node.and_then(|node| nodes.get(&node).copied()),
                track.media.and_then(|node| nodes.get(&node).copied()),
            )
        };
        let node_root = node
            .map(|node| submissions::RootedTarget::new(destination, node))
            .transpose()?;
        let media_root = media
            .map(|node| submissions::RootedTarget::new(destination, node))
            .transpose()?;
        let mut track = source.text_tracks.borrow_mut().tracks.remove(&old).unwrap();
        if let Some(node) = track.node {
            source.text_tracks.borrow_mut().nodes.remove(&node);
        }
        let id = register_platform_object(destination, PlatformObjectRecord::TextTrack);
        let canonical = source
            .text_tracks
            .borrow()
            .canonical
            .get(&old)
            .cloned()
            .unwrap_or_else(|| (Rc::downgrade(source), old));
        if let Some(owner) = canonical.0.upgrade() {
            owner
                .text_tracks
                .borrow_mut()
                .forward
                .insert(canonical.1, (Rc::downgrade(destination), id));
        }
        source
            .text_tracks
            .borrow_mut()
            .forward
            .insert(old, (Rc::downgrade(destination), id));
        destination
            .text_tracks
            .borrow_mut()
            .canonical
            .insert(id, canonical);
        let mut remapped = Vec::new();
        for old_cue in &track.cues {
            let value = cue(source, *old_cue)?;
            let local = register_platform_object(
                destination,
                PlatformObjectRecord::TextTrackCue(value.clone()),
            );
            {
                let mut cue = value.borrow_mut();
                if cue.canonical.is_none() {
                    cue.canonical = Some((Rc::downgrade(source), *old_cue));
                }
                cue.track = Some(id);
                cue.track_owner = Some(Rc::downgrade(destination));
            }
            remapped.push(local);
        }
        for (active, slot) in [
            (false, &mut track.cues_list),
            (true, &mut track.active_list),
        ] {
            if let Some(old_list) = slot.take() {
                let local = list(destination, List::Cues(id, active));
                let canonical = source
                    .text_tracks
                    .borrow()
                    .canonical
                    .get(&old_list)
                    .cloned()
                    .unwrap_or_else(|| (Rc::downgrade(source), old_list));
                if let Some(owner) = canonical.0.upgrade() {
                    owner
                        .text_tracks
                        .borrow_mut()
                        .forward
                        .insert(canonical.1, (Rc::downgrade(destination), local));
                }
                source
                    .text_tracks
                    .borrow_mut()
                    .forward
                    .insert(old_list, (Rc::downgrade(destination), local));
                destination
                    .text_tracks
                    .borrow_mut()
                    .canonical
                    .insert(local, canonical);
                *slot = Some(local);
            }
        }
        track.node = node;
        track.media = media;
        track._node = node_root;
        track._media = media_root;
        track.cues = remapped;
        track.active.clear();
        track.last_sample = None;
        track.key = None;
        let mut all = destination.text_tracks.borrow_mut();
        if let Some(node) = node {
            all.nodes.insert(node, id);
        }
        all.tracks.insert(id, track);
        all.dirty = true;
    }
    let lists = source
        .text_tracks
        .borrow()
        .lists
        .iter()
        .filter_map(|((node, kind), id)| nodes.get(node).map(|new| ((*node, *kind), *new, *id)))
        .collect::<Vec<_>>();
    for (key, node, old) in lists {
        source.text_tracks.borrow_mut().lists.remove(&key);
        let id = list(destination, List::Tracks(node));
        let canonical = source
            .text_tracks
            .borrow()
            .canonical
            .get(&old)
            .cloned()
            .unwrap_or_else(|| (Rc::downgrade(source), old));
        if let Some(owner) = canonical.0.upgrade() {
            owner
                .text_tracks
                .borrow_mut()
                .forward
                .insert(canonical.1, (Rc::downgrade(destination), id));
        }
        source
            .text_tracks
            .borrow_mut()
            .forward
            .insert(old, (Rc::downgrade(destination), id));
        let mut all = destination.text_tracks.borrow_mut();
        all.canonical.insert(id, canonical);
        all.lists.insert((node, key.1), id);
    }
    Ok(())
}
pub(super) fn retire(state: &V8BindingState) {
    let ids = state
        .text_tracks
        .borrow()
        .tracks
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in ids {
        let _ = cancel(state, id);
    }
    let mut all = state.text_tracks.borrow_mut();
    if let Some(delivery) = all.delivery.take() {
        delivery.cancel(state)
    }
    all.events.clear();
    all.delivery_targets.clear();
}
pub(super) fn add_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    media: NodeId,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    state.text_tracks.borrow_mut().self_owner = Rc::downgrade(state);
    let string = |scope: &mut v8::PinScope, index| -> Result<String, V8BindingError> {
        if args.length() <= index {
            return Ok(String::new());
        }
        Ok(args
            .get(index)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope))
    };
    let kind = string(scope, 0)?;
    if !matches!(
        kind.as_str(),
        "subtitles" | "captions" | "descriptions" | "chapters" | "metadata"
    ) {
        return Err(fail("TypeError"));
    }
    let label = string(scope, 1)?;
    let language = string(scope, 2)?;
    let id = new_track(state, None, Some(media), kind, label, language, "hidden")?;
    state.text_tracks.borrow_mut().dirty = true;
    notify_list(state, Some(media), "addtrack", id);
    object(scope, state, id).map(Some)
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object_value: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let state = owner(scope, object_value)?;
    let id = stateful_id(scope, object_value)?;
    let (state, id) = resolve(state, id)?;
    if matches!(member, "getTrackById" | "getCueById") {
        let name = args
            .get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        for item in ids(&state, id)? {
            let matches = if member == "getCueById" {
                cue(&state, item)?.borrow().data.id == name
            } else {
                let t = track(&state, item)?;
                t.node.is_some_and(|node| {
                    state
                        .dom
                        .borrow()
                        .attribute_in_null_namespace(node, "id")
                        .ok()
                        .flatten()
                        == Some(name.as_str())
                })
            };
            if matches {
                return object(scope, &state, item).map(Some);
            }
        }
        return Ok(Some(v8::null(scope).into()));
    }
    if member == "getCueAsHTML" {
        let text = cue(&state, id)?.borrow().data.text.clone();
        let fragment = cue_html(&state, &text)?;
        return wrap_node(scope, &state, fragment).map(|value| Some(value.into()));
    }
    if matches!(member, "addCue" | "removeCue") {
        track(&state, id)?;
        let wrapper =
            v8::Local::<v8::Object>::try_from(args.get(0)).map_err(|_| fail("TypeError"))?;
        let cue_owner = owner(scope, wrapper)?;
        let original = stateful_id(scope, wrapper)?;
        let data = cue(&cue_owner, original)?;
        let (previous, previous_owner) = {
            let cue = data.borrow();
            (
                cue.track,
                cue.track_owner
                    .as_ref()
                    .and_then(std::rc::Weak::upgrade)
                    .unwrap_or_else(|| cue_owner.clone()),
            )
        };
        let belongs_here = previous == Some(id) && Rc::ptr_eq(&state, &previous_owner);
        if member == "removeCue" && !belongs_here {
            return Err(fail("NotFoundError"));
        }
        if member == "addCue" && belongs_here {
            return Ok(None);
        }
        if member == "addCue" && track(&state, id)?.cues.len() >= oryn_dom::webvtt::MAX_CUES {
            return Err(fail("QuotaExceededError"));
        }
        if let Some(previous) = previous {
            if let Some(track) = previous_owner
                .text_tracks
                .borrow_mut()
                .tracks
                .get_mut(&previous)
            {
                track.cues.retain(|entry| {
                    cue(&previous_owner, *entry).is_ok_and(|value| !Rc::ptr_eq(&value, &data))
                });
            }
            previous_owner.text_tracks.borrow_mut().dirty = true;
        }
        if member == "removeCue" {
            let mut data = data.borrow_mut();
            data.track = None;
            data.track_owner = None;
        } else {
            // Local list indices reference a shared native cue, while its wrapper stays in
            // the constructor realm. Binding-local numeric IDs never cross realm boundaries.
            let local = if Rc::ptr_eq(&state, &cue_owner) {
                original
            } else {
                let existing=state.platform_objects.borrow().iter().find_map(|(id,record)|matches!(record,PlatformObjectRecord::TextTrackCue(value)if Rc::ptr_eq(value,&data)).then_some(*id));
                existing.unwrap_or_else(|| {
                    register_platform_object(
                        &state,
                        PlatformObjectRecord::TextTrackCue(data.clone()),
                    )
                })
            };
            {
                let mut value = data.borrow_mut();
                if value.canonical.is_none() {
                    value.canonical = Some((Rc::downgrade(&cue_owner), original));
                }
                value.track = Some(id);
                value.track_owner = Some(Rc::downgrade(&state));
            }
            state
                .text_tracks
                .borrow_mut()
                .tracks
                .get_mut(&id)
                .unwrap()
                .cues
                .push(local);
        }
        state.text_tracks.borrow_mut().dirty = true;
        refresh_gc_edges(scope, &state)?;
        refresh_gc_edges(scope, &cue_owner)?;
        platform_listener_graph::reconcile_activity(scope, &state);
        return Ok(None);
    }
    Err(fail("TypeError"))
}
fn cue_html(state: &V8BindingState, text: &str) -> Result<NodeId, V8BindingError> {
    let mut dom = state.dom.borrow_mut();
    let root = dom.create_fragment();
    dom.set_node_document(root, state.document_root())?;
    let mut stack = vec![(String::new(), root)];
    let mut rest = text;
    let decode = |text: &str| {
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&lrm;", "\u{200e}")
            .replace("&rlm;", "\u{200f}")
            .replace("&nbsp;", "\u{a0}")
            .replace("&amp;", "&")
    };
    while !rest.is_empty() {
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let node = dom.create_text(decode(&rest[..end]));
            dom.append_child(stack.last().unwrap().1, node)?;
            rest = &rest[end..];
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let token = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(name) = token.strip_prefix('/') {
            if stack.last().is_some_and(|(tag, _)| tag == name) && stack.len() > 1 {
                stack.pop();
            }
            continue;
        }
        if let Some(time) = oryn_dom::webvtt::timestamp(token) {
            let milliseconds = (time * 1000.).round() as u64;
            let data = format!(
                "{:02}:{:02}:{:02}.{:03}",
                milliseconds / 3600000,
                (milliseconds / 60000) % 60,
                (milliseconds / 1000) % 60,
                milliseconds % 1000
            );
            let node = dom.create_processing_instruction("timestamp", data);
            dom.append_child(stack.last().unwrap().1, node)?;
            continue;
        }
        let mut words = token.splitn(2, char::is_whitespace);
        let names = words.next().unwrap_or("");
        let annotation = words.next().unwrap_or("").trim();
        let mut classes = names.split('.');
        let name = classes.next().unwrap_or("");
        let tag = match name {
            "c" | "v" | "lang" => "span",
            "b" | "i" | "u" | "ruby" => name,
            "rt" if stack.last().is_some_and(|(name, _)| name == "ruby") => "rt",
            _ => continue,
        };
        let node = dom.create_element(tag);
        let classes = classes.collect::<Vec<_>>().join(" ");
        if !classes.is_empty() {
            dom.set_attribute(node, "class", &classes)?;
        }
        if name == "v" {
            dom.set_attribute(node, "title", annotation)?;
        }
        if name == "lang" {
            dom.set_attribute(node, "lang", annotation)?;
        }
        dom.append_child(stack.last().unwrap().1, node)?;
        stack.push((name.into(), node));
    }
    Ok(root)
}
fn indexed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object_value: v8::Local<v8::Object>,
    index: u32,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let state = owner(scope, object_value)?;
    let id = stateful_id(scope, object_value)?;
    let (state, id) = resolve(state, id)?;
    av_tracks::refresh_list(scope, &state, id)?;
    let ids = ids(&state, id)?;
    ids.get(index as usize)
        .copied()
        .map(|id| object(scope, &state, id))
        .transpose()
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
fn indexed_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let state = owner(scope, object)?;
    let id = stateful_id(scope, object)?;
    let (state, id) = resolve(state, id)?;
    if matches!(
        &*platform_object(&state, id)?,
        PlatformObjectRecord::MediaTrackList { .. }
    ) {
        av_tracks::get(scope, &state, id, "length")
    } else {
        get(scope, object, "length")
    }
}
fn indexed_enumerate(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match indexed_length(scope, args.holder()) {
        Ok(length) => {
            let values = (0..length.uint32_value(scope).unwrap_or(0))
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
        throw_type_error(scope, "Text track list has no indexed setter");
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
        throw_type_error(scope, "Text track list indices cannot be defined");
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
                throw_type_error(scope, "Text track list index cannot be deleted");
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
pub(super) fn queue_platform(
    state: &V8BindingState,
    id: u64,
    name: &'static str,
    track: Option<u64>,
) {
    let canonical = state.text_tracks.borrow().canonical.get(&id).cloned();
    let track_owner = state.text_tracks.borrow().self_owner.clone();
    if let Some((owner, original)) = canonical {
        if let Some(owner) = owner.upgrade() {
            let target = track.map_or(Target::Platform(original), |track| {
                Target::TrackEvent(original, track_owner, track)
            });
            let mut all = owner.text_tracks.borrow_mut();
            if name != "change"
                || !all.events.iter().any(|(target, event)| {
                    *event == name && matches!(target,Target::Platform(id)if *id==original)
                })
            {
                all.events.push_back((target, name));
            }
            return;
        }
    }
    let target = track.map_or(Target::Platform(id), |track| {
        Target::TrackEvent(id, track_owner, track)
    });
    let mut all = state.text_tracks.borrow_mut();
    if name != "change"
        || !all.events.iter().any(|(target, event)| {
            *event == name && matches!(target,Target::Platform(target)if *target==id)
        })
    {
        all.events.push_back((target, name));
    }
}
fn notify_list(state: &V8BindingState, media: Option<NodeId>, name: &'static str, track: u64) {
    let id = media.and_then(|media| state.text_tracks.borrow().lists.get(&(media, 0)).copied());
    if let Some(id) = id {
        queue_platform(state, id, name, (name != "change").then_some(track));
    }
}
fn track_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.native.track.event.track")
        .ok_or_else(|| fail("QuotaExceededError"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
fn set_event_track(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = track_key(scope)?;
    if event.set_private(scope, key, value) != Some(true) {
        return Err(fail("QuotaExceededError"));
    }
    Ok(())
}
pub(super) fn initialize_event(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    init: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let value =
        checked_dictionary_member(scope, init, "track")?.unwrap_or_else(|| v8::null(scope).into());
    if !value.is_null() {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| fail("TypeError"))?;
        let state = owner(scope, object)?;
        let id = stateful_id(scope, object)?;
        if !matches!(
            &*platform_object(&state, id)?,
            PlatformObjectRecord::TextTrack | PlatformObjectRecord::MediaTrack(_)
        ) {
            return Err(fail("TypeError"));
        }
    }
    set_event_track(scope, event, value)
}
pub(super) fn event_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = track_key(scope)?;
    Ok(event
        .get_private(scope, key)
        .filter(|value| !value.is_undefined())
        .unwrap_or_else(|| v8::null(scope).into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        requests: RefCell<Vec<PlatformHostRequest>>,
        cancelled: RefCell<Vec<HostOperationId>>,
    }
    impl PlatformHost for Host {
        fn submit(&self, _: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            self.requests.borrow_mut().push(request);
            Ok(())
        }
        fn cancel(&self, id: HostOperationId) {
            self.cancelled.borrow_mut().push(id)
        }
        fn retire(&self) {}
        fn drain_completions(&self, _: usize) -> Vec<crate::platform::PlatformHostCompletion> {
            Vec::new()
        }
    }
    fn take(state: &V8BindingState) -> Pending {
        let id = state
            .pending_platform_operations
            .borrow()
            .iter()
            .find_map(|(id, p)| matches!(p, PendingPlatformOperation::TextTrack(_)).then_some(*id))
            .unwrap();
        let Some(PendingPlatformOperation::TextTrack(p)) =
            state.pending_platform_operations.borrow_mut().remove(&id)
        else {
            unreachable!()
        };
        p
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn start(stream_id: u64) -> Result<PlatformHostResult, V8BindingError> {
        Ok(PlatformHostResult::HttpStarted {
            timing: None,
            internal_response: None,
            response_type: None,
            upload: None,
            cors_filtered: false,
            redirected: false,
            stream_id,
            status: 200,
            url: "https://example.test/cues".into(),
            headers: vec![("content-type".into(), "text/vtt".into())],
        })
    }
    #[test]
    fn detached_track_uses_document_query_encoding_and_terminal_ack_before_load() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<base href='/assets/'>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/page").unwrap();
        dom.borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "windows-1252".into();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        eval(
            scope,
            "globalThis.video=document.createElement('video');globalThis.element=document.createElement('track');video.append(element);element.src='cues?å';element.track.mode='showing';globalThis.loaded=0;element.onload=()=>loaded++;",
        );
        poll(scope, &state).unwrap();
        assert!(host.requests.borrow().iter().any(|request|matches!(request,PlatformHostRequest::MediaStart{request,destination,..}if destination=="track"&&request.url=="https://example.test/assets/cues?%E5"&&request.fetch.as_ref().unwrap().mode=="same-origin")));
        complete(&state, take(&state), start(5)).unwrap();
        complete(
            &state,
            take(&state),
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 5,
                bytes: b"WEBVTT\n\nid\n00:00.000 --> 00:01.000\nhello".to_vec(),
                done: false,
            }),
        )
        .unwrap();
        poll(scope, &state).unwrap();
        assert_eq!(eval(scope, "loaded").int32_value(scope), Some(0));
        complete(
            &state,
            take(&state),
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 5,
                bytes: Vec::new(),
                done: true,
            }),
        )
        .unwrap();
        state.set_execution_suspended(true);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        state.set_execution_suspended(false);
        poll(scope, &state).unwrap();
        assert!(eval(scope,"loaded===1&&element.readyState===2&&element.track.cues[0].text==='hello'&&element.track.cues.getCueById('id')===element.track.cues[0]&&element.track.cues[0].track===element.track").is_true());
    }
    #[test]
    fn changed_source_and_disabled_mode_cancel_old_stream_without_stale_cues() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video><track id=t src='/old'></video>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/").unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        eval(
            scope,
            "globalThis.t=document.getElementById('t');t.track.mode='hidden';",
        );
        poll(scope, &state).unwrap();
        let old = take(&state);
        eval(scope, "t.src='/new'");
        complete(&state, old, start(7)).unwrap();
        assert!(
            host.requests
                .borrow()
                .iter()
                .any(|request| matches!(request, PlatformHostRequest::HttpAbort { stream_id: 7 }))
        );
        poll(scope, &state).unwrap();
        complete(&state, take(&state), start(8)).unwrap();
        eval(scope, "t.track.mode='disabled'");
        poll(scope, &state).unwrap();
        assert!(
            host.requests
                .borrow()
                .iter()
                .any(|request| matches!(request, PlatformHostRequest::HttpAbort { stream_id: 8 }))
        );
        assert!(eval(scope, "t.track.cues===null&&t.track.activeCues===null").is_true());
    }
    #[test]
    fn native_cue_identity_lists_settings_and_safe_cue_fragment() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,"(()=>{const video=document.createElement('video'),track=video.addTextTrack('metadata'),cue=new VTTCue(-1,2,'<v Ada><b>safe &amp; sound</b></v><script>plain</script>');track.addCue(cue);const list=track.cues,fragment=cue.getCueAsHTML();if(cue.track!==track||list[0]!==cue||fragment.querySelector('script')||fragment.textContent!=='safe & soundplain')return false;cue.id='stable';cue.line=20;cue.snapToLines=false;if(list.getCueById('stable')!==cue)return false;track.removeCue(cue);return cue.track===null&&list.length===0;})()").is_true());
    }
    #[test]
    fn live_track_and_cue_order_follows_native_mutations() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,"(()=>{const v=document.createElement('video'),a=document.createElement('track'),b=document.createElement('track');v.append(a,b);const list=v.textTracks;v.prepend(b);if(list[0]!==b.track||list[1]!==a.track)return false;b.kind='CAPTIONS';b.label='changed';b.srclang='fr';if(b.track.kind!=='captions'||b.track.label!=='changed'||b.track.language!=='fr')return false;const t=v.addTextTrack('metadata'),x=new VTTCue(4,6,'x'),y=new VTTCue(1,2,'y'),z=new VTTCue(1,3,'z');t.addCue(x);t.addCue(y);t.addCue(z);const cues=t.cues;if(cues[0]!==z||cues[1]!==y||cues[2]!==x)return false;x.startTime=0;return cues[0]===x;})()").is_true());
    }

    #[test]
    fn cross_realm_cue_transfer_preserves_wrapper_and_track_identity() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let (first_state, foreign) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            let value = eval(
                scope,
                "globalThis.original=new VTTCue(0,10,'foreign');globalThis.originalTrack=document.createElement('video').addTextTrack('metadata');originalTrack.addCue(original);original",
            );
            (state, v8::Global::new(scope, value))
        };
        {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let _state = install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            let key = v8::String::new(scope, "foreign").unwrap();
            let value = v8::Local::new(scope, &foreign);
            assert_eq!(
                second.global(scope).set(scope, key.into(), value),
                Some(true)
            );
            assert!(eval(scope,"(()=>{const unrelated=new VTTCue(2,3,'local'),track=document.createElement('video').addTextTrack('metadata');track.addCue(foreign);if(track.cues[0]!==foreign||foreign.track!==track)return false;foreign.text='shared';track.removeCue(foreign);return foreign.track===null&&unrelated.text==='local';})()").is_true());
        }
        {
            let scope = &mut v8::ContextScope::new(scope, first);
            assert!(
                eval(
                    scope,
                    "originalTrack.cues.length===0&&original.text==='shared'&&original.track===null"
                )
                .is_true()
            );
        }
        drop(first_state);
    }

    #[test]
    fn foreign_region_native_reference_survives_owner_collection_until_cue_release() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let (region_owner, region, region_id) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            let value = eval(scope, "new VTTRegion()");
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let id = stateful_id(scope, object).unwrap();
            (state, v8::Global::new(scope, value), id)
        };
        {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            let key = v8::String::new(scope, "foreignRegion").unwrap();
            let value = v8::Local::new(scope, &region);
            second.global(scope).set(scope, key.into(), value).unwrap();
            assert!(eval(scope,"globalThis.cue=new VTTCue(0,1,'foreign');cue.region=foreignRegion;cue.region===foreignRegion").is_true());
            assert_eq!(
                region_owner
                    .text_tracks
                    .borrow()
                    .region_refs
                    .get(&region_id),
                Some(&1)
            );
            release_collected(&region_owner, region_id);
            assert!(
                region_owner
                    .platform_objects
                    .borrow()
                    .contains_key(&region_id)
            );
            eval(scope, "cue.region=foreignRegion");
            assert_eq!(
                region_owner
                    .text_tracks
                    .borrow()
                    .region_refs
                    .get(&region_id),
                Some(&1)
            );
            let cue_object = v8::Local::<v8::Object>::try_from(eval(scope, "cue")).unwrap();
            let cue_id = stateful_id(scope, cue_object).unwrap();
            release_collected(&state, cue_id);
            assert!(
                !region_owner
                    .text_tracks
                    .borrow()
                    .region_refs
                    .contains_key(&region_id)
            );
            release_collected(&region_owner, region_id);
            assert!(
                !region_owner
                    .platform_objects
                    .borrow()
                    .contains_key(&region_id)
            );
        }
    }

    #[test]
    fn cue_timeline_distinguishes_playback_skips_from_seek_discontinuity() {
        let cues = [(1, 1., 2.), (2, 2., 4.), (3, 0., 8.)];
        let (active, events) = cue_transitions(&cues, &[3], 3., Some(0.), true);
        assert_eq!(active, vec![3, 2]);
        assert_eq!(events, vec![(1, "enter"), (1, "exit"), (2, "enter")]);
        let (_, events) = cue_transitions(&cues, &[3], 3., None, true);
        assert_eq!(events, vec![(2, "enter")]);
        let (active, events) = cue_transitions(&cues, &[3, 2], 3., None, false);
        assert!(active.is_empty());
        assert_eq!(events, vec![(2, "exit"), (3, "exit")]);
    }

    #[test]
    fn adoption_keeps_track_cue_and_live_list_wrappers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let (source, video, track_wrapper, list_wrapper, cues_wrapper) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
            eval(
                scope,
                "globalThis.v=document.createElement('video');globalThis.e=document.createElement('track');v.append(e);globalThis.t=e.track;t.mode='hidden';t.addCue(new VTTCue(0,10,'retained'));globalThis.list=v.textTracks;globalThis.cues=t.cues;",
            );
            let video = eval(scope, "v");
            let video = v8::Global::new(scope, video);
            let track = eval(scope, "t");
            let track = v8::Global::new(scope, track);
            let list = eval(scope, "list");
            let list = v8::Global::new(scope, list);
            let cues = eval(scope, "cues");
            let cues = v8::Global::new(scope, cues);
            (state, video, track, list, cues)
        };
        {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let _destination =
                install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            for (name, value) in [
                ("v", &video),
                ("oldTrack", &track_wrapper),
                ("oldList", &list_wrapper),
                ("oldCues", &cues_wrapper),
            ] {
                let key = v8::String::new(scope, name).unwrap();
                let value = v8::Local::new(scope, value);
                assert_eq!(
                    second.global(scope).set(scope, key.into(), value),
                    Some(true)
                );
            }
            assert!(eval(scope,"document.adoptNode(v);v.firstChild.track===oldTrack&&v.textTracks===oldList&&oldList[0]===oldTrack&&oldTrack.cues===oldCues&&oldCues[0].text==='retained'&&oldCues[0].track===oldTrack").is_true());
        }
        drop(source);
    }

    #[test]
    fn inert_media_lists_and_cue_idl_numeric_contracts() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        assert!(eval(scope,"(()=>{const inert=document.implementation.createHTMLDocument(''),v=inert.createElement('video'),e=inert.createElement('track');v.append(e);e.src='https://example.test/cues';e.track.mode='showing';if(v.textTracks.length!==1||v.textTracks[0]!==e.track)return false;e.kind='invalid';if(e.kind!=='metadata'||e.track.kind!=='metadata'||e.track.inBandMetadataTrackDispatchType!=='')return false;const cue=new VTTCue(0,Infinity,'live');if(cue.endTime!==Infinity)return false;cue.endTime=Infinity;const region=new VTTRegion();region.id='region';region.lines=NaN;return region.id==='region'&&region.lines===0;})()").is_true());
        poll(scope, &state).unwrap();
        assert!(!host.requests.borrow().iter().any(|request|matches!(request,PlatformHostRequest::MediaStart{destination,..}if destination=="track")));
    }

    #[test]
    fn native_track_edges_keep_cue_callbacks_live_across_collection() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.entered=0;(()=>{const track=document.getElementById('v').addTextTrack('metadata');const cue=new VTTCue(0,10,'retained');cue.onenter=()=>entered++;track.addCue(cue);})()",
        );
        refresh_gc_edges(scope, &state).unwrap();
        scope.low_memory_notification();
        poll(scope, &state).unwrap();
        assert_eq!(eval(scope, "entered").int32_value(scope), Some(1));
    }

    #[test]
    fn exposed_track_materializes_previously_unwrapped_media_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(909), 1).unwrap();
        bind_owner(&state);
        let (media, node) = {
            let mut dom = state.dom.borrow_mut();
            let media = dom.create_element("video");
            let node = dom.create_element("track");
            dom.set_node_document(media, state.document_root()).unwrap();
            dom.append_child(media, node).unwrap();
            (media, node)
        };
        assert!(
            state
                .wrappers
                .borrow()
                .lookup(state.dom.borrow().arena().id(), state.realm, media)
                .is_none()
        );
        let id = ensure(&state, node).unwrap();
        let (media_wrapper, track) = {
            v8::scope!(let temporary,scope);
            let track = object(temporary, &state, id)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            let media = existing_agent_wrapper(temporary, &state, media)
                .expect("retained track publishes its unexposed media owner");
            (
                v8::Weak::new(temporary, media),
                v8::Global::new(temporary, track),
            )
        };
        scope.low_memory_notification();
        assert!(media_wrapper.to_local(scope).is_some());
        drop(track);
    }

    #[test]
    fn track_private_graph_preserves_both_separated_native_owners() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(908), 1).unwrap();
        let (media, node, track) = {
            v8::scope!(let temporary,scope);
            let values=eval(temporary,"(()=>{const media=document.createElement('video'),node=document.createElement('track');media.marker=31;node.marker=47;media.append(node);const track=node.track;node.remove();return [media,node,track]})()").try_cast::<v8::Array>().unwrap();
            let media = values
                .get_index(temporary, 0)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            let node = values
                .get_index(temporary, 1)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            let track = values
                .get_index(temporary, 2)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            refresh_gc_edges(temporary, &state).unwrap();
            (
                v8::Weak::new(temporary, media),
                v8::Weak::new(temporary, node),
                v8::Global::new(temporary, track),
            )
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let temporary,scope);
            for (object, expected) in [
                (media.to_local(temporary).unwrap(), 31),
                (node.to_local(temporary).unwrap(), 47),
            ] {
                let key = v8::String::new(temporary, "marker").unwrap();
                assert_eq!(
                    object
                        .get(temporary, key.into())
                        .unwrap()
                        .int32_value(temporary),
                    Some(expected)
                );
            }
        }
        drop(track);
        scope.low_memory_notification();
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        poll(scope, &state).unwrap();
        assert!(media.to_local(scope).is_none());
        assert!(node.to_local(scope).is_none());
    }

    #[test]
    fn detached_track_cycles_release_native_roots_after_collection() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "(()=>{const video=document.createElement('video'),track=video.addTextTrack('metadata'),cue=new VTTCue(1,2,'cycle');cue.onenter=()=>track.mode;track.addCue(cue);})()",
        );
        assert_eq!(state.text_tracks.borrow().tracks.len(), 1);
        wrapper_graphs::refresh_agent(scope, &state);
        refresh_gc_edges(scope, &state).unwrap();
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        poll(scope, &state).unwrap();
        assert!(state.text_tracks.borrow().tracks.is_empty());
    }
}

/// Native media relationships participate in V8's traced graph, so a listener
/// closing over its cue/list is collectible once native activity ends.
pub(super) fn refresh_gc_edges(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, "oryn.native.media.track.edges")
        .ok_or_else(|| fail("QuotaExceededError"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let mut edges: BTreeMap<u64, Vec<(std::rc::Weak<V8BindingState>, u64)>> = BTreeMap::new();
    let mut nodes: BTreeMap<NodeId, Vec<u64>> = BTreeMap::new();
    let self_owner = state.text_tracks.borrow().self_owner.clone();
    {
        let all = state.text_tracks.borrow();
        for (id, track) in &all.tracks {
            let related = edges.entry(*id).or_default();
            related.extend(
                track
                    .cues
                    .iter()
                    .chain(track.cues_list.iter())
                    .chain(track.active_list.iter())
                    .map(|id| (self_owner.clone(), *id)),
            );
            for node in track.node.into_iter().chain(track.media) {
                nodes.entry(node).or_default().push(*id);
            }
        }
        for ((node, _), id) in &all.lists {
            nodes.entry(*node).or_default().push(*id);
            edges.entry(*id).or_default().extend(
                all.tracks
                    .iter()
                    .filter(|(_, track)| track.media == Some(*node))
                    .map(|(id, _)| (self_owner.clone(), *id)),
            );
        }
    }
    {
        let objects = state.platform_objects.borrow();
        for (id, record) in objects.iter() {
            if state.text_tracks.borrow().forward.contains_key(id) {
                continue;
            }
            match record {
                PlatformObjectRecord::TextTrackCue(cue) => {
                    let cue = cue.borrow();
                    if let Some(track) = cue.track {
                        edges.entry(*id).or_default().push((
                            cue.track_owner
                                .clone()
                                .unwrap_or_else(|| self_owner.clone()),
                            track,
                        ));
                    }
                    if let Some(region) = &cue.region {
                        edges.entry(*id).or_default().push(region.clone());
                    }
                }
                PlatformObjectRecord::TextTrackList(List::Cues(track_id, _)) => {
                    if let Ok(track) = track(state, *track_id) {
                        edges
                            .entry(*id)
                            .or_default()
                            .push((self_owner.clone(), *track_id));
                        edges
                            .entry(*id)
                            .or_default()
                            .extend(track.cues.iter().map(|id| (self_owner.clone(), *id)));
                    }
                }
                PlatformObjectRecord::MediaTrack(_) => {
                    if let Some((media, list)) = av_tracks::association(state, *id) {
                        nodes.entry(media).or_default().push(*id);
                        edges
                            .entry(*id)
                            .or_default()
                            .extend(list.into_iter().map(|id| (self_owner.clone(), id)));
                    }
                }
                PlatformObjectRecord::MediaTrackList { media, .. } => {
                    nodes.entry(*media).or_default().push(*id);
                    edges.entry(*id).or_default().extend(
                        av_tracks::ids(state, *id)?
                            .into_iter()
                            .map(|id| (self_owner.clone(), id)),
                    );
                }
                _ => {}
            }
        }
    }
    // Canonical wrappers can belong to another realm after node/cue adoption.
    fn local<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        owner: &V8BindingState,
        id: u64,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let canonical = owner
            .text_tracks
            .borrow()
            .canonical
            .get(&id)
            .cloned()
            .or_else(|| {
                cue(owner, id)
                    .ok()
                    .and_then(|cue| cue.borrow().canonical.clone())
            });
        if let Some((state, original)) = canonical {
            if let Some(state) = state.upgrade() {
                if state.instance_id != owner.instance_id || original != id {
                    return state
                        .platform_wrappers
                        .borrow()
                        .get(&original)
                        .and_then(|wrapper| wrapper.to_local(scope));
                }
            }
        }
        owner
            .platform_wrappers
            .borrow()
            .get(&id)
            .and_then(|wrapper| wrapper.to_local(scope))
    }
    // A track can own both a track element and its media element. Accumulate
    // all node and platform edges first; publishing once avoids replacing the
    // first owner when the second node is visited.
    let mut values: BTreeMap<u64, Vec<v8::Local<v8::Value>>> = BTreeMap::new();
    for (node, ids) in nodes {
        let targets = ids
            .iter()
            .filter_map(|id| local(scope, state, *id))
            .map(Into::into)
            .collect::<Vec<v8::Local<v8::Value>>>();
        let existing = state
            .wrappers
            .borrow()
            .lookup(state.dom.borrow().arena().id(), state.realm, node)
            .and_then(|id| {
                state
                    .objects
                    .borrow()
                    .get(&id)
                    .and_then(|object| object.handle.to_local(scope))
            });
        let wrapper = if existing.is_none() && !targets.is_empty() {
            let owner = scope
                .get_slot::<BindingStateRegistry>()
                .and_then(|registry| {
                    registry
                        .instances
                        .borrow()
                        .get(&state.instance_id)
                        .and_then(RcWeak::upgrade)
                })
                .ok_or(V8BindingError::Allocation("track graph owner"))?;
            Some(wrap_node(scope, &owner, node)?)
        } else {
            existing
        };
        if let Some(wrapper) = wrapper {
            let array = v8::Array::new_with_elements(scope, &targets);
            wrapper
                .set_private(scope, key, array.into())
                .filter(|ok| *ok)
                .ok_or(V8BindingError::JavaScriptException)?;
            for id in ids {
                values.entry(id).or_default().push(wrapper.into());
            }
        }
    }
    for (id, related) in edges {
        let values = values.entry(id).or_default();
        for (owner, id) in related {
            if let Some(owner) = owner.upgrade() {
                if let Some(value) = local(scope, &owner, id) {
                    values.push(value.into());
                }
            }
        }
    }
    for (id, values) in values {
        if let Some(target) = local(scope, state, id) {
            let array = v8::Array::new_with_elements(scope, &values);
            target
                .set_private(scope, key, array.into())
                .filter(|ok| *ok)
                .ok_or(V8BindingError::JavaScriptException)?;
        }
    }
    Ok(())
}
pub(super) fn active_platforms(state: &V8BindingState) -> BTreeSet<u64> {
    let all = state.text_tracks.borrow();
    let mut result = all.delivery_targets.clone();
    for (target, _) in &all.events {
        match target {
            Target::Platform(id) => {
                result.insert(*id);
            }
            Target::TrackEvent(id, owner, track) => {
                result.insert(*id);
                if owner
                    .upgrade()
                    .is_some_and(|owner| owner.instance_id == state.instance_id)
                {
                    result.insert(*track);
                }
            }
            Target::Node(_) => {}
        }
    }
    for (id, track) in &all.tracks {
        if track.ready == 1 {
            result.insert(*id);
            result.extend(track.cues.iter().copied());
        }
    }
    result
}
pub(super) fn release_collected(state: &V8BindingState, id: u64) {
    let removable = match state.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::TextTrackCue(cue)) => cue.borrow().track.is_none(),
        Some(PlatformObjectRecord::VttRegion(_)) => {
            state
                .text_tracks
                .borrow()
                .region_refs
                .get(&id)
                .copied()
                .unwrap_or(0)
                == 0
        }
        _ => false,
    };
    if removable {
        state.platform_objects.borrow_mut().remove(&id);
    }
}

fn prune_detached(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let live_node = |scope: &mut v8::PinScope, node: NodeId| {
        let dom = state.dom.borrow();
        if dom.is_connected(node).unwrap_or(false) {
            return true;
        }
        let id = state
            .wrappers
            .borrow()
            .lookup(dom.arena().id(), state.realm, node);
        id.and_then(|id| {
            state
                .objects
                .borrow()
                .get(&id)
                .and_then(|object| object.handle.to_local(scope))
        })
        .is_some()
    };
    let active = active_platforms(state);
    let ids = state
        .text_tracks
        .borrow()
        .tracks
        .iter()
        .filter_map(|(id, track)| {
            let wrapper = state
                .platform_wrappers
                .borrow()
                .get(id)
                .and_then(|wrapper| wrapper.to_local(scope))
                .is_some();
            (!wrapper
                && !active.contains(id)
                && !track
                    .node
                    .into_iter()
                    .chain(track.media)
                    .any(|node| live_node(scope, node)))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    for id in ids {
        cancel(state, id)?;
        let Some(track) = state.text_tracks.borrow_mut().tracks.remove(&id) else {
            continue;
        };
        if let Some(node) = track.node {
            state.text_tracks.borrow_mut().nodes.remove(&node);
        }
        for cue_id in track.cues {
            if let Ok(cue) = cue(state, cue_id) {
                let mut cue = cue.borrow_mut();
                cue.track = None;
                cue.track_owner = None;
            }
            if state
                .platform_wrappers
                .borrow()
                .get(&cue_id)
                .and_then(|wrapper| wrapper.to_local(scope))
                .is_none()
            {
                state.platform_objects.borrow_mut().remove(&cue_id);
                state.platform_wrappers.borrow_mut().remove(&cue_id);
            }
        }
        for list_id in track.cues_list.into_iter().chain(track.active_list) {
            if state
                .platform_wrappers
                .borrow()
                .get(&list_id)
                .and_then(|wrapper| wrapper.to_local(scope))
                .is_none()
            {
                state.platform_objects.borrow_mut().remove(&list_id);
                state.platform_wrappers.borrow_mut().remove(&list_id);
            }
        }
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }
    let lists = state
        .text_tracks
        .borrow()
        .lists
        .iter()
        .filter_map(|(key, id)| {
            (!active.contains(id)
                && state
                    .platform_wrappers
                    .borrow()
                    .get(id)
                    .and_then(|wrapper| wrapper.to_local(scope))
                    .is_none()
                && !live_node(scope, key.0))
            .then_some((*key, *id))
        })
        .collect::<Vec<_>>();
    for (key, id) in lists {
        state.text_tracks.borrow_mut().lists.remove(&key);
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }

    let regions = state
        .platform_objects
        .borrow()
        .iter()
        .filter_map(|(id, record)| {
            (matches!(record, PlatformObjectRecord::VttRegion(_))
                && state
                    .platform_wrappers
                    .borrow()
                    .get(id)
                    .and_then(|wrapper| wrapper.to_local(scope))
                    .is_none())
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    for id in regions {
        release_collected(state, id);
    }

    Ok(())
}

pub(super) fn owned_native_roots(
    state: &V8BindingState,
    roots: &mut traced_native_roots::Snapshot,
) -> Result<(), V8BindingError> {
    for track in state.text_tracks.borrow().tracks.values() {
        for target in track._node.iter().chain(track._media.iter()) {
            roots.add_lease(&target.native_lease())?;
        }
    }
    Ok(())
}
