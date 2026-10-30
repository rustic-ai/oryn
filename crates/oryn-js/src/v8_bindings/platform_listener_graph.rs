//! JS targets trace listeners. Native dispatch records carry only weak handles,
//! so a callback closing over its own target is an ordinary collectible JS cycle.
use super::*;

/// Reverse native references between materialized platform wrappers. These IDs
/// are not GC roots: the private JavaScript edges below retain the objects.
#[derive(Default)]
pub(super) struct Edges {
    outgoing: BTreeMap<u64, BTreeSet<u64>>,
    incoming: BTreeMap<u64, BTreeSet<u64>>,
    activity_candidates: BTreeSet<u64>,
    pub(super) media_candidates: BTreeSet<u64>,
    pub(super) response_candidates: BTreeSet<u64>,
    pub(super) native_lease_candidates: BTreeSet<u64>,
    pub(super) unpublished_candidates: BTreeMap<u64,Rc<Cell<owned_unpublished_cleanup::Status>>>,
    pub(super) unpublished_cursor: Option<u64>,
    pub(super) unpublished_ready: Rc<Cell<usize>>,
}
impl Edges {
    pub(super) fn discard_unpublished(&mut self,id:u64) {
        owned_unpublished_cleanup::retire(self,id);
        self.replace(id,BTreeSet::new());
        if let Some(owners)=self.incoming.remove(&id) {
            for owner in owners {
                if let Some(targets)=self.outgoing.get_mut(&owner) {
                    targets.remove(&id);
                    if targets.is_empty() {self.outgoing.remove(&owner);}
                }
            }
        }
        self.activity_candidates.remove(&id);
        self.media_candidates.remove(&id);
        self.response_candidates.remove(&id);
        self.native_lease_candidates.remove(&id);
    }
    pub(super) fn registered(&mut self, id: u64, record: &PlatformObjectRecord) {
        for (set, selected) in [
            (
                &mut self.media_candidates,
                matches!(
                    record,
                    PlatformObjectRecord::TextTrackCue(_)
                        | PlatformObjectRecord::TextTrackList(_)
                        | PlatformObjectRecord::MediaTrack(_)
                        | PlatformObjectRecord::MediaTrackList { .. }
                ),
            ),
            (
                &mut self.response_candidates,
                matches!(record, PlatformObjectRecord::Response(_)),
            ),
            (
                &mut self.native_lease_candidates,
                matches!(
                    record,
                    PlatformObjectRecord::SubmitButtonBehavior(_)
                        | PlatformObjectRecord::MediaTrackList { .. }
                ),
            ),
        ] {
            if selected {
                set.insert(id);
            } else {
                set.remove(&id);
            }
        }
        if matches!(
            record,
            PlatformObjectRecord::XMLHttpRequest(_)
                | PlatformObjectRecord::IdbTransaction(_)
                | PlatformObjectRecord::EventTarget
                | PlatformObjectRecord::MediaQueryList { .. }
                | PlatformObjectRecord::WebSocket(_)
                | PlatformObjectRecord::Worker(_)
                | PlatformObjectRecord::MessagePort(_)
                | PlatformObjectRecord::BroadcastChannel(_)
        ) {
            self.activity_candidates.insert(id);
        } else {
            self.activity_candidates.remove(&id);
        }
    }
    pub(super) fn add(&mut self, owner: u64, target: u64) {
        self.outgoing.entry(owner).or_default().insert(target);
        self.incoming.entry(target).or_default().insert(owner);
    }
    fn replace(&mut self, owner: u64, targets: BTreeSet<u64>) {
        let old = self.outgoing.remove(&owner).unwrap_or_default();
        for target in old.difference(&targets) {
            if let Some(parents) = self.incoming.get_mut(target) {
                parents.remove(&owner);
                if parents.is_empty() {
                    self.incoming.remove(target);
                }
            }
        }
        for target in targets.difference(&old) {
            self.incoming.entry(*target).or_default().insert(owner);
        }
        if !targets.is_empty() {
            self.outgoing.insert(owner, targets);
        }
    }
}
#[derive(Clone, Default)]
pub(super) struct Callback(Option<Rc<v8::Weak<v8::Object>>>);
impl Callback {
    pub(super) fn new(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Self {
        Self(
            v8::Local::<v8::Object>::try_from(value)
                .ok()
                .map(|object| Rc::new(v8::Weak::new(scope, object))),
        )
    }
    pub(super) fn local<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.0.as_ref()?.to_local(scope).map(Into::into)
    }
    pub(super) fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.local(scope).unwrap_or_else(|| v8::null(scope).into())
    }
    pub(super) fn equals(&self, scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> bool {
        self.local(scope)
            .is_some_and(|callback| callback.strict_equals(value))
    }
}
/// Object payloads are traced by their owning wrapper; primitives cannot
/// introduce a callback/target cycle and may remain native persistent values.
#[derive(Clone)]
pub(super) enum Value {
    Object(Callback),
    Primitive(v8::Global<v8::Value>),
}
impl Value {
    pub(super) fn new(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Self {
        if value.is_object() {
            Self::Object(Callback::new(scope, value))
        } else {
            Self::Primitive(v8::Global::new(scope, value))
        }
    }
    pub(super) fn local<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Object(callback) => callback.local(scope),
            Self::Primitive(value) => Some(v8::Local::new(scope, value)),
        }
    }
    pub(super) fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.local(scope)
            .unwrap_or_else(|| v8::undefined(scope).into())
    }
}
fn refresh_edges(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    reclaim(state);
    let target = if id == global_events::GLOBAL_TARGET {
        state.global_object.to_local(scope)
    } else {
        state
            .platform_wrappers
            .borrow()
            .get(&id)
            .and_then(|wrapper| wrapper.to_local(scope))
    };
    let Some(target) = target else {
        return Ok(());
    };
    let related = {
        let objects = state.platform_objects.borrow();
        match objects.get(&id) {
            Some(PlatformObjectRecord::AbortSignal(signal)) => signal.followers.clone(),
            Some(PlatformObjectRecord::AbortController { signal_id }) => vec![*signal_id],
            Some(PlatformObjectRecord::Request(request)) => {
                vec![request.signal_id, request.headers_id]
            }
            Some(PlatformObjectRecord::MessageChannel { port1_id, port2_id }) => {
                vec![*port1_id, *port2_id]
            }
            Some(PlatformObjectRecord::Worker(client)) => {
                client.borrow().port.into_iter().collect()
            }
            Some(PlatformObjectRecord::AudioWorklet(object)) => audio_worklet::related(object),
            Some(PlatformObjectRecord::XMLHttpRequest(xhr)) => vec![xhr.upload_id],
            Some(PlatformObjectRecord::IdbRequest(request)) => request
                .transaction_id
                .into_iter()
                .chain(request.source_id)
                .collect(),
            Some(PlatformObjectRecord::IdbTransaction(transaction)) => {
                vec![transaction.database_id]
            }
            Some(PlatformObjectRecord::IdbObjectStore(store)) => vec![store.transaction_id],
            Some(PlatformObjectRecord::IdbIndex(index)) => vec![index.object_store_id],
            Some(PlatformObjectRecord::IdbCursor(cursor)) => {
                vec![cursor.request_id, cursor.source_id]
            }
            _ => Vec::new(),
        }
    };
    state
        .platform_edges
        .borrow_mut()
        .replace(id, related.into_iter().collect());
    let mut values = Vec::new();
    {
        let listeners = state.platform_listeners.borrow();
        for ((target, _), entries) in listeners.iter() {
            if *target == id {
                for entry in entries {
                    if let Some(callback) = entry.callback.local(scope) {
                        values.push(callback);
                    }
                }
            }
        }
        let handlers = state.platform_handlers.borrow();
        for ((target, _), handler) in handlers.iter() {
            if *target == id
                && let Some(callback) = handler.local(scope)
            {
                values.push(callback);
            }
        }
    }
    {
        let objects = state.platform_objects.borrow();
        let mut related = Vec::new();
        match objects.get(&id) {
            Some(PlatformObjectRecord::AbortSignal(signal)) => {
                if let Some(value) = signal.reason.as_ref().and_then(|value| value.local(scope)) {
                    values.push(value);
                }
                if !signal.aborted {
                    for follower in &signal.followers {
                        if state
                            .platform_listeners
                            .borrow()
                            .get(&(*follower, "abort".into()))
                            .is_some_and(|listeners| !listeners.is_empty())
                            || state
                                .platform_handlers
                                .borrow()
                                .contains_key(&(*follower, "abort".into()))
                        {
                            related.push(*follower);
                        }
                    }
                }
            }
            Some(PlatformObjectRecord::AbortController { signal_id }) => related.push(*signal_id),
            Some(PlatformObjectRecord::Request(request)) => {
                related.extend([request.signal_id, request.headers_id])
            }
            Some(PlatformObjectRecord::MessageChannel { port1_id, port2_id }) => {
                related.extend([*port1_id, *port2_id])
            }
            Some(PlatformObjectRecord::Worker(client)) => related.extend(client.borrow().port),
            Some(PlatformObjectRecord::AudioWorklet(object)) => {
                related.extend(audio_worklet::related(object))
            }
            Some(PlatformObjectRecord::XMLHttpRequest(xhr)) => {
                if let Some(value) = xhr
                    .response_object
                    .as_ref()
                    .and_then(|value| value.local(scope))
                {
                    values.push(value);
                }
                related.push(xhr.upload_id);
            }
            Some(PlatformObjectRecord::IdbRequest(request)) => {
                if let Some(value) = request.result.as_ref().and_then(|value| value.local(scope)) {
                    values.push(value);
                }
                related.extend(request.transaction_id);
                related.extend(request.source_id);
            }
            Some(PlatformObjectRecord::IdbTransaction(transaction)) => {
                related.push(transaction.database_id)
            }
            Some(PlatformObjectRecord::IdbObjectStore(store)) => related.push(store.transaction_id),
            Some(PlatformObjectRecord::IdbIndex(index)) => related.push(index.object_store_id),
            Some(PlatformObjectRecord::IdbCursor(cursor)) => {
                related.extend([cursor.request_id, cursor.source_id]);
                for value in [
                    &cursor.key_value,
                    &cursor.primary_key_value,
                    &cursor.record_value,
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(value) = value.local(scope) {
                        values.push(value);
                    }
                }
            }
            Some(PlatformObjectRecord::IdbRecord {
                key,
                primary_key,
                value,
            }) => {
                for value in [key, primary_key, value] {
                    if let Some(value) = value.local(scope) {
                        values.push(value);
                    }
                }
            }
            _ => {}
        }
        for related in related {
            if let Some(wrapper) = state
                .platform_wrappers
                .borrow()
                .get(&related)
                .and_then(|wrapper| wrapper.to_local(scope))
            {
                values.push(wrapper.into());
            }
            if let Some(PlatformObjectRecord::AbortSignal(signal)) = objects.get(&related)
                && let Some(value) = signal.reason.as_ref().and_then(|value| value.local(scope))
            {
                values.push(value);
            }
        }
    }
    let foreign = {
        let objects = state.platform_objects.borrow();
        match objects.get(&id) {
            Some(PlatformObjectRecord::AbortSignal(signal)) => {
                let mut links = signal.foreign_sources.clone();
                if !signal.aborted {
                    links.extend(
                        signal
                            .foreign_followers
                            .iter()
                            .filter(|link| {
                                link.owner.upgrade().is_some_and(|owner| {
                                    owner
                                        .platform_listeners
                                        .borrow()
                                        .get(&(link.id, "abort".into()))
                                        .is_some_and(|listeners| !listeners.is_empty())
                                        || owner
                                            .platform_handlers
                                            .borrow()
                                            .contains_key(&(link.id, "abort".into()))
                                })
                            })
                            .cloned(),
                    );
                }
                links
            }
            _ => Vec::new(),
        }
    };
    for link in foreign {
        if let Some(owner) = link.owner.upgrade()
            && let Some(wrapper) = owner
                .platform_wrappers
                .borrow()
                .get(&link.id)
                .and_then(|wrapper| wrapper.to_local(scope))
        {
            values.push(wrapper.into());
        }
    }
    let array = v8::Array::new_with_elements(scope, &values);
    let name = v8::String::new(scope, "oryn.platform.listener.edges")
        .ok_or(V8BindingError::Allocation("listener graph key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    target
        .set_private(scope, key, array.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
pub(super) fn materialized(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    if stream_graph::owns(state, id) {
        stream_graph::refresh(scope, state)?;
    }
    refresh_edges(scope, state, id)?;
    let parents = state
        .platform_edges
        .borrow()
        .incoming
        .get(&id)
        .cloned()
        .unwrap_or_default();
    for parent in parents {
        refresh_edges(scope, state, parent)?;
    }
    let owners = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for owner in owners {
        if owner.instance_id == state.instance_id {
            continue;
        }
        let parents = owner
            .platform_objects
            .borrow()
            .iter()
            .filter_map(|(parent, record)| match record {
                PlatformObjectRecord::AbortSignal(signal)
                    if signal
                        .foreign_followers
                        .iter()
                        .chain(&signal.foreign_sources)
                        .any(|link| {
                            link.id == id
                                && link
                                    .owner
                                    .upgrade()
                                    .is_some_and(|target| target.instance_id == state.instance_id)
                        }) =>
                {
                    Some(*parent)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for parent in parents {
            refresh_edges(scope, &owner, parent)?;
        }
    }
    text_tracks::refresh_gc_edges(scope, state)?;
    reconcile_activity(scope, state);
    Ok(())
}
pub(super) fn refresh(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
) -> Result<(), V8BindingError> {
    materialized(scope, state, id)
}
pub(super) fn refresh_all(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    stream_graph::refresh(scope, state)?;
    let mut ids = state
        .platform_wrappers
        .borrow()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    ids.push(global_events::GLOBAL_TARGET);
    for id in ids {
        refresh_edges(scope, state, id)?;
    }
    reconcile_activity(scope, state);
    Ok(())
}
pub(super) fn handler_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    event_type: &str,
) -> v8::Local<'s, v8::Value> {
    state
        .platform_handlers
        .borrow()
        .get(&(id, event_type.into()))
        .and_then(|callback| callback.local(scope))
        .unwrap_or_else(|| v8::null(scope).into())
}
pub(super) fn handler_set(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    event_type: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = (id, event_type.to_owned());
    let previous = state.platform_listeners.borrow().get(&key).cloned();
    let registration = if value.is_object()
        && !previous
            .as_ref()
            .is_some_and(|listeners| listeners.iter().any(|listener| listener.global_handler))
    {
        let registration = state.next_event_listener_id.get();
        state.next_event_listener_id.set(
            registration
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("event handler identity"))?,
        );
        Some(registration)
    } else {
        None
    };
    let callback = Callback::new(scope, value);
    let old = if value.is_object() {
        state
            .platform_handlers
            .borrow_mut()
            .insert(key.clone(), callback.clone())
    } else {
        state.platform_handlers.borrow_mut().remove(&key)
    };
    {
        let mut map = state.platform_listeners.borrow_mut();
        let listeners = map.entry(key.clone()).or_default();
        if !value.is_object() {
            listeners.retain(|listener| !listener.global_handler);
        } else if let Some(listener) = listeners
            .iter_mut()
            .find(|listener| listener.global_handler)
        {
            listener.callback = callback;
        } else {
            listeners.push(PlatformEventListener {
                registration_id: registration.unwrap(),
                global_handler: true,
                callback,
                capture: false,
                once: false,
                passive: false,
                signal: None,
            });
        }
    }
    if let Err(error) = refresh(scope, state, id) {
        {
            let mut handlers = state.platform_handlers.borrow_mut();
            if let Some(old) = old {
                handlers.insert(key.clone(), old);
            } else {
                handlers.remove(&key);
            }
        }
        {
            let mut listeners = state.platform_listeners.borrow_mut();
            if let Some(previous) = previous {
                listeners.insert(key, previous);
            } else {
                listeners.remove(&key);
            }
        }
        let _ = refresh(scope, state, id);
        return Err(error);
    }
    Ok(())
}
pub(super) fn watch(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    object: v8::Local<v8::Object>,
) {
    reclaim(state);
    let serial = state.platform_listener_watch_serial.get();
    state.platform_listener_watch_serial.set(
        serial
            .checked_add(1)
            .expect("platform listener watcher identity exhausted"),
    );
    let releases = state.platform_listener_releases.clone();
    let watcher = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            releases.borrow_mut().push((id, serial));
        }),
    );
    state
        .platform_listener_watchers
        .borrow_mut()
        .insert(id, (serial, watcher));
}
pub(super) fn reclaim(state: &V8BindingState) {
    owned_unpublished_cleanup::drain(state);
    let releases = std::mem::take(&mut *state.platform_listener_releases.borrow_mut());
    for (id, serial) in releases {
        if state
            .platform_listener_watchers
            .borrow()
            .get(&id)
            .is_some_and(|(current, _)| *current == serial)
        {
            if !owned_unpublished_cleanup::weak_released(state,id) {
                state.platform_listener_releases.borrow_mut().push((id,serial));
                continue;
            }
            let cleanup = {
                let Ok(mut records) = state.platform_objects.try_borrow_mut() else {
                    state
                        .platform_listener_releases
                        .borrow_mut()
                        .push((id, serial));
                    continue;
                };
                match records.get_mut(&id) {
                    Some(PlatformObjectRecord::IdbTransaction(transaction))
                        if transaction.settled =>
                    {
                        Some((
                            std::mem::take(&mut transaction.object_stores),
                            transaction.scope_custody.take(),
                        ))
                    }
                    _ => None,
                }
            };
            state.platform_listener_watchers.borrow_mut().remove(&id);
            state
                .platform_edges
                .borrow_mut()
                .replace(id, BTreeSet::new());
            state
                .platform_listeners
                .borrow_mut()
                .retain(|(target, _), _| *target != id);
            state
                .platform_handlers
                .borrow_mut()
                .retain(|(target, _), _| *target != id);
            native_platform::release_collected_status(state, id);
            web_crypto::release_collected_key(state, id);
            submit_behavior::release_collected(state, id);
            message_ports::release_collected(state, id);
            web_animations::release_collected(state, id);
            text_tracks::release_collected(state, id);
            // Encoded Blob bytes outlive their deferred delivery task. Their
            // admitted native custody ends only with the exact wrapper release.
            if state.canvas_blob_custody.borrow().contains_key(&id) {
                state.platform_objects.borrow_mut().remove(&id);
                state.canvas_blob_custody.borrow_mut().remove(&id);
                state.platform_wrappers.borrow_mut().remove(&id);
                state.platform_edges.borrow_mut().discard_unpublished(id);
            }
            if matches!(state.platform_objects.borrow().get(&id), Some(PlatformObjectRecord::ImageBitmap(_) | PlatformObjectRecord::CanvasPattern(_))) {
                state.platform_objects.borrow_mut().remove(&id);
                state.image_bitmap_custody.borrow_mut().remove(&id);
                state.platform_wrappers.borrow_mut().remove(&id);
                state.platform_edges.borrow_mut().discard_unpublished(id);
            }
            // TimeRanges is immutable native leaf storage. Its wrapper finalizer
            // releases the snapshot itself, not just listeners and reverse edges.
            if matches!(state.platform_objects.borrow().get(&id), Some(PlatformObjectRecord::TimeRanges(_))) {
                state.platform_objects.borrow_mut().remove(&id);
                state.platform_wrappers.borrow_mut().remove(&id);
            }
            if let Some((stores, custody)) = cleanup {
                drop(stores);
                drop(custody);
            }
        }
    }
}

/// Native activity is an explicit lifetime root, not ownership of a callback.
/// Once requests/tasks/source capabilities retire, the target/listener JS graph
/// can be collected together even when the callback closes over the target.
pub(super) fn reconcile_activity(scope: &mut v8::PinScope, state: &V8BindingState) {
    reclaim(state);
    if state.execution_retired() {
        state.platform_listener_roots.borrow_mut().clear();
        return;
    }
    let mut active = text_tracks::active_platforms(state);
    let pending_target = |pending: &PendingPlatformOperation| match pending {
        PendingPlatformOperation::XhrStart { xhr_id, .. }
        | PendingPlatformOperation::XhrRead { xhr_id, .. } => Some(*xhr_id),
        PendingPlatformOperation::WebSocketOpen { websocket_id }
        | PendingPlatformOperation::WebSocketClose { websocket_id, .. } => Some(*websocket_id),
        PendingPlatformOperation::IndexedDb { request_id, .. } => Some(*request_id),
        _ => None,
    };
    for pending in state.pending_platform_operations.borrow().values() {
        active.extend(pending_target(pending));
    }
    for queued in state.queued_idb_operations.borrow().iter() {
        active.extend(pending_target(&queued.pending));
    }
    active.extend(idb_delivery::roots(state));
    for task in state.deferred_platform_tasks.borrow().iter() {
        match task {
            DeferredPlatformTask::PortMessage { port_id } => {
                active.insert(*port_id);
            }
            DeferredPlatformTask::AbortIdbRequest { request_id }
            | DeferredPlatformTask::FailIdbOpen { request_id } => {
                active.insert(*request_id);
            }
            DeferredPlatformTask::Event { target_id, .. } => {
                active.insert(*target_id);
            }
            DeferredPlatformTask::FinalizeIdbOpen {
                request_id,
                database_id,
                transaction_id,
            } => {
                active.extend([*request_id, *database_id, *transaction_id]);
            }
            DeferredPlatformTask::CompleteBlobXhr { xhr_id, .. } => {
                active.insert(*xhr_id);
            }
            DeferredPlatformTask::FinalizeIdbTransaction { transaction_id, .. } => {
                active.insert(*transaction_id);
            }
            _ => {}
        }
    }
    {
        let listeners = state.platform_listeners.borrow();
        let handlers = state.platform_handlers.borrow();
        let objects = state.platform_objects.borrow();
        let listens = |id: u64| {
            listeners
                .iter()
                .any(|((target, _), entries)| *target == id && !entries.is_empty())
                || handlers.keys().any(|(target, _)| *target == id)
        };
        for id in &state.platform_edges.borrow().activity_candidates {
            let Some(object) = objects.get(id) else {
                continue;
            };
            match object {
                PlatformObjectRecord::XMLHttpRequest(xhr) if xhr.send_invoked => {
                    active.extend([*id, xhr.upload_id]);
                }
                PlatformObjectRecord::IdbTransaction(transaction) if !transaction.settled => {
                    active.extend([*id, transaction.database_id]);
                    active.extend(transaction.request_queue.iter().copied());
                }
                PlatformObjectRecord::EventTarget
                    if native_platform::status_listener_active(state, *id) =>
                {
                    active.insert(*id);
                }
                PlatformObjectRecord::MediaQueryList { .. } if listens(*id) => {
                    active.insert(*id);
                }
                PlatformObjectRecord::WebSocket(socket)
                    if socket.ready_state < 3 && listens(*id) =>
                {
                    active.insert(*id);
                }
                PlatformObjectRecord::Worker(client)
                    if listens(*id) && workers::listener_target_active(&client.borrow()) =>
                {
                    active.insert(*id);
                }
                PlatformObjectRecord::MessagePort(port)
                    if ["message", "messageerror"].iter().any(|event| {
                        listeners
                            .get(&(*id, (*event).into()))
                            .is_some_and(|entries| !entries.is_empty())
                            || handlers.contains_key(&(*id, (*event).into()))
                    }) && message_ports::listener_target_active(port) =>
                {
                    active.insert(*id);
                }
                PlatformObjectRecord::BroadcastChannel(channel)
                    if listens(*id)
                        && broadcast_channels::listener_target_active(&channel.borrow()) =>
                {
                    active.insert(*id);
                }
                _ => {}
            }
        }
        // A pending request also traces its transaction and database, whose
        // bubbling error/abort listeners are observable without a JS reference.
        let requests = active.iter().copied().collect::<Vec<_>>();
        for id in requests {
            if let Some(PlatformObjectRecord::IdbRequest(request)) = objects.get(&id)
                && let Some(transaction) = request.transaction_id
            {
                active.insert(transaction);
                if let Some(PlatformObjectRecord::IdbTransaction(record)) =
                    objects.get(&transaction)
                {
                    active.insert(record.database_id);
                }
            }
        }
    }
    let mut roots = state.platform_listener_roots.borrow_mut();
    roots.retain(|id, _| active.contains(id));
    let wrappers = state.platform_wrappers.borrow();
    for id in active {
        if !roots.contains_key(&id)
            && let Some(wrapper) = wrappers
                .get(&id)
                .and_then(|wrapper| wrapper.to_local(scope))
        {
            roots.insert(id, v8::Global::new(scope, wrapper));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> String {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope)
    }
    #[test]
    fn collected_idb_custody_waits_for_record_borrow_then_releases_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(234), 1).unwrap();
        let ledger = std::sync::Arc::new(crate::idb_owned_admission::Ledger::default());
        let owned = crate::idb_owned_admission::Scope::copy_names(
            &ledger,
            &["store".into()],
            1024 * 1024,
            0,
        )
        .unwrap();
        let (stores, custody) = owned.into_parts();
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                request_queue: VecDeque::new(),
                abort_event_queued: false,
                settled: true,
                database_id: 0,
                request_id: None,
                mode: "readonly".into(),
                durability: "default".into(),
                object_stores: stores,
                scope_custody: Some(custody),
                aborted: false,
                error: None,
                pending_requests: 0,
                completion_requested: false,
                completion_queued: false,
                explicit_commit: false,
                active: false,
                finished: true,
            }),
        );
        let wrapper = new_platform_object(scope, &state, "IDBTransaction", id).unwrap();
        let serial = state
            .platform_listener_watchers
            .borrow()
            .get(&id)
            .unwrap()
            .0;
        state
            .platform_listener_releases
            .borrow_mut()
            .push((id, serial));
        let borrowed = state.platform_objects.borrow();
        reclaim(&state);
        assert!(ledger.live_bytes() > 0);
        assert_eq!(
            *state.platform_listener_releases.borrow(),
            vec![(id, serial)]
        );
        assert!(state.platform_listener_watchers.borrow().contains_key(&id));
        drop(borrowed);
        reclaim(&state);
        assert_eq!(ledger.live_bytes(), 0);
        assert!(state.platform_listener_releases.borrow().is_empty());
        assert!(!state.platform_listener_watchers.borrow().contains_key(&id));
        reclaim(&state);
        assert_eq!(ledger.live_bytes(), 0);
        let _ = wrapper;
    }
    #[test]
    fn unreachable_target_callback_cycles_collect_while_live_target_callbacks_survive() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let baseline = state.platform_listeners.borrow().len();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{for(let i=0;i<128;i++){const target=new EventTarget();target.addEventListener('cycle',()=>target.dispatchEvent(new Event('unused')))}globalThis.liveTarget=new EventTarget();globalThis.calls=0;liveTarget.addEventListener('x',()=>{if(!liveTarget)throw Error('target lost');calls++});return 'ok'})()"#
            ),
            "ok"
        );
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(state.platform_listeners.borrow().len(), baseline + 1);
        assert_eq!(
            eval(
                scope,
                "liveTarget.dispatchEvent(new Event('x'));String(calls)"
            ),
            "1"
        );
        eval(scope, "liveTarget=null;true");
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(state.platform_listeners.borrow().len(), baseline);
    }
    #[test]
    fn native_queued_event_roots_target_only_until_delivery_finishes() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
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
            "globalThis.calls=0;(()=>{const target=new EventTarget();globalThis.pendingTarget=target;target.addEventListener('complete',event=>{if(event.target!==target)throw Error('identity');calls++})})();true",
        );
        let id = state
            .platform_objects
            .borrow()
            .iter()
            .filter_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::EventTarget).then_some(*id)
            })
            .max()
            .unwrap();
        state
            .deferred_platform_tasks
            .borrow_mut()
            .push_back(DeferredPlatformTask::Event {
                target_id: id,
                event_type: "complete",
            });
        {
            v8::scope!(let inner,scope);
            reconcile_activity(inner, &state);
        }
        eval(scope, "pendingTarget=null;true");
        scope.low_memory_notification();
        assert!(state.platform_listener_roots.borrow().contains_key(&id));
        {
            v8::scope!(let inner,scope);
            state.drain_platform_completions(inner, 1).unwrap();
        }
        assert_eq!(eval(scope, "String(calls)"), "1");
        assert!(!state.platform_listener_roots.borrow().contains_key(&id));
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(
            !state
                .platform_listeners
                .borrow()
                .keys()
                .any(|(target, _)| *target == id)
        );
    }
    #[test]
    fn removal_abort_once_and_reentrant_listener_mutation_update_traced_edges() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert_eq!(
            eval(
                scope,
                r#"(()=>{const target=new EventTarget(),controller=new AbortController(),log=[];const later=()=>log.push('removed');target.addEventListener('x',()=>{log.push('first');target.removeEventListener('x',later)},{once:true});target.addEventListener('x',later);target.addEventListener('x',()=>log.push('aborted'),{signal:controller.signal});controller.abort();const event=new Event('x');event.toString=()=>{throw Error('dispatchEvent coerced event')};target.dispatchEvent(event);target.dispatchEvent(new Event('x'));return log.join(',')})()"#
            ),
            "first"
        );
    }
    #[test]
    fn abort_reason_object_cycles_collect_and_live_controller_keeps_reason_identity() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
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
            "for(let i=0;i<64;i++){const c=new AbortController(),signal=c.signal;c.abort({signal})}globalThis.controller=new AbortController();controller.abort({controller});globalThis.lazyController=new AbortController();lazyController.abort({marker:'lazy'});globalThis.request=new Request('https://example.test/');request.signal.marker='signal';request.headers.marker='headers';true",
        );
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(
            eval(
                scope,
                "controller.signal.reason.controller===controller && lazyController.signal.reason.marker==='lazy' && request.signal.marker==='signal' && request.headers.marker==='headers'"
            ),
            "true"
        );
        eval(
            scope,
            "controller=null;lazyController=null;request=null;true",
        );
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert!(state.platform_objects.borrow().values().all(|record| {
            match record {
                PlatformObjectRecord::AbortSignal(signal) => signal
                    .reason
                    .as_ref()
                    .is_none_or(|reason| reason.local(scope).is_none()),
                _ => true,
            }
        }));
    }
    #[test]
    fn idb_record_and_cursor_cached_values_follow_wrapper_reachability() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (record_id, cursor_id);
        {
            v8::scope!(let inner,scope);
            let value = v8::Object::new(inner);
            let key = v8::Array::new(inner, 0);
            record_id = register_platform_object(
                &state,
                PlatformObjectRecord::IdbRecord {
                    key: Value::new(inner, key.into()),
                    primary_key: Value::new(inner, key.into()),
                    value: Value::new(inner, value.into()),
                },
            );
            let record = new_platform_object(inner, &state, "IDBRecord", record_id).unwrap();
            let name = v8::String::new(inner, "owner").unwrap();
            value.set(inner, name.into(), record.into()).unwrap();
            let name = v8::String::new(inner, "liveRecord").unwrap();
            context
                .global(inner)
                .set(inner, name.into(), record.into())
                .unwrap();
            let value = v8::Object::new(inner);
            let heap = inner.get_heap_statistics();
            let used = heap.used_heap_size().checked_add(heap.external_memory()).unwrap();
            let (direction, direction_custody) = crate::idb_owned_admission::Text::copy_string_cancelled(
                &state.idb_payload_ledger, "next", heap.heap_size_limit(), used,
                &mut || inner.is_execution_terminating() || state.execution_retired(),
            ).unwrap().into_parts();
            cursor_id = register_platform_object(
                &state,
                PlatformObjectRecord::IdbCursor(IdbCursorRecord {
                    request_id: 0,
                    source_id: 0,
                    direction,
                    _direction_custody: Some(direction_custody),
                    query: None,
                    records: Vec::new(),
                    index: 0,
                    got_value: true,
                    iteration_pending: false,
                    with_value: true,
                    key_value: None,
                    primary_key_value: None,
                    record_value: Some(Value::new(inner, value.into())),
                }),
            );
            let cursor =
                new_platform_object(inner, &state, "IDBCursorWithValue", cursor_id).unwrap();
            let name = v8::String::new(inner, "owner").unwrap();
            value.set(inner, name.into(), cursor.into()).unwrap();
            let name = v8::String::new(inner, "liveCursor").unwrap();
            context
                .global(inner)
                .set(inner, name.into(), cursor.into())
                .unwrap();
        }
        scope.low_memory_notification();
        assert_eq!(
            eval(
                scope,
                "liveRecord.value.owner===liveRecord && liveCursor.value.owner===liveCursor"
            ),
            "true"
        );
        eval(scope, "liveRecord=null;liveCursor=null;true");
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        let records = state.platform_objects.borrow();
        let PlatformObjectRecord::IdbRecord { value, .. } = &records[&record_id] else {
            unreachable!()
        };
        assert!(value.local(scope).is_none());
        let PlatformObjectRecord::IdbCursor(cursor) = &records[&cursor_id] else {
            unreachable!()
        };
        assert!(cursor.record_value.as_ref().unwrap().local(scope).is_none());
    }
    #[test]
    fn xhr_cached_response_preserves_identity_without_rooting_response_request_cycles() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(scope, "globalThis.heldXhr=new XMLHttpRequest();true");
        let id = state
            .platform_objects
            .borrow()
            .iter()
            .find_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::XMLHttpRequest(_)).then_some(*id)
            })
            .unwrap();
        {
            v8::scope!(let inner,scope);
            let source = v8::String::new(inner, "({xhr:heldXhr})").unwrap();
            let value = v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap();
            if let PlatformObjectRecord::XMLHttpRequest(xhr) =
                &mut *platform_object_mut(&state, id).unwrap()
            {
                xhr.response_type = "json".into();
                xhr.ready_state = 4;
                xhr.status = 200;
                xhr.response_object = Some(Value::new(inner, value));
            }
            refresh(inner, &state, id).unwrap();
        }
        scope.low_memory_notification();
        assert_eq!(
            eval(
                scope,
                "heldXhr.response===heldXhr.response && heldXhr.response.xhr===heldXhr"
            ),
            "true"
        );
        eval(scope, "heldXhr=null;true");
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        let record = platform_object(&state, id).unwrap();
        let PlatformObjectRecord::XMLHttpRequest(xhr) = &*record else {
            unreachable!()
        };
        assert!(xhr.response_object.as_ref().unwrap().local(scope).is_none());
    }
    #[test]
    fn idb_result_is_traced_from_live_request_without_native_result_root_cycle() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let id = new_idb_request(&state, None);
        {
            v8::scope!(let inner,scope);
            let request = new_platform_object(inner, &state, "IDBRequest", id).unwrap();
            let name = v8::String::new(inner, "heldRequest").unwrap();
            context
                .global(inner)
                .set(inner, name.into(), request.into())
                .unwrap();
            let source = v8::String::new(inner, "({request:heldRequest})").unwrap();
            let value = v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap();
            set_idb_request_result(inner, &state, id, value).unwrap();
            if let PlatformObjectRecord::IdbRequest(request) =
                &mut *platform_object_mut(&state, id).unwrap()
            {
                request.done = true;
            }
        }
        scope.low_memory_notification();
        assert_eq!(
            eval(scope, "heldRequest.result.request===heldRequest"),
            "true"
        );
        eval(scope, "heldRequest=null;true");
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        let record = platform_object(&state, id).unwrap();
        let PlatformObjectRecord::IdbRequest(request) = &*record else {
            unreachable!()
        };
        assert!(request.result.as_ref().unwrap().local(scope).is_none());
    }
}
