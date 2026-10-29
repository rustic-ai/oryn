//! Native cross-arena adoption bridge. Wrapper objects are rehomed in place;
//! JavaScript identity, prototype, expandos and callback values are preserved.
use super::*;
use crate::bindings::WrapperMetadata;

struct CapturedWrapper {
    source: Rc<V8BindingState>,
    old: WrapperId,
    metadata: WrapperMetadata,
    object: v8::Global<v8::Object>,
}
pub(super) struct Wrappers {
    captured: Vec<CapturedWrapper>,
}
impl Wrappers {
    pub(super) fn capture(
        scope: &mut v8::PinScope,
        sources: &[Rc<V8BindingState>],
        destination: &Rc<V8BindingState>,
        nodes: &BTreeSet<NodeId>,
    ) -> Result<Self, V8BindingError> {
        let mut captured = Vec::new();
        for source in sources {
            for (id, entry) in source.objects.borrow().iter() {
                let Some(metadata) = source.wrappers.borrow().metadata(*id) else {
                    continue;
                };
                if !nodes.contains(&metadata.node) {
                    continue;
                }
                if let Some(object) = entry.handle.to_local(scope) {
                    captured.push(CapturedWrapper {
                        source: source.clone(),
                        old: *id,
                        metadata,
                        object: v8::Global::new(scope, object),
                    });
                }
            }
        }
        destination
            .wrappers
            .borrow()
            .preflight_migration(captured.len())?;
        Ok(Self { captured })
    }
    pub(super) fn publish(
        &mut self,
        scope: &mut v8::PinScope,
        destination: &Rc<V8BindingState>,
        nodes: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), V8BindingError> {
        let arena = destination.dom.borrow().arena().id();
        while !self.captured.is_empty() {
            let captured = &self.captured[0];
            let node = *nodes
                .get(&captured.metadata.node)
                .ok_or(V8BindingError::Allocation("incomplete node migration map"))?;
            let id = destination.wrappers.borrow_mut().accept_migrated(
                captured.metadata,
                arena,
                destination.realm,
                destination.wrapper_generation,
                node,
            )?;
            let object = v8::Local::new(scope, &captured.object);
            object.set_internal_field(0, v8::BigInt::new_from_u64(scope, id.0).into());
            set_dom_wrapper_identity(
                scope,
                object,
                node,
                destination.realm,
                destination.wrapper_generation,
            );
            let registry = Rc::downgrade(&destination.wrappers);
            let handle = v8::Weak::with_finalizer(
                scope,
                object,
                Box::new(move |_| {
                    if let Some(registry) = registry.upgrade() {
                        registry.borrow_mut().enqueue_weak_release(id);
                    }
                }),
            );
            // Dropping the obsolete Weak cancels its original callback. Any
            // callback already queued is removed with the old registry entry.
            captured.source.objects.borrow_mut().remove(&captured.old);
            captured
                .source
                .wrappers
                .borrow_mut()
                .remove_migrated(captured.old)?;
            destination.objects.borrow_mut().insert(
                id,
                WrapperObject {
                    retention_component: None,
                    handle,
                    connected_root: None,
                },
            );
            let associated = captured
                .source
                .associated_wrappers
                .borrow()
                .keys()
                .filter(|(owner, _)| *owner == captured.metadata.node)
                .copied()
                .collect::<Vec<_>>();
            for key in associated {
                if let Some(wrapper) = captured
                    .source
                    .associated_wrappers
                    .borrow_mut()
                    .remove(&key)
                {
                    if let Some(local) = wrapper.to_local(scope) {
                        local.set_internal_field(0, v8::BigInt::new_from_u64(scope, id.0).into());
                    }
                    destination
                        .associated_wrappers
                        .borrow_mut()
                        .insert((node, key.1), wrapper);
                }
            }
            captured.source.wrapper_graph_revision.set(None);
            captured
                .source
                .wrapper_graph_components
                .borrow_mut()
                .clear();
            destination
                .wrapper_graph_new_nodes
                .borrow_mut()
                .insert(node);
            self.captured.remove(0);
        }
        destination.wrapper_graph_revision.set(None);
        if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
            registry.cross_realm_wrappers.set(true);
        }
        Ok(())
    }
}

pub(super) fn move_node_maps(
    source: &Rc<V8BindingState>,
    destination: &Rc<V8BindingState>,
    nodes: &BTreeMap<NodeId, NodeId>,
) {
    for (old, new) in nodes {
        let keys = source
            .listeners
            .borrow()
            .keys()
            .filter(|(node, _)| node == old)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(listeners) = source.listeners.borrow_mut().remove(&key) {
                destination
                    .listeners
                    .borrow_mut()
                    .entry((*new, key.1))
                    .or_default()
                    .extend(listeners);
            }
        }
        if let Some(value) = source.custom_states.borrow_mut().remove(old) {
            destination.custom_states.borrow_mut().insert(*new, value);
        }
        if let Some(value) = source.custom_state_order.borrow_mut().remove(old) {
            destination
                .custom_state_order
                .borrow_mut()
                .insert(*new, value);
        }
        if let Some(value) = source.text_control_selections.borrow_mut().remove(old) {
            destination
                .text_control_selections
                .borrow_mut()
                .insert(*new, value);
        }
        let handlers = source
            .event_handlers
            .borrow()
            .keys()
            .filter(|(node, _)| node == old)
            .cloned()
            .collect::<Vec<_>>();
        for key in handlers {
            if let Some(handler) = source.event_handlers.borrow_mut().remove(&key) {
                destination
                    .event_handlers
                    .borrow_mut()
                    .insert((*new, key.1), handler);
            }
        }
        macro_rules! move_entry {
            ($field:ident) => {
                if let Some(value) = source.$field.borrow_mut().remove(old) {
                    destination.$field.borrow_mut().insert(*new, value);
                }
            };
        }
        move_entry!(canvas_images);
        move_entry!(canvas_context_settings);
        move_entry!(canvas_draw_states);
        move_entry!(canvas_saved_states);
        move_entry!(canvas_paths);
        move_entry!(decoded_images);
        move_entry!(decoded_svg_images);
        move_entry!(scroll_offsets);
        move_entry!(image_response_urls);
        if source.click_in_progress.borrow_mut().remove(old) {
            destination.click_in_progress.borrow_mut().insert(*new);
        }
        if source.tainted_canvases.borrow_mut().remove(old) {
            destination.tainted_canvases.borrow_mut().insert(*new);
        }
        if source.suppressed_content_handlers.borrow_mut().remove(old) {
            destination
                .suppressed_content_handlers
                .borrow_mut()
                .insert(*new);
        }
    }
    destination.next_event_listener_id.set(
        destination
            .next_event_listener_id
            .get()
            .max(source.next_event_listener_id.get()),
    );
}

/// Materialize source removal records while their original native identities
/// still exist. The arrays retain original Node wrappers across relocation.
pub(super) fn freeze_observers(
    scope: &mut v8::PinScope,
    sources: &[Rc<V8BindingState>],
) -> Result<(), V8BindingError> {
    for state in sources {
        let ids = state
            .mutation_observers
            .borrow()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            let records = {
                let mut observers = state.mutation_observers.borrow_mut();
                let observer = observers.get_mut(&id).unwrap();
                collect_observer_records(&state.dom.borrow(), observer);
                observer.records.clone()
            };
            if records.is_empty() {
                continue;
            }
            let array = mutation_record_array(scope, state, &records)?;
            if let Some(observer) = state.mutation_observers.borrow_mut().get_mut(&id) {
                observer
                    .prepared_records
                    .push(v8::Global::new(scope, array));
                observer.records.drain(..records.len());
            }
        }
    }
    Ok(())
}
pub(super) fn take_observer_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    collect_remote_observers(scope)?;
    let Some((records, prepared)) =
        state
            .mutation_observers
            .borrow_mut()
            .get_mut(&id)
            .map(|observer| {
                collect_observer_records(&state.dom.borrow(), observer);
                (observer.records.clone(), observer.prepared_records.clone())
            })
    else {
        return Ok(v8::Array::new(scope, 0));
    };
    let current = mutation_record_array(scope, state, &records)?;
    let mut entries = Vec::new();
    for batch in &prepared {
        let batch = v8::Local::new(scope, batch);
        for index in 0..batch.length() {
            entries.push(
                batch
                    .get_index(scope, index)
                    .ok_or(V8BindingError::JavaScriptException)?,
            );
        }
    }
    for index in 0..current.length() {
        entries.push(
            current
                .get_index(scope, index)
                .ok_or(V8BindingError::JavaScriptException)?,
        );
    }
    let array = v8::Array::new_with_elements(scope, &entries);
    if let Some(observer) = state.mutation_observers.borrow_mut().get_mut(&id) {
        observer.records.drain(..records.len());
        observer.prepared_records.drain(..prepared.len());
    }
    Ok(array)
}

#[derive(Default)]
pub(super) struct ObserverRoutes(Rc<RefCell<Vec<ObserverRoute>>>);
struct ObserverRoute {
    owner: RcWeak<V8BindingState>,
    id: u64,
    target: RcWeak<V8BindingState>,
    observer: MutationObserverEntry,
}
fn mirror(
    owner: &MutationObserverEntry,
    cursor: oryn_dom::MutationCursor,
) -> MutationObserverEntry {
    MutationObserverEntry {
        callback: owner.callback.clone(),
        object: owner.object.clone(),
        registrations: BTreeMap::new(),
        transients: BTreeMap::new(),
        records: Vec::new(),
        prepared_records: Vec::new(),
        pending_order: None,
        cursor,
    }
}
pub(super) fn collect_remote_observers(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let Some(registry) = scope.get_slot::<BindingStateRegistry>() else {
        return Ok(());
    };
    let routes = registry.migrated_observers.0.clone();
    let mut routes = routes.borrow_mut();
    routes.retain(|route| {
        route.owner.upgrade().is_some_and(|owner| {
            !owner.execution_retired() && owner.mutation_observers.borrow().contains_key(&route.id)
        }) && route
            .target
            .upgrade()
            .is_some_and(|target| !target.execution_retired())
    });
    for route in routes.iter_mut() {
        let owner = route.owner.upgrade().unwrap();
        let target = route.target.upgrade().unwrap();
        collect_observer_records(&target.dom.borrow(), &mut route.observer);
        if route.observer.records.is_empty() {
            continue;
        }
        let array = mutation_record_array(scope, &target, &route.observer.records)?;
        if let Some(observer) = owner.mutation_observers.borrow_mut().get_mut(&route.id) {
            observer
                .prepared_records
                .push(v8::Global::new(scope, array));
            if let Some(order) = route.observer.pending_order.take() {
                observer.pending_order =
                    Some(observer.pending_order.map_or(order, |old| old.min(order)));
            }
        }
        route.observer.records.clear();
        if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
            registry
                .observing_states
                .borrow_mut()
                .insert(owner.instance_id);
        }
    }
    Ok(())
}
pub(super) fn clear_remote_observer(
    scope: &mut v8::PinScope,
    owner: &V8BindingState,
    id: u64,
    disconnect: bool,
) {
    if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
        let mut routes = registry.migrated_observers.0.borrow_mut();
        if disconnect {
            routes.retain(|route| {
                route.id != id
                    || !route
                        .owner
                        .upgrade()
                        .is_some_and(|state| state.instance_id == owner.instance_id)
            });
        } else {
            for route in routes.iter_mut().filter(|route| {
                route.id == id
                    && route
                        .owner
                        .upgrade()
                        .is_some_and(|state| state.instance_id == owner.instance_id)
            }) {
                route.observer.transients.clear();
            }
        }
    }
}
/// Remove transient registrations by their original registration, across all
/// arenas. The source node may remain behind while a detached subtree migrates.
pub(super) fn clear_registration_transients(
    scope: &mut v8::PinScope,
    owner: &V8BindingState,
    id: u64,
    order: u64,
) {
    if let Some(observer) = owner.mutation_observers.borrow_mut().get_mut(&id) {
        observer
            .transients
            .retain(|(source, _), _| *source != order);
    }
    if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
        for route in registry.migrated_observers.0.borrow_mut().iter_mut() {
            if route.id == id
                && route
                    .owner
                    .upgrade()
                    .is_some_and(|state| state.instance_id == owner.instance_id)
            {
                route
                    .observer
                    .transients
                    .retain(|(source, _), _| *source != order);
            }
        }
    }
}
pub(super) fn take_remote_registration(
    scope: &mut v8::PinScope,
    owner: &Rc<V8BindingState>,
    id: u64,
    target: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Option<u64>, V8BindingError> {
    collect_remote_observers(scope)?;
    let mut order = None;
    if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
        for route in registry.migrated_observers.0.borrow_mut().iter_mut() {
            if route.id == id
                && route
                    .owner
                    .upgrade()
                    .is_some_and(|value| Rc::ptr_eq(&value, owner))
                && route
                    .target
                    .upgrade()
                    .is_some_and(|value| Rc::ptr_eq(&value.dom, &target.dom))
                && let Some(options) = route.observer.registrations.remove(&node)
            {
                order = Some(order.map_or(options.registration_order, |old: u64| {
                    old.min(options.registration_order)
                }));
            }
        }
    }
    if let Some(order) = order {
        clear_registration_transients(scope, owner, id, order);
    }
    Ok(order)
}
pub(super) fn observe_remote(
    scope: &mut v8::PinScope,
    owner: &Rc<V8BindingState>,
    id: u64,
    target: &Rc<V8BindingState>,
    node: NodeId,
    mut options: MutationObserverOptions,
) -> Result<(), V8BindingError> {
    let previous_order = take_remote_registration(scope, owner, id, target, node)?;
    let registry = scope
        .get_slot::<BindingStateRegistry>()
        .ok_or(V8BindingError::Allocation("observer agent registry"))?;
    let mut routes = registry.migrated_observers.0.borrow_mut();
    let position = routes.iter().position(|route| {
        route.id == id
            && route
                .owner
                .upgrade()
                .is_some_and(|state| Rc::ptr_eq(&state, owner))
            && route
                .target
                .upgrade()
                .is_some_and(|state| Rc::ptr_eq(&state.dom, &target.dom))
    });
    let index = if let Some(index) = position {
        index
    } else {
        let observers = owner.mutation_observers.borrow();
        let source = observers
            .get(&id)
            .ok_or(V8BindingError::Allocation("observer identity"))?;
        let observer = mirror(source, target.dom.borrow().mutation_cursor());
        routes.push(ObserverRoute {
            owner: Rc::downgrade(owner),
            id,
            target: Rc::downgrade(target),
            observer,
        });
        routes.len() - 1
    };
    let observer = &mut routes[index].observer;
    options.registration_order = previous_order
        .unwrap_or_else(|| NEXT_OBSERVER_REGISTRATION_ORDER.fetch_add(1, AtomicOrdering::Relaxed));
    observer.registrations.insert(node, options);
    registry
        .observing_states
        .borrow_mut()
        .insert(owner.instance_id);
    Ok(())
}
pub(super) fn migrate_observers(
    scope: &mut v8::PinScope,
    sources: &[Rc<V8BindingState>],
    destination: &Rc<V8BindingState>,
    nodes: &BTreeMap<NodeId, NodeId>,
) -> Result<(), V8BindingError> {
    let registry = scope
        .get_slot::<BindingStateRegistry>()
        .ok_or(V8BindingError::Allocation("observer agent registry"))?;
    let mut additions = Vec::new();
    for owner in sources {
        for (id, source) in owner.mutation_observers.borrow_mut().iter_mut() {
            let mut observer = mirror(source, destination.dom.borrow().mutation_cursor());
            let moved = source
                .registrations
                .keys()
                .filter(|node| nodes.contains_key(node))
                .copied()
                .collect::<Vec<_>>();
            for old in moved {
                observer
                    .registrations
                    .insert(nodes[&old], source.registrations.remove(&old).unwrap());
            }
            let moved = source
                .transients
                .keys()
                .filter(|(_, node)| nodes.contains_key(node))
                .copied()
                .collect::<Vec<_>>();
            for key in moved {
                observer.transients.insert(
                    (key.0, nodes[&key.1]),
                    source.transients.remove(&key).unwrap(),
                );
            }
            if !observer.registrations.is_empty() || !observer.transients.is_empty() {
                additions.push(ObserverRoute {
                    owner: Rc::downgrade(owner),
                    id: *id,
                    target: Rc::downgrade(destination),
                    observer,
                });
            }
        }
    }
    let mut routes = registry.migrated_observers.0.borrow_mut();
    for route in routes.iter_mut() {
        if !route.target.upgrade().is_some_and(|target| {
            sources
                .iter()
                .any(|source| Rc::ptr_eq(&target.dom, &source.dom))
        }) {
            continue;
        }
        let mut observer = mirror(&route.observer, destination.dom.borrow().mutation_cursor());
        let keys = route
            .observer
            .registrations
            .keys()
            .filter(|node| nodes.contains_key(node))
            .copied()
            .collect::<Vec<_>>();
        for old in keys {
            observer.registrations.insert(
                nodes[&old],
                route.observer.registrations.remove(&old).unwrap(),
            );
        }
        let keys = route
            .observer
            .transients
            .keys()
            .filter(|(_, node)| nodes.contains_key(node))
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            observer.transients.insert(
                (key.0, nodes[&key.1]),
                route.observer.transients.remove(&key).unwrap(),
            );
        }
        if !observer.registrations.is_empty() || !observer.transients.is_empty() {
            additions.push(ObserverRoute {
                owner: route.owner.clone(),
                id: route.id,
                target: Rc::downgrade(destination),
                observer,
            });
        }
    }
    routes.extend(additions);
    Ok(())
}

fn migrate_ranges(
    scope: &mut v8::PinScope,
    source: &Rc<V8BindingState>,
    destination: &Rc<V8BindingState>,
    _nodes: &BTreeMap<NodeId, NodeId>,
    ranges: &BTreeMap<LiveRangeId, LiveRangeId>,
) -> Result<(), V8BindingError> {
    let ids = source.ranges.borrow().keys().copied().collect::<Vec<_>>();
    for old in ids {
        if source.static_ranges.borrow().contains(&old) {
            continue;
        }
        let Some(new) = ranges.get(&LiveRangeId(old)) else {
            continue;
        };
        let new = new.0;
        let migrated: RangeRecord = destination
            .dom
            .borrow()
            .live_range(LiveRangeId(new))?
            .into();
        let object = source
            .range_objects
            .borrow()
            .get(&old)
            .and_then(|wrapper| wrapper.handle.to_local(scope));
        let owner = object
            .map(|object| {
                traced_native_roots::prepare_range_owner(scope, destination, object, &migrated)
            })
            .transpose()?;
        source.ranges.borrow_mut().remove(&old);
        destination.ranges.borrow_mut().insert(new, migrated);
        if let Some(wrapper) = source.range_objects.borrow_mut().remove(&old) {
            wrapper.traced_endpoints.set(None);
            if let Some(object) = wrapper.handle.to_local(scope) {
                object.set_internal_field(0, v8::Number::new(scope, new as f64).into());
                object.set_internal_field(
                    1,
                    v8::BigInt::new_from_u64(scope, destination.instance_id).into(),
                );
            }
            destination.range_objects.borrow_mut().insert(new, wrapper);
        }
        if let Some(owner) = owner {
            owner.publish();
        }
        for (primary, composed) in source.range_companions.borrow().iter() {
            if let (Some(primary), Some(composed)) = (primary.upgrade(), composed.upgrade())
                && (primary.get() == Some(old) || composed.get() == Some(old))
            {
                primary.set(None);
                composed.set(None);
            }
        }
        source
            .range_memory_delta
            .set(source.range_memory_delta.get() - range_payload_bytes(false));
        destination
            .range_memory_delta
            .set(destination.range_memory_delta.get() + range_payload_bytes(false));
    }
    Ok(())
}
fn cancel_source_resources(
    source: &Rc<V8BindingState>,
    nodes: &BTreeSet<NodeId>,
) -> Result<(), V8BindingError> {
    let mut failure = None;
    if let Err(error) = text_tracks::cancel_nodes(source, nodes) {
        failure.get_or_insert(error);
    }
    for node in nodes {
        if let Err(error) = source.cancel_image_requests(Some(*node)) {
            failure.get_or_insert(error);
        }
    }
    source
        .pending_dynamic_scripts
        .borrow_mut()
        .retain(|script| !nodes.contains(&script.node));
    source
        .pending_dynamic_stylesheets
        .borrow_mut()
        .retain(|node| !nodes.contains(node));
    source
        .prepared_stylesheets
        .borrow_mut()
        .retain(|node, _| !nodes.contains(node));
    source
        .pending_dynamic_frames
        .borrow_mut()
        .retain(|node| !nodes.contains(node));
    source
        .prepared_frames
        .borrow_mut()
        .retain(|node| !nodes.contains(node));
    source.layout_cache_key.borrow_mut().take();
    failure.map_or(Ok(()), Err)
}

/// Adopt an actual native Node into a different arena. Preparation retains both
/// graphs while old observer records and GC-backed views are materialized.
/// No JavaScript source, serializer or replacement Node participates.
#[derive(Default)]
pub(super) struct Publications(RefCell<VecDeque<PendingPublication>>);
enum PendingPublication {
    Cross(Box<Publication>),
    Same(Box<SameArenaPublication>),
}
impl PendingPublication {
    fn publish(&mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        match self {
            Self::Cross(pending) => pending.publish(scope),
            Self::Same(pending) => pending.publish(scope),
        }
    }
}
struct SameArenaPublication {
    _leases: Vec<oryn_dom::TrackedNodeLease>,
    source: Rc<V8BindingState>,
    destination: Rc<V8BindingState>,
    nodes: BTreeMap<NodeId, NodeId>,
    stage: u8,
}
#[cfg(test)]
thread_local! {static FAIL_SAME_PUBLICATION_STAGE:Cell<Option<u8>>=const{Cell::new(None)};}
impl SameArenaPublication {
    fn publish(&mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        // Advance each completed tail independently: a later failure must not
        // replay stylesheet ownership reset or track wrapper transfer.
        while self.stage < 4 {
            #[cfg(test)]
            if FAIL_SAME_PUBLICATION_STAGE.with(|fault| {
                if fault.get() == Some(self.stage) {
                    fault.set(None);
                    true
                } else {
                    false
                }
            }) {
                return Err(V8BindingError::Allocation(
                    "injected same-arena adoption publication failure",
                ));
            }
            match self.stage {
                0 => text_tracks::migrate(&self.source, &self.destination, &self.nodes)?,
                1 => av_tracks::migrate(&self.source, &self.destination, &self.nodes)?,
                2 => stylesheet_adoption::publish(
                    std::slice::from_ref(&self.source),
                    &self.destination,
                    &self.nodes,
                )?,
                3 => web_animations::migrate_targets(
                    scope,
                    &self.source,
                    &self.destination,
                    &self.nodes,
                ),
                _ => unreachable!(),
            }
            self.stage += 1;
        }
        Ok(())
    }
}
struct Publication {
    _pin: oryn_dom::PreparedNodeMigration,
    sources: Vec<Rc<V8BindingState>>,
    destination: Rc<V8BindingState>,
    document: NodeId,
    migration: oryn_dom::NodeMigration,
    wrappers: Wrappers,
    lists: node_lists::MigrationPrepared,
    statics: static_range_custody::Prepared,
    iterators: node_iterators::Prepared,
    aria: aria::Migration,
    custom: Vec<custom_elements::NodeMigration>,
    templates: template_links::Prepared,
    stage: u8,
    range_source: usize,
}
#[cfg(test)]
thread_local! {static FAIL_PUBLICATION_ONCE:Cell<bool>=const{Cell::new(false)};}
impl Publication {
    fn publish(&mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        // Native root ledgers are corrected before any fallible V8 publication.
        if self.stage == 0 {
            self.iterators
                .commit_ownership(&self.destination, &self.migration.iterators)?;
            self.lists
                .commit_ownership(&self.destination, &self.migration.nodes)?;
            self.statics
                .commit_ownership(&self.destination, &self.migration.nodes)?;
            self.stage = 1;
        }
        #[cfg(test)]
        if self.stage == 1 && FAIL_PUBLICATION_ONCE.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "injected wrapper publication failure",
            ));
        }
        if self.stage == 1 {
            self.wrappers
                .publish(scope, &self.destination, &self.migration.nodes)?;
            for source in &self.sources {
                web_animations::migrate_targets(
                    scope,
                    source,
                    &self.destination,
                    &self.migration.nodes,
                );
            }
            self.stage = 2;
        }
        if self.stage == 2 {
            self.lists
                .publish(scope, &self.destination, &self.migration.nodes)?;
            self.stage = 3;
        }
        if self.stage == 3 {
            stylesheet_adoption::publish(&self.sources, &self.destination, &self.migration.nodes)?;
            for source in &self.sources {
                text_tracks::migrate(source, &self.destination, &self.migration.nodes)?;
                av_tracks::migrate(source, &self.destination, &self.migration.nodes)?;
                move_node_maps(source, &self.destination, &self.migration.nodes);
            }
            for custom in &mut self.custom {
                custom.publish(&self.destination, &self.migration.nodes, self.document);
            }
            self.stage = 4;
        }
        if self.stage == 4 {
            while self.range_source < self.sources.len() {
                migrate_ranges(
                    scope,
                    &self.sources[self.range_source],
                    &self.destination,
                    &self.migration.nodes,
                    &self.migration.ranges,
                )?;
                self.range_source += 1;
            }
            self.stage = 5;
        }
        if self.stage == 5 {
            migrate_observers(
                scope,
                &self.sources,
                &self.destination,
                &self.migration.nodes,
            )?;
            self.stage = 6;
        }
        if self.stage == 6 {
            self.aria.publish(scope)?;
            self.templates.publish(scope)?;
            self.stage = 7;
        }
        wrapper_graphs::refresh_agent(scope, &self.destination);
        Ok(())
    }
}
/// Resume retained publication before another native DOM operation observes
/// the moved graph. A failure leaves the current and subsequent transactions
/// owned by the agent; no old-arena GC lease is silently abandoned.
pub(super) fn has_publications(scope: &mut v8::PinScope<'_, '_, ()>) -> bool {
    scope
        .get_slot::<BindingStateRegistry>()
        .is_some_and(|registry| !registry.node_publications.0.borrow().is_empty())
}
pub(super) fn resume_publications(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    loop {
        let pending = scope
            .get_slot::<BindingStateRegistry>()
            .and_then(|registry| registry.node_publications.0.borrow_mut().pop_front());
        let Some(mut pending) = pending else {
            oryn_dom::drain_node_migration_pins()?;
            return Ok(());
        };
        if let Err(error) = pending.publish(scope) {
            scope
                .get_slot::<BindingStateRegistry>()
                .expect("migration agent remains installed")
                .node_publications
                .0
                .borrow_mut()
                .push_front(pending);
            return Err(error);
        }
        drop(pending);
        oryn_dom::drain_node_migration_pins()?;
    }
}
pub(super) fn adopt(
    scope: &mut v8::PinScope,
    destination: &Rc<V8BindingState>,
    document: NodeId,
    source: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<NodeId, V8BindingError> {
    resume_publications(scope)?;
    if Rc::ptr_eq(&source.dom, &destination.dom) {
        let (reference, document_id, changed, nodes) = {
            let dom = source.dom.borrow();
            let changed = dom.arena().node_document(node)? != Some(document);
            let mut pending = vec![node];
            let mut nodes = BTreeMap::new();
            while let Some(node) = pending.pop() {
                if nodes.insert(node, node).is_some() {
                    continue;
                }
                let record = dom.arena().get(node)?;
                pending.extend(record.children.iter().copied().filter(|child| {
                    !matches!(
                        dom.arena().get(*child).map(|record| &record.kind),
                        Ok(NodeKind::Document)
                    )
                }));
                pending.extend(record.template_content);
            }
            (
                dom.arena().node_ref(node)?,
                dom.arena().document_id(document)?,
                changed,
                nodes,
            )
        };
        let pending = if changed {
            let registry = scope
                .get_slot::<BindingStateRegistry>()
                .ok_or(V8BindingError::Allocation("node migration agent"))?;
            registry
                .node_publications
                .0
                .borrow_mut()
                .try_reserve(1)
                .map_err(|_| {
                    V8BindingError::Allocation("same-arena adoption publication capacity")
                })?;
            let leases = nodes
                .keys()
                .map(|node| oryn_dom::TrackedNodeLease::new(&source.dom, *node))
                .collect::<Result<Vec<_>, _>>()?;
            cancel_source_resources(source, &nodes.keys().copied().collect())?;
            Some(SameArenaPublication {
                _leases: leases,
                source: source.clone(),
                destination: destination.clone(),
                nodes,
                stage: 0,
            })
        } else {
            None
        };
        destination
            .dom
            .borrow_mut()
            .adopt_node(document_id, reference)?;
        if let Some(pending) = pending {
            // Retain immediately after commit, before any fallible publication.
            // A parser retry may now observe the new ownerDocument and skip
            // Adopt; its checkpoint still drains this agent-owned queue.
            scope
                .get_slot::<BindingStateRegistry>()
                .expect("migration agent remains installed")
                .node_publications
                .0
                .borrow_mut()
                .push_back(PendingPublication::Same(Box::new(pending)));
            resume_publications(scope)?;
        }
        return Ok(node);
    }
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .ok_or(V8BindingError::Allocation("node migration agent"))?
        .instances
        .borrow()
        .values()
        .filter_map(RcWeak::upgrade)
        .collect::<Vec<_>>();
    let sources = states
        .iter()
        .filter(|state| Rc::ptr_eq(&state.dom, &source.dom))
        .cloned()
        .collect::<Vec<_>>();
    for state in &sources {
        state.process_weak_releases()?;
    }
    let mut prepared =
        DomRuntime::prepare_subtree_migration(&source.dom, &destination.dom, node, document)?;
    let nodes = prepared.plan.nodes.iter().copied().collect::<BTreeSet<_>>();
    for state in &sources {
        cancel_source_resources(state, &nodes)?;
        custom_elements::collect_for_migration(scope, state)?;
    }
    collect_remote_observers(scope)?;
    freeze_observers(scope, &sources)?;
    let lists = node_lists::capture_migration(scope, &states, &source.dom, destination, &nodes)?;
    let statics = static_range_custody::capture(scope, &states, &source.dom, &nodes)?;
    let iterators = node_iterators::capture(scope, &states, &source.dom, &nodes)?;
    let aria = aria::capture_migration(scope, &states, &source.dom, &nodes)?;
    let mut custom = Vec::new();
    for state in &sources {
        custom.push(custom_elements::capture_node_migration(
            scope,
            state,
            destination,
            &nodes,
            document,
        )?);
    }
    let templates = template_links::capture(scope, source, &nodes)?;
    let wrappers = Wrappers::capture(scope, &sources, destination, &nodes)?;
    let migration =
        DomRuntime::commit_subtree_migration(&source.dom, &destination.dom, &mut prepared)?;
    let root = migration.root;
    let pending = Publication {
        _pin: prepared,
        sources,
        destination: destination.clone(),
        document,
        migration,
        wrappers,
        lists,
        statics,
        iterators,
        aria,
        custom,
        templates,
        stage: 0,
        range_source: 0,
    };
    scope
        .get_slot::<BindingStateRegistry>()
        .expect("migration agent remains installed")
        .node_publications
        .0
        .borrow_mut()
        .push_back(PendingPublication::Cross(Box::new(pending)));
    resume_publications(scope)?;
    Ok(root)
}

/// Cross-arena validity is checked without detaching the argument. This is
/// the pre-insert check, before the adoption/removal side effects.
pub(super) fn validate_foreign_insertion(
    destination: &V8BindingState,
    parent: NodeId,
    source: &V8BindingState,
    node: NodeId,
    reference: Option<NodeId>,
    replaced: Option<NodeId>,
) -> Result<(), V8BindingError> {
    let target = destination.dom.borrow();
    let origin = source.dom.borrow();
    let parent_document = matches!(target.arena().get(parent)?.kind, NodeKind::Document);
    if !matches!(
        target.arena().get(parent)?.kind,
        NodeKind::Document
            | NodeKind::DocumentFragment
            | NodeKind::Element { .. }
            | NodeKind::ShadowRoot { .. }
    ) {
        return Err(hierarchy_request_error());
    }
    for candidate in [reference, replaced].into_iter().flatten() {
        if target.parent(candidate)? != Some(parent) {
            return Err(V8BindingError::Unsupported {
                interface: "Node".into(),
                member: "NotFoundError".into(),
            });
        }
    }
    let incoming = if matches!(origin.arena().get(node)?.kind, NodeKind::DocumentFragment) {
        light_children(&origin, node)?
    } else {
        vec![node]
    };
    let mut kinds = Vec::new();
    for child in light_children(&target, parent)? {
        if Some(child) == reference {
            for item in &incoming {
                kinds.push(origin.arena().get(*item)?.kind.clone());
            }
        }
        if Some(child) != replaced {
            kinds.push(target.arena().get(child)?.kind.clone());
        }
    }
    if reference.is_none() {
        for item in incoming {
            kinds.push(origin.arena().get(item)?.kind.clone());
        }
    }
    let (mut elements, mut doctypes, mut saw_element) = (0usize, 0usize, false);
    for kind in kinds {
        match kind {
            NodeKind::Document | NodeKind::Attribute { .. } | NodeKind::ShadowRoot { .. } => {
                return Err(hierarchy_request_error());
            }
            NodeKind::Element { .. } if parent_document => {
                elements += 1;
                saw_element = true;
            }
            NodeKind::DocumentType { .. } => {
                if !parent_document || saw_element {
                    return Err(hierarchy_request_error());
                }
                doctypes += 1;
            }
            NodeKind::Text { .. } | NodeKind::CDataSection { .. } if parent_document => {
                return Err(hierarchy_request_error());
            }
            _ => {}
        }
    }
    if elements > 1 || doctypes > 1 {
        return Err(hierarchy_request_error());
    }
    Ok(())
}

pub(super) struct InsertionArgument {
    pub owner: Rc<V8BindingState>,
    pub parent: NodeId,
    pub node: NodeId,
    pub reference: Option<NodeId>,
    pub replaced: Option<NodeId>,
    pub replacement_removed: bool,
}
pub(super) fn insertion_argument(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    parent: NodeId,
    value: v8::Local<v8::Value>,
    reference: Option<NodeId>,
    replaced: Option<NodeId>,
) -> Result<InsertionArgument, V8BindingError> {
    insertion_argument_inner(scope, state, parent, value, reference, replaced, false)
}
pub(super) fn replacement_argument(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    parent: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<InsertionArgument, V8BindingError> {
    insertion_argument_inner(scope, state, parent, value, None, None, true)
}
fn insertion_argument_inner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    parent: NodeId,
    value: v8::Local<v8::Value>,
    reference: Option<NodeId>,
    replaced: Option<NodeId>,
    replace_all: bool,
) -> Result<InsertionArgument, V8BindingError> {
    // Hold original wrappers: a source parent's children-changed script can
    // adopt the destination, reference or source into another native arena.
    let parent_wrapper = wrap_node(scope, state, parent)?;
    let mut reference_wrapper = reference
        .map(|node| wrap_node(scope, state, node))
        .transpose()?;
    let replaced_wrapper = replaced
        .map(|node| wrap_node(scope, state, node))
        .transpose()?;
    let mut replacement_removed = false;
    for _ in 0..4096 {
        let (destination, parent) = node_identity::argument(scope, parent_wrapper.into())?
            .ok_or(DomError::StaleNode(parent))?;
        let resolve = |scope: &mut v8::PinScope,
                       wrapper: Option<v8::Local<v8::Object>>|
         -> Result<Option<NodeId>, V8BindingError> {
            match wrapper {
                None => Ok(None),
                Some(wrapper) => {
                    let (owner, node) = node_identity::argument(scope, wrapper.into())?
                        .ok_or(DomError::StaleNode(parent))?;
                    if !Rc::ptr_eq(&owner.dom, &destination.dom) {
                        return Err(V8BindingError::Unsupported {
                            interface: "Node".into(),
                            member: "NotFoundError".into(),
                        });
                    }
                    Ok(Some(node))
                }
            }
        };
        let reference = resolve(scope, reference_wrapper)?;
        let replaced = if replacement_removed {
            None
        } else {
            resolve(scope, replaced_wrapper)?
        };
        let (source, node) =
            node_identity::argument(scope, value)?.ok_or(V8BindingError::Unsupported {
                interface: "Node".into(),
                member: "TypeError".into(),
            })?;
        template_links::validate(scope, &destination, parent, &source, node)?;
        if replace_all {
            if Rc::ptr_eq(&source.dom, &destination.dom) {
                let dom = destination.dom.borrow();
                let children = if matches!(dom.arena().get(node)?.kind, NodeKind::DocumentFragment)
                {
                    light_children(&dom, node)?
                } else {
                    vec![node]
                };
                validate_child_sequence(&dom, parent, &children)?;
            } else {
                validate_foreign_replacement(&destination, parent, &source, node)?;
            }
        } else if Rc::ptr_eq(&source.dom, &destination.dom) {
            validate_pre_insertion(&destination.dom.borrow(), parent, node, reference, replaced)?;
        } else {
            validate_foreign_insertion(&destination, parent, &source, node, reference, replaced)?;
        }
        post_connection_scripts::check_insertion_capacity(
            scope,
            &destination,
            parent,
            &source.dom.borrow(),
            node,
        )?;
        let old_parent = source.dom.borrow().parent(node)?;
        if let Some(old_parent) = old_parent
            && script_children_changed::is_script_parent(&source, old_parent)?
        {
            if Rc::ptr_eq(&source.dom, &destination.dom) && reference == Some(node) {
                let next = {
                    let dom = source.dom.borrow();
                    let siblings = light_children(&dom, old_parent)?;
                    siblings
                        .iter()
                        .position(|child| *child == node)
                        .and_then(|index| siblings.get(index + 1).copied())
                };
                reference_wrapper = next
                    .map(|node| wrap_node(scope, &source, node))
                    .transpose()?;
            }
            let preparation =
                script_children_changed::parent_change(scope, &source, Some(old_parent))?;
            if Rc::ptr_eq(&source.dom, &destination.dom) && replaced == Some(node) {
                replacement_removed = true;
            }
            let result = source.dom.borrow_mut().remove_child(old_parent, node);
            preparation.finish_mutation(scope, result)?;
            continue;
        }
        let node = if Rc::ptr_eq(&source.dom, &destination.dom) {
            node
        } else {
            let document = owner_document_for_node(&destination, parent)?;
            adopt(scope, &destination, document, &source, node)?
        };
        return Ok(InsertionArgument {
            owner: destination,
            parent,
            node,
            reference,
            replaced,
            replacement_removed,
        });
    }
    Err(V8BindingError::Allocation(
        "script children-change reentrancy bound",
    ))
}

pub(super) fn validate_foreign_replacement(
    destination: &V8BindingState,
    parent: NodeId,
    source: &V8BindingState,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let target = destination.dom.borrow();
    let origin = source.dom.borrow();
    let document = matches!(target.arena().get(parent)?.kind, NodeKind::Document);
    if !matches!(
        target.arena().get(parent)?.kind,
        NodeKind::Document
            | NodeKind::Element { .. }
            | NodeKind::ShadowRoot { .. }
            | NodeKind::DocumentFragment
    ) {
        return Err(hierarchy_request_error());
    }
    let incoming = if matches!(origin.arena().get(node)?.kind, NodeKind::DocumentFragment) {
        light_children(&origin, node)?
    } else {
        vec![node]
    };
    let (mut elements, mut doctypes, mut saw_element) = (0, 0, false);
    for node in incoming {
        match origin.arena().get(node)?.kind {
            NodeKind::Document | NodeKind::Attribute { .. } | NodeKind::ShadowRoot { .. } => {
                return Err(hierarchy_request_error());
            }
            NodeKind::Element { .. } if document => {
                elements += 1;
                saw_element = true;
            }
            NodeKind::DocumentType { .. } => {
                if !document || saw_element {
                    return Err(hierarchy_request_error());
                }
                doctypes += 1;
            }
            NodeKind::Text { .. } | NodeKind::CDataSection { .. } if document => {
                return Err(hierarchy_request_error());
            }
            _ => {}
        }
    }
    if elements > 1 || doctypes > 1 {
        return Err(hierarchy_request_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_dom::html;
    #[test]
    fn same_arena_adoption_retains_each_publication_tail_after_native_commit() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<main></main>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(112), 1).unwrap();
        for stage in 0..4 {
            let script=v8::String::new(scope,"(()=>{const target=document.implementation.createHTMLDocument('target');const node=document.createElement('video');node.setAttribute('style','color:red');const track=document.createElement('track');node.append(track);document.body.append(node);return [target,node,track,node.textTracks,track.track]})()").unwrap();
            let values = v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let values = v8::Local::<v8::Array>::try_from(values).unwrap();
            let target = values.get_index(scope, 0).unwrap();
            let object = values.get_index(scope, 1).unwrap();
            let child = values.get_index(scope, 2).unwrap();
            let (destination, document) = node_identity::argument(scope, target).unwrap().unwrap();
            let (source, node) = node_identity::argument(scope, object).unwrap().unwrap();
            let (_, child_id) = node_identity::argument(scope, child).unwrap().unwrap();
            assert!(
                Rc::ptr_eq(&source.dom, &destination.dom),
                "fixture must exercise same-arena documents"
            );
            state
                .dom
                .borrow_mut()
                .arena_mut()
                .get_mut(node)
                .unwrap()
                .style_attribute_cssom = true;
            FAIL_SAME_PUBLICATION_STAGE.with(|fault| fault.set(Some(stage)));
            assert!(adopt(scope, &destination, document, &source, node).is_err());
            assert_eq!(
                state.dom.borrow().arena().node_document(node).unwrap(),
                Some(document)
            );
            assert_eq!(
                state.dom.borrow().arena().node_document(child_id).unwrap(),
                Some(document)
            );
            assert!(has_publications(scope));
            // At stage 3 stylesheet publication already completed. A retry must
            // execute only the animation tail rather than reset stylesheet state.
            if stage == 3 {
                state
                    .dom
                    .borrow_mut()
                    .arena_mut()
                    .get_mut(node)
                    .unwrap()
                    .style_attribute_cssom = true;
            }
            resume_publications(scope).unwrap();
            assert!(!has_publications(scope));
            assert_eq!(
                state
                    .dom
                    .borrow()
                    .arena()
                    .get(node)
                    .unwrap()
                    .style_attribute_cssom,
                stage == 3
            );
            assert_eq!(
                node_identity::argument(scope, object).unwrap().unwrap().1,
                node
            );
            assert_eq!(
                node_identity::argument(scope, child).unwrap().unwrap().1,
                child_id
            );
            // This is the parser's retry shape: current document now matches,
            // so no second adoption is needed after the publication checkpoint.
            resume_publications(scope).unwrap();
            assert!(!has_publications(scope));
        }
    }
    #[test]
    fn cross_arena_adoption_preserves_objects_and_consumers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (source, document) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                html::parse("<main></main>").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(110), 1).unwrap();
            let document = wrap_node(scope, &state, state.document_root()).unwrap();
            (state, v8::Global::new(scope, document))
        };
        let second = v8::Context::new(scope, Default::default());
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                html::parse("<main></main>").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(111), 1).unwrap();
            let object = wrap_node(scope, &state, state.document_root()).unwrap();
            v8::Global::new(scope, object)
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let target = v8::Local::new(scope, &target);
        let key = v8::String::new(scope, "targetDocument").unwrap();
        first.global(scope).set(scope, key.into(), target.into());
        let script=v8::String::new(scope,r#"
            const check=(x,m)=>{if(!x)throw Error(m)};
            const node=document.createElement('section'),text=document.createTextNode('payload');node.append(text);document.body.append(node);
            node.marker={original:true};const proto=Object.getPrototypeOf(node),marker=node.marker,children=node.childNodes,attributes=node.attributes;
            node.setAttribute('data-label','value');const attr=node.getAttributeNode('data-label'),dataset=node.dataset;
            const snapshot=document.querySelectorAll('section'),range=document.createRange();range.selectNodeContents(node);
            const walker=document.createTreeWalker(node);walker.firstChild();
            const observer=new MutationObserver(()=>{});observer.observe(node,{attributes:true,childList:true,subtree:true});
            const removal=new MutationObserver(()=>{});removal.observe(document.body,{childList:true});
            let clicks=0;node.addEventListener('click',()=>clicks++);node.onclick=()=>clicks++;
            check(targetDocument.adoptNode(node)===node,'same wrapper');
            check(node.ownerDocument===targetDocument&&text.ownerDocument===targetDocument&&node.parentNode===null,'native owner');
            check(Object.getPrototypeOf(node)===proto&&node.marker===marker,'prototype and expando');
            check(node.childNodes===children&&children[0]===text&&snapshot[0]===node,'live and static list identities');
            check(walker.root===node&&walker.currentNode===text&&walker.parentNode()===node,'TreeWalker ownership');
            check(node.attributes===attributes&&attributes[0]===attr&&attr.ownerElement===node&&attr.ownerDocument===targetDocument,'attribute identities');
            check(node.dataset===dataset&&dataset.label==='value','associated wrapper');
            const rows=removal.takeRecords();check(rows.length===1&&rows[0].removedNodes[0]===node,'old source removal');
            node.setAttribute('after','migration');const changed=observer.takeRecords();check(changed.length===1&&changed[0].target===node,'observer follows native node');
            node.dispatchEvent(new Event('click'));check(clicks===2,'listeners and handlers');
            targetDocument.body.appendChild(node);check(targetDocument.querySelector('section')===node,'destination canonical wrapper');
            document.adoptNode(node);document.body.appendChild(node);check(children[0]===text&&snapshot[0]===node&&node.ownerDocument===document,'round trip');
            node.setAttribute('roundtrip','yes');check(observer.takeRecords().some(row=>row.target===node&&row.attributeName==='roundtrip'),'observer repeated move');
            const foreign=targetDocument.createElement('aside');foreign.append(targetDocument.createTextNode('foreign'));
            const copied=document.importNode(foreign,true);check(copied!==foreign&&copied.ownerDocument===document&&copied.textContent==='foreign'&&foreign.ownerDocument===targetDocument,'import clones only');
            check(document.importNode(foreign).childNodes.length===0,'shallow import');
            document.body.appendChild(foreign);check(foreign.ownerDocument===document,'implicit adoption');
            const template=document.createElement('template'),content=template.content;content.append(document.createElement('b'));
            targetDocument.adoptNode(content);check(template.content===content&&content.ownerDocument===targetDocument,'cross-arena template content identity');
            let cycle=false;try{content.appendChild(template)}catch(e){cycle=e.name==='HierarchyRequestError'}check(cycle&&template.ownerDocument===document,'retained template host cycle');
            template.innerHTML='<span>external</span>';check(content.firstChild.textContent==='external'&&template.innerHTML==='<span>external</span>','foreign template markup');const templateCopy=template.cloneNode(true);check(templateCopy.content!==content&&templateCopy.content.firstChild.textContent==='external'&&templateCopy.content.ownerDocument!==targetDocument,'foreign template clone');
            const a=document.createElement('a'),b=document.createElement('b'),owner=document.createElement('div');owner.ariaControlsElements=[a,b];
            document.body.append(owner,a,b);targetDocument.adoptNode(a);check(owner.ariaControlsElements.length===1&&owner.ariaControlsElements[0]===b,'foreign ARIA projection');
            document.adoptNode(a);document.body.append(a);check(owner.ariaControlsElements[0]===a&&owner.ariaControlsElements[1]===b,'retained ARIA order');
            const attribute=targetDocument.createAttribute('foreign');attribute.value='kept';check(node.setAttributeNode(attribute)===null&&attribute.ownerDocument===document&&node.getAttributeNode('foreign')===attribute,'foreign Attr adoption');
            const adjacent=targetDocument.createElement('i');node.insertAdjacentElement('beforeend',adjacent);check(adjacent.parentNode===node&&adjacent.ownerDocument===document,'adjacent adoption');
            const editor=document.createElement('p');editor.append(document.createTextNode('abcd'));document.body.append(editor);const insertion=document.createRange();insertion.setStart(editor.firstChild,2);insertion.collapse(true);const emphasis=targetDocument.createElement('em');emphasis.textContent='X';insertion.insertNode(emphasis);check(editor.textContent==='abXcd'&&emphasis.ownerDocument===document&&insertion.endContainer===editor&&insertion.endOffset===2,'Range split and adoption');
            const detached=document.createTextNode('detached'),unmoved=targetDocument.createElement('i');detached.before(unmoved);check(unmoved.ownerDocument===targetDocument,'detached ChildNode no adoption');
            const forbidden=targetDocument.createTextNode('no');targetDocument.body.append(forbidden);let rejected=false;try{document.appendChild(forbidden)}catch(e){rejected=e.name==='HierarchyRequestError'}
            check(rejected&&forbidden.parentNode===targetDocument.body&&forbidden.ownerDocument===targetDocument,'foreign validation before removal');
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let code=v8::String::new(scope,"(()=>{const node=document.createElement('aside');globalThis.failedMigrationRange=document.createRange();failedMigrationRange.selectNodeContents(node);return node})()").unwrap();
        let object = v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let (_, node) = node_identity::argument(scope, object).unwrap().unwrap();
        let destination = binding_state_for_receiver(scope, target).unwrap();
        FAIL_PUBLICATION_ONCE.with(|flag| flag.set(true));
        let failure = adopt(
            scope,
            &destination,
            destination.document_root(),
            &source,
            node,
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            V8BindingError::Allocation("injected wrapper publication failure")
        ));
        assert!(has_publications(scope));
        custom_elements::refresh_native_registry_roots(scope).unwrap();
        let (failed_pin_owner, failed_pin) = failed_abandoned_pin();
        assert!(matches!(
            resume_publications(scope),
            Err(V8BindingError::Dom(DomError::Cycle))
        ));
        assert!(
            !has_publications(scope),
            "completed migration must not be replayed after a pin release error"
        );
        assert_eq!(
            failed_pin_owner
                .borrow()
                .native_root_count(failed_pin)
                .unwrap(),
            0
        );
        failed_pin_owner
            .borrow_mut()
            .arena_mut()
            .get_mut(failed_pin)
            .unwrap()
            .parent = None;
        resume_publications(scope).unwrap();
        assert!(!has_publications(scope));
        let (owner, moved) = node_identity::argument(scope, object).unwrap().unwrap();
        assert!(Rc::ptr_eq(&owner.dom, &destination.dom));
        assert!(owner.dom.borrow().arena().get(moved).is_ok());
        let retained = v8::String::new(scope, "failedMigrationRange").unwrap();
        let retained = v8::Script::compile(scope, retained, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .try_cast::<v8::Object>()
            .unwrap();
        let (range_owner, range_id) = range_boundaries::owner(scope, retained).unwrap();
        assert!(Rc::ptr_eq(&range_owner.dom, &destination.dom));
        assert_eq!(
            destination
                .dom
                .borrow()
                .live_range(LiveRangeId(range_id))
                .unwrap()
                .start
                .container,
            moved
        );
        custom_elements::refresh_native_registry_roots(scope).unwrap();
        drop(document);
        drop(source);
    }
}

#[cfg(test)]
fn failed_abandoned_pin() -> (Rc<RefCell<DomRuntime>>, NodeId) {
    let source = Rc::new(RefCell::new(DomRuntime::new()));
    let destination = Rc::new(RefCell::new(DomRuntime::new()));
    let node = source.borrow_mut().create_text("abandoned");
    let document = destination.borrow().document();
    let prepared =
        DomRuntime::prepare_subtree_migration(&source, &destination, node, document).unwrap();
    source
        .borrow_mut()
        .arena_mut()
        .get_mut(node)
        .unwrap()
        .parent = Some(node);
    drop(prepared);
    (source, node)
}
#[cfg(test)]
mod pin_drain_tests {
    use super::*;
    #[test]
    fn empty_publication_checkpoint_surfaces_committed_pin_release_error_once() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (source, node) = failed_abandoned_pin();
        assert!(matches!(
            resume_publications(scope),
            Err(V8BindingError::Dom(DomError::Cycle))
        ));
        assert_eq!(source.borrow().native_root_count(node).unwrap(), 0);
        source
            .borrow_mut()
            .arena_mut()
            .get_mut(node)
            .unwrap()
            .parent = None;
        resume_publications(scope).unwrap();
        assert_eq!(source.borrow().native_root_count(node).unwrap(), 0);
        source.borrow_mut().retain_native(node).unwrap();
        source.borrow_mut().release_native(node).unwrap();
        assert!(source.borrow().arena().get(node).is_err());
    }
}
