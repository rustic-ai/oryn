//! URL media resource selection and bounded, owned range loading.
//! Transport completions only enqueue native media tasks; authored handlers run
//! at the document checkpoint, after source/lifetime revalidation.
use super::*;
const LIMIT: usize = 8 * 1024 * 1024;
#[derive(Default)]
pub(super) struct State {
    delivery: Option<Delivery>,
    polling: bool,
    detached: BTreeSet<NodeId>,
    types: BTreeMap<String, bool>,
    releases: BTreeSet<String>,
    play_requests: BTreeSet<NodeId>,
    forced: BTreeSet<NodeId>,
    cursor: Option<u64>,
    entries: BTreeMap<NodeId, Entry>,
    next: u64,
}
struct Delivery {
    epoch: u64,
    event: retained_dom_event::Event,
}
impl State {
    pub(super) fn has_work(&self) -> bool {
        self.delivery.is_some()
            || !self.releases.is_empty()
            || self
                .entries
                .values()
                .any(|e| e.network == 2 || !e.events.is_empty())
    }
}
struct Entry {
    progress_at: std::time::Instant,
    stalled: bool,
    key: Key,
    alternatives: VecDeque<(String, Option<NodeId>, String)>,
    token: u64,
    network: u16,
    current_src: String,
    events: VecDeque<(NodeId, &'static str)>,
    duration: Option<f64>,
}
#[derive(Clone, PartialEq, Eq)]
struct Key {
    url: String,
    source_node: Option<NodeId>,
    source_mime: String,
    alternatives: Vec<(String, Option<NodeId>, String)>,
    cors: Option<String>,
    destination: String,
    preload: String,
}
pub(super) struct Pending {
    origin_clean: bool,
    node: NodeId,
    token: u64,
    document: NodeId,
    generation: u64,
    epoch: u64,
    key: Key,
    stream: Option<u64>,
    offset: u64,
    range_start: u64,
    total: Option<u64>,
    partial: bool,
    bytes: Vec<u8>,
    rounds: u16,
}
impl Pending {
    pub(super) fn initiator(&self) -> &str {
        &self.key.destination
    }
}
fn error(message: &str) -> V8BindingError {
    V8BindingError::Message(message.into())
}
fn current(state: &V8BindingState, p: &Pending) -> bool {
    !state.execution_retired()
        && state.document_root() == p.document
        && state.document_generation() == p.generation
        && state.document_stream_epoch() == p.epoch
        && state
            .native_media
            .borrow()
            .resources
            .entries
            .get(&p.node)
            .is_some_and(|e| e.token == p.token)
}
fn has_stream(state: &V8BindingState, node: NodeId) -> bool {
    state
        .native_media
        .borrow()
        .playback
        .get(&node)
        .is_some_and(media_element::has_stream)
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    !state.document_load_stopped()
        && (state.native_media.borrow().resources.delivery.is_some()
            || state.native_media.borrow().resources.cursor
                != Some(state.dom.borrow().mutation_cursor().0)
            || state
                .native_media
                .borrow()
                .resources
                .entries
                .values()
                .any(|e| !e.events.is_empty()))
}
#[cfg(test)]
pub(super) fn pending(state: &V8BindingState) -> bool {
    state.native_media.borrow().resources.delivery.is_some()
        || state
            .native_media
            .borrow()
            .resources
            .entries
            .values()
            .any(|e| e.network == 2 || !e.events.is_empty())
}
pub(super) fn stalled(state: &V8BindingState, node: NodeId) -> bool {
    state
        .native_media
        .borrow()
        .resources
        .entries
        .get(&node)
        .is_some_and(|entry| entry.stalled)
}
pub(super) fn get(state: &V8BindingState, node: NodeId, member: &str) -> (String, f64) {
    let media = state.native_media.borrow();
    let entry = media.resources.entries.get(&node);
    match member {
        "currentSrc" => (entry.map(|e| e.current_src.clone()).unwrap_or_default(), 0.),
        "networkState" => (String::new(), entry.map_or(0., |e| f64::from(e.network))),
        "duration" => (
            String::new(),
            entry.and_then(|e| e.duration).unwrap_or(f64::NAN),
        ),
        _ => (String::new(), 0.),
    }
}
pub(super) fn request_play(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    admit_play_request(state, node)?;
    if state
        .native_media
        .borrow()
        .resources
        .entries
        .get(&node)
        .is_some_and(|entry| entry.network == 2)
    {
        return Ok(());
    }
    cancel(state, node)?;
    let mut media = state.native_media.borrow_mut();
    media.resources.forced.insert(node);
    media.resources.play_requests.insert(node);
    media.resources.cursor = None;
    Ok(())
}
pub(super) fn admit_play_request(
    state: &V8BindingState,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let media = state.native_media.borrow();
    if media
        .resources
        .entries
        .get(&node)
        .is_some_and(|entry| entry.network == 2)
    {
        return Ok(());
    }
    if (media.resources.forced.len() >= 256 && !media.resources.forced.contains(&node))
        || (media.resources.play_requests.len() >= 256
            && !media.resources.play_requests.contains(&node))
    {
        return Err(error("media selection inventory exceeded"));
    }
    Ok(())
}
pub(super) fn request_load(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    {
        let media = state.native_media.borrow();
        if media.resources.forced.len() >= 256 && !media.resources.forced.contains(&node) {
            return Err(error("media selection inventory exceeded"));
        }
    }
    media_element::admit_load_reset(state, node)?;
    cancel(state, node)?;
    media_element::url_reset(state, node)?;
    state
        .native_media
        .borrow_mut()
        .resources
        .forced
        .insert(node);
    Ok(())
}
pub(super) fn cancel(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    {
        let mut media = state.native_media.borrow_mut();
        media.resources.entries.remove(&node);
    }
    let ids = state
        .pending_platform_operations
        .borrow()
        .iter()
        .filter_map(|(id, p)| {
            matches!(p,PendingPlatformOperation::MediaResource(p)if p.node==node).then_some(*id)
        })
        .collect::<Vec<_>>();
    let mut failure = None;
    for id in ids {
        let pending = state.pending_platform_operations.borrow_mut().remove(&id);
        state.cancelled_platform_operations.borrow_mut().insert(id);
        if let Some(host) = state.platform_host.borrow().clone() {
            host.cancel(id);
        }
        if let Some(PendingPlatformOperation::MediaResource(p)) = pending
            && let Some(stream_id) = p.stream
            && let Err(e) = submit_platform_operation(
                state,
                PlatformHostRequest::HttpAbort { stream_id },
                PendingPlatformOperation::WebSocketCommand,
            )
        {
            failure = Some(e);
        }
    }
    failure.map_or(Ok(()), Err)
}
pub(super) fn clear(state: &V8BindingState) -> Result<(), V8BindingError> {
    let delivery = state.native_media.borrow_mut().resources.delivery.take();
    drop(delivery);
    state.native_media.borrow_mut().resources.detached.clear();
    state
        .native_media
        .borrow_mut()
        .resources
        .play_requests
        .clear();
    state.native_media.borrow_mut().resources.forced.clear();
    let nodes = state
        .native_media
        .borrow()
        .resources
        .entries
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let mut failure = None;
    for node in nodes {
        if let Err(e) = cancel(state, node) {
            failure = Some(e)
        }
    }
    failure.map_or(Ok(()), Err)
}
fn select(state: &V8BindingState, node: NodeId) -> Result<Option<Key>, V8BindingError> {
    let dom = state.dom.borrow();
    if dom.node_document(node).ok().flatten() != Some(state.document_root())
        || (!dom.is_connected(node).unwrap_or(false)
            && !state.native_media.borrow().resources.forced.contains(&node)
            && !state
                .native_media
                .borrow()
                .resources
                .detached
                .contains(&node))
    {
        return Ok(None);
    }
    let destination = if dom.is_element(node, "http://www.w3.org/1999/xhtml", "audio") {
        "audio"
    } else if dom.is_element(node, "http://www.w3.org/1999/xhtml", "video") {
        "video"
    } else {
        return Ok(None);
    };
    let mut sources = if let Some(src) = dom.attribute_in_null_namespace(node, "src")? {
        vec![(src.to_owned(), None, String::new())]
    } else {
        dom.arena()
            .get(node)?
            .children
            .iter()
            .filter_map(|child| {
                if !dom.is_element(*child, "http://www.w3.org/1999/xhtml", "source") {
                    return None;
                }
                dom.attribute_in_null_namespace(*child, "src")
                    .ok()
                    .flatten()
                    .map(|src| {
                        (
                            src.to_owned(),
                            Some(*child),
                            dom.attribute_in_null_namespace(*child, "type")
                                .ok()
                                .flatten()
                                .unwrap_or("")
                                .to_owned(),
                        )
                    })
            })
            .take(256)
            .collect::<Vec<_>>()
    }
    .into_iter()
    .filter(|(src, _, _)| !src.trim().is_empty())
    .filter_map(|(src, node, mime)| {
        state
            .native_document_resource_url(src.trim())
            .ok()
            .map(|url| (url.to_string(), node, mime))
    })
    .collect::<Vec<_>>();
    if sources.is_empty() {
        return Ok(None);
    }
    let (url, source_node, source_mime) = sources.remove(0);
    let alternatives = sources;
    let cors = dom
        .attribute_in_null_namespace(node, "crossorigin")?
        .map(|v| {
            if v.eq_ignore_ascii_case("use-credentials") {
                "use-credentials".into()
            } else {
                "anonymous".into()
            }
        });
    let preload = if state.native_media.borrow().resources.forced.contains(&node) {
        "auto".into()
    } else {
        dom.attribute_in_null_namespace(node, "preload")?
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    Ok(Some(Key {
        url,
        source_node,
        source_mime,
        alternatives,
        cors,
        destination: destination.into(),
        preload,
    }))
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired()
        || state.execution_suspended()
        || state.document_load_stopped()
        || state.native_media.borrow().resources.polling
    {
        return Ok(0);
    }
    state.native_media.borrow_mut().resources.polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.native_media.borrow_mut().resources.polling = false;
        }
    }
    let _guard = Guard(state);
    let mut count = poll_delivery(scope, state)?;
    if state.native_media.borrow().resources.delivery.is_some() || state.execution_suspended() {
        return Ok(count);
    }
    let releases = state
        .native_media
        .borrow()
        .resources
        .releases
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for id in releases {
        release_asset(state, id);
    }
    let cursor = state.dom.borrow().mutation_cursor().0;
    let mut nodes = state
        .dom
        .borrow()
        .arena()
        .descendants_inclusive(state.document_root())?;
    for node in state
        .native_media
        .borrow()
        .resources
        .forced
        .iter()
        .chain(state.native_media.borrow().resources.detached.iter())
        .copied()
    {
        if !nodes.contains(&node) {
            nodes.push(node);
        }
    }
    let mut wanted = BTreeMap::new();
    for node in nodes {
        if has_stream(state, node) {
            continue;
        }
        if let Some(key) = select(state, node)? {
            wanted.insert(node, key);
        }
    }
    // Admit the complete selected population and token range before canceling
    // old fetches. Selection failure must retain their host-operation custody.
    if wanted.len() > 256 {
        return Err(error("media resource inventory exceeded"));
    }
    let changed = {
        let media = state.native_media.borrow();
        let count = wanted
            .iter()
            .filter(|(node, key)| {
                media
                    .resources
                    .entries
                    .get(*node)
                    .is_none_or(|entry| &entry.key != *key)
            })
            .count();
        media
            .resources
            .next
            .checked_add(u64::try_from(count).map_err(|_| error("media identity exhausted"))?)
            .ok_or_else(|| error("media identity exhausted"))?;
        wanted
            .iter()
            .filter_map(|(node, key)| {
                media
                    .resources
                    .entries
                    .get(node)
                    .is_none_or(|entry| &entry.key != key)
                    .then_some((*node, media.resources.play_requests.contains(node)))
            })
            .collect::<Vec<_>>()
    };
    for (node, preserve_play) in changed {
        media_element::admit_selection_reset(state, node, preserve_play)?;
    }
    let removed = state
        .native_media
        .borrow()
        .resources
        .entries
        .iter()
        .filter_map(|(node, e)| (wanted.get(node) != Some(&e.key)).then_some(*node))
        .collect::<Vec<_>>();
    let reset_nodes = removed
        .iter()
        .copied()
        .filter(|node| {
            !wanted.contains_key(node)
                && state.dom.borrow().node_document(*node).ok().flatten()
                    == Some(state.document_root())
        })
        .collect::<BTreeSet<_>>();
    // Collected/adopted nodes still need old-owner fetch cancellation, but must
    // never be recreated or have the new document's output reset through ensure.
    for node in &reset_nodes {
        media_element::admit_load_reset(state, *node)?;
    }
    for node in removed {
        cancel(state, node)?;
        if reset_nodes.contains(&node) {
            media_element::url_reset(state, node)?;
        }
    }
    state
        .native_media
        .borrow_mut()
        .resources
        .detached
        .retain(|node| !state.dom.borrow().is_connected(*node).unwrap_or(false));
    state
        .native_media
        .borrow_mut()
        .resources
        .forced
        .retain(|node| wanted.contains_key(node));
    for (node, key) in wanted {
        if state
            .native_media
            .borrow()
            .resources
            .entries
            .contains_key(&node)
        {
            continue;
        }
        if state.native_media.borrow().resources.entries.len() >= 256 {
            return Err(error("media resource inventory exceeded"));
        }
        let preserve_play = state
            .native_media
            .borrow()
            .resources
            .play_requests
            .contains(&node);
        media_element::url_reset_for_selection(state, node, preserve_play)?;
        let token = {
            let mut media = state.native_media.borrow_mut();
            media.resources.next = media
                .resources
                .next
                .checked_add(1)
                .ok_or_else(|| error("media identity exhausted"))?;
            let token = media.resources.next;
            media.resources.entries.insert(
                node,
                Entry {
                    progress_at: std::time::Instant::now(),
                    stalled: false,
                    key: key.clone(),
                    alternatives: key.alternatives.iter().cloned().collect(),
                    token,
                    network: if key.preload == "none" { 1 } else { 2 },
                    current_src: key.url.clone(),
                    events: VecDeque::new(),
                    duration: None,
                },
            );
            token
        };
        state
            .native_media
            .borrow_mut()
            .resources
            .play_requests
            .remove(&node);
        if key.preload != "none" {
            queue(state, node, token, "loadstart");
            let p = Pending {
                origin_clean: false,
                node,
                token,
                document: state.document_root(),
                generation: state.document_generation(),
                epoch: state.document_stream_epoch(),
                key,
                stream: None,
                offset: 0,
                range_start: 0,
                total: None,
                partial: false,
                bytes: Vec::new(),
                rounds: 0,
            };
            start(state, p)?;
        }
    }
    {
        let mut media = state.native_media.borrow_mut();
        media.resources.cursor = Some(cursor);
        media.retried = Some(std::time::Instant::now());
    }
    let nodes = state
        .native_media
        .borrow()
        .resources
        .entries
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for node in &nodes {
        let mut media = state.native_media.borrow_mut();
        if let Some(entry) = media.resources.entries.get_mut(node)
            && entry.network == 2
            && !entry.stalled
            && entry.progress_at.elapsed() >= std::time::Duration::from_secs(3)
        {
            entry.stalled = true;
            entry.events.push_back((*node, "stalled"));
        }
    }
    for node in nodes {
        loop {
            if state.execution_retired() || state.execution_suspended() {
                return Ok(count);
            }
            let next = state
                .native_media
                .borrow()
                .resources
                .entries
                .get(&node)
                .and_then(|e| e.events.front().copied().map(|name| (e.token, name)));
            let Some((token, (target, name))) = next else {
                break;
            };
            if select(state, node)?.as_ref()
                != state
                    .native_media
                    .borrow()
                    .resources
                    .entries
                    .get(&node)
                    .map(|e| &e.key)
            {
                cancel(state, node)?;
                break;
            }
            if target == node && name == "loadedmetadata" {
                media_element::url_metadata(state, node)?;
            } else if target == node && name == "error" {
                media_element::url_error(scope, state, node)?;
            }
            let event = create_event_object(scope, state, name, false, false)?;
            let id = event_id(scope, event)?;
            if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                record.is_trusted = true;
            }
            media_element::publish_state(state, node)?;
            let event = retained_dom_event::Event::new(scope, state, target, name, event)?;
            if let Some(e) = state
                .native_media
                .borrow_mut()
                .resources
                .entries
                .get_mut(&node)
                .filter(|e| e.token == token)
            {
                e.events.pop_front();
            } else {
                break;
            }
            state.native_media.borrow_mut().resources.delivery = Some(Delivery {
                epoch: state.document_stream_epoch(),
                event,
            });
            count += poll_delivery(scope, state)?;
            if state.native_media.borrow().resources.delivery.is_some()
                || state.execution_suspended()
            {
                return Ok(count);
            }
            if select(state, node)?.as_ref()
                != state
                    .native_media
                    .borrow()
                    .resources
                    .entries
                    .get(&node)
                    .map(|e| &e.key)
            {
                cancel(state, node)?;
                break;
            }
        }
    }
    Ok(count)
}
fn poll_delivery(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let Some(mut delivery) = state.native_media.borrow_mut().resources.delivery.take() else {
        return Ok(0);
    };
    if state.execution_retired() || delivery.epoch != state.document_stream_epoch() {
        return Ok(0);
    }
    match delivery.event.poll(scope, state) {
        Ok(Some(_)) => Ok(1),
        result => {
            state.native_media.borrow_mut().resources.delivery = Some(delivery);
            result.map(|_| 0)
        }
    }
}
fn queue(state: &V8BindingState, node: NodeId, token: u64, name: &'static str) {
    if let Some(e) = state
        .native_media
        .borrow_mut()
        .resources
        .entries
        .get_mut(&node)
        .filter(|e| e.token == token)
        && (name != "progress" || !e.events.contains(&(node, name)))
    {
        e.events.push_back((node, name))
    }
}
fn start(state: &V8BindingState, mut p: Pending) -> Result<(), V8BindingError> {
    if !p.key.source_mime.trim().is_empty() && !supported_type(state, &p.key.source_mime) {
        return failed(state, p);
    }
    p.range_start = p.offset;
    let request = PlatformHttpRequest {
        upgrade_mixed_image: false,
        fetch: Some(crate::platform::FetchOptions {
            keepalive: false,
            privileged_range: true,
            cache: Default::default(),
            mode: if p.key.cors.is_some() {
                "cors"
            } else {
                "no-cors"
            }
            .into(),
            redirect: "follow".into(),
            integrity: String::new(),
        }),
        use_cors_preflight: false,
        method: "GET".into(),
        url: p.key.url.clone(),
        headers: vec![
            ("accept".into(), "*/*".into()),
            ("range".into(), format!("bytes={}-", p.offset)),
        ],
        body: Vec::new(),
        credentials: if p.key.cors.as_deref() == Some("anonymous") {
            "same-origin"
        } else {
            "include"
        }
        .into(),
        initiator_url: Some(state.request_initiator_url()?),
        referrer_policy: Some(state.document_referrer_policy()),
    };
    let node = p.node;
    let token = p.token;
    let command = PlatformHostRequest::MediaStart {
        request,
        cors_enabled: p.key.cors.is_some(),
        destination: p.key.destination.clone(),
    };
    if let Err(e) =
        submit_platform_operation(state, command, PendingPlatformOperation::MediaResource(p))
    {
        terminal(state, node, token, false);
        return Err(e);
    }
    Ok(())
}
fn failed(state: &V8BindingState, mut p: Pending) -> Result<(), V8BindingError> {
    abort(state, &mut p)?;
    if let Some(source) = p.key.source_node
        && let Some(entry) = state
            .native_media
            .borrow_mut()
            .resources
            .entries
            .get_mut(&p.node)
            .filter(|entry| entry.token == p.token)
    {
        entry.events.push_back((source, "error"));
    }
    let next = state
        .native_media
        .borrow_mut()
        .resources
        .entries
        .get_mut(&p.node)
        .filter(|e| e.token == p.token)
        .and_then(|e| e.alternatives.pop_front());
    if let Some((url, source_node, source_mime)) = next {
        p.key.url = url.clone();
        p.key.source_node = source_node;
        p.key.source_mime = source_mime;
        p.offset = 0;
        p.range_start = 0;
        p.total = None;
        p.partial = false;
        p.bytes.clear();
        p.rounds = 0;
        if let Some(e) = state
            .native_media
            .borrow_mut()
            .resources
            .entries
            .get_mut(&p.node)
        {
            e.current_src = url;
        }
        start(state, p)
    } else {
        terminal(state, p.node, p.token, false);
        Ok(())
    }
}
fn terminal(state: &V8BindingState, node: NodeId, token: u64, success: bool) {
    if let Some(e) = state
        .native_media
        .borrow_mut()
        .resources
        .entries
        .get_mut(&node)
        .filter(|e| e.token == token)
    {
        e.stalled = false;
        e.network = if success { 1 } else { 3 };
        if success || e.key.source_node.is_none() {
            e.events
                .push_back((node, if success { "suspend" } else { "error" }));
        }
    }
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
fn content_total(headers: &[(String, String)]) -> Option<u64> {
    let value = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-range"))?
        .1
        .strip_prefix("bytes ")?;
    let (_, total) = value.split_once('/')?;
    total.parse().ok()
}
// RIFF chunk sizes are checked before indexing; media metadata does not
// require materializing the declared (potentially hours-long) PCM data chunk.
fn wav_duration(bytes: &[u8]) -> Option<f64> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return None;
    }
    let mut offset = 12usize;
    let mut rate = None;
    while offset.checked_add(8)? <= bytes.len() {
        let name = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        offset += 8;
        if name == b"fmt " {
            if size < 16 {
                return None;
            }
            let fmt = bytes.get(offset..offset.checked_add(16)?)?;
            let encoding = u16::from_le_bytes(fmt[0..2].try_into().ok()?);
            let channels = u16::from_le_bytes(fmt[2..4].try_into().ok()?);
            let samples = u32::from_le_bytes(fmt[4..8].try_into().ok()?);
            let byte_rate = u32::from_le_bytes(fmt[8..12].try_into().ok()?);
            let align = u16::from_le_bytes(fmt[12..14].try_into().ok()?);
            let bits = u16::from_le_bytes(fmt[14..16].try_into().ok()?);
            if !matches!(encoding, 1 | 3)
                || channels == 0
                || samples == 0
                || align == 0
                || bits == 0
                || u64::from(align) * u64::from(samples) != u64::from(byte_rate)
            {
                return None;
            }
            rate = Some(byte_rate);
        }
        if name == b"data" {
            return Some(size as f64 / f64::from(rate?));
        }
        offset = offset.checked_add(size)?.checked_add(size % 2)?;
    }
    None
}
pub(super) fn complete(
    state: &V8BindingState,
    mut p: Pending,
    result: Result<PlatformHostResult, V8BindingError>,
) -> Result<(), V8BindingError> {
    if !current(state, &p) {
        if let Ok(PlatformHostResult::HttpStarted { stream_id, .. })
        | Ok(PlatformHostResult::HttpChunk {
            stream_id,
            done: false,
            ..
        }) = result
        {
            submit_platform_operation(
                state,
                PlatformHostRequest::HttpAbort { stream_id },
                PendingPlatformOperation::WebSocketCommand,
            )?;
        }
        return Ok(());
    }
    match result {
        Ok(PlatformHostResult::HttpStarted {
            stream_id,
            status,
            headers,
            internal_response,
            response_type,
            url,
            ..
        }) => {
            p.origin_clean = p.key.cors.is_some()
                || (!matches!(
                    response_type,
                    Some(
                        crate::platform::FetchResponseType::Opaque
                            | crate::platform::FetchResponseType::OpaqueRedirect
                    )
                ) && Url::parse(&url)
                    .ok()
                    .zip(
                        state
                            .request_initiator_url()
                            .ok()
                            .and_then(|url| Url::parse(&url).ok()),
                    )
                    .is_some_and(|(url, origin)| {
                        (matches!(url.scheme(), "data" | "blob") || url.origin() == origin.origin())
                    }));
            p.stream = Some(stream_id);
            let (status, headers) = internal_response
                .map(|v| (v.status, v.headers))
                .unwrap_or((status, headers));
            if !(200..300).contains(&status) {
                return failed(state, p);
            }
            p.partial = status == 206;
            p.total = content_total(&headers);
        }
        Ok(PlatformHostResult::HttpChunk {
            encoded_body_size: _,
            stream_id,
            bytes,
            done,
        }) => {
            if p.stream != Some(stream_id) {
                abort(state, &mut p)?;
                terminal(state, p.node, p.token, false);
                return Err(error("media stream identity mismatch"));
            }
            let retained = state
                .pending_platform_operations
                .borrow()
                .values()
                .filter_map(|pending| {
                    if let PendingPlatformOperation::MediaResource(p) = pending {
                        Some(p.bytes.len())
                    } else {
                        None
                    }
                })
                .sum::<usize>();
            if bytes.len() > LIMIT.saturating_sub(p.bytes.len())
                || retained
                    .saturating_add(p.bytes.len())
                    .saturating_add(bytes.len())
                    > 32 * 1024 * 1024
            {
                return failed(state, p);
            }
            p.offset = p
                .offset
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| error("media offset exhausted"))?;
            p.bytes.extend_from_slice(&bytes);
            if !bytes.is_empty() {
                if let Some(entry) = state
                    .native_media
                    .borrow_mut()
                    .resources
                    .entries
                    .get_mut(&p.node)
                {
                    entry.progress_at = std::time::Instant::now();
                    entry.stalled = false;
                }
                queue(state, p.node, p.token, "progress");
            }
            if p.key.preload == "metadata"
                && !media_element::url_wants_data(state, p.node)
                && let Some(duration) = wav_duration(&p.bytes)
            {
                if let Some(e) = state
                    .native_media
                    .borrow_mut()
                    .resources
                    .entries
                    .get_mut(&p.node)
                    .filter(|e| e.token == p.token)
                {
                    e.duration = Some(duration);
                }
                queue(state, p.node, p.token, "loadedmetadata");
                // The UA may suspend preloading once metadata is available.
                // Keep the selected source while releasing live network custody.
                if done {
                    p.stream = None
                } else {
                    abort(state, &mut p)?;
                }
                terminal(state, p.node, p.token, true);
                return Ok(());
            }
            if done {
                p.stream = None;
                if !p.partial || p.total.is_none_or(|total| p.offset >= total) {
                    match native_platform::media_request(
                        state,
                        crate::media_capture::Request::UrlOpen {
                            bytes_base64: BASE64.encode(&p.bytes),
                        },
                    ) {
                        Ok(crate::media_capture::Response::UrlAsset(metadata)) => {
                            if media_element::url_decoded(state, p.node, &metadata, p.origin_clean)
                                .is_err()
                            {
                                return failed(state, p);
                            }
                            if let Some(e) = state
                                .native_media
                                .borrow_mut()
                                .resources
                                .entries
                                .get_mut(&p.node)
                            {
                                e.duration = metadata["duration"].as_f64();
                            }
                            queue(state, p.node, p.token, "loadedmetadata");
                            for name in ["loadeddata", "canplay", "canplaythrough"] {
                                queue(state, p.node, p.token, name);
                            }
                            terminal(state, p.node, p.token, true);
                            return Ok(());
                        }
                        _ => return failed(state, p),
                    }
                }
                if p.partial
                    && p.total.is_some_and(|total| p.offset < total)
                    && p.offset > p.range_start
                {
                    p.rounds += 1;
                    if p.rounds > 128 {
                        terminal(state, p.node, p.token, false);
                        return Ok(());
                    }
                    return start(state, p);
                }
                return failed(state, p);
            }
        }
        _ => return failed(state, p),
    }
    let stream_id = p.stream.ok_or_else(|| error("media body stream missing"))?;
    let (node, token) = (p.node, p.token);
    if let Err(e) = submit_platform_operation(
        state,
        PlatformHostRequest::HttpRead {
            stream_id,
            max_bytes: 64 * 1024,
        },
        PendingPlatformOperation::MediaResource(p),
    ) {
        terminal(state, node, token, false);
        let _ = submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        );
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_token_failure_preserves_fetch_and_allows_retry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<audio id=a src='https://media.test/a'></audio>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        let node = dom.borrow().find_by_id("a").unwrap();
        let token = state.native_media.borrow().resources.entries[&node].token;
        let ids = state
            .pending_platform_operations
            .borrow()
            .iter()
            .filter_map(|(id, p)| {
                matches!(p, PendingPlatformOperation::MediaResource(p) if p.node==node)
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        assert!(!ids.is_empty());
        let requests = host.requests.borrow().len();
        state.native_media.borrow_mut().resources.next = u64::MAX;
        dom.borrow_mut()
            .set_attribute(node, "src", "https://media.test/b")
            .unwrap();
        assert!(poll(scope, &state).is_err());
        assert_eq!(
            state.native_media.borrow().resources.entries[&node].token,
            token
        );
        assert_eq!(host.requests.borrow().len(), requests);
        assert!(host.cancelled.borrow().is_empty());
        for id in &ids {
            assert!(state.pending_platform_operations.borrow().contains_key(id));
        }
        // Repairing admission must retry the same authored selection, not need
        // another mutation or silently consume its cursor on the failed attempt.
        state.native_media.borrow_mut().resources.next = token;
        poll(scope, &state).unwrap();
        assert_eq!(
            state.native_media.borrow().resources.entries[&node].key.url,
            "https://media.test/b"
        );
        assert_eq!(
            state.native_media.borrow().resources.entries[&node].token,
            token + 1
        );
        for id in ids {
            assert!(host.cancelled.borrow().contains(&id));
        }
    }
    #[test]
    fn full_forced_inventory_rejects_new_slot_but_reuses_existing_slot() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let html = format!(
            "<audio id=a src='https://media.test/a'></audio>{}",
            (0..256)
                .map(|i| format!("<audio id=b{i}></audio>"))
                .collect::<String>()
        );
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(&html).into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        let node = dom.borrow().find_by_id("a").unwrap();
        let token = state.native_media.borrow().resources.entries[&node].token;
        for i in 0..256 {
            state
                .native_media
                .borrow_mut()
                .resources
                .forced
                .insert(dom.borrow().find_by_id(&format!("b{i}")).unwrap());
        }
        assert!(request_load(&state, node).is_err());
        assert_eq!(
            state.native_media.borrow().resources.entries[&node].token,
            token
        );
        assert!(host.cancelled.borrow().is_empty());
        let other = dom.borrow().find_by_id("b0").unwrap();
        state
            .native_media
            .borrow_mut()
            .resources
            .forced
            .remove(&other);
        state
            .native_media
            .borrow_mut()
            .resources
            .forced
            .insert(node);
        assert!(request_load(&state, node).is_ok());
        assert_eq!(state.native_media.borrow().resources.forced.len(), 256);
        assert!(!host.cancelled.borrow().is_empty());
    }
    #[test]
    fn load_admission_failure_preserves_pending_fetch_and_host_custody() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<audio id=a src='https://media.test/a'></audio>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        let node = dom.borrow().find_by_id("a").unwrap();
        let token = state.native_media.borrow().resources.entries[&node].token;
        let ids = state
            .pending_platform_operations
            .borrow()
            .iter()
            .filter_map(|(id, pending)| {
                matches!(pending, PendingPlatformOperation::MediaResource(p) if p.node == node)
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        assert!(
            !ids.is_empty(),
            "fixture must own an actual pending media fetch"
        );
        let requests = host.requests.borrow().len();
        media_element::exhaust_reset_session_for_test(&state, node);
        assert!(request_load(&state, node).is_err());
        assert_eq!(
            state.native_media.borrow().resources.entries[&node].token,
            token
        );
        for id in ids {
            assert!(state.pending_platform_operations.borrow().contains_key(&id));
            assert!(!state.cancelled_platform_operations.borrow().contains(&id));
        }
        assert!(host.cancelled.borrow().is_empty());
        assert_eq!(host.requests.borrow().len(), requests);
    }
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
            .find_map(|(id, p)| {
                matches!(p, PendingPlatformOperation::MediaResource(_)).then_some(*id)
            })
            .unwrap();
        let Some(PendingPlatformOperation::MediaResource(p)) =
            state.pending_platform_operations.borrow_mut().remove(&id)
        else {
            unreachable!()
        };
        p
    }
    fn started(
        stream_id: u64,
        status: u16,
        total: Option<&str>,
    ) -> Result<PlatformHostResult, V8BindingError> {
        Ok(PlatformHostResult::HttpStarted {
            timing: None,
            internal_response: None,
            response_type: None,
            upload: None,
            cors_filtered: false,
            redirected: false,
            stream_id,
            status,
            url: "https://media.test/a".into(),
            headers: total
                .map(|value| vec![("content-range".into(), value.into())])
                .unwrap_or_default(),
        })
    }
    #[test]
    fn media_primary_and_source_alternatives_use_document_output_encoding() {
        super::super::tests::initialize_v8();
        for (encoding, query) in [
            ("windows-1252", "%E5"),
            ("windows-1251", "%26%23229%3B"),
            ("UTF-16LE", "%C3%A5"),
        ] {
            let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<base href='/assets/'><audio id=a src='audio?å'></audio><video id=v><source src='first?å'><source src='second?å'></video>").into_runtime().unwrap()));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state =
                install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
            state.commit_location("https://example.test/page").unwrap();
            dom.borrow_mut()
                .arena_mut()
                .get_mut(state.document_root())
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .encoding = encoding.into();
            let audio = dom.borrow().find_by_id("a").unwrap();
            let video = dom.borrow().find_by_id("v").unwrap();
            assert_eq!(
                select(&state, audio).unwrap().unwrap().url,
                format!("https://example.test/assets/audio?{query}")
            );
            let key = select(&state, video).unwrap().unwrap();
            assert_eq!(
                key.url,
                format!("https://example.test/assets/first?{query}")
            );
            assert_eq!(
                key.alternatives[0].0,
                format!("https://example.test/assets/second?{query}")
            );
        }
    }
    #[test]
    fn wav_metadata_checks_chunks_without_allocating_declared_pcm_body() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&1_000_036u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&1_000_000u32.to_le_bytes());
        assert_eq!(wav_duration(&bytes), Some(62.5));
        assert_eq!(wav_duration(&bytes[..43]), None);
        bytes[32] = 0;
        assert_eq!(wav_duration(&bytes), None);
    }
    #[test]
    fn media_source_range_continues_at_received_bytes_and_late_completion_is_cancelled() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><audio id=a src='https://media.test/a'></audio>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        assert!(host.requests.borrow().iter().any(|request|matches!(request,PlatformHostRequest::MediaStart{request,destination,cors_enabled:false}if destination=="audio"&&request.headers.contains(&("range".into(),"bytes=0-".into()))&&!request.headers.iter().any(|(name,_)|name=="accept-encoding"))));
        complete(&state, take(&state), started(1, 206, Some("bytes 0-1/100"))).unwrap();
        complete(
            &state,
            take(&state),
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 1,
                bytes: vec![0; 7],
                done: false,
            }),
        )
        .unwrap();
        complete(
            &state,
            take(&state),
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 1,
                bytes: Vec::new(),
                done: true,
            }),
        )
        .unwrap();
        assert!(host.requests.borrow().iter().any(|request|matches!(request,PlatformHostRequest::MediaStart{request,..}if request.headers.contains(&("range".into(),"bytes=7-".into())))));
        let late = take(&state);
        let node = dom.borrow().find_by_id("a").unwrap();
        dom.borrow_mut()
            .set_attribute(node, "src", "https://media.test/b")
            .unwrap();
        poll(scope, &state).unwrap();
        complete(&state, late, started(2, 200, None)).unwrap();
        assert!(
            host.requests
                .borrow()
                .iter()
                .any(|request| matches!(request, PlatformHostRequest::HttpAbort { stream_id: 2 }))
        );
        clear(&state).unwrap();
        assert!(!pending(&state));
        assert!(!host.cancelled.borrow().is_empty());
    }
    #[test]
    fn detached_src_selection_stall_and_source_errors_use_native_tasks() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let source=v8::String::new(scope,"globalThis.audio=document.createElement('audio');globalThis.stalled=false;audio.onstalled=()=>stalled=audio.matches(':stalled');audio.src='https://media.test/a';").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        poll(scope, &state).unwrap();
        assert!(
            host.requests
                .borrow()
                .iter()
                .any(|request| matches!(request, PlatformHostRequest::MediaStart { .. }))
        );
        let node = state
            .native_media
            .borrow()
            .resources
            .entries
            .keys()
            .next()
            .copied()
            .unwrap();
        state
            .native_media
            .borrow_mut()
            .resources
            .entries
            .get_mut(&node)
            .unwrap()
            .progress_at = std::time::Instant::now() - std::time::Duration::from_secs(4);
        poll(scope, &state).unwrap();
        let check = v8::String::new(scope, "stalled").unwrap();
        assert!(
            v8::Script::compile(scope, check, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let parent = dom.borrow_mut().create_element("video");
        let child = dom.borrow_mut().create_element("source");
        dom.borrow_mut()
            .set_attribute(child, "src", "https://media.test/denied")
            .unwrap();
        dom.borrow_mut().append_child(parent, child).unwrap();
        request_load(&state, parent).unwrap();
        poll(scope, &state).unwrap();
        let id = state
            .pending_platform_operations
            .borrow()
            .iter()
            .find_map(|(id, p)| {
                matches!(p,PendingPlatformOperation::MediaResource(p)if p.node==parent)
                    .then_some(*id)
            })
            .unwrap();
        let Some(PendingPlatformOperation::MediaResource(p)) =
            state.pending_platform_operations.borrow_mut().remove(&id)
        else {
            panic!()
        };
        complete(&state, p, Err(error("policy denied"))).unwrap();
        assert!(
            state.native_media.borrow().resources.entries[&parent]
                .events
                .contains(&(child, "error"))
        );
        assert!(
            !state.native_media.borrow().resources.entries[&parent]
                .events
                .contains(&(parent, "error"))
        );
    }
    #[test]
    fn media_stop_cancels_pending_header_admission_without_retiring_document() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><video src='https://media.test/v'></video>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        assert!(pending(&state));
        state.stop_pending_document_load().unwrap();
        assert!(!pending(&state));
        assert!(!state.execution_retired());
        let count = host.requests.borrow().len();
        poll(scope, &state).unwrap();
        assert_eq!(host.requests.borrow().len(), count);
        assert!(!host.cancelled.borrow().is_empty());
    }
}

pub(super) fn release_asset(state: &V8BindingState, id: String) {
    match native_platform::media_request(
        state,
        crate::media_capture::Request::UrlRelease { id: id.clone() },
    ) {
        Ok(crate::media_capture::Response::Stopped) => {
            state
                .native_media
                .borrow_mut()
                .resources
                .releases
                .remove(&id);
        }
        _ => {
            state
                .native_media
                .borrow_mut()
                .resources
                .releases
                .insert(id);
        }
    }
}

fn supported_type(state: &V8BindingState, mime: &str) -> bool {
    if let Some(value) = state.native_media.borrow().resources.types.get(mime) {
        return *value;
    }
    let supported = matches!(
        native_platform::media_request(
            state,
            crate::media_capture::Request::UrlCanPlay { mime: mime.into() }
        ),
        Ok(crate::media_capture::Response::UrlSupport(true))
    );
    let mut media = state.native_media.borrow_mut();
    if media.resources.types.len() < 128 {
        media.resources.types.insert(mime.into(), supported);
    }
    supported
}

pub(super) fn selection_changed(state: &V8BindingState, node: NodeId) {
    let disconnected = !state.dom.borrow().is_connected(node).unwrap_or(false);
    let mut media = state.native_media.borrow_mut();
    media.resources.cursor = None;
    if disconnected && media.resources.detached.len() < 256 {
        media.resources.detached.insert(node);
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn resource_event_listener_suffix_survives_allocation_failure_and_source_cancellation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<video id=v></video>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let node = dom.borrow().find_by_id("v").unwrap();
        eval(
            scope,
            "globalThis.log=[];const video=document.getElementById('v');video.addEventListener('loadeddata',e=>{log.push('first');globalThis.original=e},{once:true});video.addEventListener('loadeddata',e=>log.push(e===original?'second':'wrong'))",
        );
        let event = create_event_object(scope, &state, "loadeddata", false, false).unwrap();
        let delivery =
            retained_dom_event::Event::new(scope, &state, node, "loadeddata", event).unwrap();
        state.native_media.borrow_mut().resources.delivery = Some(Delivery {
            epoch: state.document_stream_epoch(),
            event: delivery,
        });
        native_platform::publication::fail_after_next_callback();
        assert!(poll_delivery(scope, &state).is_err());
        assert!(eval(scope, "log.join()==='first'").is_true());
        cancel(&state, node).unwrap();
        assert!(pending(&state));
        state.set_execution_suspended(true);
        assert_eq!(poll_delivery(scope, &state).unwrap(), 0);
        state.set_execution_suspended(false);
        assert_eq!(poll_delivery(scope, &state).unwrap(), 1);
        assert!(eval(scope, "log.join()==='first,second'").is_true());
        assert!(!pending(&state));
    }
}
