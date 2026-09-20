//! Native InterestEvent payloads and interest invocation lifecycle.
use super::*;
use native_platform::publication::{Notification, pending_dom_dispatch};
use std::time::{Duration, Instant};

const MAX_INTERESTS: usize = 4096;
#[derive(Clone, Copy)]
struct Job {
    source: NodeId,
    target: NodeId,
    gain: bool,
    cancelable: bool,
    due: Instant,
}
struct Transition {
    skip_popover: bool,
    job: Job,
    event: Option<v8::Global<v8::Object>>,
    dispatch: Option<Notification>,
    accepted: Option<bool>,
}
#[derive(Default)]
pub(super) struct Registry {
    epoch: Option<(NodeId, u64, u64)>,
    observed: oryn_dom::MutationCursor,
    hovered: Option<NodeId>,
    active: Vec<(NodeId, NodeId)>,
    active_leases: BTreeMap<NodeId, (oryn_dom::TrackedNodeLease, oryn_dom::TrackedNodeLease)>,
    jobs: VecDeque<Job>,
    desired: BTreeSet<NodeId>,
    input_targets: BTreeMap<NodeId, Option<NodeId>>,
    suppressed: BTreeSet<NodeId>,
    loss_attempted: BTreeSet<NodeId>,
    sticky: BTreeSet<NodeId>,
    pseudo_down: BTreeMap<i32, NodeId>,
    transition: Option<Transition>,
    retry: Option<Instant>,
    publishing: bool,
}

fn within(dom: &DomRuntime, node: Option<NodeId>, ancestor: NodeId) -> bool {
    let mut node = node;
    let mut seen = BTreeSet::new();
    while let Some(current) = node.filter(|node| seen.insert(*node)) {
        if current == ancestor {
            return true;
        }
        node = dom.flat_tree_parent(current).ok().flatten();
    }
    false
}
fn delay_for(state: &V8BindingState, node: NodeId, gain: bool) -> Duration {
    let layout = state.layout.borrow();
    let value = layout.styles.get(&node_key(node)).and_then(|style| {
        style.get(if gain {
            "interest-delay-start"
        } else {
            "interest-delay-end"
        })
    });
    let seconds = value
        .and_then(|value| value.strip_suffix("s"))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.)
        .unwrap_or(0.5);
    Duration::from_secs_f64(seconds.min(86400.))
}
fn reconcile(state: &Rc<V8BindingState>) -> Result<(), V8BindingError> {
    let (hovered, active, sticky) = {
        let registry = state.interests.borrow();
        (
            registry.hovered,
            registry.active.clone(),
            registry.sticky.clone(),
        )
    };
    let dom = state.dom.borrow();
    let focused = dom.focused();
    let focused = if focused.and_then(|node| dom.node_document(node).ok().flatten())
        == Some(state.document_root())
    {
        focused
    } else {
        dom.focused_element_in(state.document_root())?
    };
    let mut inputs = BTreeSet::new();
    for mut cursor in [hovered, focused] {
        let mut seen = BTreeSet::new();
        while let Some(node) = cursor.filter(|node| seen.insert(*node)) {
            if dom.node_document(node).ok().flatten() != Some(state.document_root()) {
                break;
            }
            if dom.supports_interest_invocation(node) {
                inputs.insert(node);
            }
            cursor = dom.flat_tree_parent(node).ok().flatten();
        }
    }
    let mut registry = state.interests.borrow_mut();
    registry
        .input_targets
        .retain(|source, _| inputs.contains(source));
    for source in &inputs {
        if !registry.input_targets.contains_key(source) {
            registry
                .input_targets
                .insert(*source, dom.interest_invoker_target(*source)?);
        }
    }
    // A target is captured when a hover/focus cycle starts. Attribute changes or
    // reinserting a missing target cannot manufacture a new input cycle.
    let mut desired = BTreeMap::new();
    for source in &inputs {
        if let Some(target) = registry.input_targets.get(source).copied().flatten()
            && dom.interest_invoker_target(*source)? == Some(target)
        {
            desired.insert(*source, target);
        }
    }
    for source in sticky {
        if let Some(target) = dom.interest_invoker_target(source)? {
            desired.insert(source, target);
        }
    }
    // Interest chains keep outer invokers alive while an inner target contains
    // the native hover/focus point, even when targets are not DOM descendants.
    let mut retained = desired.keys().copied().collect::<BTreeSet<_>>();
    for (source, target) in &active {
        if within(&dom, hovered, *target) || within(&dom, focused, *target) {
            retained.insert(*source);
        }
    }
    loop {
        let mut changed = false;
        for (source, target) in &active {
            if !retained.contains(source)
                && retained
                    .iter()
                    .any(|inner| within(&dom, Some(*inner), *target))
            {
                changed |= retained.insert(*source);
            }
        }
        if !changed {
            break;
        }
    }
    for (source, target) in &active {
        if retained.contains(source)
            && dom.interest_invoker_target(*source).ok().flatten() == Some(*target)
        {
            desired.insert(*source, *target);
        }
    }
    registry
        .suppressed
        .retain(|source| inputs.contains(source) || desired.contains_key(source));
    registry
        .loss_attempted
        .retain(|source| !desired.contains_key(source));
    registry.jobs.retain(|job| {
        if job.gain {
            desired.get(&job.source) == Some(&job.target)
        } else {
            desired.get(&job.source) != Some(&job.target) || !job.cancelable
        }
    });
    for (source, target) in &active {
        let valid = dom.node_document(*source).ok().flatten() == Some(state.document_root())
            && dom.interest_invoker_target(*source).ok().flatten() == Some(*target);
        if (desired.get(source) != Some(target) || !valid)
            && (!valid || !registry.loss_attempted.contains(source))
            && !registry
                .jobs
                .iter()
                .any(|job| job.source == *source && !job.gain)
            && registry
                .transition
                .as_ref()
                .is_none_or(|transition| transition.job.source != *source)
        {
            registry.jobs.push_back(Job {
                source: *source,
                target: *target,
                gain: false,
                cancelable: valid,
                due: Instant::now()
                    + if valid {
                        delay_for(state, *source, false)
                    } else {
                        Duration::ZERO
                    },
            });
        }
    }
    for (source, target) in &desired {
        if !active.iter().any(|entry| entry.0 == *source)
            && !registry.suppressed.contains(source)
            && !registry.jobs.iter().any(|job| job.source == *source)
            && registry
                .transition
                .as_ref()
                .is_none_or(|transition| transition.job.source != *source)
        {
            if registry.jobs.len() + active.len() >= MAX_INTERESTS {
                return Err(V8BindingError::Allocation("interest transition limit"));
            }
            registry.jobs.push_back(Job {
                source: *source,
                target: *target,
                gain: true,
                cancelable: true,
                due: Instant::now() + delay_for(state, *source, true),
            });
        }
    }
    registry.desired = desired.into_keys().collect();
    Ok(())
}

pub(super) fn delay(state: &V8BindingState) -> Option<Duration> {
    let registry = state.interests.borrow();
    if registry.publishing {
        return None;
    }
    let now = Instant::now();
    if registry.retry.is_some_and(|deadline| deadline > now) {
        return registry
            .retry
            .map(|deadline| deadline.saturating_duration_since(now));
    }
    if registry.observed != state.dom.borrow().mutation_cursor() {
        return Some(Duration::ZERO);
    }
    if registry.transition.is_some() {
        return Some(registry.retry.unwrap_or(now).saturating_duration_since(now));
    }
    registry
        .jobs
        .iter()
        .map(|job| job.due.saturating_duration_since(now))
        .min()
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    delay(state).is_some_and(|delay| delay.is_zero())
}
pub(super) fn pointer(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: Option<NodeId>,
) -> Result<(), V8BindingError> {
    state.interests.borrow_mut().hovered = target;
    poll(scope, state).map(|_| ())
}
pub(super) fn escape(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    {
        let mut registry = state.interests.borrow_mut();
        let active = registry.active.clone();
        if active.is_empty() {
            return Ok(false);
        }
        registry.jobs.clear();
        registry.sticky.clear();
        let desired = registry.desired.clone();
        registry.suppressed.extend(desired);
        for (source, target) in active.into_iter().rev() {
            registry.jobs.push_back(Job {
                source,
                target,
                gain: false,
                cancelable: false,
                due: Instant::now(),
            });
        }
    }
    poll(scope, state)?;
    Ok(true)
}

fn pseudo_hit(state: &V8BindingState, source: NodeId, x: f64, y: f64) -> bool {
    let layout = state.layout.borrow();
    layout.generated_nodes.iter().rev().any(|node|{
        node.tag=="::interest-button"&&node.parent_id==Some(node_key(source))
            &&!node.style.get("pointer-events").is_some_and(|value|value=="none")
            &&!node.style.get("visibility").is_some_and(|value|matches!(value.as_str(),"hidden"|"collapse"))
            &&layout.hit_projections.get(&node.node_id).map(|projection|projection.contains_point(x,y)).unwrap_or_else(||matches!(layout.geometry.get(&node.node_id),Some(BindingGeometry::Available(rect))if rect.width>0.&&rect.height>0.&&x>=rect.x&&y>=rect.y&&x<=rect.x+rect.width&&y<=rect.y+rect.height))
    })
}
pub(super) fn admit_pointer_down(
    state: &Rc<V8BindingState>,
    id: i32,
    target: NodeId,
    x: f64,
    y: f64,
) -> Result<(), V8BindingError> {
    if pseudo_hit(state, target, x, y) {
        let mut registry = state.interests.borrow_mut();
        if registry.pseudo_down.len() >= MAX_INTERESTS && !registry.pseudo_down.contains_key(&id) {
            return Err(V8BindingError::Allocation("interest pointer limit"));
        }
        registry.pseudo_down.insert(id, target);
    } else {
        state.interests.borrow_mut().pseudo_down.remove(&id);
    }
    let active = state.interests.borrow().active.clone();
    let outside = active
        .into_iter()
        .filter(|(source, interestee)| {
            let dom = state.dom.borrow();
            !within(&dom, Some(target), *source) && !within(&dom, Some(target), *interestee)
        })
        .map(|entry| entry.0)
        .collect::<Vec<_>>();
    {
        let mut registry = state.interests.borrow_mut();
        for source in outside {
            registry.sticky.remove(&source);
        }
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn pointer_down(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: i32,
    target: NodeId,
    x: f64,
    y: f64,
) -> Result<(), V8BindingError> {
    admit_pointer_down(state, id, target, x, y)?;
    poll(scope, state)?;
    Ok(())
}
pub(super) fn cancel_pointer(state: &V8BindingState, id: i32) {
    state.interests.borrow_mut().pseudo_down.remove(&id);
}
pub(super) fn mark_pseudo_click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event: v8::Local<v8::Object>,
    id: i32,
    target: NodeId,
    x: f64,
    y: f64,
) -> Result<(), V8BindingError> {
    let source = state.interests.borrow().pseudo_down.get(&id).copied();
    if source != Some(target) || !pseudo_hit(state, target, x, y) {
        state.interests.borrow_mut().pseudo_down.remove(&id);
        return Ok(());
    }
    let name = v8::String::new(scope, "oryn.interest.pseudoActivation")
        .ok_or(V8BindingError::Allocation("interest activation key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let target = wrap_node(scope, state, target)?;
    event
        .set_private(scope, key, target.into())
        .filter(|set| *set)
        .ok_or(V8BindingError::Allocation("interest activation source"))?;
    state.interests.borrow_mut().pseudo_down.remove(&id);
    Ok(())
}
pub(super) fn pseudo_activation(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
) -> Option<NodeId> {
    let name = v8::String::new(scope, "oryn.interest.pseudoActivation")?;
    let key = v8::Private::for_api(scope, Some(name));
    let source = event
        .get_private(scope, key)?
        .try_cast::<v8::Object>()
        .ok()?;
    event.delete_private(scope, key);
    embedded_node_identity(scope, source).map(|identity| identity.0)
}
pub(super) fn admit_activation(
    state: &Rc<V8BindingState>,
    source: NodeId,
) -> Result<(), V8BindingError> {
    let Some(target) = state.dom.borrow().interest_invoker_target(source)? else {
        return Ok(());
    };
    {
        let mut registry = state.interests.borrow_mut();
        let active = registry.active.iter().any(|entry| entry.0 == source);
        if !active
            && registry
                .jobs
                .iter()
                .filter(|job| job.source != source)
                .count()
                + registry.active.len()
                >= MAX_INTERESTS
        {
            return Err(V8BindingError::Allocation("interest transition limit"));
        }
        registry.sticky.insert(source);
        registry.suppressed.remove(&source);
        registry.jobs.retain(|job| job.source != source);
        if !active {
            registry.jobs.push_back(Job {
                source,
                target,
                gain: true,
                cancelable: true,
                due: Instant::now(),
            });
        }
    }

    Ok(())
}
pub(super) fn activate(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: NodeId,
) -> Result<(), V8BindingError> {
    admit_activation(state, source)?;
    poll(scope, state)?;
    Ok(())
}
fn publish_selectors(state: &V8BindingState, changed: [NodeId; 2]) -> Result<(), V8BindingError> {
    let active = state.interests.borrow().active.clone();
    let mut dom = state.dom.borrow_mut();
    let mut nodes = dom.arena().descendants_inclusive(state.document_root())?;
    nodes.extend(changed);
    for node in nodes {
        if dom.node_document(node).ok().flatten() == Some(state.document_root()) {
            dom.set_interest_selector_state(
                node,
                active.iter().any(|entry| entry.0 == node),
                active.iter().any(|entry| entry.1 == node),
            )?;
        }
    }
    state.layout_cache_key.borrow_mut().take();
    Ok(())
}
fn advance(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    transition: &mut Transition,
) -> Result<bool, V8BindingError> {
    let job = transition.job;
    if transition.accepted.is_none() {
        if transition.dispatch.is_none() {
            if state.dom.borrow().node_document(job.target).ok().flatten()
                != Some(state.document_root())
                || !state.dom.borrow().is_connected(job.target).unwrap_or(false)
            {
                transition.accepted = Some(!job.gain);
            } else {
                let name = if job.gain { "interest" } else { "loseinterest" };
                let event = create_event_object_with_interface(
                    scope,
                    state,
                    "InterestEvent",
                    name,
                    false,
                    job.cancelable,
                )?;
                let key =
                    marker(scope).ok_or(V8BindingError::Allocation("InterestEvent source key"))?;
                let source = wrap_node(scope, state, job.source)?;
                event
                    .set_private(scope, key, source.into())
                    .filter(|set| *set)
                    .ok_or(V8BindingError::Allocation("InterestEvent source"))?;
                let id = event_id(scope, event)?;
                if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                    record.is_trusted = true;
                    record.composed = true;
                }
                let target = wrap_node(scope, state, job.target)?;
                transition.dispatch = Some(pending_dom_dispatch(scope, state, target, name, event));
                transition.event = Some(v8::Global::new(scope, event));
            }
        }
        if let Some(dispatch) = transition.dispatch.as_mut() {
            if !dispatch.resume(scope, state)? {
                return Ok(false);
            }
            let event = v8::Local::new(scope, transition.event.as_ref().unwrap());
            let id = event_id(scope, event)?;
            transition.accepted = Some(
                !state
                    .events
                    .borrow()
                    .get(&id)
                    .is_some_and(|record| record.default_prevented),
            );
            dispatch.finish(scope, state);
            transition.dispatch = None;
        }
    }
    if state.execution_retired() {
        return Ok(true);
    }
    if state.interests.borrow().epoch.is_some_and(|epoch| {
        epoch
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            )
    }) {
        retire(state);
        return Ok(true);
    }
    if !transition.accepted.unwrap_or(false) {
        let mut registry = state.interests.borrow_mut();
        if job.gain {
            registry.suppressed.insert(job.source);
        } else {
            registry.loss_attempted.insert(job.source);
        }
        return Ok(true);
    }
    if job.gain {
        if state
            .dom
            .borrow()
            .interest_invoker_target(job.source)
            .ok()
            .flatten()
            != Some(job.target)
        {
            return Ok(true);
        }
        let mut registry = state.interests.borrow_mut();
        if !registry.active.iter().any(|entry| entry.0 == job.source) {
            let source = oryn_dom::TrackedNodeLease::new(&state.dom, job.source)?;
            let target = oryn_dom::TrackedNodeLease::new(&state.dom, job.target)?;
            registry.active_leases.insert(job.source, (source, target));
            registry.active.push((job.source, job.target));
        }
    } else {
        state
            .interests
            .borrow_mut()
            .active
            .retain(|entry| entry.0 != job.source);
    }
    publish_selectors(state, [job.source, job.target])?;
    if !transition.skip_popover
        && state.dom.borrow().is_connected(job.target).unwrap_or(false)
        && state
            .dom
            .borrow()
            .attribute_in_null_namespace(job.target, "popover")?
            .is_some()
    {
        top_layer::toggle(scope, state, job.target, Some(job.gain), Some(job.source))?;
    }
    if !job.gain {
        state
            .interests
            .borrow_mut()
            .active_leases
            .remove(&job.source);
    }
    Ok(true)
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired()
        || state.execution_tasks_stopped()
        || state.interests.borrow().publishing
    {
        return Ok(0);
    }
    let epoch = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    if state
        .interests
        .borrow()
        .epoch
        .is_some_and(|previous| previous != epoch)
    {
        retire(state);
    }
    state.interests.borrow_mut().epoch = Some(epoch);
    refresh_layout(state)?;
    reconcile(state)?;
    if state
        .interests
        .borrow()
        .retry
        .is_some_and(|deadline| deadline > Instant::now())
    {
        return Ok(0);
    }
    state.interests.borrow_mut().publishing = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.interests.borrow_mut().publishing = false;
        }
    }
    let _guard = Guard(state);
    let mut count = 0;
    loop {
        let transition = {
            let mut registry = state.interests.borrow_mut();
            registry.transition.take().or_else(|| {
                let index = registry
                    .jobs
                    .iter()
                    .position(|job| job.due <= Instant::now())?;
                let job = registry.jobs.remove(index)?;
                Some(Transition {
                    skip_popover: false,
                    job,
                    event: None,
                    dispatch: None,
                    accepted: None,
                })
            })
        };
        let Some(mut transition) = transition else {
            break;
        };
        match advance(scope, state, &mut transition) {
            Ok(true) => {
                state.interests.borrow_mut().retry = None;
                count += 1;
            }
            result => {
                let mut registry = state.interests.borrow_mut();
                registry.transition = Some(transition);
                registry.retry = Some(Instant::now() + Duration::from_millis(25));
                return result.map(|_| count);
            }
        }
        if state.execution_retired() || state.execution_tasks_stopped() {
            break;
        }
        reconcile(state)?;
    }
    state.interests.borrow_mut().observed = state.dom.borrow().mutation_cursor();
    Ok(count)
}

/// A popover owns this loss continuation while it closes. Cancellation preserves
/// interest state but does not cancel the independently requested popover hide.
pub(super) struct Loss {
    jobs: VecDeque<Job>,
    current: Option<Transition>,
}
impl Loss {
    pub(super) fn new(state: &V8BindingState, target: NodeId) -> Self {
        let jobs = state
            .interests
            .borrow()
            .active
            .iter()
            .rev()
            .filter(|entry| entry.1 == target)
            .map(|(source, target)| Job {
                source: *source,
                target: *target,
                gain: false,
                cancelable: true,
                due: Instant::now(),
            })
            .collect();
        Self {
            jobs,
            current: None,
        }
    }
    pub(super) fn cancel(&mut self, state: &V8BindingState) {
        if let Some(current) = self.current.take()
            && let Some(dispatch) = current.dispatch
        {
            dispatch.cancel(state);
        }
        self.jobs.clear();
    }
    pub(super) fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        let previous = state.interests.borrow().publishing;
        state.interests.borrow_mut().publishing = true;
        struct Guard<'a>(&'a V8BindingState, bool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.interests.borrow_mut().publishing = self.1;
            }
        }
        let _guard = Guard(state, previous);
        loop {
            if self.current.is_none() {
                let Some(job) = self.jobs.pop_front() else {
                    return Ok(true);
                };
                self.current = Some(Transition {
                    skip_popover: true,
                    job,
                    event: None,
                    dispatch: None,
                    accepted: None,
                });
            }
            let current = self.current.as_mut().unwrap();
            if !advance(scope, state, current)? {
                return Ok(false);
            }
            let source = current.job.source;
            state.interests.borrow_mut().suppressed.insert(source);
            state.interests.borrow_mut().sticky.remove(&source);
            self.current = None;
        }
    }
}

pub(super) fn retire(state: &V8BindingState) {
    let previous = std::mem::take(&mut *state.interests.borrow_mut());
    if let Some(transition) = previous.transition
        && let Some(dispatch) = transition.dispatch
    {
        dispatch.cancel(state);
    }
    let mut dom = state.dom.borrow_mut();
    for (source, target) in previous.active {
        if dom.arena().get(source).is_ok() {
            let _ = dom.set_interest_selector_state(source, false, false);
        }
        if dom.arena().get(target).is_ok() {
            let _ = dom.set_interest_selector_state(target, false, false);
        }
    }
}

fn marker<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, "oryn.interest.source")?;
    Some(v8::Private::for_api(scope, Some(name)))
}
pub(super) fn event_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = marker(scope)?;
    event
        .get_private(scope, key)
        .filter(|value| !value.is_undefined())
}
pub(super) fn set_event_source(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    source: v8::Local<v8::Value>,
) -> bool {
    let Some(key) = marker(scope) else {
        return false;
    };
    if event
        .get_private(scope, key)
        .is_none_or(|value| value.is_undefined())
    {
        return false;
    }
    event.set_private(scope, key, source) == Some(true)
}
pub(super) fn initialize_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let source =
        event_init_member(scope, init, "source").ok_or(V8BindingError::JavaScriptException)?;
    let source = if source.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        let object = source.try_cast::<v8::Object>().map_err(|_| {
            throw_type_error(scope, "InterestEvent source must be an Element");
            V8BindingError::JavaScriptException
        })?;
        if resolve_receiver(scope, state, object, "Element").is_err() {
            throw_type_error(scope, "InterestEvent source must be an Element");
            return Err(V8BindingError::JavaScriptException);
        }
        source
    };
    let key = marker(scope).ok_or(V8BindingError::Allocation("InterestEvent source key"))?;
    event
        .set_private(scope, key, source)
        .filter(|set| *set)
        .ok_or(V8BindingError::Allocation("InterestEvent source payload"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    fn expire(state: &V8BindingState) {
        let mut registry = state.interests.borrow_mut();
        for job in &mut registry.jobs {
            job.due = Instant::now();
        }
        registry.retry = None;
    }
    #[test]
    fn native_pseudo_click_is_sticky_and_does_not_activate_its_link() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><a id=s href='/wrong' interestfor=t>link</a><div id=t></div><div id=outside></div></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        let outside = state.dom.borrow().find_by_id("outside").unwrap();
        let pseudo = u64::MAX - 100;
        let snapshot = oryn_common::v4::PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            node_id: pseudo,
            parent_id: Some(node_key(source)),
            tag: "::interest-button".into(),
            geometry: oryn_common::v4::NodeGeometry {
                availability: oryn_common::v4::GeometryAvailability::Available,
                box_model: None,
                client_rects: Vec::new(),
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            },
            projection: None,
            style: BTreeMap::new(),
            text: Vec::new(),
            image_resource: None,
        };
        state.set_layout(BindingLayout {
            generated_nodes: vec![snapshot],
            geometry: BTreeMap::from([(
                pseudo,
                BindingGeometry::Available(BindingRect {
                    x: 150.,
                    y: 10.,
                    width: 50.,
                    height: 50.,
                }),
            )]),
            ..Default::default()
        });
        eval(
            scope,
            "globalThis.clicks=0;document.getElementById('s').addEventListener('click',()=>clicks++);",
        );
        pointer_down(scope, &state, 9, source, 175., 30.).unwrap();
        let event = create_native_input_event(
            scope,
            &state,
            NativeInputEventInputs {
                interface: "PointerEvent",
                event_type: "click",
                bubbles: true,
                cancelable: true,
                composed: true,
                detail: 0,
                related: None,
                fields: &[("button", NativeInputValue::Number(0.))],
            },
        )
        .unwrap();
        mark_pseudo_click(scope, &state, event, 9, source, 175., 30.).unwrap();
        assert!(dispatch_event(scope, &state, source, event).unwrap());
        assert!(pseudo_activation(scope, event).is_none());
        assert!(state.pending_default_actions.borrow().is_empty());
        assert!(
            eval(
                scope,
                "clicks===1&&document.getElementById('t').matches(':interest-target')"
            )
            .is_true()
        );
        pointer(scope, &state, None).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "document.getElementById('t').matches(':interest-target')"
            )
            .is_true()
        );
        pointer_down(scope, &state, 10, outside, 0., 0.).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "!document.getElementById('t').matches(':interest-target')"
            )
            .is_true()
        );
    }
    #[test]
    fn declarative_root_reuse_and_reference_target_mutation_are_native() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=h><template shadowrootmode=open shadowrootreferencetarget=t><span id=t></span></template></div></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"
            const h=document.getElementById('h'),old=h.shadowRoot;
            const reference=old.referenceTarget;const reused=h.attachShadow({mode:'open'});
            const other=document.body.appendChild(document.createElement('div'));const root=other.attachShadow({mode:'open',referenceTarget:'inner'});
            const first=root.referenceTarget;root.referenceTarget=null;const second=root.referenceTarget;root.referenceTarget='';
            reference==='t'&&reused===old&&old.childNodes.length===0&&first==='inner'&&second===null&&root.referenceTarget===''
        "#).is_true());
    }
    #[test]
    fn constructor_native_source_and_shadow_dispatch() {
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
        assert!(eval(scope,r#"
            const host=document.body.appendChild(document.createElement('div'));
            const shadow=host.attachShadow({mode:'closed'}), slot=shadow.appendChild(document.createElement('slot'));
            let inside=0,outside=0;slot.addEventListener('interest',e=>{if(e.source===slot)inside++;});host.addEventListener('interest',()=>outside++);
            const event=new InterestEvent('interest',{source:slot,bubbles:true,composed:true});
            event.__oryn_source=host;slot.dispatchEvent(event);
            let invalid=0;for(const source of [false,true,{},new EventTarget()]){try{new InterestEvent('interest',{source});}catch(error){if(error instanceof TypeError)invalid++;}}
            const defaults=new InterestEvent('interest').source===null, relatedAbsent=event.relatedTarget===undefined;
            if(inside!==1||outside!==0||invalid!==4||!defaults||!relatedAbsent)throw Error(JSON.stringify({inside,outside,invalid,defaults,relatedAbsent}));
            inside===1&&outside===0&&invalid===4&&defaults&&relatedAbsent
        "#).is_true());
        let event=eval(scope,"inside=0;outside=0;new InterestEvent('interest',{source:slot,bubbles:true,composed:true})").try_cast::<v8::Object>().unwrap();
        let target = eval(scope, "slot");
        let (_, target) = node_identity::argument(scope, target).unwrap().unwrap();
        let mut delivery =
            retained_dom_event::Event::new(scope, &state, target, "interest", event).unwrap();
        assert_eq!(delivery.poll(scope, &state).unwrap(), Some(true));
        assert!(eval(scope, "inside===1&&outside===0").is_true());
    }
    #[test]
    fn delays_cancellation_and_escape_keep_native_state_ordered() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><button id=s interestfor=t></button><div id=t></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "s")
            .unwrap()
            .unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.s=document.getElementById('s');globalThis.t=document.getElementById('t');t.addEventListener('interest',e=>calls.push(['gain',s.matches(':interest-source'),e.source===s,e.isTrusted]));t.addEventListener('loseinterest',e=>{calls.push(['lose',s.matches(':interest-source'),e.cancelable]);if(e.cancelable)e.preventDefault();});",
        );
        pointer(scope, &state, Some(source)).unwrap();
        assert!(!ready(&state));
        assert!(delay(&state).unwrap() > Duration::ZERO);
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(eval(scope,"JSON.stringify(calls)==='[[\"gain\",false,true,true]]'&&s.matches(':interest-source')&&t.matches(':interest-target')").is_true());
        pointer(scope, &state, None).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        poll(scope, &state).unwrap();
        assert!(eval(scope, "calls.length===2&&s.matches(':interest-source')").is_true());
        escape(scope, &state).unwrap();
        assert!(eval(scope,"calls.length===3&&calls[2][2]===false&&!s.matches(':interest-source')&&!t.matches(':interest-target')").is_true());
    }
    #[test]
    fn interrupted_listener_dispatch_resumes_once_without_busy_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><button id=s interestfor=t></button><div id=t></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "s")
            .unwrap()
            .unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.t=document.getElementById('t');t.addEventListener('interest',()=>calls.push(1));t.addEventListener('interest',()=>calls.push(2));",
        );
        pointer(scope, &state, Some(source)).unwrap();
        expire(&state);
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        assert!(!ready(&state));
        assert!(delay(&state).unwrap() > Duration::ZERO);
        assert!(eval(scope, "calls.join(',')==='1'").is_true());
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "calls.join(',')==='1,2'&&t.matches(':interest-target')"
            )
            .is_true()
        );
    }
    #[test]
    fn reassignment_and_reinsertion_do_not_create_a_new_hover_cycle() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><button id=s interestfor=t></button><div id=t></div><div id=u></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = state.dom.borrow().find_by_id("s").unwrap();
        eval(
            scope,
            "globalThis.s=document.getElementById('s');globalThis.t=document.getElementById('t');globalThis.u=document.getElementById('u');globalThis.calls=0;t.addEventListener('interest',()=>calls++);u.addEventListener('interest',()=>calls++);",
        );
        pointer(scope, &state, Some(source)).unwrap();
        eval(scope, "s.interestForElement=u");
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(eval(scope, "calls===0").is_true());
        pointer(scope, &state, None).unwrap();
        eval(scope, "s.interestForElement=t;t.remove()");
        pointer(scope, &state, Some(source)).unwrap();
        eval(scope, "document.body.append(t)");
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(eval(scope, "calls===0").is_true());
        pointer(scope, &state, None).unwrap();
        pointer(scope, &state, Some(source)).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(eval(scope, "calls===1").is_true());
    }
    #[test]
    fn nested_targets_keep_transitive_invokers_interested() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><button id=a interestfor=b></button><div id=b><button id=c interestfor=d></button></div><div id=d><span id=e></span></div></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        for id in ["a", "c", "e"] {
            let target = state.dom.borrow().find_by_id(id).unwrap();
            pointer(scope, &state, Some(target)).unwrap();
            expire(&state);
            poll(scope, &state).unwrap();
        }
        assert!(eval(scope,"document.getElementById('a').matches(':interest-source')&&document.getElementById('c').matches(':interest-source')").is_true());
    }
    #[test]
    fn native_focus_in_hosted_document_keeps_embedding_target_interested() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><button id=s interestfor=frame></button><iframe id=frame></iframe><button id=outside></button></body>").into_runtime().unwrap()));
        let frame = dom.borrow().find_by_id("frame").unwrap();
        let source = dom.borrow().find_by_id("s").unwrap();
        let outside = dom.borrow().find_by_id("outside").unwrap();
        let child = oryn_dom::html::parse("<body><button id=inner>child</button></body>");
        let child_document = dom
            .borrow_mut()
            .import_subtree(&child.dom, child.document)
            .unwrap();
        dom.borrow_mut()
            .set_hosted_document(frame, Some(child_document))
            .unwrap();
        let inner = dom
            .borrow()
            .get_element_by_id(child_document, "inner")
            .unwrap()
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.dom.borrow_mut().focus(Some(source)).unwrap();
        poll(scope, &state).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        state.dom.borrow_mut().focus(Some(inner)).unwrap();
        poll(scope, &state).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(state.dom.borrow().element_interest_source(source).unwrap());
        state.dom.borrow_mut().focus(Some(outside)).unwrap();
        poll(scope, &state).unwrap();
        expire(&state);
        poll(scope, &state).unwrap();
        assert!(!state.dom.borrow().element_interest_source(source).unwrap());
    }
}
