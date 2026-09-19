//! Native Navigation API identities, entry state and navigation lifecycle.
//! This module is a child of the binding implementation so no script property
//! is used as an authority or as a substitute for a cached platform wrapper.
#[path = "history_projection.rs"]
mod history_projection;
use super::*;
pub use history_projection::NativeHistoryPublisher;

pub use crate::navigation_history::{
    HistoryScrollMode, HistoryScrollState, NavigationHistorySnapshot, NavigationHistoryUpdate,
};

#[derive(Default, Clone)]
pub struct NativeNavigationSource {
    pub execution_observer: Option<crate::navigation::NavigationExecutionObserver>,
    pub source_element: Option<NodeId>,
    pub source_element_object: Option<v8::Global<v8::Object>>,
    pub source_window: Option<v8::Global<v8::Object>>,
    pub source_origin_key: Option<String>,
    /// Frozen native DOM-access identity; never a Fetch or storage origin.
    pub source_domain_key: Option<String>,
    pub user_initiated: bool,
    pub form_data: Option<v8::Global<v8::Object>>,
    pub download: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeNavigationToken {
    binding: u64,
    generation: u64,
    epoch: u64,
    revision: u64,
    event: Option<u64>,
    destination: Option<String>,
}
pub enum NativeNavigationAdmission {
    Handled,
    Proceed {
        operation: Option<NativeNavigationToken>,
        destination: Option<NavigationHistorySnapshot>,
    },
}

/// A callback may replace a Document stream without retiring its Window.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct AdmissionDocument {
    instance: u64,
    document: NodeId,
    generation: u64,
    stream: u64,
    authority: u64,
}
impl AdmissionDocument {
    pub(super) fn capture(state: &V8BindingState) -> Self {
        Self {
            instance: state.instance_id,
            document: state.document_root(),
            generation: state.document_generation(),
            stream: state.document_stream_epoch(),
            authority: state.document_authority_epoch(),
        }
    }
    pub(super) fn current(self, state: &V8BindingState) -> bool {
        !state.execution_retired() && self == Self::capture(state)
    }
}

struct Entry {
    snapshot: NavigationHistorySnapshot,
    object: v8::Global<v8::Object>,
    platform_id: u64,
    disposed: bool,
}
struct Activation {
    object: v8::Global<v8::Object>,
    entry: String,
    from: Option<String>,
    navigation_type: String,
}
#[derive(Clone)]
struct PendingNavigation {
    execution_observer: Option<crate::navigation::NavigationExecutionObserver>,
    committed: v8::Global<v8::PromiseResolver>,
    finished: v8::Global<v8::PromiseResolver>,
    destination: NavigationHistorySnapshot,
    navigation_type: String,
    event_id: Option<u64>,
    transition: Option<u64>,
    committed_entry: bool,
    remaining: usize,
    plan: Option<CommitPlan>,
}
#[derive(Clone)]
struct CommitPlan {
    base_entries: Vec<NavigationHistorySnapshot>,
    entries: Vec<NavigationHistorySnapshot>,
    current: usize,
    from: String,
    kind: String,
    intercepted: bool,
}
struct PrecommitController {
    object: v8::Global<v8::Object>,
    event: u64,
}
struct Destination {
    object: v8::Global<v8::Object>,
    snapshot: NavigationHistorySnapshot,
    index: i64,
    same_document: bool,
}
struct Transition {
    object: v8::Global<v8::Object>,
    from: String,
    to: u64,
    navigation_type: String,
    committed: v8::Global<v8::PromiseResolver>,
    finished: v8::Global<v8::PromiseResolver>,
}
struct NavigateRecord {
    object: v8::Global<v8::Object>,
    destination: u64,
    destination_object: v8::Global<v8::Object>,
    navigation_type: String,
    can_intercept: bool,
    hash_change: bool,
    user_initiated: bool,
    signal_id: u64,
    signal_object: v8::Global<v8::Object>,
    info: v8::Global<v8::Value>,
    form_data: v8::Global<v8::Value>,
    download: v8::Global<v8::Value>,
    visual_transition: bool,
    source_element: v8::Global<v8::Value>,
    intercepted: bool,
    handlers: Vec<v8::Global<v8::Function>>,
    precommit_handlers: Vec<v8::Global<v8::Function>>,
    focus_reset: String,
    focus_revision: u64,
    scroll: String,
    scrolled: bool,
    committed: bool,
    trusted: bool,
    admission_complete: bool,
    admission_failed: bool,
}

#[derive(Default)]
pub(super) struct NavigationState {
    history_projection: history_projection::State,
    revision: u64,
    detaching: bool,
    detach_busy: bool,
    native_source: Option<NativeNavigationSource>,
    object: Option<v8::Global<v8::Object>>,
    platform_id: Option<u64>,
    entries: BTreeMap<String, Entry>,
    order: Vec<String>,
    current: Option<String>,
    activations: BTreeMap<u64, Activation>,
    activation: Option<u64>,
    pending: Option<PendingNavigation>,
    history_update: Option<NavigationHistoryUpdate>,
    history_values: BTreeMap<String, v8::Global<v8::Value>>,
    initialized: bool,
    initial_document: bool,
    joint_initial_replacement: bool,
    destinations: BTreeMap<u64, Destination>,
    transitions: BTreeMap<u64, Transition>,
    events: BTreeMap<u64, NavigateRecord>,
    controllers: BTreeMap<u64, PrecommitController>,
    staged_swap: Option<NavigationHistorySnapshot>,
    staged_swap_activation: Option<(String, String, Option<String>, u64)>,
    finishing_transition: Option<u64>,
    load_restore: Option<(AdmissionDocument, NavigationHistorySnapshot)>,
    load_restore_is_traversal: bool,
    joint_tasks: std::collections::VecDeque<JointTraversal>,
    hash_tasks: std::collections::VecDeque<HistoryPublication>,
    intercepted_history_barriers: BTreeMap<u64, Rc<Cell<u8>>>,
    growing_scroll: Option<(ScrollRestore, std::time::Instant)>,
    restoring_scroll: bool,
    publications: std::collections::VecDeque<HistoryPublication>,
    publication_retry_at: Option<std::time::Instant>,
    publication_depth: usize,
    native_receipts: BTreeMap<u64, bool>,
    parser_receipts: BTreeMap<u64, u64>,
    next_native_receipt: u64,
    queued_finishes: BTreeSet<u64>,
    continuation_requests: BTreeSet<(u64, bool)>,
    pending_rejections: BTreeMap<u64, v8::Global<v8::Value>>,
}
struct CommitSettlement {
    event: u64,
    entry: v8::Global<v8::Object>,
    pending: PendingNavigation,
    transition: Option<v8::Global<v8::PromiseResolver>>,
    committed: bool,
    transition_committed: bool,
}
#[derive(Clone)]
struct ScrollRestore {
    honor_manual: bool,
    document: AdmissionDocument,
    entry: String,
    position: Option<(f64, f64)>,
    after_events: usize,
}
impl ScrollRestore {
    fn allowed(&self, state: &V8BindingState) -> bool {
        if !self.document.current(state) {
            return false;
        }
        let navigation = state.navigation_api.borrow();
        navigation.current.as_ref() == Some(&self.entry)
            && navigation.entries.get(&self.entry).is_some_and(|entry| {
                (!self.honor_manual || entry.snapshot.scroll.mode == HistoryScrollMode::Auto)
            })
    }
    fn apply(&self, state: &V8BindingState) -> Result<(), V8BindingError> {
        if !self.allowed(state) {
            return Ok(());
        }
        let previous = state.navigation_api.borrow().restoring_scroll;
        state.navigation_api.borrow_mut().restoring_scroll = true;
        let result: Result<(), V8BindingError> = (|| {
            if let Some((x, y)) = self.position {
                let root = {
                    let dom = state.dom.borrow();
                    document_scrolling_element(&dom, &state.layout.borrow(), state.document_root())
                };
                if let Some(root) = root {
                    perform_scroll(state, root, (Some(x), Some(y)), false)?;
                }
            } else if state
                .navigation_history_snapshot()
                .and_then(|entry| Url::parse(&entry.url).ok())
                .is_some_and(|url| url.fragment().is_some())
            {
                perform_navigation_scroll(state)?;
            }
            Ok(())
        })();
        state.navigation_api.borrow_mut().restoring_scroll = previous;
        result?;
        // Callback scrolling may have updated the current entry while this
        // receipt retained its original target. Persist the restored target.
        if let Some(position) = self.position {
            let changed = {
                let mut navigation = state.navigation_api.borrow_mut();
                navigation
                    .entries
                    .get_mut(&self.entry)
                    .is_some_and(|entry| {
                        let changed = entry.snapshot.scroll.position != Some(position);
                        entry.snapshot.scroll.position = Some(position);
                        changed
                    })
            };
            if changed {
                publish_history_update(state, "replace");
            }
        }
        if self
            .position
            .is_some_and(|position| position != state.document_scroll_offset())
        {
            let mut navigation = state.navigation_api.borrow_mut();
            let deadline = navigation
                .growing_scroll
                .as_ref()
                .filter(|(old, _)| old.entry == self.entry && old.document == self.document)
                .map(|(_, deadline)| *deadline)
                .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(5));
            navigation.growing_scroll = Some((self.clone(), deadline));
        } else {
            state.navigation_api.borrow_mut().growing_scroll = None;
        }
        Ok(())
    }
}
pub(super) fn cancel_scroll_restoration(state: &V8BindingState) {
    let mut navigation = state.navigation_api.borrow_mut();
    if !navigation.restoring_scroll {
        navigation.growing_scroll = None;
    }
}
pub(super) fn retry_scroll_restoration(state: &V8BindingState) -> Result<(), V8BindingError> {
    let pending = state.navigation_api.borrow().growing_scroll.clone();
    let Some((restore, deadline)) = pending else {
        return Ok(());
    };
    if std::time::Instant::now() >= deadline || !restore.allowed(state) {
        state.navigation_api.borrow_mut().growing_scroll = None;
        return Ok(());
    }
    restore.apply(state)
}
pub(super) fn record_layout_scroll(state: &V8BindingState, position: (f64, f64)) {
    if state.navigation_api.borrow().growing_scroll.is_none() {
        record_document_scroll(state, position);
    }
}
struct HistoryPublication {
    execution_observer: Option<(
        AdmissionDocument,
        crate::navigation::NavigationExecutionObserver,
    )>,
    deferred_hash: Option<native_platform::publication::Notification>,
    scroll_restore: Option<ScrollRestore>,
    owner: RcWeak<V8BindingState>,
    generation: u64,
    epoch: u64,
    settlement: Option<CommitSettlement>,
    completion: Option<Completion>,
    receipt: Option<u64>,
    lifecycle_complete: Option<(u64, bool)>,
    joint_finish: Option<(String, u64)>,
    joint_barrier: Option<(Rc<Cell<u8>>, u8)>,
    script: Option<ScriptPublication>,
    admission: Option<Box<AdmissionPublication>>,
    events: std::collections::VecDeque<native_platform::publication::Notification>,
}
impl Drop for HistoryPublication {
    fn drop(&mut self) {
        if let Some((_, observer)) = self.execution_observer.take() {
            observer.complete_document_operation(
                crate::navigation::NavigationExecutionOutcome::Canceled,
            );
        }
        if let Some(owner) = self.owner.upgrade() {
            for event in &self.events {
                event.cancel(&owner);
            }
            if let Some(admission) = &self.admission {
                admission.notification.cancel(&owner);
            }
            if let Some(completion) = &self.completion {
                if let Some(focus) = &completion.focus {
                    focus.cancel(&owner);
                }
            }
        }
    }
}
fn reserve_publication(state: &V8BindingState) -> Result<(), V8BindingError> {
    let mut navigation = state.navigation_api.borrow_mut();
    if navigation
        .publications
        .len()
        .saturating_add(navigation.publication_depth)
        .saturating_add(navigation.queued_finishes.len())
        >= 128
    {
        return Err(failure("QuotaExceededError"));
    }
    navigation
        .publications
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("history publication queue"))
}
fn enqueue_publication(navigation: &mut NavigationState, batch: HistoryPublication) {
    if navigation.publication_depth > 0 {
        navigation.publications.push_front(batch);
    } else {
        navigation.publications.push_back(batch);
    }
}
pub(crate) fn document_lifecycle_publication_pending(state: &V8BindingState) -> bool {
    state
        .navigation_api
        .borrow()
        .publications
        .iter()
        .any(|pending| pending.lifecycle_complete.is_some())
}
fn publication(state: &Rc<V8BindingState>) -> HistoryPublication {
    HistoryPublication {
        execution_observer: None,
        deferred_hash: None,
        scroll_restore: None,
        owner: Rc::downgrade(state),
        generation: state.document_generation(),
        epoch: state.document_authority_epoch(),
        settlement: None,
        completion: None,
        receipt: None,
        lifecycle_complete: None,
        joint_finish: None,
        joint_barrier: None,
        script: None,
        admission: None,
        events: std::collections::VecDeque::new(),
    }
}
pub(super) fn publications_ready(state: &V8BindingState) -> bool {
    let navigation = state.navigation_api.borrow();
    (navigation.history_projection.pending.is_some()
        || !navigation.joint_tasks.is_empty()
        || !navigation.hash_tasks.is_empty()
        || !navigation.publications.is_empty()
        || !navigation.continuation_requests.is_empty()
        || !navigation.pending_rejections.is_empty())
        && navigation
            .publication_retry_at
            .is_none_or(|deadline| deadline <= std::time::Instant::now())
}
pub(super) fn publication_delay(state: &V8BindingState) -> Option<std::time::Duration> {
    let navigation = state.navigation_api.borrow();
    if navigation.history_projection.pending.is_none()
        && navigation.joint_tasks.is_empty()
        && navigation.hash_tasks.is_empty()
        && navigation.publications.is_empty()
        && navigation.continuation_requests.is_empty()
        && navigation.pending_rejections.is_empty()
    {
        None
    } else {
        Some(
            navigation
                .publication_retry_at
                .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap_or_default(),
        )
    }
}
pub(super) fn retire_publications(state: &V8BindingState) {
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.history_projection.pending = None;
    navigation.publications.clear();
    navigation.joint_tasks.clear();
    navigation.hash_tasks.clear();
    navigation.intercepted_history_barriers.clear();
    navigation.native_receipts.clear();
    navigation.parser_receipts.clear();
    navigation.queued_finishes.clear();
    navigation.continuation_requests.clear();
    navigation.pending_rejections.clear();
    navigation.publication_retry_at = None;
}
pub(super) fn poll_publications(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if !publications_ready(state) {
        return Ok(0);
    }
    history_projection::flush(state)?;
    loop {
        let request = {
            state
                .navigation_api
                .borrow_mut()
                .pending_rejections
                .pop_first()
        };
        let Some((event, reason)) = request else {
            break;
        };
        if !state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(event))
        {
            continue;
        }
        let value = v8::Local::new(scope, &reason);
        if let Err(error) = reject_pending(scope, state, value) {
            let mut navigation = state.navigation_api.borrow_mut();
            if navigation
                .pending
                .as_ref()
                .is_some_and(|pending| pending.event_id == Some(event))
            {
                navigation.pending_rejections.insert(event, reason);
            }
            navigation.publication_retry_at =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
            return Err(error);
        }
    }
    loop {
        let request = {
            state
                .navigation_api
                .borrow()
                .continuation_requests
                .first()
                .copied()
        };
        let Some((event, commit)) = request else {
            break;
        };
        state
            .navigation_api
            .borrow_mut()
            .continuation_requests
            .remove(&(event, commit));
        if !state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(event))
        {
            continue;
        }
        let result = if commit {
            apply_commit(scope, state, event)
        } else {
            finish_navigation(scope)
        };
        if let Err(error) = result {
            let mut navigation = state.navigation_api.borrow_mut();
            if navigation
                .pending
                .as_ref()
                .is_some_and(|pending| pending.event_id == Some(event))
            {
                navigation.continuation_requests.insert((event, commit));
            }
            navigation.publication_retry_at =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
            return Err(error);
        }
    }
    let mut count = 0;
    loop {
        // A nested publication may retain a failed callback while its parent
        // completes. Preserve the retry boundary established by that child.
        if state
            .navigation_api
            .borrow()
            .publication_retry_at
            .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return Ok(count);
        }
        let pending = { state.navigation_api.borrow_mut().publications.pop_front() };
        let Some(mut pending) = pending else {
            state.navigation_api.borrow_mut().publication_retry_at = None;
            return Ok(count);
        };
        if state.execution_retired()
            || pending.generation != state.document_generation()
            || pending.epoch != state.document_authority_epoch()
        {
            continue;
        }
        state.navigation_api.borrow_mut().publication_depth += 1;
        let result = (|| {
            if let Some(settlement) = pending.settlement.as_mut() {
                {
                    let mut navigation = state.navigation_api.borrow_mut();
                    if let Some(event) = navigation.events.get_mut(&settlement.event) {
                        event.committed = true;
                    }
                    if let Some(pending) = navigation
                        .pending
                        .as_mut()
                        .filter(|pending| pending.event_id == Some(settlement.event))
                    {
                        pending.committed_entry = true;
                    }
                }
                if !settlement.committed {
                    let value = v8::Local::new(scope, &settlement.entry);
                    v8::Local::new(scope, &settlement.pending.committed)
                        .resolve(scope, value.into())
                        .ok_or(V8BindingError::JavaScriptException)?;
                    settlement.committed = true;
                }
                if !settlement.transition_committed {
                    if let Some(resolver) = &settlement.transition {
                        let value = v8::undefined(scope);
                        v8::Local::new(scope, resolver)
                            .resolve(scope, value.into())
                            .ok_or(V8BindingError::JavaScriptException)?;
                    }
                    settlement.transition_committed = true;
                }
            }
            if let Some(admission) = pending.admission.as_mut() {
                if !admission.resume(scope, state)? {
                    return Ok(false);
                }
            }
            if let Some(script) = pending.script.as_mut() {
                if !script.resume(scope, state)? {
                    return Ok(false);
                }
            }
            if let Some(completion) = pending.completion.as_mut() {
                if !completion.resume(scope, state)? {
                    return Ok(false);
                }
            }
            loop {
                if state.execution_retired()
                    || pending.generation != state.document_generation()
                    || pending.epoch != state.document_authority_epoch()
                {
                    return Ok(true);
                }
                if pending
                    .scroll_restore
                    .as_ref()
                    .is_some_and(|restore| restore.after_events == 0)
                {
                    pending.scroll_restore.as_ref().unwrap().apply(state)?;
                    pending.scroll_restore = None;
                }
                let Some(event) = pending.events.front_mut() else {
                    break;
                };
                if !event.resume(scope, state)? {
                    return Ok(false);
                }
                event.finish(scope, state);
                pending.events.pop_front();
                if let Some(restore) = pending.scroll_restore.as_mut() {
                    restore.after_events = restore.after_events.saturating_sub(1);
                }
            }
            Ok(true)
        })();
        {
            let mut navigation = state.navigation_api.borrow_mut();
            navigation.publication_depth = navigation.publication_depth.saturating_sub(1);
        }
        match result {
            Ok(true) => {
                if let Some((document, observer)) = pending.execution_observer.take() {
                    observer.complete_document_operation(if document.current(state) {
                        crate::navigation::NavigationExecutionOutcome::Completed
                    } else {
                        crate::navigation::NavigationExecutionOutcome::Canceled
                    });
                }
                let joint_ready = pending.joint_barrier.as_ref().is_none_or(|(barrier, bit)| {
                    barrier.set(barrier.get() | *bit);
                    if *bit == 2 {
                        state
                            .navigation_api
                            .borrow_mut()
                            .intercepted_history_barriers
                            .retain(|_, stored| !Rc::ptr_eq(stored, barrier));
                    }
                    barrier.get() == 3
                });
                if let Some((id, stream)) = pending.joint_finish.take().filter(|_| joint_ready) {
                    if !state.execution_retired()
                        && state.document_stream_epoch() == stream
                        && state.document_generation() == pending.generation
                        && state.document_authority_epoch() == pending.epoch
                    {
                        let group = state.joint_history.borrow().clone();
                        let route = state.joint_history_route();
                        group.borrow_mut().record_publication(&route, &id);
                        if state.document_completely_loaded.get() {
                            group.borrow_mut().record_load(&route, &id);
                        }
                    }
                }
                if let Some(event) = pending.deferred_hash.take() {
                    let mut task = publication(state);
                    task.events.push_back(event);
                    state.navigation_api.borrow_mut().hash_tasks.push_back(task);
                }
                if let Some((stream, content_loaded)) = pending.lifecycle_complete {
                    if !state.execution_retired()
                        && state.document_stream_epoch() == stream
                        && state.document_generation() == pending.generation
                        && state.document_authority_epoch() == pending.epoch
                    {
                        if content_loaded {
                            service_workers::enable_messages(state);
                            joint_document_interactive(state);
                        } else {
                            state.document_completely_loaded.set(true);
                            joint_document_loaded(state);
                        }
                    }
                }
                if let Some(receipt) = pending.receipt {
                    if let Some(completed) = state
                        .navigation_api
                        .borrow_mut()
                        .native_receipts
                        .get_mut(&receipt)
                    {
                        *completed = true;
                    }
                }
                count += 1;
            }
            other => {
                let suspended_yield = matches!(&other, Ok(false)) && state.execution_suspended();
                let mut navigation = state.navigation_api.borrow_mut();
                navigation.publications.push_front(pending);
                // Successful suspension retains its cursor until the owner resumes.
                // It must not create a failure delay; preserve any delay already
                // established by a failed nested publication.
                if !suspended_yield {
                    navigation.publication_retry_at =
                        std::time::Instant::now().checked_add(std::time::Duration::from_millis(25));
                }
                return other.map(|_| count);
            }
        }
    }
}
fn failure(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Navigation".into(),
        member: name.into(),
    }
}
fn token() -> Result<String, V8BindingError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| V8BindingError::Allocation("navigation identity entropy"))?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}
fn current_state(scope: &mut v8::PinScope) -> Result<Rc<V8BindingState>, V8BindingError> {
    scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("navigation realm"))
}
fn native_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    if object.is_proxy() {
        return Err(failure("TypeError"));
    }
    let state = binding_state_for_receiver(scope, object).ok_or_else(|| failure("TypeError"))?;
    let id = stateful_id(scope, object)?;
    Ok((state, id))
}
/// Unpublished entry wrappers own their native registrations until the whole
/// history replacement is ready. Failure cannot leave half a new entry map.
struct StagedEntries {
    state: Rc<V8BindingState>,
    entries: Vec<Entry>,
    registrations: Vec<u64>,
    committed: bool,
}
impl StagedEntries {
    fn commit(mut self) {
        let mut navigation = self.state.navigation_api.borrow_mut();
        for entry in self.entries.drain(..) {
            navigation.entries.insert(entry.snapshot.id.clone(), entry);
        }
        self.committed = true;
    }
}
impl Drop for StagedEntries {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for id in &self.registrations {
            self.state.platform_objects.borrow_mut().remove(id);
            self.state.platform_wrappers.borrow_mut().remove(id);
            self.state
                .platform_listener_watchers
                .borrow_mut()
                .remove(id);
            self.state.platform_listener_roots.borrow_mut().remove(id);
            self.state
                .platform_listeners
                .borrow_mut()
                .retain(|(target, _), _| target != id);
            self.state
                .platform_handlers
                .borrow_mut()
                .retain(|(target, _), _| target != id);
        }
    }
}
fn stage_entries(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    snapshots: &[NavigationHistorySnapshot],
) -> Result<StagedEntries, V8BindingError> {
    let missing = {
        let navigation = state.navigation_api.borrow();
        let missing = snapshots
            .iter()
            .filter(|snapshot| !navigation.entries.contains_key(&snapshot.id))
            .cloned()
            .collect::<Vec<_>>();
        if navigation.entries.len().saturating_add(missing.len()) > 4096 {
            return Err(failure("QuotaExceededError"));
        }
        missing
    };
    let mut batch = StagedEntries {
        state: state.clone(),
        entries: Vec::new(),
        registrations: Vec::new(),
        committed: false,
    };
    batch
        .entries
        .try_reserve(missing.len())
        .map_err(|_| V8BindingError::Allocation("navigation entry staging"))?;
    batch
        .registrations
        .try_reserve(missing.len())
        .map_err(|_| V8BindingError::Allocation("navigation registration staging"))?;
    for snapshot in missing {
        #[cfg(test)]
        if FAIL_ENTRY_STAGING.with(|remaining| match remaining.get() {
            Some(0) => {
                remaining.set(None);
                true
            }
            Some(count) => {
                remaining.set(Some(count - 1));
                false
            }
            None => false,
        }) {
            return Err(V8BindingError::Allocation(
                "injected navigation entry staging failure",
            ));
        }
        let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
        batch.registrations.push(id);
        let object = new_platform_object(scope, state, "NavigationHistoryEntry", id)?;
        batch.entries.push(Entry {
            snapshot,
            object: v8::Global::new(scope, object),
            platform_id: id,
            disposed: false,
        });
    }
    Ok(batch)
}
#[cfg(test)]
thread_local! {static FAIL_ENTRY_STAGING:Cell<Option<usize>>=const{Cell::new(None)};}
fn add_entry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    snapshot: NavigationHistorySnapshot,
) -> Result<(), V8BindingError> {
    stage_entries(scope, state, &[snapshot])?.commit();
    Ok(())
}
pub(super) fn ensure(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.navigation_api.borrow().object.is_none() {
        let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
        let object = new_platform_object(scope, state, "Navigation", id)?;
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.platform_id = Some(id);
        navigation.object = Some(v8::Global::new(scope, object));
    }
    if state.navigation_api.borrow().current.is_none() {
        let url = state.request_initiator_url()?;
        let key = token()?;
        let snapshot = NavigationHistorySnapshot {
            scroll: Default::default(),
            key,
            id: token()?,
            url,
            document_id: format!("{}:{}", state.instance_id, state.document_authority_epoch()),
            state: None,
            history_state: None,
        };
        let id = snapshot.id.clone();
        add_entry(scope, state, snapshot)?;
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.order.push(id.clone());
        navigation.current = Some(id);
    }
    Ok(())
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    ensure(scope, state)?;
    let member = GENERATED_INTERFACES
        .iter()
        .find(|interface| interface.name == "Window")
        .and_then(|interface| {
            interface
                .members
                .iter()
                .find(|member| member.name == "navigation")
        })
        .ok_or(V8BindingError::Allocation("Window.navigation metadata"))?;
    let data = v8::Integer::new(scope, member.id.0 as i32);
    let getter = v8::Function::builder(generated_attribute_getter)
        .data(data.into())
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.navigation getter"))?;
    let key = v8::String::new(scope, "navigation").unwrap();
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let global = scope.get_current_context().global(scope);
    if global.define_property(scope, key.into(), &descriptor) != Some(true) {
        return Err(V8BindingError::Allocation("Window.navigation property"));
    }
    Ok(())
}
pub(super) fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    ensure(scope, state)?;
    let navigation = state.navigation_api.borrow();
    Ok(v8::Local::new(scope, navigation.object.as_ref().unwrap()).into())
}

fn entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    key: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let navigation = state.navigation_api.borrow();
    let entry = navigation
        .entries
        .get(key)
        .ok_or_else(|| failure("InvalidStateError"))?;
    Ok(v8::Local::new(scope, &entry.object))
}
/// Validate the native brand without reading prototypes, constructors or traps.
pub fn validate_activation(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if value.is_null() {
        return Ok(());
    }
    let object = value
        .try_cast::<v8::Object>()
        .map_err(|_| failure("TypeError"))?;
    let (state, id) = native_id(scope, object)?;
    let navigation = state.navigation_api.borrow();
    let activation = navigation
        .activations
        .get(&id)
        .ok_or_else(|| failure("TypeError"))?;
    if v8::Local::new(scope, &activation.object) != object {
        return Err(failure("TypeError"));
    }
    Ok(())
}
/// PageSwapEvent and Navigation.currentEntry share these exact cached objects.
pub fn activation_for_swap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination_url: &str,
    navigation_type: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !matches!(navigation_type, "push" | "replace" | "reload" | "traverse") {
        return Err(failure("TypeError"));
    }
    let state = current_state(scope)?;
    ensure(scope, &state)?;
    let (from, pending) = {
        let navigation = state.navigation_api.borrow();
        (
            navigation.current.clone(),
            navigation.staged_swap.clone().or_else(|| {
                navigation
                    .pending
                    .as_ref()
                    .map(|pending| pending.destination.clone())
            }),
        )
    };
    let destination = match pending {
        Some(destination) if destination.url == destination_url => destination,
        _ => {
            let destination = NavigationHistorySnapshot {
                scroll: Default::default(),
                key: token()?,
                id: token()?,
                url: destination_url.into(),
                document_id: token()?,
                state: None,
                history_state: None,
            };
            state.navigation_api.borrow_mut().staged_swap = Some(destination.clone());
            destination
        }
    };
    // Reload's departing activation identifies the existing entry; the newly
    // committed document receives its replacement identity only after unload.
    let key = if navigation_type == "reload" {
        from.clone().unwrap_or_else(|| destination.id.clone())
    } else {
        destination.id.clone()
    };
    {
        let navigation = state.navigation_api.borrow();
        if let Some((entry, kind, source, id)) = &navigation.staged_swap_activation {
            if entry == &key && kind == navigation_type && source == &from {
                if let Some(activation) = navigation.activations.get(id) {
                    return Ok(v8::Local::new(scope, &activation.object));
                }
            }
        }
        if navigation.activations.len() >= 256 {
            return Err(failure("QuotaExceededError"));
        }
    }
    let mut staged = if navigation_type == "reload" && from.is_some() {
        stage_entries(scope, &state, &[])?
    } else {
        stage_entries(scope, &state, &[destination])?
    };
    staged
        .registrations
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("swap activation registration"))?;
    let id = register_platform_object(&state, PlatformObjectRecord::EventTarget);
    staged.registrations.push(id);
    let object = new_platform_object(scope, &state, "NavigationActivation", id)?;
    staged.commit();
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.staged_swap_activation =
        Some((key.clone(), navigation_type.into(), from.clone(), id));
    navigation.activations.insert(
        id,
        Activation {
            object: v8::Global::new(scope, object),
            entry: key,
            from,
            navigation_type: navigation_type.into(),
        },
    );
    Ok(object)
}
pub(super) fn consume_swap_destination(state: &V8BindingState) {
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.staged_swap = None;
    navigation.staged_swap_activation = None;
}

pub(super) fn is_interface(interface: &str) -> bool {
    matches!(
        interface,
        "Navigation"
            | "NavigationHistoryEntry"
            | "NavigationActivation"
            | "NavigationTransition"
            | "NavigationDestination"
            | "NavigationPrecommitController"
            | "NavigateEvent"
            | "NavigationCurrentEntryChangeEvent"
    )
}
fn entry_id(state: &V8BindingState, platform_id: u64) -> Option<String> {
    state
        .navigation_api
        .borrow()
        .entries
        .iter()
        .find(|(_, entry)| entry.platform_id == platform_id)
        .map(|(id, _)| id.clone())
}
fn checked_receiver(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<u64, V8BindingError> {
    let (owner, id) = native_id(scope, object)?;
    if owner.instance_id != state.instance_id {
        return Err(failure("TypeError"));
    }
    let navigation = state.navigation_api.borrow();
    let expected = match interface {
        "Navigation" => navigation
            .platform_id
            .filter(|actual| *actual == id)
            .and(navigation.object.as_ref()),
        "NavigationHistoryEntry" => navigation
            .entries
            .values()
            .find(|entry| entry.platform_id == id)
            .map(|entry| &entry.object),
        "NavigationActivation" => navigation
            .activations
            .get(&id)
            .map(|activation| &activation.object),
        "NavigationTransition" => navigation
            .transitions
            .get(&id)
            .map(|transition| &transition.object),
        "NavigationDestination" => navigation
            .destinations
            .get(&id)
            .map(|destination| &destination.object),
        "NavigationPrecommitController" => navigation
            .controllers
            .get(&id)
            .map(|controller| &controller.object),
        _ => None,
    }
    .ok_or_else(|| failure("TypeError"))?;
    if v8::Local::new(scope, expected) != object {
        return Err(failure("TypeError"));
    }
    Ok(id)
}
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = checked_receiver(scope, state, object, interface)?;
    if member.starts_with("on") {
        return Ok(platform_listener_graph::handler_get(
            scope,
            state,
            id,
            &member[2..],
        ));
    }
    match interface {
        "Navigation" => {
            let disabled = entries_disabled(state);
            let navigation = state.navigation_api.borrow();
            if disabled {
                return match member {
                    "currentEntry" | "activation" | "transition" => Ok(v8::null(scope).into()),
                    "canGoBack" | "canGoForward" => Ok(v8::Boolean::new(scope, false).into()),
                    _ => Err(failure("TypeError")),
                };
            }
            match member {
                "currentEntry" => navigation
                    .current
                    .as_ref()
                    .map(|id| v8::Local::new(scope, &navigation.entries[id].object).into())
                    .ok_or_else(|| failure("InvalidStateError")),
                "activation" => Ok(navigation
                    .activation
                    .and_then(|id| navigation.activations.get(&id))
                    .map(|activation| v8::Local::new(scope, &activation.object).into())
                    .unwrap_or_else(|| v8::null(scope).into())),
                "transition" => Ok(navigation
                    .pending
                    .as_ref()
                    .and_then(|pending| pending.transition)
                    .or(navigation.finishing_transition)
                    .and_then(|id| navigation.transitions.get(&id))
                    .map(|transition| v8::Local::new(scope, &transition.object).into())
                    .unwrap_or_else(|| v8::null(scope).into())),
                "canGoBack" | "canGoForward" => {
                    let index = navigation
                        .current
                        .as_ref()
                        .and_then(|id| navigation.order.iter().position(|entry| entry == id));
                    Ok(v8::Boolean::new(
                        scope,
                        index.is_some_and(|index| {
                            if member == "canGoBack" {
                                index > 0
                            } else {
                                index + 1 < navigation.order.len()
                            }
                        }),
                    )
                    .into())
                }
                _ => Err(failure("TypeError")),
            }
        }
        "NavigationHistoryEntry" => {
            let navigation = state.navigation_api.borrow();
            let entry = navigation
                .entries
                .values()
                .find(|entry| entry.platform_id == id)
                .ok_or_else(|| failure("TypeError"))?;
            match member {
                "key" => v8_string(scope, &entry.snapshot.key),
                "id" => v8_string(scope, &entry.snapshot.id),
                "url" => v8_string(scope, &entry.snapshot.url),
                "index" => Ok(v8::Number::new(
                    scope,
                    navigation
                        .order
                        .iter()
                        .position(|id| id == &entry.snapshot.id)
                        .map(|index| index as f64)
                        .unwrap_or(-1.0),
                )
                .into()),
                "sameDocument" => Ok(v8::Boolean::new(
                    scope,
                    navigation
                        .current
                        .as_ref()
                        .and_then(|id| navigation.entries.get(id))
                        .is_some_and(|current| {
                            current.snapshot.document_id == entry.snapshot.document_id
                        }),
                )
                .into()),
                _ => Err(failure("TypeError")),
            }
        }
        "NavigationDestination" => {
            let navigation = state.navigation_api.borrow();
            let destination = &navigation.destinations[&id];
            match member {
                "url" => v8_string(scope, &destination.snapshot.url),
                "key" => v8_string(scope, &destination.snapshot.key),
                "id" => v8_string(scope, &destination.snapshot.id),
                "index" => Ok(v8::Number::new(scope, destination.index as f64).into()),
                "sameDocument" => Ok(v8::Boolean::new(scope, destination.same_document).into()),
                _ => Err(failure("TypeError")),
            }
        }
        "NavigationTransition" => {
            let navigation = state.navigation_api.borrow();
            let transition = &navigation.transitions[&id];
            match member {
                "navigationType" => v8_string(scope, &transition.navigation_type),
                "from" => {
                    Ok(v8::Local::new(scope, &navigation.entries[&transition.from].object).into())
                }
                "to" => Ok(
                    v8::Local::new(scope, &navigation.destinations[&transition.to].object).into(),
                ),
                "committed" => {
                    let resolver = v8::Local::new(scope, &transition.committed);
                    Ok(resolver.get_promise(scope).into())
                }
                "finished" => {
                    let resolver = v8::Local::new(scope, &transition.finished);
                    Ok(resolver.get_promise(scope).into())
                }
                _ => Err(failure("TypeError")),
            }
        }
        "NavigationActivation" => {
            let navigation = state.navigation_api.borrow();
            let activation = &navigation.activations[&id];
            match member {
                "navigationType" => v8_string(scope, &activation.navigation_type),
                "entry" => {
                    Ok(v8::Local::new(scope, &navigation.entries[&activation.entry].object).into())
                }
                "from" => Ok(activation
                    .from
                    .as_ref()
                    .and_then(|id| navigation.entries.get(id))
                    .map(|entry| v8::Local::new(scope, &entry.object).into())
                    .unwrap_or_else(|| v8::null(scope).into())),
                _ => Err(failure("TypeError")),
            }
        }
        _ => Err(failure("TypeError")),
    }
}
/// Commit a native session-history snapshot. Native IDs are never read from an
/// author object; callers pass only the visible same-origin history segment.
pub fn sync_history(
    scope: &mut v8::PinScope,
    entries: Vec<NavigationHistorySnapshot>,
    current: usize,
    navigation_type: &str,
) -> Result<(), V8BindingError> {
    sync_history_inner(scope, entries, current, navigation_type, None, None)
}
/// Native callers receive durable acceptance independently of synchronous
/// notification success. The receipt is acknowledged only after all callbacks.
pub fn start_native_history_publication(
    scope: &mut v8::PinScope,
    update: NavigationHistoryUpdate,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    let result = sync_history_inner(
        scope,
        update.entries,
        update.current,
        &update.navigation_type,
        Some(receipt),
        None,
    );
    if state
        .navigation_api
        .borrow()
        .native_receipts
        .contains_key(&receipt)
    {
        Ok(receipt)
    } else {
        result.map(|_| receipt)
    }
}
pub fn start_native_history_transaction(
    scope: &mut v8::PinScope,
    update: NavigationHistoryUpdate,
    session: NavigationHistoryUpdate,
    activation: Option<Option<NavigationHistorySnapshot>>,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    let result = sync_history_inner(
        scope,
        update.entries,
        update.current,
        &update.navigation_type,
        Some(receipt),
        Some((session, activation)),
    );
    if state
        .navigation_api
        .borrow()
        .native_receipts
        .contains_key(&receipt)
    {
        Ok(receipt)
    } else {
        result.map(|_| receipt)
    }
}
pub fn start_native_readiness_publication(
    scope: &mut v8::PinScope,
    readiness: oryn_dom::DocumentReadyState,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    reserve_publication(&state)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    let document = state.document_root();
    let mut batch = publication(&state);
    batch.receipt = Some(receipt);
    let changed = {
        let dom = state.dom.borrow();
        dom.arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_none_or(|metadata| metadata.ready_state != readiness)
    };
    if changed {
        let target = wrap_node(scope, &state, document)?;
        let event = create_event_object(scope, &state, "readystatechange", false, false)?;
        batch.events.push_back(
            native_platform::publication::pending_dom_dispatch(
                scope,
                &state,
                target,
                "readystatechange",
                event,
            )
            .for_parser_stream(state.document_stream_epoch()),
        );
        let mut dom = state.dom.borrow_mut();
        let metadata = dom
            .arena_mut()
            .get_mut(document)?
            .document_metadata
            .get_or_insert_with(|| Box::new(oryn_dom::DocumentMetadata::default()));
        metadata.ready_state = readiness;
        match readiness {
            oryn_dom::DocumentReadyState::Interactive => state
                .record_navigation_timing_stage(crate::navigation_timing::Stage::DomInteractive),
            oryn_dom::DocumentReadyState::Complete => {
                state.record_navigation_timing_stage(crate::navigation_timing::Stage::DomComplete)
            }
            _ => {}
        }
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.native_receipts.insert(receipt, false);
        navigation
            .parser_receipts
            .insert(receipt, state.document_stream_epoch());
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, &state);
    Ok(receipt)
}
/// Accept the entire native lifecycle event sequence before changing readiness.
/// The caller owns the receipt through native resource and frame-load barriers.
pub fn start_native_document_lifecycle_publication(
    scope: &mut v8::PinScope,
    content_loaded: bool,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    reserve_publication(&state)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    prepare_document_event_handlers(scope, &state)?;
    let document = state.document_root();
    let stream = state.document_stream_epoch();
    let target = wrap_node(scope, &state, document)?;
    let readiness = if content_loaded {
        oryn_dom::DocumentReadyState::Interactive
    } else {
        oryn_dom::DocumentReadyState::Complete
    };
    let mut batch = publication(&state);
    batch.receipt = Some(receipt);
    batch.lifecycle_complete = Some((stream, content_loaded));
    let changed = {
        let dom = state.dom.borrow();
        dom.arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_none_or(|metadata| metadata.ready_state != readiness)
    };
    if changed {
        let event = create_event_object(scope, &state, "readystatechange", false, false)?;
        batch.events.push_back(
            native_platform::publication::pending_dom_dispatch(
                scope,
                &state,
                target,
                "readystatechange",
                event,
            )
            .for_parser_stream(stream),
        );
    }
    let name = if content_loaded {
        "DOMContentLoaded"
    } else {
        "load"
    };
    let event = create_event_object(scope, &state, name, content_loaded, false)?;
    let notification = if content_loaded {
        native_platform::publication::pending_dom_dispatch(scope, &state, target, name, event)
    } else {
        native_platform::publication::pending_platform_dispatch(
            scope,
            &state,
            global_events::GLOBAL_TARGET,
            name,
            event,
        )
        .with_document_target()
    };
    batch.events.push_back(
        notification
            .for_parser_stream(stream)
            .for_navigation_timing(
                if content_loaded {
                    crate::navigation_timing::Stage::DomContentLoadedStart
                } else {
                    crate::navigation_timing::Stage::LoadStart
                },
                if content_loaded {
                    crate::navigation_timing::Stage::DomContentLoadedEnd
                } else {
                    crate::navigation_timing::Stage::LoadEnd
                },
            ),
    );
    let pageshow = !content_loaded && !state.initial_pageshow_delivered.get();
    if pageshow {
        let event = create_event_object_with_interface(
            scope,
            &state,
            "PageTransitionEvent",
            "pageshow",
            false,
            false,
        )?;
        let persisted = v8::Boolean::new(scope, false);
        event.set_internal_field(2, persisted.into());
        batch.events.push_back(
            native_platform::publication::pending_platform_dispatch(
                scope,
                &state,
                global_events::GLOBAL_TARGET,
                "pageshow",
                event,
            )
            .for_parser_stream(stream)
            .for_initial_pageshow(),
        );
    }
    // Fresh history documents (root or frame) restore after their initial
    // pageshow, under the same retained parser-stream receipt as load.
    let restore = if !content_loaded {
        state.navigation_api.borrow().load_restore.clone()
    } else {
        None
    };
    if let Some((owner, entry)) = restore.as_ref().filter(|(owner, entry)| {
        owner.current(&state) && state.navigation_api.borrow().current.as_ref() == Some(&entry.id)
    }) {
        let traversed = state.navigation_api.borrow().load_restore_is_traversal;
        if traversed {
            let value = history_state_value(scope, &state)?;
            let event = history_events::create(scope, &state, "popstate", &[("state", value)])?;
            batch.events.push_back(
                native_platform::publication::pending_platform_dispatch(
                    scope,
                    &state,
                    global_events::GLOBAL_TARGET,
                    "popstate",
                    event,
                )
                .for_parser_stream(stream),
            );
        }
        batch.scroll_restore = Some(ScrollRestore {
            honor_manual: true,
            document: *owner,
            entry: entry.id.clone(),
            position: entry.scroll.position,
            after_events: if traversed { batch.events.len() } else { 0 },
        });
    }
    if !content_loaded && restore.is_none() && pageshow {
        if let Some(entry) = state.navigation_history_snapshot().filter(|entry| {
            Url::parse(&entry.url)
                .ok()
                .is_some_and(|url| url.fragment().is_some())
        }) {
            // A newly loaded fragment is navigation scrolling, independent of
            // history.scrollRestoration. Resolve it after parsing and before
            // load handlers, which may deliberately scroll elsewhere.
            batch.scroll_restore = Some(ScrollRestore {
                honor_manual: false,
                document: AdmissionDocument::capture(&state),
                entry: entry.id,
                position: None,
                after_events: 0,
            });
        }
    }
    if changed {
        let mut dom = state.dom.borrow_mut();
        let metadata = dom
            .arena_mut()
            .get_mut(document)?
            .document_metadata
            .get_or_insert_with(|| Box::new(oryn_dom::DocumentMetadata::default()));
        metadata.ready_state = readiness;
        match readiness {
            oryn_dom::DocumentReadyState::Interactive => state
                .record_navigation_timing_stage(crate::navigation_timing::Stage::DomInteractive),
            oryn_dom::DocumentReadyState::Complete => {
                state.record_navigation_timing_stage(crate::navigation_timing::Stage::DomComplete)
            }
            _ => {}
        }
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if !content_loaded {
            navigation.load_restore = None;
        }
        navigation.native_receipts.insert(receipt, false);
        navigation.parser_receipts.insert(receipt, stream);
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, &state);
    Ok(receipt)
}
/// Preallocate a retained resource event, publish native state, then expose it.
/// On preparation failure the caller still owns the response; once committed,
/// callback retries belong to the document publication queue.
pub(crate) fn publish_resource_state(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    success: bool,
    commit: impl FnOnce() -> Result<(), V8BindingError>,
) -> Result<(), V8BindingError> {
    publish_resource_state_event(
        scope,
        state,
        node,
        Some(if success { "load" } else { "error" }),
        commit,
    )
}
pub(crate) fn publish_resource_state_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    name: Option<&str>,
    commit: impl FnOnce() -> Result<(), V8BindingError>,
) -> Result<(), V8BindingError> {
    let Some(name) = name else { return commit() };
    reserve_publication(state)?;
    let target = wrap_node(scope, state, node)?;
    let event = create_event_object(scope, state, name, false, false)?;
    let mut batch = publication(state);
    batch.events.push_back(
        native_platform::publication::pending_dom_dispatch(scope, state, target, name, event)
            .for_parser_stream(state.document_stream_epoch())
            .without_parser_insertion_point(),
    );
    // Native callers still own replacement cleanup after this return. Authored
    // callbacks run from the ordinary checkpoint, after that ownership is settled.
    commit()?;
    enqueue_publication(&mut state.navigation_api.borrow_mut(), batch);
    Ok(())
}
pub fn start_native_frame_load_publication(
    scope: &mut v8::PinScope,
    frame: NodeId,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    reserve_publication(&state)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    let target = wrap_node(scope, &state, frame)?;
    let event = create_event_object(scope, &state, "load", false, false)?;
    let mut batch = publication(&state);
    batch.receipt = Some(receipt);
    batch.events.push_back(
        native_platform::publication::pending_dom_dispatch(scope, &state, target, "load", event)
            .for_parser_stream(state.document_stream_epoch()),
    );
    {
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.native_receipts.insert(receipt, false);
        navigation
            .parser_receipts
            .insert(receipt, state.document_stream_epoch());
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, &state);
    Ok(receipt)
}
pub fn start_native_script_completion_publication(
    scope: &mut v8::PinScope,
    frame: NodeId,
    success: bool,
) -> Result<u64, V8BindingError> {
    start_native_script_completion_publication_with_insertion(scope, frame, success, false)
}
pub fn start_native_script_completion_publication_with_insertion(
    scope: &mut v8::PinScope,
    frame: NodeId,
    success: bool,
    insertion_point: bool,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    reserve_publication(&state)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    let name = if success { "load" } else { "error" };
    let target = wrap_node(scope, &state, frame)?;
    let event = create_event_object(scope, &state, name, false, false)?;
    let mut batch = publication(&state);
    batch.receipt = Some(receipt);
    let notification =
        native_platform::publication::pending_dom_dispatch(scope, &state, target, name, event)
            .for_parser_stream(state.document_stream_epoch());
    batch.events.push_back(if insertion_point {
        notification
    } else {
        notification.without_parser_insertion_point()
    });
    {
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.native_receipts.insert(receipt, false);
        navigation
            .parser_receipts
            .insert(receipt, state.document_stream_epoch());
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, &state);
    Ok(receipt)
}
/// Retiring an iframe child cannot interrupt its already-started parent event.
/// Unstarted queued dispatch is canceled; started dispatch keeps its callback tail.
pub fn retire_native_event_receipt(state: &V8BindingState, receipt: u64) {
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.native_receipts.remove(&receipt);
    navigation.parser_receipts.remove(&receipt);
    navigation.publications.retain_mut(|batch| {
        if batch.receipt != Some(receipt) {
            return true;
        }
        if batch.events.iter().any(|event| event.started()) {
            batch.receipt = None;
            true
        } else {
            false
        }
    });
}
pub fn cancel_native_history_receipt(state: &V8BindingState, receipt: u64) {
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.native_receipts.remove(&receipt);
    navigation.parser_receipts.remove(&receipt);
    navigation
        .publications
        .retain(|batch| batch.receipt != Some(receipt));
}
pub(super) fn retire_parser_publications(state: &V8BindingState) {
    state.retire_joint_history_load();
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.load_restore = None;
    navigation.growing_scroll = None;
    navigation.joint_tasks.clear();
    navigation.hash_tasks.clear();
    navigation.intercepted_history_barriers.clear();
    let receipts = std::mem::take(&mut navigation.parser_receipts);
    for receipt in receipts.keys() {
        navigation.native_receipts.remove(receipt);
    }
    navigation.publications.retain(|batch| {
        batch
            .receipt
            .is_none_or(|receipt| !receipts.contains_key(&receipt))
    });
}
pub fn resume_native_history_publication(
    scope: &mut v8::PinScope,
    receipt: u64,
) -> Result<bool, V8BindingError> {
    let state = current_state(scope)?;
    poll_publications(scope, &state)?;
    let mut navigation = state.navigation_api.borrow_mut();
    match navigation.native_receipts.get(&receipt) {
        Some(true) => {
            navigation.native_receipts.remove(&receipt);
            navigation.parser_receipts.remove(&receipt);
            Ok(true)
        }
        Some(false) => Ok(false),
        None => Err(failure("InvalidStateError")),
    }
}
pub(super) fn queue_page_transition(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    kind: &str,
    persisted: bool,
) -> Result<(), V8BindingError> {
    if !Rc::ptr_eq(&current_state(scope)?, state) {
        return Err(failure("InvalidStateError"));
    }
    let receipt = start_native_event_publication(
        scope,
        NativePublicationEvent::Page {
            kind: kind.into(),
            persisted,
        },
    )?;
    // Legacy synchronous callers transfer continuation ownership to the same
    // native publication queue; no abandoned receipt table entry remains.
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.native_receipts.remove(&receipt);
    for batch in &mut navigation.publications {
        if batch.receipt == Some(receipt) {
            batch.receipt = None;
        }
    }
    Ok(())
}
#[derive(Clone)]
pub enum NativePublicationEvent {
    Unload {
        timing: crate::navigation_timing::UnloadTiming,
    },
    Page {
        kind: String,
        persisted: bool,
    },
    Print {
        kind: String,
    },
    History {
        previous: String,
        current: String,
        traversed: bool,
    },
    HistoryRestore {
        entry: NavigationHistorySnapshot,
    },
}
pub fn start_native_event_publication(
    scope: &mut v8::PinScope,
    event: NativePublicationEvent,
) -> Result<u64, V8BindingError> {
    let state = current_state(scope)?;
    reserve_publication(&state)?;
    let receipt = {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.native_receipts.len() >= 128 {
            return Err(failure("QuotaExceededError"));
        }
        let next = navigation
            .next_native_receipt
            .checked_add(1)
            .ok_or_else(|| failure("QuotaExceededError"))?;
        navigation.next_native_receipt = next;
        next
    };
    let mut batch = publication(&state);
    batch.receipt = Some(receipt);
    let lifecycle_visibility = match &event {
        NativePublicationEvent::Page { kind, .. } => Some(kind == "pagehide"),
        _ => None,
    };
    let unload_timing = match &event {
        NativePublicationEvent::Unload { timing } => Some(timing.clone()),
        _ => None,
    };
    let parser_scoped = matches!(
        &event,
        NativePublicationEvent::Unload { .. }
            | NativePublicationEvent::Print { .. }
            | NativePublicationEvent::History { .. }
            | NativePublicationEvent::HistoryRestore { .. }
    );
    if lifecycle_visibility == Some(false) {
        batch
            .events
            .push_back(native_platform::publication::pending_lifecycle_visibility(
                scope, &state, false,
            )?);
    }
    let mut events = Vec::new();
    match event {
        NativePublicationEvent::Unload { .. } => {
            let event = create_event_object(scope, &state, "unload", false, false)?;
            events.push(("unload".into(), event));
        }
        NativePublicationEvent::Page { kind, persisted } => {
            if !matches!(kind.as_str(), "pageshow" | "pagehide") {
                return Err(failure("TypeError"));
            }
            let event = create_event_object_with_interface(
                scope,
                &state,
                "PageTransitionEvent",
                &kind,
                false,
                false,
            )?;
            let persisted = v8::Boolean::new(scope, persisted);
            event.set_internal_field(2, persisted.into());
            events.push((kind, event));
        }
        NativePublicationEvent::Print { kind } => {
            if !matches!(kind.as_str(), "beforeprint" | "afterprint") {
                return Err(failure("TypeError"));
            }
            let event = create_event_object(scope, &state, &kind, false, false)?;
            events.push((kind, event));
        }
        NativePublicationEvent::HistoryRestore { entry } => {
            batch.joint_finish = Some((entry.id.clone(), state.document_stream_epoch()));
            if !entry.scroll.valid() {
                return Err(failure("InvalidStateError"));
            }
            let value = history_state_value(scope, &state)?;
            let event = history_events::create(scope, &state, "popstate", &[("state", value)])?;
            events.push(("popstate".into(), event));
            batch.scroll_restore = Some(ScrollRestore {
                honor_manual: true,
                document: AdmissionDocument::capture(&state),
                entry: entry.id,
                position: entry.scroll.position,
                after_events: 1,
            });
        }
        NativePublicationEvent::History {
            previous,
            current,
            traversed,
        } => {
            if traversed {
                if let Some(entry) = state.navigation_history_snapshot() {
                    batch.joint_finish = Some((entry.id.clone(), state.document_stream_epoch()));
                    batch.scroll_restore = Some(ScrollRestore {
                        honor_manual: true,
                        document: AdmissionDocument::capture(&state),
                        entry: entry.id,
                        position: entry.scroll.position,
                        after_events: 1,
                    });
                }
                let value = history_state_value(scope, &state)?;
                let event = history_events::create(scope, &state, "popstate", &[("state", value)])?;
                events.push(("popstate".into(), event));
            }
            let fragment = |url: &str| {
                Url::parse(url)
                    .ok()
                    .and_then(|url| url.fragment().map(str::to_owned))
            };
            if fragment(&previous) != fragment(&current) {
                let old = v8_string(scope, &previous)?;
                let new = v8_string(scope, &current)?;
                let event = history_events::create(
                    scope,
                    &state,
                    "hashchange",
                    &[("oldURL", old), ("newURL", new)],
                )?;
                events.push(("hashchange".into(), event));
            }
        }
    }
    for (name, event) in events {
        let notification = native_platform::publication::prepare_platform_dispatch(
            scope,
            &state,
            global_events::GLOBAL_TARGET,
            &name,
            event,
        )?;
        let notification = if let Some(timing) = unload_timing.as_ref() {
            notification.for_unload_timing(timing.clone())
        } else {
            notification
        };
        batch.events.push_back(if parser_scoped {
            notification.for_parser_stream(state.document_stream_epoch())
        } else {
            notification
        });
    }
    if lifecycle_visibility == Some(true) {
        batch
            .events
            .push_back(native_platform::publication::pending_lifecycle_visibility(
                scope, &state, true,
            )?);
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.native_receipts.insert(receipt, false);
        if parser_scoped {
            navigation
                .parser_receipts
                .insert(receipt, state.document_stream_epoch());
        }
        enqueue_publication(&mut navigation, batch);
    }
    // Admission and callback execution have separate outcomes. The native owner
    // must retain the returned receipt even if a synchronous callback fails.
    let _ = poll_publications(scope, &state);
    Ok(receipt)
}
fn publish_fragment_events(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    previous: &str,
    current: &str,
    execution_observer: Option<crate::navigation::NavigationExecutionObserver>,
) -> Result<(), V8BindingError> {
    reserve_publication(state)?;
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.hash_tasks.len() + navigation.publications.len() >= 128 {
            return Err(failure("QuotaExceededError: history event tasks"));
        }
        navigation
            .hash_tasks
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("hashchange task"))?;
    }
    let stream = state.document_stream_epoch();
    let value = history_state_value(scope, state)?;
    let pop = history_events::create(scope, state, "popstate", &[("state", value)])?;
    let old = v8_string(scope, previous)?;
    let new = v8_string(scope, current)?;
    let hash = history_events::create(
        scope,
        state,
        "hashchange",
        &[("oldURL", old), ("newURL", new)],
    )?;
    let mut batch = publication(state);
    batch.execution_observer =
        execution_observer.map(|observer| (AdmissionDocument::capture(state), observer));
    batch.events.push_back(
        native_platform::publication::pending_platform_dispatch(
            scope,
            state,
            global_events::GLOBAL_TARGET,
            "popstate",
            pop,
        )
        .for_parser_stream(stream),
    );
    batch.deferred_hash = Some(
        native_platform::publication::pending_platform_dispatch(
            scope,
            state,
            global_events::GLOBAL_TARGET,
            "hashchange",
            hash,
        )
        .for_parser_stream(stream),
    );
    enqueue_publication(&mut state.navigation_api.borrow_mut(), batch);
    let _ = poll_publications(scope, state);
    Ok(())
}
// History event tasks are separate from navigation promise/microtask publication.
// They retain the same event cursor and frozen parser stream as other native
// publications once admitted; a retry never reconstructs a popstate event.
pub(super) fn poll_hash_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired()
        || state.execution_suspended()
        || !state.navigation_api.borrow().publications.is_empty()
    {
        return Ok(0);
    }
    let count = state.navigation_api.borrow().hash_tasks.len();
    let mut delivered = 0;
    for _ in 0..count {
        reserve_publication(state)?;
        let task = { state.navigation_api.borrow_mut().hash_tasks.pop_front() };
        let Some(task) = task else { break };
        enqueue_publication(&mut state.navigation_api.borrow_mut(), task);
        delivered += poll_publications(scope, state)?;
        if state.execution_retired()
            || state.execution_suspended()
            || scope.is_execution_terminating()
            || !state.navigation_api.borrow().publications.is_empty()
        {
            break;
        }
    }
    Ok(delivered)
}
fn queue_intercepted_popstate(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    entry: &str,
    event_id: u64,
) -> Result<(), V8BindingError> {
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.hash_tasks.len() + navigation.publications.len() >= 128 {
            return Err(failure("QuotaExceededError: history event tasks"));
        }
        navigation
            .hash_tasks
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("intercepted popstate task"))?;
    }
    let stream = state.document_stream_epoch();
    let value = history_state_value(scope, state)?;
    let event = history_events::create(scope, state, "popstate", &[("state", value)])?;
    let mut task = publication(state);
    task.joint_finish = Some((entry.to_owned(), stream));
    task.events.push_back(
        native_platform::publication::pending_platform_dispatch(
            scope,
            state,
            global_events::GLOBAL_TARGET,
            "popstate",
            event,
        )
        .for_parser_stream(stream),
    );
    let barrier = Rc::new(Cell::new(0));
    task.joint_barrier = Some((barrier.clone(), 1));
    let mut navigation = state.navigation_api.borrow_mut();
    navigation
        .intercepted_history_barriers
        .insert(event_id, barrier);
    navigation.hash_tasks.push_back(task);
    Ok(())
}
pub(super) fn publish_history_events(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    previous: &str,
    current: &str,
    traversed: bool,
) -> Result<(), V8BindingError> {
    reserve_publication(state)?;
    let mut batch = publication(state);
    batch.script = Some(ScriptPublication::HistoryEvent {
        previous: previous.into(),
        current: current.into(),
        traversed,
    });
    enqueue_publication(&mut state.navigation_api.borrow_mut(), batch);
    let _ = poll_publications(scope, state);
    Ok(())
}
fn sync_joint_history(
    state: &V8BindingState,
    entries: &[NavigationHistorySnapshot],
    current: usize,
    kind: &str,
) -> Result<(), V8BindingError> {
    let group = state.joint_history.borrow().clone();
    let mut joint = group.borrow_mut();
    let route = state.joint_route.borrow().clone();
    let snapshot = entries[current].clone();
    let result = if joint.is_empty() {
        joint.seed(route, entries.to_vec(), current)
    } else if joint.selected(&route).is_none() {
        joint.attach_at(route, snapshot, state.joint_attach_step.get())
    } else if joint
        .selected(&route)
        .is_some_and(|old| old.id == snapshot.id)
    {
        joint.replace(&route, snapshot)
    } else if joint.knows(&route, &snapshot.id) {
        joint.select(&route, &snapshot.id).map(|_| ())
    } else if state.navigation_api.borrow().joint_initial_replacement
        || matches!(kind, "replace" | "reload")
    {
        joint.replace(&route, snapshot)
    } else {
        joint.push(&route, snapshot)
    };
    result.map_err(|_| failure("QuotaExceededError: joint history"))?;
    state.navigation_api.borrow_mut().joint_initial_replacement = false;
    Ok(())
}
fn sync_history_inner(
    scope: &mut v8::PinScope,
    entries: Vec<NavigationHistorySnapshot>,
    current: usize,
    navigation_type: &str,
    receipt: Option<u64>,
    native: Option<(
        NavigationHistoryUpdate,
        Option<Option<NavigationHistorySnapshot>>,
    )>,
) -> Result<(), V8BindingError> {
    if entries.is_empty()
        || current >= entries.len()
        || entries.len() > 4096
        || !matches!(navigation_type, "push" | "replace" | "reload" | "traverse")
    {
        return Err(failure("InvalidStateError"));
    }
    let mut ids = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for entry in &entries {
        if !entry.scroll.valid()
            || entry.id.is_empty()
            || entry.key.is_empty()
            || !ids.insert(entry.id.clone())
            || !keys.insert(entry.key.clone())
        {
            return Err(failure("InvalidStateError"));
        }
    }
    let state = current_state(scope)?;
    ensure(scope, &state)?;
    // A new synchronous history operation must first finish older retained
    // publication. Reentrant operations may run while their parent's batch is
    // on the stack; no older queued batch is skipped or overwritten.
    if state.navigation_api.borrow().publication_depth == 0
        && !state.navigation_api.borrow().publications.is_empty()
    {
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state)?;
        if !state.navigation_api.borrow().publications.is_empty() {
            return Err(failure("InvalidStateError"));
        }
    }
    let (from, initialized) = {
        let navigation = state.navigation_api.borrow();
        (navigation.current.clone(), navigation.initialized)
    };
    let full_history_span = {
        let navigation = state.navigation_api.borrow();
        let history = state.history.borrow();
        history
            .as_ref()
            .filter(|history| history.index < history.entries.len())
            .map(|history| {
                let visible = |index: usize| {
                    navigation
                        .order
                        .contains(&history.entries[index].navigation_id)
                };
                let mut start = history.index;
                let mut end = history.index + 1;
                while start > 0 && visible(start - 1) {
                    start -= 1;
                }
                while end < history.entries.len() && visible(end) {
                    end += 1;
                }
                (
                    history.entries[..start].to_vec(),
                    history.entries[end..].to_vec(),
                )
            })
    };
    let mut snapshots = entries.clone();
    if let Some((_, Some(Some(from)))) = &native {
        if !snapshots.iter().any(|entry| entry.id == from.id) {
            snapshots.push(from.clone());
        }
    }
    let mut staged = stage_entries(scope, &state, &snapshots)?;
    let disposed = state
        .navigation_api
        .borrow()
        .order
        .iter()
        .filter(|id| !ids.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    let scroll_restoration = entries[current].scroll.mode.as_str().to_owned();
    let mut session_entries = full_history_span
        .as_ref()
        .map(|(prefix, _)| prefix.clone())
        .unwrap_or_default();
    let session_current = session_entries.len() + current;
    session_entries.extend(entries.iter().map(|entry| HistoryEntry {
        url: entry.url.clone(),
        state_json: entry.history_state.clone(),
        navigation_id: entry.id.clone(),
        document_id: entry.document_id.clone(),
    }));
    if navigation_type != "push" {
        if let Some((_, suffix)) = full_history_span {
            session_entries.extend(suffix);
        }
    }
    let mut replacement = HistoryRecord {
        entries: session_entries,
        index: session_current,
        scroll_restoration,
    };
    let mut activation = None;
    if let Some((session, from)) = native {
        if session.entries.is_empty()
            || session.current >= session.entries.len()
            || session.entries.len() > 4096
            || session.entries[session.current].id != entries[current].id
        {
            return Err(failure("InvalidStateError"));
        }
        let mut ids = BTreeSet::new();
        if session.entries.iter().any(|entry| {
            !entry.scroll.valid() || entry.id.is_empty() || !ids.insert(entry.id.clone())
        }) {
            return Err(failure("InvalidStateError"));
        }
        replacement.entries = session
            .entries
            .into_iter()
            .map(|entry| HistoryEntry {
                url: entry.url,
                state_json: entry.history_state,
                navigation_id: entry.id,
                document_id: entry.document_id,
            })
            .collect();
        replacement.index = session.current;
        if let Some(from) = from {
            if state.navigation_api.borrow().activations.len() >= 4096 {
                return Err(failure("QuotaExceededError"));
            }
            staged
                .registrations
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("native activation staging"))?;
            let id = register_platform_object(&state, PlatformObjectRecord::EventTarget);
            staged.registrations.push(id);
            let object = new_platform_object(scope, &state, "NavigationActivation", id)?;
            activation = Some((
                id,
                Activation {
                    object: v8::Global::new(scope, object),
                    entry: entries[current].id.clone(),
                    from: from.map(|entry| entry.id),
                    navigation_type: navigation_type.into(),
                },
            ));
        }
    }
    let order = entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    reserve_publication(&state)?;
    let mut delivery = publication(&state);
    let pending = state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .filter(|pending| pending.destination.id == entries[current].id)
        .cloned();
    if let Some(pending) = pending {
        if let Some(event) = pending.event_id {
            let object = state
                .navigation_api
                .borrow()
                .entries
                .get(&entries[current].id)
                .map(|entry| entry.object.clone())
                .or_else(|| {
                    staged
                        .entries
                        .iter()
                        .find(|entry| entry.snapshot.id == entries[current].id)
                        .map(|entry| entry.object.clone())
                })
                .ok_or_else(|| failure("InvalidStateError"))?;
            let transition = pending.transition.and_then(|id| {
                state
                    .navigation_api
                    .borrow()
                    .transitions
                    .get(&id)
                    .map(|transition| transition.committed.clone())
            });
            delivery.settlement = Some(CommitSettlement {
                event,
                entry: object,
                pending,
                transition,
                committed: false,
                transition_committed: false,
            });
        }
    }
    if initialized && !entries_disabled(&state) {
        if from.as_deref() != Some(entries[current].id.as_str()) {
            if let Some(from) = &from {
                let (id, event) =
                    create_current_change(scope, &state, from, Some(navigation_type))?;
                delivery
                    .events
                    .push_back(native_platform::publication::prepare_platform_dispatch(
                        scope,
                        &state,
                        id,
                        "currententrychange",
                        event,
                    )?);
            }
        }
        for id in &disposed {
            let platform = state.navigation_api.borrow().entries[id].platform_id;
            let event = create_event_object(scope, &state, "dispose", false, false)?;
            delivery
                .events
                .push_back(native_platform::publication::prepare_platform_dispatch(
                    scope, &state, platform, "dispose", event,
                )?);
        }
    }
    delivery.receipt = receipt;
    let projection = history_projection::prepare(&state, replacement.entries.len())?;
    sync_joint_history(&state, &entries, current, navigation_type)?;
    staged.commit();
    {
        let mut navigation = state.navigation_api.borrow_mut();
        for entry in &entries {
            navigation.entries.get_mut(&entry.id).unwrap().snapshot = entry.clone();
        }
        navigation.order = order;
        navigation.current = Some(entries[current].id.clone());
        navigation.initialized = true;
        if let Some((id, activation)) = activation {
            navigation.activations.insert(id, activation);
            navigation.activation = Some(id);
        }
        for id in &disposed {
            navigation.entries.get_mut(id).unwrap().disposed = true;
        }
    }
    *state.history.borrow_mut() = Some(replacement);
    state
        .joint_history
        .borrow()
        .borrow_mut()
        .record_commit(&state.joint_route.borrow(), &entries[current].id);
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if let Some(receipt) = receipt {
            navigation.native_receipts.insert(receipt, false);
        }
        enqueue_publication(&mut navigation, delivery);
    }
    if let Some(projection) = projection {
        projection.commit()?;
    }
    poll_publications(scope, &state)?;
    Ok(())
}
fn create_current_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    from: &str,
    navigation_type: Option<&str>,
) -> Result<(u64, v8::Local<'s, v8::Object>), V8BindingError> {
    let event = create_event_object_with_interface(
        scope,
        state,
        "NavigationCurrentEntryChangeEvent",
        "currententrychange",
        false,
        false,
    )?;
    let from = entry_object(scope, state, from)?;
    let kind = match navigation_type {
        Some(kind) => v8_string(scope, kind)?,
        None => v8::null(scope).into(),
    };
    let payload = v8::Array::new_with_elements(scope, &[kind, from.into()]);
    event.set_internal_field(2, payload.into());
    let brand = v8::String::new(scope, "NavigationCurrentEntryChangeEvent")
        .ok_or(V8BindingError::Allocation("navigation change event brand"))?;
    event.set_internal_field(4, brand.into());
    let id = state
        .navigation_api
        .borrow()
        .platform_id
        .ok_or_else(|| failure("InvalidStateError"))?;
    Ok((id, event))
}

pub(super) fn event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if object.is_proxy() {
        return Err(failure("TypeError"));
    }
    let state = binding_state_for_receiver(scope, object).ok_or_else(|| failure("TypeError"))?;
    let id = event_id(scope, object)?;
    if !state.events.borrow().contains_key(&id) {
        return Err(failure("TypeError"));
    }
    if interface == "NavigateEvent" {
        return navigate_event_attribute(scope, &state, object, id, member);
    }
    let brand = object
        .get_internal_field(scope, 4)
        .and_then(|value| value.try_cast::<v8::String>().ok())
        .map(|value| value.to_rust_string_lossy(scope));
    if brand.as_deref() != Some(interface) {
        return Err(failure("TypeError"));
    }
    let payload = object
        .get_internal_field(scope, 2)
        .and_then(|value| value.try_cast::<v8::Array>().ok())
        .ok_or_else(|| failure("TypeError"))?;
    let index = match (interface, member) {
        ("NavigationCurrentEntryChangeEvent", "navigationType") => 0,
        ("NavigationCurrentEntryChangeEvent", "from") => 1,
        _ => return Err(failure("TypeError")),
    };
    payload
        .get_index(scope, index)
        .ok_or_else(|| failure("TypeError"))
}
pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    interface: &str,
) -> Result<(), V8BindingError> {
    if !args.is_construct_call() || args.length() < 2 {
        return Err(failure("TypeError"));
    }
    let state = current_state(scope)?;
    let event_type = required_string(scope, args, 0)?;
    let init = args
        .get(1)
        .try_cast::<v8::Object>()
        .map_err(|_| failure("TypeError"))?;
    let bubbles = event_init_member(scope, Some(init), "bubbles")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    let cancelable = event_init_member(scope, Some(init), "cancelable")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    let composed = event_init_member(scope, Some(init), "composed")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    if interface == "NavigateEvent" {
        return construct_navigate(
            scope,
            &state,
            args.this(),
            &event_type,
            init,
            bubbles,
            cancelable,
            composed,
        );
    }
    if interface == "NavigationCurrentEntryChangeEvent" {
        let from = event_init_member(scope, Some(init), "from")
            .ok_or(V8BindingError::JavaScriptException)?
            .try_cast::<v8::Object>()
            .map_err(|_| failure("TypeError"))?;
        let (owner, _) = native_id(scope, from)?;
        checked_receiver(scope, &owner, from, "NavigationHistoryEntry")?;
        let kind = event_init_member(scope, Some(init), "navigationType")
            .ok_or(V8BindingError::JavaScriptException)?;
        let kind = if kind.is_null_or_undefined() {
            v8::null(scope).into()
        } else {
            let kind = kind
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            if !matches!(kind.as_str(), "push" | "replace" | "reload" | "traverse") {
                return Err(failure("TypeError"));
            }
            v8_string(scope, &kind)?
        };
        let event = create_event_object_with_interface(
            scope,
            &state,
            interface,
            &event_type,
            bubbles,
            cancelable,
        )?;
        let id = event_id(scope, event)?;
        args.this()
            .set_internal_field(0, v8::Number::new(scope, id as f64).into());
        tag_event_object(scope, &state, args.this());
        let payload = v8::Array::new_with_elements(scope, &[kind, from.into()]);
        args.this().set_internal_field(2, payload.into());
        let brand = v8::String::new(scope, "NavigationCurrentEntryChangeEvent").unwrap();
        args.this().set_internal_field(4, brand.into());
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.composed = composed;
            record.is_trusted = false;
        }
        return Ok(());
    }
    Err(failure("TypeError"))
}

impl V8BindingState {
    pub fn discard_joint_history_navigable(&self) -> Result<(), V8BindingError> {
        self.joint_history
            .borrow()
            .borrow_mut()
            .discard_tree(&self.joint_route.borrow())
            .map_err(|_| failure("InvalidStateError: joint navigable disposal"))
    }
    pub fn fail_joint_history_entry(
        &self,
        id: &str,
        failure: crate::joint_history::JointTraversalFailure,
    ) {
        self.joint_history.borrow().borrow_mut().fail_route(
            &self.joint_route.borrow(),
            id,
            failure,
        );
    }
    pub fn cancel_joint_history_traversals(&self) {
        self.joint_history.borrow().borrow_mut().cancel_executions();
    }
    pub fn retire_joint_history_load(&self) {
        if let Some(entry) = self.navigation_history_snapshot() {
            self.joint_history.borrow().borrow_mut().fail_route(
                &self.joint_route.borrow(),
                &entry.id,
                crate::joint_history::JointTraversalFailure::Retired,
            );
        }
    }
    pub fn joint_history_snapshot(&self) -> crate::joint_history::Timeline {
        self.joint_history.borrow().borrow().clone()
    }
    pub fn restore_joint_history(
        &self,
        mut timeline: crate::joint_history::Timeline,
    ) -> Result<(), V8BindingError> {
        timeline.rebind_execution_leases();
        timeline
            .validate()
            .map_err(|_| failure("InvalidStateError: joint history"))?;
        *self.joint_history.borrow().borrow_mut() = timeline;
        Ok(())
    }
    pub fn joint_history_route(&self) -> String {
        self.joint_route.borrow().clone()
    }
    pub fn restore_joint_history_route(&self, route: &str) -> Result<(), V8BindingError> {
        let group = self.joint_history.borrow().clone();
        let mut history = group.borrow_mut();
        if history.selected(route).is_none() {
            return Err(failure("InvalidStateError: missing saved joint route"));
        }
        let old = self.joint_route.borrow().clone();
        // Only remove the provisional route created for this freshly parsed
        // frame. A saved route never authenticates another live Document.
        if old != route
            && history.selected(&old).is_some_and(|entry| {
                self.navigation_history_snapshot()
                    .is_some_and(|current| entry.id == current.id)
            })
        {
            history
                .discard(&old)
                .map_err(|_| failure("InvalidStateError: joint route replacement"))?;
        }
        *self.joint_route.borrow_mut() = route.to_owned();
        Ok(())
    }
    /// A replacement Document inherits the navigable, never its old execution authority.
    pub fn inherit_joint_history_route(&self, previous: &V8BindingState) {
        *self.joint_history.borrow_mut() = previous.joint_history.borrow().clone();
        *self.joint_route.borrow_mut() = previous.joint_route.borrow().clone();
        self.joint_attach_step.set(previous.joint_attach_step.get());
        self.navigation_api.borrow_mut().joint_initial_replacement =
            previous.navigation_api.borrow().initial_document;
    }
    pub fn plan_joint_history(
        &self,
        delta: i64,
    ) -> Result<Option<crate::joint_history::Traversal>, V8BindingError> {
        self.joint_history
            .borrow()
            .borrow()
            .plan(delta)
            .map_err(|_| failure("InvalidStateError: joint history"))
    }
    pub fn commit_joint_history(
        &self,
        plan: &crate::joint_history::Traversal,
    ) -> Result<bool, V8BindingError> {
        self.joint_history
            .borrow()
            .borrow_mut()
            .commit(plan)
            .map_err(|_| failure("InvalidStateError: joint history"))
    }
}
impl V8BindingState {
    /// Script-close eligibility uses the full session history entry count,
    /// including entries hidden by Navigation.entries() origin filtering.
    pub fn native_history_has_single_entry(&self) -> bool {
        self.history.borrow().as_ref().map_or_else(
            || self.navigation_initial_document(),
            |history| history.entries.len() == 1,
        )
    }
    pub fn flush_native_history_projection(&self) -> Result<(), V8BindingError> {
        history_projection::flush(self)
    }
    pub fn set_native_history_publisher(&self, publisher: Rc<dyn NativeHistoryPublisher>) {
        self.navigation_api
            .borrow_mut()
            .history_projection
            .publisher = Some(publisher);
    }
    pub fn navigation_discard_pending(&self) -> bool {
        self.navigation_api.borrow().detaching
    }
    pub fn navigation_initial_document(&self) -> bool {
        self.navigation_api.borrow().initial_document
    }
    pub fn set_navigation_initial_document(&self, initial: bool) {
        let mut navigation = self.navigation_api.borrow_mut();
        if navigation.initial_document && !initial {
            navigation.joint_initial_replacement = true;
        }
        navigation.initial_document = initial;
    }
    pub fn pending_navigation_destination(&self) -> Option<NavigationHistorySnapshot> {
        self.navigation_api
            .borrow()
            .pending
            .as_ref()
            .map(|pending| pending.destination.clone())
    }
    pub fn navigation_history_snapshot(&self) -> Option<NavigationHistorySnapshot> {
        let navigation = self.navigation_api.borrow();
        navigation
            .current
            .as_ref()
            .and_then(|id| navigation.entries.get(id))
            .map(|entry| entry.snapshot.clone())
    }
    pub fn navigation_history_entries(&self) -> Vec<NavigationHistorySnapshot> {
        let navigation = self.navigation_api.borrow();
        navigation
            .order
            .iter()
            .filter_map(|id| {
                navigation
                    .entries
                    .get(id)
                    .map(|entry| entry.snapshot.clone())
            })
            .collect()
    }
    pub fn take_navigation_history_update(&self) -> Option<NavigationHistoryUpdate> {
        let needs_initial = {
            let navigation = self.navigation_api.borrow();
            navigation.load_restore.is_none()
                && navigation
                    .current
                    .as_ref()
                    .and_then(|id| navigation.entries.get(id))
                    .is_some_and(|entry| entry.snapshot.scroll.position.is_none())
        };
        if needs_initial {
            record_document_scroll(self, self.document_scroll_offset());
        }
        self.navigation_api.borrow_mut().history_update.take()
    }
}
pub(super) fn record_document_scroll(state: &V8BindingState, position: (f64, f64)) {
    if state.execution_retired() || !position.0.is_finite() || !position.1.is_finite() {
        return;
    }
    if state.navigation_api.borrow().restoring_scroll {
        return;
    }
    cancel_scroll_restoration(state);
    let changed = {
        let mut navigation = state.navigation_api.borrow_mut();
        let Some(id) = navigation.current.clone() else {
            return;
        };
        let Some(entry) = navigation.entries.get_mut(&id) else {
            return;
        };
        if entry.snapshot.scroll.position == Some(position) {
            false
        } else {
            entry.snapshot.scroll.position = Some(position);
            true
        }
    };
    if changed {
        publish_history_update(state, "replace");
    }
}
pub(super) fn set_scroll_restoration(
    state: &V8BindingState,
    value: &str,
) -> Result<(), V8BindingError> {
    let mode = match value {
        "auto" => HistoryScrollMode::Auto,
        "manual" => HistoryScrollMode::Manual,
        _ => return Err(failure("TypeError")),
    };
    if mode == HistoryScrollMode::Manual {
        cancel_scroll_restoration(state);
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        let id = navigation
            .current
            .clone()
            .ok_or_else(|| failure("InvalidStateError"))?;
        navigation
            .entries
            .get_mut(&id)
            .ok_or_else(|| failure("InvalidStateError"))?
            .snapshot
            .scroll
            .mode = mode;
    }
    if let Some(history) = state.history.borrow_mut().as_mut() {
        history.scroll_restoration = value.into();
    }
    publish_history_update(state, "replace");
    Ok(())
}

fn publish_history_update(state: &V8BindingState, kind: &str) {
    if let Some(snapshot) = state.navigation_history_snapshot() {
        let group = state.joint_history.borrow().clone();
        let route = state.joint_route.borrow().clone();
        if group
            .borrow()
            .selected(&route)
            .is_some_and(|selected| selected.id == snapshot.id)
        {
            let _ = group.borrow_mut().replace(&route, snapshot);
        }
    }
    let mut navigation = state.navigation_api.borrow_mut();
    let current = navigation
        .current
        .as_ref()
        .and_then(|id| navigation.order.iter().position(|entry| entry == id));
    if let Some(current) = current {
        let entries = navigation
            .order
            .iter()
            .map(|id| navigation.entries[id].snapshot.clone())
            .collect();
        let truncates_forward = kind == "push"
            || navigation
                .history_update
                .as_ref()
                .is_some_and(|update| update.truncates_forward);
        navigation.history_update = Some(NavigationHistoryUpdate {
            entries,
            current,
            navigation_type: kind.into(),
            truncates_forward,
        });
    }
}
pub(super) fn history_mutated(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    url: &str,
    kind: &str,
    value: Option<crate::message_wire::WireValue>,
) -> Result<bool, V8BindingError> {
    let document = AdmissionDocument::capture(state);
    ensure(scope, state)?;
    if !abort_ongoing(scope, state)? {
        return Ok(false);
    }
    // Capture the outgoing entry even when no scroll event or parent poll has
    // occurred yet (the initial viewport is a real saved position).
    record_document_scroll(state, state.document_scroll_offset());
    let mut entries = state.navigation_history_entries();
    let current_snapshot = state
        .navigation_history_snapshot()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let current = entries
        .iter()
        .position(|entry| entry.id == current_snapshot.id)
        .ok_or_else(|| failure("InvalidStateError"))?;
    let entry = NavigationHistorySnapshot {
        scroll: current_snapshot.scroll.clone(),
        key: if kind == "replace" {
            current_snapshot.key.clone()
        } else {
            token()?
        },
        id: token()?,
        url: url.into(),
        document_id: current_snapshot.document_id.clone(),
        state: None,
        history_state: value,
    };
    if entries_disabled(state) {
        let index = if kind == "replace" {
            entries[current] = entry;
            current
        } else {
            entries.truncate(current + 1);
            entries.push(entry);
            entries.len() - 1
        };
        state.commit_location(url)?;
        sync_history(scope, entries, index, kind)?;
        publish_history_update(state, kind);
        return Ok(true);
    }
    let committed = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("history navigation committed"))?;
    let finished = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("history navigation finished"))?;
    committed.get_promise(scope).mark_as_handled();
    finished.get_promise(scope).mark_as_handled();
    state.navigation_api.borrow_mut().pending = Some(PendingNavigation {
        execution_observer: None,
        committed: v8::Global::new(scope, committed),
        finished: v8::Global::new(scope, finished),
        destination: entry.clone(),
        navigation_type: kind.into(),
        event_id: None,
        transition: None,
        committed_entry: false,
        remaining: 0,
        plan: None,
    });
    let dispatched = dispatch_navigate_with_hash(
        scope,
        state,
        &entry,
        kind,
        current,
        &current_snapshot,
        None,
        Some(false),
    );
    if !document.current(state)
        && matches!(&dispatched,Err(V8BindingError::Unsupported{member,..}) if member=="AbortError")
    {
        return Ok(false);
    }
    let event_id = dispatched?;
    if !document.current(state) {
        return Ok(false);
    }
    if !state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .is_some_and(|pending| pending.event_id == Some(event_id))
    {
        return Ok(false);
    }
    if state
        .events
        .borrow()
        .get(&event_id)
        .is_some_and(|event| event.default_prevented)
    {
        abort_navigation(scope, "History mutation cancelled")?;
        return Ok(false);
    }
    let intercepted = state.navigation_api.borrow().events[&event_id].intercepted;
    if intercepted {
        prepare_transition(scope, state, event_id, &current_snapshot.id, kind)?;
    }
    let base_entries = entries.clone();
    let index = if kind == "replace" {
        entries[current] = entry;
        current
    } else {
        entries.truncate(current + 1);
        entries.push(entry);
        entries.len() - 1
    };
    begin_commit(
        scope,
        state,
        event_id,
        CommitPlan {
            base_entries,
            entries,
            current: index,
            from: current_snapshot.id,
            kind: kind.into(),
            intercepted,
        },
    )?;
    Ok(state
        .navigation_history_snapshot()
        .is_some_and(|entry| entry.url == url))
}

pub fn finish_navigation(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let state = current_state(scope)?;
    complete_navigation(scope, &state, None)
}

/// Window.stop aborts the active navigation synchronously, including a
/// dispatching NavigateEvent, without settling completed cross-document work.
pub(super) fn stop_navigation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    advance_native_revision(state)?;
    let event = state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .and_then(|pending| pending.event_id);
    if let Some(id) = event {
        if let Some(event) = state.events.borrow_mut().get_mut(&id) {
            if event.dispatching && event.cancelable {
                event.default_prevented = true;
            }
        }
    }
    let reason = dom_exception_value(scope, "AbortError")?;
    reject_pending(scope, state, reason)
}

/// Author removal aborts the pending navigation while its realm is still
/// callable. The host retains the closed frame until this suffix completes.
/// Reentrant removal cannot retire the environment beneath an abort listener.
pub fn abort_navigation_for_discard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    if state.execution_retired() {
        return Ok(true);
    }
    // Another retained abort may currently own a callback outside the queue.
    // Keep its realm alive until that stack publishes the remaining listeners.
    if native_platform::abort_dispatch_active(state) {
        state.navigation_api.borrow_mut().detaching = true;
        return Ok(false);
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.detach_busy {
            return Ok(false);
        }
        navigation.detach_busy = true;
        navigation.detaching = true;
    }
    let result = (|| {
        let global = state
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("discard navigation realm"))?;
        let context = global
            .get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("discard navigation context"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        // Reuse a previously admitted completion after publication failure.
        // Ordinary queued history notifications have no destination now.
        let reason = dom_exception_value(scope, "AbortError")?;
        {
            let mut navigation = state.navigation_api.borrow_mut();
            let signals = navigation
                .events
                .iter()
                .map(|(id, event)| (*id, event.signal_id))
                .collect::<BTreeMap<_, _>>();
            navigation.publications.retain_mut(|batch| {
                let Some(completion) = batch.completion.as_mut() else {
                    return false;
                };
                if completion.reason.is_none() {
                    completion.reason = Some(v8::Global::new(scope, reason));
                    completion.entry = None;
                    completion.focus = None;
                    completion.signal = completion
                        .pending
                        .event_id
                        .and_then(|id| signals.get(&id).copied());
                    completion.stage = 0;
                }
                batch.events.clear();
                true
            });
            navigation.publication_retry_at = None;
        }
        if state.navigation_api.borrow().pending.is_some() {
            reject_pending(scope, state, reason)?;
        }
        poll_publications(scope, state)?;
        Ok(state.navigation_api.borrow().pending.is_none()
            && state.navigation_api.borrow().publications.is_empty()
            && !native_platform::abort_publication::pending(state)
            && !scope.is_execution_terminating())
    })();
    state.navigation_api.borrow_mut().detach_busy = false;
    result
}

pub fn abort_navigation(scope: &mut v8::PinScope, _message: &str) -> Result<(), V8BindingError> {
    let state = current_state(scope)?;
    let reason = dom_exception_value(scope, "AbortError")?;
    reject_pending(scope, &state, reason)
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    if interface == "NavigateEvent" {
        event_operation(scope, state, object, member, args)?;
        return Ok(None);
    }
    let id = checked_receiver(scope, state, object, interface)?;
    if interface == "NavigationPrecommitController" {
        precommit_operation(scope, state, id, member, args)?;
        return Ok(None);
    }
    if interface == "NavigationDestination" && member == "getState" {
        let wire = state.navigation_api.borrow().destinations[&id]
            .snapshot
            .state
            .clone();
        return Ok(Some(match wire {
            Some(wire) => structured_clone::deserialize_storage(scope, wire)
                .ok_or(V8BindingError::JavaScriptException)?,
            None => v8::undefined(scope).into(),
        }));
    }
    if interface == "NavigationHistoryEntry" && member == "getState" {
        let wire = {
            let navigation = state.navigation_api.borrow();
            navigation
                .entries
                .values()
                .find(|entry| entry.platform_id == id)
                .and_then(|entry| entry.snapshot.state.clone())
        };
        return Ok(Some(match wire {
            Some(wire) => structured_clone::deserialize_storage(scope, wire)
                .ok_or(V8BindingError::JavaScriptException)?,
            None => v8::undefined(scope).into(),
        }));
    }
    if interface != "Navigation" {
        return Err(failure("TypeError"));
    }
    ensure(scope, state)?;
    if member == "entries" && entries_disabled(state) {
        return Ok(Some(v8::Array::new(scope, 0).into()));
    }
    if member == "entries" {
        let values = {
            let navigation = state.navigation_api.borrow();
            navigation
                .order
                .iter()
                .map(|id| v8::Local::new(scope, &navigation.entries[id].object).into())
                .collect::<Vec<v8::Local<v8::Value>>>()
        };
        return Ok(Some(v8::Array::new_with_elements(scope, &values).into()));
    }
    if member == "updateCurrentEntry" {
        if state.execution_retired() {
            return Err(failure("InvalidStateError"));
        }
        let options = args
            .get(0)
            .try_cast::<v8::Object>()
            .map_err(|_| failure("TypeError"))?;
        let key = v8::String::new(scope, "state").unwrap();
        if !options.has(scope, key.into()).unwrap_or(false) {
            return Err(failure("TypeError"));
        }
        let value = options
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        let wire = structured_clone::serialize_storage(scope, value)
            .ok_or(V8BindingError::JavaScriptException)?;
        if entries_disabled(state) {
            return Err(failure("InvalidStateError"));
        }
        let current = state
            .navigation_api
            .borrow()
            .current
            .clone()
            .ok_or_else(|| failure("InvalidStateError"))?;
        reserve_publication(state)?;
        let (id, event) = create_current_change(scope, state, &current, None)?;
        let mut delivery = publication(state);
        delivery
            .events
            .push_back(native_platform::publication::prepare_platform_dispatch(
                scope,
                state,
                id,
                "currententrychange",
                event,
            )?);
        state
            .navigation_api
            .borrow_mut()
            .entries
            .get_mut(&current)
            .unwrap()
            .snapshot
            .state = Some(wire);
        publish_history_update(state, "replace");
        enqueue_publication(&mut state.navigation_api.borrow_mut(), delivery);
        poll_publications(scope, state)?;
        return Ok(None);
    }
    let committed = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("navigation committed promise"))?;
    let finished = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("navigation finished promise"))?;
    let result = v8::Object::new(scope);
    for (name, promise) in [
        ("committed", committed.get_promise(scope)),
        ("finished", finished.get_promise(scope)),
    ] {
        let key = v8::String::new(scope, name).unwrap();
        result.create_data_property(scope, key.into(), promise.into());
    }
    let document = AdmissionDocument::capture(state);
    let action = (|| -> Result<(), V8BindingError> {
        if state.execution_retired()
            || state.is_unloading()
            || state.navigation_api.borrow().detaching
        {
            return Err(failure("InvalidStateError"));
        }
        let current = state
            .navigation_history_snapshot()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let entries = state.navigation_history_entries();
        let index = entries
            .iter()
            .position(|entry| entry.id == current.id)
            .ok_or_else(|| failure("InvalidStateError"))?;
        let options_index = if matches!(member, "navigate" | "traverseTo") {
            1
        } else {
            0
        };
        let options_value = args.get(options_index);
        let options = if options_value.is_null_or_undefined() {
            None
        } else {
            Some(
                options_value
                    .try_cast::<v8::Object>()
                    .map_err(|_| failure("TypeError"))?,
            )
        };
        let mut kind = "push";
        let mut destination = match member {
            "back" => {
                kind = "traverse";
                entries
                    .get(
                        index
                            .checked_sub(1)
                            .ok_or_else(|| failure("InvalidStateError"))?,
                    )
                    .cloned()
                    .ok_or_else(|| failure("InvalidStateError"))?
            }
            "forward" => {
                kind = "traverse";
                entries
                    .get(index + 1)
                    .cloned()
                    .ok_or_else(|| failure("InvalidStateError"))?
            }
            "traverseTo" => {
                kind = "traverse";
                let key = required_string(scope, args, 0)?;
                entries
                    .iter()
                    .find(|entry| entry.key == key)
                    .cloned()
                    .ok_or_else(|| failure("InvalidStateError"))?
            }
            "reload" => {
                kind = "reload";
                let mut destination = current.clone();
                destination.id = token()?;
                destination.document_id = token()?;
                destination
            }
            "navigate" => {
                let input = required_string(scope, args, 0)?;
                let base = document_base_url(scope, state, true)?;
                let url = Url::parse(&base)
                    .and_then(|base| base.join(&input))
                    .map_err(|_| failure("SyntaxError"))?;
                if url.scheme() == "javascript" {
                    return Err(failure("NotSupportedError"));
                }
                let history = event_init_member(scope, options, "history")
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !history.is_undefined() {
                    let history = history
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope);
                    match history.as_str() {
                        "replace" => kind = "replace",
                        "push" | "auto" => {}
                        _ => return Err(failure("TypeError")),
                    }
                }
                NavigationHistorySnapshot {
                    scroll: Default::default(),
                    key: if kind == "replace" {
                        current.key.clone()
                    } else {
                        token()?
                    },
                    id: token()?,
                    url: url.to_string(),
                    document_id: token()?,
                    state: None,
                    history_state: None,
                }
            }
            _ => return Err(failure("TypeError")),
        };
        if matches!(member, "navigate" | "reload") {
            let value = event_init_member(scope, options, "state")
                .ok_or(V8BindingError::JavaScriptException)?;
            if !value.is_undefined() || member == "navigate" {
                destination.state = Some(
                    structured_clone::serialize_storage(scope, value)
                        .ok_or(V8BindingError::JavaScriptException)?,
                );
            }
        }
        if !document.current(state) || state.navigation_api.borrow().detaching {
            return Err(failure("AbortError"));
        }
        if entries_disabled(state) {
            if !matches!(member, "navigate" | "reload") {
                return Err(failure("InvalidStateError"));
            }
            if !abort_ongoing(scope, state)? {
                return Err(failure("AbortError"));
            }
            location::navigate_from_api(
                scope,
                state,
                Url::parse(&destination.url).map_err(|_| failure("SyntaxError"))?,
                kind == "replace" || state.navigation_api.borrow().initial_document,
                kind == "reload",
            )?;
            return Ok(());
        }
        let mut old = Url::parse(&current.url).map_err(|_| failure("InvalidStateError"))?;
        let mut new = Url::parse(&destination.url).map_err(|_| failure("SyntaxError"))?;
        old.set_fragment(None);
        new.set_fragment(None);
        let mut same_document = kind == "traverse"
            && destination.document_id == current.document_id
            || kind != "reload"
                && Url::parse(&destination.url)
                    .ok()
                    .is_some_and(|url| url.fragment().is_some())
                && old == new;
        if same_document {
            destination.document_id = current.document_id.clone();
            if kind != "traverse" {
                destination.scroll = current.scroll.clone();
            }
        }
        if !abort_ongoing(scope, state)? {
            return Err(failure("AbortError"));
        }
        state.navigation_api.borrow_mut().pending = Some(PendingNavigation {
            execution_observer: None,
            committed: v8::Global::new(scope, committed),
            finished: v8::Global::new(scope, finished),
            destination: destination.clone(),
            navigation_type: kind.into(),
            event_id: None,
            transition: None,
            committed_entry: false,
            remaining: 0,
            plan: None,
        });
        let event_id =
            dispatch_navigate(scope, state, &destination, kind, index, &current, options)?;
        if !document.current(state) || state.navigation_api.borrow().detaching {
            return Err(failure("AbortError"));
        }
        if !state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(event_id))
        {
            return Ok(());
        }
        let cancelled = state
            .events
            .borrow()
            .get(&event_id)
            .is_some_and(|event| event.default_prevented);
        if cancelled {
            abort_navigation(scope, "Navigation was cancelled")?;
            return Ok(());
        }
        let intercepted = state.navigation_api.borrow().events[&event_id].intercepted;
        if intercepted {
            same_document = true;
            destination.document_id = current.document_id.clone();
            if matches!(kind, "push" | "replace") {
                destination.scroll = current.scroll.clone();
            }
            state
                .navigation_api
                .borrow_mut()
                .pending
                .as_mut()
                .unwrap()
                .destination = destination.clone();
            prepare_transition(scope, state, event_id, &current.id, kind)?;
        }
        if same_document {
            let base_entries = entries.clone();
            let mut entries = entries;
            let selected = if kind == "traverse" {
                entries
                    .iter()
                    .position(|entry| entry.id == destination.id)
                    .unwrap()
            } else if matches!(kind, "replace" | "reload") {
                entries[index] = destination.clone();
                index
            } else {
                entries.truncate(index + 1);
                entries.push(destination.clone());
                entries.len() - 1
            };
            begin_commit(
                scope,
                state,
                event_id,
                CommitPlan {
                    base_entries,
                    entries,
                    current: selected,
                    from: current.id.clone(),
                    kind: kind.into(),
                    intercepted,
                },
            )?;
        } else if kind == "traverse" {
            let selected = entries
                .iter()
                .position(|entry| entry.id == destination.id)
                .ok_or_else(|| failure("InvalidStateError"))?;
            history_tasks::enqueue(state, selected as i64 - index as i64)?;
        } else {
            location::navigate_from_api(
                scope,
                state,
                Url::parse(&destination.url).map_err(|_| failure("SyntaxError"))?,
                kind == "replace",
                kind == "reload",
            )?;
            if !state.execution_retired()
                && state
                    .navigation_api
                    .borrow()
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.event_id == Some(event_id))
            {
                if let Some(request) = state.pending_location_navigation.borrow_mut().as_mut() {
                    request.navigation_destination = Some(destination);
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = action {
        let reason = dom_exception_value(
            scope,
            match &error {
                V8BindingError::Unsupported { member, .. } => member.as_str(),
                _ => "InvalidStateError",
            },
        )?;
        committed.reject(scope, reason);
        finished.reject(scope, reason);
        let own = state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| v8::Local::new(scope, &pending.committed) == committed);
        if own {
            let retained = {
                let navigation = state.navigation_api.borrow();
                navigation
                    .pending
                    .as_ref()
                    .and_then(|pending| pending.event_id)
                    .is_some_and(|event| {
                        navigation
                            .events
                            .get(&event)
                            .is_some_and(|event| event.admission_failed)
                    })
            };
            if !retained {
                state.navigation_api.borrow_mut().pending = None;
            }
        }
    }
    Ok(Some(result.into()))
}

pub(super) fn history_traversed(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: &str,
) -> Result<(), V8BindingError> {
    let entries = state.navigation_history_entries();
    let current = entries
        .iter()
        .position(|entry| entry.id == id)
        .ok_or_else(|| failure("InvalidStateError"))?;
    let receipt = start_native_history_publication(
        scope,
        NavigationHistoryUpdate {
            entries,
            current,
            navigation_type: "traverse".into(),
            truncates_forward: false,
        },
    )?;
    state
        .navigation_api
        .borrow_mut()
        .native_receipts
        .remove(&receipt);
    publish_history_update(state, "traverse");
    Ok(())
}
pub fn sync_activation(
    scope: &mut v8::PinScope,
    from: Option<NavigationHistorySnapshot>,
    navigation_type: &str,
) -> Result<(), V8BindingError> {
    if !matches!(navigation_type, "push" | "replace" | "reload" | "traverse") {
        return Err(failure("TypeError"));
    }
    let state = current_state(scope)?;
    ensure(scope, &state)?;
    let from_id = from.as_ref().map(|entry| entry.id.clone());
    let snapshots = from.into_iter().collect::<Vec<_>>();
    let mut staged = stage_entries(scope, &state, &snapshots)?;
    let entry = {
        let navigation = state.navigation_api.borrow();
        if navigation.activations.len() >= 4096 {
            return Err(failure("QuotaExceededError"));
        }
        navigation
            .current
            .clone()
            .ok_or_else(|| failure("InvalidStateError"))?
    };
    staged
        .registrations
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("navigation activation staging"))?;
    let id = register_platform_object(&state, PlatformObjectRecord::EventTarget);
    staged.registrations.push(id);
    let object = new_platform_object(scope, &state, "NavigationActivation", id)?;
    let activation = Activation {
        object: v8::Global::new(scope, object),
        entry,
        from: from_id,
        navigation_type: navigation_type.into(),
    };
    staged.commit();
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.activations.insert(id, activation);
    navigation.activation = Some(id);
    if matches!(navigation_type, "traverse" | "reload") && !state.document_completely_loaded() {
        navigation.load_restore_is_traversal = navigation_type == "traverse";
        let snapshot = navigation
            .current
            .as_ref()
            .and_then(|id| navigation.entries.get(id))
            .map(|entry| entry.snapshot.clone());
        navigation.load_restore = snapshot.map(|entry| (AdmissionDocument::capture(&state), entry));
    }
    Ok(())
}

fn destination_object(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    snapshot: NavigationHistorySnapshot,
    index: i64,
    same_document: bool,
) -> Result<u64, V8BindingError> {
    if state.navigation_api.borrow().destinations.len() >= 4096 {
        return Err(failure("QuotaExceededError"));
    }
    let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
    let object = new_platform_object(scope, state, "NavigationDestination", id)?;
    state.navigation_api.borrow_mut().destinations.insert(
        id,
        Destination {
            object: v8::Global::new(scope, object),
            snapshot,
            index,
            same_document,
        },
    );
    Ok(id)
}
fn dispatch_navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    destination: &NavigationHistorySnapshot,
    kind: &str,
    index: usize,
    from: &NavigationHistorySnapshot,
    options: Option<v8::Local<'s, v8::Object>>,
) -> Result<u64, V8BindingError> {
    dispatch_navigate_with_hash(scope, state, destination, kind, index, from, options, None)
}
fn dispatch_navigate_with_hash<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    destination: &NavigationHistorySnapshot,
    kind: &str,
    _index: usize,
    from: &NavigationHistorySnapshot,
    options: Option<v8::Local<'s, v8::Object>>,
    hash_change: Option<bool>,
) -> Result<u64, V8BindingError> {
    let mut admission =
        prepare_navigate_dispatch(scope, state, destination, kind, from, options, hash_change)?;
    let result = admission.notification.resume(scope, state);
    if !admission.document.current(state) {
        admission.notification.finish(scope, state);
        return Err(result.err().unwrap_or_else(|| failure("AbortError")));
    }
    if matches!(result, Ok(true)) && !scope.is_execution_terminating() {
        admission.notification.finish(scope, state);
        if let Some(event) = state
            .navigation_api
            .borrow_mut()
            .events
            .get_mut(&admission.event)
        {
            event.admission_complete = true;
        }
        return Ok(admission.event);
    }
    retain_failed_navigate_dispatch(state, admission);
    Err(result.err().unwrap_or(V8BindingError::JavaScriptException))
}
// Finish allocation/preparation before entering author navigate listeners.
#[inline(never)]
fn prepare_navigate_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    destination: &NavigationHistorySnapshot,
    kind: &str,
    from: &NavigationHistorySnapshot,
    options: Option<v8::Local<'s, v8::Object>>,
    hash_change: Option<bool>,
) -> Result<Box<AdmissionPublication>, V8BindingError> {
    let admission_document = AdmissionDocument::capture(state);
    let native_source = state
        .navigation_api
        .borrow_mut()
        .native_source
        .take()
        .unwrap_or_default();
    let target = Url::parse(&destination.url).map_err(|_| failure("SyntaxError"))?;
    let source = Url::parse(&from.url).map_err(|_| failure("InvalidStateError"))?;
    let same_document = destination.document_id == from.document_id;
    let can_intercept = target.origin() == source.origin() && !state.execution_retired();
    let destination_index = if matches!(kind, "traverse" | "reload") {
        state
            .navigation_history_entries()
            .iter()
            .position(|entry| entry.id == destination.id)
            .map(|index| index as i64)
            .unwrap_or(-1)
    } else {
        -1
    };
    let mut destination_snapshot = destination.clone();
    if matches!(kind, "push" | "replace") {
        destination_snapshot.key.clear();
        destination_snapshot.id.clear();
    }
    let destination_id = destination_object(
        scope,
        state,
        destination_snapshot,
        destination_index,
        same_document,
    )?;
    let info =
        event_init_member(scope, options, "info").ok_or(V8BindingError::JavaScriptException)?;
    if !state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .is_some_and(|pending| pending.destination.id == destination.id)
        || !admission_document.current(state)
    {
        return Err(failure("AbortError"));
    }
    let signal_id = register_platform_object(
        state,
        PlatformObjectRecord::AbortSignal(AbortSignalRecord {
            aborted: false,
            reason: None,
            followers: Vec::new(),
            foreign_followers: Vec::new(),
            foreign_sources: Vec::new(),
        }),
    );
    let event =
        create_event_object_with_interface(scope, state, "NavigateEvent", "navigate", false, true)?;
    let event_id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&event_id) {
        record.is_trusted = true;
    }
    let null: v8::Local<v8::Value> = v8::null(scope).into();
    let signal = new_platform_object(scope, state, "AbortSignal", signal_id)?;
    let destination_object = {
        let navigation = state.navigation_api.borrow();
        v8::Local::new(scope, &navigation.destinations[&destination_id].object)
    };
    state.navigation_api.borrow_mut().events.insert(
        event_id,
        NavigateRecord {
            object: v8::Global::new(scope, event),
            destination: destination_id,
            destination_object: v8::Global::new(scope, destination_object),
            navigation_type: kind.into(),
            can_intercept,
            hash_change: hash_change
                .unwrap_or(same_document && target.fragment() != source.fragment()),
            user_initiated: false,
            signal_id,
            signal_object: v8::Global::new(scope, signal),
            info: v8::Global::new(scope, info),
            form_data: v8::Global::new(scope, null),
            download: v8::Global::new(scope, null),
            visual_transition: false,
            source_element: v8::Global::new(scope, null),
            intercepted: false,
            handlers: Vec::new(),
            precommit_handlers: Vec::new(),
            focus_reset: "after-transition".into(),
            focus_revision: state.focus_transition_revision.get(),
            scroll: "after-transition".into(),
            scrolled: false,
            committed: false,
            trusted: true,
            admission_complete: false,
            admission_failed: false,
        },
    );
    let source_owner = match native_source.source_window.as_ref() {
        Some(global) => {
            let global = v8::Local::new(scope, global);
            if crate::v8_context_bridge::can_access_window(scope, global) {
                binding_state_for_receiver(scope, global).filter(|owner| !owner.execution_retired())
            } else {
                None
            }
        }
        None => Some(state.clone()),
    };
    let source_visible = native_source_can_fire_cross_document(scope, state, &native_source)?;
    let source_element: v8::Local<v8::Value> = if source_visible {
        match &native_source.source_element_object {
            Some(object) => v8::Local::new(scope, object).into(),
            None => match native_source.source_element {
                Some(node) => {
                    wrap_node(scope, source_owner.as_ref().unwrap_or(state), node)?.into()
                }
                None => v8::null(scope).into(),
            },
        }
    } else {
        v8::null(scope).into()
    };
    let form_data: v8::Local<v8::Value> = match (native_source.form_data, source_visible) {
        (Some(data), true) => v8::Local::new(scope, &data).into(),
        _ => v8::null(scope).into(),
    };
    let download = match native_source.download {
        Some(value) => v8_string(scope, &value)?,
        None => v8::null(scope).into(),
    };
    {
        let mut navigation = state.navigation_api.borrow_mut();
        let event = navigation.events.get_mut(&event_id).unwrap();
        event.user_initiated = native_source.user_initiated;
        event.source_element = v8::Global::new(scope, source_element);
        event.form_data = v8::Global::new(scope, form_data);
        event.download = v8::Global::new(scope, download);
    }
    state
        .navigation_api
        .borrow_mut()
        .pending
        .as_mut()
        .ok_or_else(|| failure("AbortError"))?
        .event_id = Some(event_id);
    let id = state.navigation_api.borrow().platform_id.unwrap();
    reserve_publication(state)?;
    let reason = dom_exception_value(scope, "AbortError")?;
    let pending = state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .filter(|pending| pending.event_id == Some(event_id))
        .cloned()
        .ok_or_else(|| failure("AbortError"))?;
    let notification = native_platform::publication::pending_platform_dispatch(
        scope, state, id, "navigate", event,
    );
    Ok(Box::new(AdmissionPublication {
        document: admission_document,
        event: event_id,
        notification,
        pending,
        reason: v8::Global::new(scope, reason),
        complete: false,
        cancellation: None,
    }))
}
#[inline(never)]
fn retain_failed_navigate_dispatch(
    state: &Rc<V8BindingState>,
    admission: Box<AdmissionPublication>,
) {
    // Infrastructure failure revokes admission, retaining the exact event tail.
    if let Some(record) = state
        .navigation_api
        .borrow_mut()
        .events
        .get_mut(&admission.event)
    {
        record.admission_failed = true;
    }
    if let Some(record) = state.events.borrow_mut().get_mut(&admission.event) {
        if record.cancelable {
            record.default_prevented = true;
        }
    }
    let mut batch = publication(state);
    batch.admission = Some(admission);
    enqueue_publication(&mut state.navigation_api.borrow_mut(), batch);
    state.navigation_api.borrow_mut().publication_retry_at =
        Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
}

struct AdmissionPublication {
    document: AdmissionDocument,
    event: u64,
    notification: native_platform::publication::Notification,
    pending: PendingNavigation,
    reason: v8::Global<v8::Value>,
    complete: bool,
    cancellation: Option<Completion>,
}
impl AdmissionPublication {
    fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        if !self.complete {
            if !self.notification.resume(scope, state)? {
                return Ok(false);
            }
            self.notification.finish(scope, state);
            self.complete = true;
            if let Some(event) = state
                .navigation_api
                .borrow_mut()
                .events
                .get_mut(&self.event)
            {
                event.admission_complete = true;
            }
        }
        if state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(self.event))
        {
            let reason = v8::Local::new(scope, &self.reason);
            reject_pending(scope, state, reason)?;
            return Ok(true);
        }
        // A reentrant navigation may have replaced the global pending slot.
        // Settle only the captured old resolvers and abort its captured signal.
        if self.cancellation.is_none() {
            let transition = self.pending.transition.and_then(|id| {
                state
                    .navigation_api
                    .borrow()
                    .transitions
                    .get(&id)
                    .map(|transition| (transition.committed.clone(), transition.finished.clone()))
            });
            let signal = state
                .navigation_api
                .borrow()
                .events
                .get(&self.event)
                .map(|event| event.signal_id);
            self.cancellation = Some(Completion {
                document: self.document,
                pending: self.pending.clone(),
                entry: None,
                reason: Some(self.reason.clone()),
                transition,
                focus: None,
                stage: 0,
                signal,
            });
        }
        self.cancellation.as_mut().unwrap().resume(scope, state)
    }
}
fn admission_allows_native(navigation: &NavigationState, event: u64) -> bool {
    navigation
        .events
        .get(&event)
        .is_some_and(|event| event.admission_complete && !event.admission_failed)
}
fn event_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    if object.is_proxy() {
        return Err(failure("TypeError"));
    }
    let id = event_id(scope, object)?;
    {
        let navigation = state.navigation_api.borrow();
        let event = navigation
            .events
            .get(&id)
            .ok_or_else(|| failure("TypeError"))?;
        if v8::Local::new(scope, &event.object) != object {
            return Err(failure("TypeError"));
        }
        if !event.trusted {
            return Err(failure("SecurityError"));
        }
    }
    if member == "scroll" {
        let mut navigation = state.navigation_api.borrow_mut();
        if !navigation
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(id))
        {
            return Err(failure("InvalidStateError"));
        }
        let event = navigation.events.get_mut(&id).unwrap();
        if !event.intercepted || !event.committed || event.scrolled {
            return Err(failure("InvalidStateError"));
        }
        event.scrolled = true;
        drop(navigation);
        perform_navigation_scroll(state)?;
        return Ok(());
    }
    if member != "intercept" {
        return Err(failure("TypeError"));
    }
    let record = state
        .events
        .borrow()
        .get(&id)
        .cloned()
        .ok_or_else(|| failure("InvalidStateError"))?;
    if !record.dispatching || record.default_prevented {
        return Err(failure("InvalidStateError"));
    }
    if !state.navigation_api.borrow().events[&id].can_intercept {
        return Err(failure("SecurityError"));
    }
    let value = args.get(0);
    let options = if value.is_null_or_undefined() {
        None
    } else {
        Some(
            value
                .try_cast::<v8::Object>()
                .map_err(|_| failure("TypeError"))?,
        )
    };
    let focus = event_init_member(scope, options, "focusReset")
        .ok_or(V8BindingError::JavaScriptException)?;
    let handler =
        event_init_member(scope, options, "handler").ok_or(V8BindingError::JavaScriptException)?;
    let precommit = event_init_member(scope, options, "precommitHandler")
        .ok_or(V8BindingError::JavaScriptException)?;
    let scroll =
        event_init_member(scope, options, "scroll").ok_or(V8BindingError::JavaScriptException)?;
    let focus = if focus.is_undefined() {
        None
    } else {
        Some(
            focus
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope),
        )
    };
    let scroll = if scroll.is_undefined() {
        None
    } else {
        Some(
            scroll
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope),
        )
    };
    if focus
        .as_deref()
        .is_some_and(|value| !matches!(value, "after-transition" | "manual"))
        || scroll
            .as_deref()
            .is_some_and(|value| !matches!(value, "after-transition" | "manual"))
    {
        return Err(failure("TypeError"));
    }
    let handler = if handler.is_undefined() {
        None
    } else {
        Some(
            handler
                .try_cast::<v8::Function>()
                .map_err(|_| failure("TypeError"))?,
        )
    };
    let precommit = if precommit.is_undefined() {
        None
    } else {
        Some(
            precommit
                .try_cast::<v8::Function>()
                .map_err(|_| failure("TypeError"))?,
        )
    };
    if precommit.is_some() && !record.cancelable {
        return Err(failure("InvalidStateError"));
    }
    if !state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.dispatching && !event.default_prevented)
    {
        return Err(failure("InvalidStateError"));
    }
    let mut navigation = state.navigation_api.borrow_mut();
    let event = navigation
        .events
        .get_mut(&id)
        .ok_or_else(|| failure("InvalidStateError"))?;
    event.intercepted = true;
    if let Some(focus) = focus {
        event.focus_reset = focus;
    }
    if let Some(scroll) = scroll {
        event.scroll = scroll;
    }
    if let Some(handler) = handler {
        if event.handlers.len() >= 256 {
            return Err(failure("QuotaExceededError"));
        }
        event.handlers.push(v8::Global::new(scope, handler));
    }
    if let Some(precommit) = precommit {
        if event.precommit_handlers.len() >= 256 {
            return Err(failure("QuotaExceededError"));
        }
        event
            .precommit_handlers
            .push(v8::Global::new(scope, precommit));
    }
    Ok(())
}
fn commit_pending(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_id: u64,
) -> Result<(), V8BindingError> {
    if state
        .navigation_api
        .borrow()
        .events
        .get(&event_id)
        .is_some_and(|event| event.committed)
    {
        return Ok(());
    }
    let (current, pending, transition) = {
        let navigation = state.navigation_api.borrow();
        let current = navigation
            .current
            .clone()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let pending = navigation
            .pending
            .as_ref()
            .filter(|pending| pending.event_id == Some(event_id))
            .cloned()
            .ok_or_else(|| failure("AbortError"))?;
        let transition = pending.transition.and_then(|id| {
            navigation
                .transitions
                .get(&id)
                .map(|transition| transition.committed.clone())
        });
        (current, pending, transition)
    };
    let entry = entry_object(scope, state, &current)?;
    reserve_publication(state)?;
    let mut delivery = publication(state);
    delivery.settlement = Some(CommitSettlement {
        event: event_id,
        entry: v8::Global::new(scope, entry),
        pending,
        transition,
        committed: false,
        transition_committed: false,
    });
    enqueue_publication(&mut state.navigation_api.borrow_mut(), delivery);
    poll_publications(scope, state)?;
    Ok(())
}
fn run_intercept_handlers(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_id: u64,
    _from: &str,
    _kind: &str,
) -> Result<(), V8BindingError> {
    reserve_publication(state)?;
    let handlers = state.navigation_api.borrow().events[&event_id]
        .handlers
        .clone();
    let count = handlers.len();
    let mut batch = publication(state);
    batch.script = Some(ScriptPublication::Handlers(HandlerPublication {
        event: event_id,
        precommit: false,
        handlers: handlers.into(),
        controller: None,
        active: None,
    }));
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if let Some(pending) = navigation
            .pending
            .as_mut()
            .filter(|pending| pending.event_id == Some(event_id))
        {
            pending.remaining = count;
        }
        navigation
            .events
            .get_mut(&event_id)
            .unwrap()
            .handlers
            .clear();
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, state);
    Ok(())
}
#[cfg(test)]
thread_local! {static FAIL_HANDLER_AFTER_CALL:Cell<bool>=const{Cell::new(false)};}
struct ActiveHandler {
    resolver: v8::Global<v8::PromiseResolver>,
    value: Option<v8::Global<v8::Value>>,
    threw: bool,
}
struct HandlerPublication {
    event: u64,
    precommit: bool,
    handlers: std::collections::VecDeque<v8::Global<v8::Function>>,
    controller: Option<v8::Global<v8::Object>>,
    active: Option<ActiveHandler>,
}
enum ScriptPublication {
    HistoryEvent {
        previous: String,
        current: String,
        traversed: bool,
    },
    Handlers(HandlerPublication),
    Commit {
        event: u64,
        plan: CommitPlan,
        stage: u8,
        receipt: Option<u64>,
    },
}
impl ScriptPublication {
    fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        match self {
            Self::HistoryEvent {
                previous,
                current,
                traversed,
            } => {
                let receipt = start_native_event_publication(
                    scope,
                    NativePublicationEvent::History {
                        previous: previous.clone(),
                        current: current.clone(),
                        traversed: *traversed,
                    },
                )?;
                state
                    .navigation_api
                    .borrow_mut()
                    .native_receipts
                    .remove(&receipt);
                Ok(true)
            }
            Self::Handlers(batch) => batch.resume(scope, state),
            Self::Commit {
                event,
                plan,
                stage,
                receipt,
            } => {
                if !state
                    .navigation_api
                    .borrow()
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.event_id == Some(*event))
                {
                    return Ok(true);
                }
                if *stage == 0 {
                    state.commit_location(&plan.entries[plan.current].url)?;
                    *stage = 1;
                }
                if *stage == 1 {
                    *receipt = Some(start_native_history_publication(
                        scope,
                        NavigationHistoryUpdate {
                            entries: plan.entries.clone(),
                            current: plan.current,
                            navigation_type: plan.kind.clone(),
                            truncates_forward: plan.kind == "push",
                        },
                    )?);
                    *stage = 2;
                }
                if *stage == 2 {
                    if !resume_native_history_publication(scope, receipt.unwrap())? {
                        return Ok(false);
                    }
                    *stage = 3;
                }
                if !state
                    .navigation_api
                    .borrow()
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.event_id == Some(*event))
                {
                    return Ok(true);
                }
                if *stage == 3 {
                    if plan.intercepted && plan.kind == "traverse" {
                        queue_intercepted_popstate(
                            scope,
                            state,
                            &plan.entries[plan.current].id,
                            *event,
                        )?;
                    }
                    *stage = 4;
                }
                if *stage == 4 {
                    publish_history_update(state, &plan.kind);
                    commit_pending(scope, state, *event)?;
                    *stage = 5;
                }
                if *stage == 5 {
                    if plan.intercepted {
                        run_intercept_handlers(scope, state, *event, &plan.from, &plan.kind)?;
                    } else {
                        queue_navigation_finish(scope, state, *event)?;
                    }
                    *stage = 6;
                }
                Ok(true)
            }
        }
    }
}
impl HandlerPublication {
    fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        if !state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(self.event))
        {
            return Ok(true);
        }
        if self.handlers.is_empty() {
            if self.precommit {
                apply_commit(scope, state, self.event)?;
            } else {
                queue_navigation_finish(scope, state, self.event)?;
            }
            return Ok(true);
        }
        if self.precommit && self.controller.is_none() {
            let mut staged = stage_entries(scope, state, &[])?;
            staged
                .registrations
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("precommit controller staging"))?;
            let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
            staged.registrations.push(id);
            let object = new_platform_object(scope, state, "NavigationPrecommitController", id)?;
            let object = v8::Global::new(scope, object);
            staged.commit();
            state.navigation_api.borrow_mut().controllers.insert(
                id,
                PrecommitController {
                    object: object.clone(),
                    event: self.event,
                },
            );
            self.controller = Some(object);
        }
        while let Some(handler) = self.handlers.front() {
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            if !state
                .navigation_api
                .borrow()
                .pending
                .as_ref()
                .is_some_and(|pending| pending.event_id == Some(self.event))
            {
                return Ok(true);
            }
            if self.active.is_none() {
                let promise = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("navigation handler promise"))?;
                let data = v8::Number::new(scope, self.event as f64);
                let success = if self.precommit {
                    v8::Function::builder(precommit_fulfilled)
                        .data(data.into())
                        .build(scope)
                } else {
                    v8::Function::builder(intercept_fulfilled)
                        .data(data.into())
                        .build(scope)
                }
                .ok_or(V8BindingError::Allocation("navigation handler fulfillment"))?;
                let failure = v8::Function::builder(intercept_rejected)
                    .data(data.into())
                    .build(scope)
                    .ok_or(V8BindingError::Allocation("navigation handler rejection"))?;
                promise
                    .get_promise(scope)
                    .then2(scope, success, failure)
                    .ok_or(V8BindingError::JavaScriptException)?;
                self.active = Some(ActiveHandler {
                    resolver: v8::Global::new(scope, promise),
                    value: None,
                    threw: false,
                });
            }
            let active = self.active.as_mut().unwrap();
            if active.value.is_none() {
                v8::tc_scope!(let tc,scope);
                let callback = v8::Local::new(tc, handler);
                let receiver = v8::undefined(tc);
                let args = self
                    .controller
                    .as_ref()
                    .map(|controller| vec![v8::Local::new(tc, controller).into()])
                    .unwrap_or_default();
                let value = callback.call(tc, receiver.into(), &args);
                active.threw = value.is_none();
                let value = value
                    .or_else(|| tc.exception())
                    .unwrap_or_else(|| v8::undefined(tc).into());
                active.value = Some(v8::Global::new(tc, value));
                if !tc.is_execution_terminating() {
                    tc.reset();
                }
            }
            #[cfg(test)]
            if FAIL_HANDLER_AFTER_CALL.with(|failure| failure.replace(false)) {
                return Err(V8BindingError::Allocation(
                    "injected navigation handler publication failure",
                ));
            }
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            let value = v8::Local::new(scope, active.value.as_ref().unwrap());
            if active.threw {
                reject_pending(scope, state, value)?;
                return Ok(true);
            }
            v8::Local::new(scope, &active.resolver)
                .resolve(scope, value)
                .ok_or(V8BindingError::JavaScriptException)?;
            self.handlers.pop_front();
            self.active = None;
        }
        Ok(true)
    }
}

fn intercept_fulfilled(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(state) = current_state(scope) else {
        return;
    };
    let id = args.data().integer_value(scope).unwrap_or(-1) as u64;
    let finished = {
        let mut navigation = state.navigation_api.borrow_mut();
        let Some(pending) = navigation
            .pending
            .as_mut()
            .filter(|pending| pending.event_id == Some(id))
        else {
            return;
        };
        pending.remaining = pending.remaining.saturating_sub(1);
        pending.remaining == 0
    };
    if finished {
        {
            let mut navigation = state.navigation_api.borrow_mut();
            navigation.continuation_requests.insert((id, false));
            navigation.publication_retry_at = None;
        }
        if let Err(error) = poll_publications(scope, &state) {
            throw_binding_error(scope, &error);
        }
    }
}
fn intercept_rejected(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(state) = current_state(scope) else {
        return;
    };
    let id = args.data().integer_value(scope).unwrap_or(-1) as u64;
    if state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .is_some_and(|pending| pending.event_id == Some(id))
    {
        {
            let reason = v8::Global::new(scope, args.get(0));
            let mut navigation = state.navigation_api.borrow_mut();
            navigation.pending_rejections.entry(id).or_insert(reason);
            navigation.publication_retry_at = None;
        }
        if let Err(error) = poll_publications(scope, &state) {
            throw_binding_error(scope, &error);
        }
    }
}
fn reject_pending(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if let Some(entry) = state.pending_navigation_destination() {
        state.fail_joint_history_entry(
            &entry.id,
            crate::joint_history::JointTraversalFailure::Canceled,
        );
    }
    complete_navigation(scope, state, Some(reason))
}
struct Completion {
    document: AdmissionDocument,
    pending: PendingNavigation,
    entry: Option<v8::Global<v8::Object>>,
    reason: Option<v8::Global<v8::Value>>,
    transition: Option<(
        v8::Global<v8::PromiseResolver>,
        v8::Global<v8::PromiseResolver>,
    )>,
    focus: Option<native_platform::focus_publication::Transition>,
    stage: u8,
    signal: Option<u64>,
}
impl Completion {
    fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<bool, V8BindingError> {
        loop {
            if scope.is_execution_terminating() {
                return Ok(false);
            }
            if !self.document.current(state) {
                if let Some(observer) = &self.pending.execution_observer {
                    observer.complete_document_operation(
                        crate::navigation::NavigationExecutionOutcome::Canceled,
                    );
                }
                return Ok(true);
            }
            let reason = self
                .reason
                .as_ref()
                .map(|reason| v8::Local::new(scope, reason));
            match self.stage {
                0 => {
                    let resolver = v8::Local::new(scope, &self.pending.committed);
                    if let Some(reason) = reason {
                        resolver
                            .reject(scope, reason)
                            .ok_or(V8BindingError::JavaScriptException)?;
                    } else {
                        let entry = v8::Local::new(scope, self.entry.as_ref().unwrap());
                        resolver
                            .resolve(scope, entry.into())
                            .ok_or(V8BindingError::JavaScriptException)?;
                    }
                    self.stage = 1;
                }
                1 => {
                    if let Some(reason) = reason {
                        v8::Local::new(scope, &self.pending.finished)
                            .reject(scope, reason)
                            .ok_or(V8BindingError::JavaScriptException)?;
                    }
                    self.stage = 2;
                }
                2 => {
                    if let Some((committed, _)) = &self.transition {
                        let resolver = v8::Local::new(scope, committed);
                        if let Some(reason) = reason {
                            resolver
                                .reject(scope, reason)
                                .ok_or(V8BindingError::JavaScriptException)?;
                        } else {
                            let value = v8::undefined(scope);
                            resolver
                                .resolve(scope, value.into())
                                .ok_or(V8BindingError::JavaScriptException)?;
                        }
                    }
                    self.stage = 3;
                }
                3 => {
                    if let (Some(reason), Some((_, finished))) = (reason, &self.transition) {
                        v8::Local::new(scope, finished)
                            .reject(scope, reason)
                            .ok_or(V8BindingError::JavaScriptException)?;
                    }
                    self.stage = 4;
                }
                4 => {
                    if let Some(focus) = &mut self.focus {
                        if !focus.resume(scope, state)? {
                            return Ok(false);
                        }
                    }
                    self.stage = 5;
                }
                5 => {
                    if let (Some(signal), Some(reason)) = (self.signal, reason) {
                        // start owns the dependency batch before any callback. A
                        // later retry drains that batch even though aborted is true.
                        abort_signal_with_reason(scope, state, signal, reason)?;
                        if native_platform::abort_publication::pending(state) {
                            return Ok(false);
                        }
                    }
                    self.stage = 6;
                }
                6 => {
                    if reason.is_none() {
                        let scroll = self
                            .pending
                            .event_id
                            .and_then(|id| {
                                state.navigation_api.borrow().events.get(&id).map(|event| {
                                    event.intercepted
                                        && event.scroll == "after-transition"
                                        && !event.scrolled
                                })
                            })
                            .unwrap_or(false);
                        if scroll && state.navigation_api.borrow().pending.is_none() {
                            perform_navigation_scroll(state)?;
                        }
                    }
                    self.stage = 7;
                }
                7 => {
                    if reason.is_none() {
                        let entry = v8::Local::new(scope, self.entry.as_ref().unwrap());
                        v8::Local::new(scope, &self.pending.finished)
                            .resolve(scope, entry.into())
                            .ok_or(V8BindingError::JavaScriptException)?;
                    }
                    self.stage = 8;
                }
                8 => {
                    if reason.is_none() {
                        if let Some((_, finished)) = &self.transition {
                            let value = v8::undefined(scope);
                            v8::Local::new(scope, finished)
                                .resolve(scope, value.into())
                                .ok_or(V8BindingError::JavaScriptException)?;
                        }
                    }
                    self.stage = 9;
                }
                _ => {
                    if state.navigation_api.borrow().finishing_transition == self.pending.transition
                    {
                        state.navigation_api.borrow_mut().finishing_transition = None;
                    }
                    if let Some(observer) = &self.pending.execution_observer {
                        observer.complete_document_operation(if self.reason.is_some() {
                            crate::navigation::NavigationExecutionOutcome::Canceled
                        } else {
                            crate::navigation::NavigationExecutionOutcome::Completed
                        });
                    }
                    return Ok(true);
                }
            }
        }
    }
}
fn complete_navigation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    reason: Option<v8::Local<v8::Value>>,
) -> Result<(), V8BindingError> {
    let Some(pending) = state.navigation_api.borrow().pending.clone() else {
        return Ok(());
    };
    reserve_publication(state)?;
    let mut batch = publication(state);
    let intercepted = pending.event_id.is_some_and(|id| {
        state
            .navigation_api
            .borrow()
            .events
            .get(&id)
            .is_some_and(|event| event.intercepted)
    });
    if reason.is_none() && pending.navigation_type == "traverse" {
        batch.joint_finish = Some((
            pending.destination.id.clone(),
            state.document_stream_epoch(),
        ));
        if intercepted {
            if let Some(barrier) = pending.event_id.and_then(|id| {
                state
                    .navigation_api
                    .borrow()
                    .intercepted_history_barriers
                    .get(&id)
                    .cloned()
            }) {
                batch.joint_barrier = Some((barrier, 2));
            }
        }
    }
    let entry = if reason.is_none() {
        let snapshot = state
            .navigation_history_snapshot()
            .ok_or_else(|| failure("InvalidStateError"))?;
        {
            let object = entry_object(scope, state, &snapshot.id)?;
            Some(v8::Global::new(scope, object))
        }
    } else {
        None
    };
    let transition = pending.transition.and_then(|id| {
        state
            .navigation_api
            .borrow()
            .transitions
            .get(&id)
            .map(|transition| (transition.committed.clone(), transition.finished.clone()))
    });
    let reset = pending
        .event_id
        .and_then(|id| {
            state.navigation_api.borrow().events.get(&id).map(|event| {
                event.intercepted
                    && event.committed
                    && event.focus_reset == "after-transition"
                    && event.focus_revision == state.focus_transition_revision.get()
            })
        })
        .unwrap_or(false);
    let focus = if reset {
        let mut target = state.document_root();
        let nodes = {
            let dom = state.dom.borrow();
            traversal_nodes(&dom, state.document_root())?
        };
        for node in nodes {
            let autofocus = {
                let dom = state.dom.borrow();
                matches!(dom.arena().get(node)?.kind, NodeKind::Element { .. })
                    && dom
                        .attribute_in_null_namespace(node, "autofocus")?
                        .is_some()
            };
            if autofocus
                && focus_navigation::eligible_for(state, node, false)?
                && !css_suppresses_focus(state, node)?
            {
                if let Some(node) = focus_navigation::delegated_target(state, node)? {
                    target = node;
                    break;
                }
            }
        }
        Some(native_platform::focus_publication::Transition::new(
            state, target,
        ))
    } else {
        None
    };
    let signal = reason.and_then(|_| pending.event_id).and_then(|id| {
        state
            .navigation_api
            .borrow()
            .events
            .get(&id)
            .map(|event| event.signal_id)
    });
    let name = if reason.is_some() {
        "navigateerror"
    } else {
        "navigatesuccess"
    };
    let event = if let Some(reason) = reason {
        let event =
            create_event_object_with_interface(scope, state, "ErrorEvent", name, false, false)?;
        let empty = v8::String::new(scope, "")
            .ok_or(V8BindingError::Allocation("navigation error message"))?;
        let zero = v8::Integer::new(scope, 0);
        let payload = v8::Array::new_with_elements(
            scope,
            &[empty.into(), empty.into(), zero.into(), zero.into(), reason],
        );
        event.set_internal_field(2, payload.into());
        let brand = {
            let mut brand = state.error_event_brand.borrow_mut();
            if brand.is_none() {
                let symbol = v8::Symbol::new(scope, None);
                *brand = Some(v8::Global::new(scope, symbol));
            }
            v8::Local::new(scope, brand.as_ref().unwrap())
        };
        event.set_internal_field(3, brand.into());
        event
    } else {
        create_event_object(scope, state, name, false, false)?
    };
    let id = state
        .navigation_api
        .borrow()
        .platform_id
        .ok_or_else(|| failure("InvalidStateError"))?;
    batch
        .events
        .push_back(native_platform::publication::prepare_platform_dispatch(
            scope, state, id, name, event,
        )?);
    batch.completion = Some(Completion {
        document: AdmissionDocument::capture(state),
        pending: pending.clone(),
        entry,
        reason: reason.map(|reason| v8::Global::new(scope, reason)),
        transition,
        focus,
        stage: 0,
        signal,
    });
    if reason.is_some() {
        advance_native_revision(state)?;
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if let Some(event) = pending.event_id {
            navigation.intercepted_history_barriers.remove(&event);
            navigation.queued_finishes.remove(&event);
        }
        navigation.pending = None;
        navigation.finishing_transition = pending.transition;
        enqueue_publication(&mut navigation, batch);
        navigation.publication_retry_at = None;
    }
    poll_publications(scope, state).map(|_| ())
}

fn perform_navigation_scroll(state: &V8BindingState) -> Result<(), V8BindingError> {
    let fragment = state
        .navigation_history_snapshot()
        .and_then(|entry| Url::parse(&entry.url).ok())
        .and_then(|url| url.fragment().map(str::to_owned));
    if let Some(fragment) = fragment {
        let fragment = decode_fragment(&fragment);
        let target = {
            let dom = state.dom.borrow();
            let mut target = dom.get_element_by_id(state.document_root(), &fragment)?;
            if target.is_none() {
                for node in traversal_nodes(&dom, state.document_root())? {
                    if matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name}if name.local.as_ref()=="a"&&name.ns.as_ref()=="http://www.w3.org/1999/xhtml")
                        && dom.attribute_in_null_namespace(node, "name")?.as_deref()
                            == Some(fragment.as_str())
                    {
                        target = Some(node);
                        break;
                    }
                }
            }
            target
        };
        if let Some(target) = target {
            return state.scroll_node_into_view(target);
        }
        if !fragment.is_empty() && !fragment.eq_ignore_ascii_case("top") {
            return Ok(());
        }
    }
    state.set_document_scroll_offset((0.0, 0.0))
}

fn navigate_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let navigation = state.navigation_api.borrow();
    let event = navigation
        .events
        .get(&id)
        .ok_or_else(|| failure("TypeError"))?;
    if v8::Local::new(scope, &event.object) != object {
        return Err(failure("TypeError"));
    }
    match member {
        "navigationType" => v8_string(scope, &event.navigation_type),
        "destination" => Ok(v8::Local::new(scope, &event.destination_object).into()),
        "signal" => Ok(v8::Local::new(scope, &event.signal_object).into()),
        "canIntercept" => Ok(v8::Boolean::new(scope, event.can_intercept).into()),
        "hashChange" => Ok(v8::Boolean::new(scope, event.hash_change).into()),
        "userInitiated" => Ok(v8::Boolean::new(scope, event.user_initiated).into()),
        "hasUAVisualTransition" => Ok(v8::Boolean::new(scope, event.visual_transition).into()),
        "sourceElement" => Ok(v8::Local::new(scope, &event.source_element)),
        "info" => Ok(v8::Local::new(scope, &event.info)),
        "formData" => Ok(v8::Local::new(scope, &event.form_data)),
        "downloadRequest" => Ok(v8::Local::new(scope, &event.download)),
        _ => Err(failure("TypeError")),
    }
}
fn construct_navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    event_type: &str,
    init: v8::Local<'s, v8::Object>,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) -> Result<(), V8BindingError> {
    let can_intercept = event_init_member(scope, Some(init), "canIntercept")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    let destination = event_init_member(scope, Some(init), "destination")
        .ok_or(V8BindingError::JavaScriptException)?
        .try_cast::<v8::Object>()
        .map_err(|_| failure("TypeError"))?;
    let (destination_owner, destination_id) = native_id(scope, destination)?;
    checked_receiver(
        scope,
        &destination_owner,
        destination,
        "NavigationDestination",
    )?;
    let download = event_init_member(scope, Some(init), "downloadRequest")
        .ok_or(V8BindingError::JavaScriptException)?;
    let download: v8::Local<v8::Value> = if download.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        download
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .into()
    };
    let form_data = event_init_member(scope, Some(init), "formData")
        .ok_or(V8BindingError::JavaScriptException)?;
    let form_data: v8::Local<v8::Value> = if form_data.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        let object = form_data
            .try_cast::<v8::Object>()
            .map_err(|_| failure("TypeError"))?;
        let (owner, id) = native_id(scope, object)?;
        if !matches!(
            &*platform_object(&owner, id)?,
            PlatformObjectRecord::FormData(_)
        ) {
            return Err(failure("TypeError"));
        }
        object.into()
    };
    let visual_transition = event_init_member(scope, Some(init), "hasUAVisualTransition")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    let hash_change = event_init_member(scope, Some(init), "hashChange")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    let info =
        event_init_member(scope, Some(init), "info").ok_or(V8BindingError::JavaScriptException)?;
    let kind = event_init_member(scope, Some(init), "navigationType")
        .ok_or(V8BindingError::JavaScriptException)?;
    let kind = if kind.is_undefined() {
        "push".into()
    } else {
        kind.to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope)
    };
    if !matches!(kind.as_str(), "push" | "replace" | "reload" | "traverse") {
        return Err(failure("TypeError"));
    }
    let signal = event_init_member(scope, Some(init), "signal")
        .ok_or(V8BindingError::JavaScriptException)?
        .try_cast::<v8::Object>()
        .map_err(|_| failure("TypeError"))?;
    let (signal_owner, signal_id) = native_id(scope, signal)?;
    if !matches!(
        &*platform_object(&signal_owner, signal_id)?,
        PlatformObjectRecord::AbortSignal(_)
    ) {
        return Err(failure("TypeError"));
    }
    let source = event_init_member(scope, Some(init), "sourceElement")
        .ok_or(V8BindingError::JavaScriptException)?;
    let source: v8::Local<v8::Value> = if source.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        let object = source
            .try_cast::<v8::Object>()
            .map_err(|_| failure("TypeError"))?;
        if object.is_proxy() {
            return Err(failure("TypeError"));
        }
        let owner =
            binding_state_for_receiver(scope, object).ok_or_else(|| failure("TypeError"))?;
        resolve_receiver(scope, &owner, object, "Element")?;
        object.into()
    };
    let user_initiated = event_init_member(scope, Some(init), "userInitiated")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    let event = create_event_object_with_interface(
        scope,
        state,
        "NavigateEvent",
        event_type,
        bubbles,
        cancelable,
    )?;
    let id = event_id(scope, event)?;
    object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
    tag_event_object(scope, state, object);
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.composed = composed;
        record.is_trusted = false;
    }
    state.navigation_api.borrow_mut().events.insert(
        id,
        NavigateRecord {
            object: v8::Global::new(scope, object),
            destination: destination_id,
            destination_object: v8::Global::new(scope, destination),
            navigation_type: kind,
            can_intercept,
            hash_change,
            user_initiated,
            signal_id,
            signal_object: v8::Global::new(scope, signal),
            info: v8::Global::new(scope, info),
            form_data: v8::Global::new(scope, form_data),
            download: v8::Global::new(scope, download),
            visual_transition,
            source_element: v8::Global::new(scope, source),
            intercepted: false,
            handlers: Vec::new(),
            precommit_handlers: Vec::new(),
            focus_reset: "after-transition".into(),
            focus_revision: state.focus_transition_revision.get(),
            scroll: "after-transition".into(),
            scrolled: false,
            committed: false,
            trusted: false,
            admission_complete: false,
            admission_failed: false,
        },
    );
    Ok(())
}
fn decode_fragment(fragment: &str) -> String {
    let input = fragment.as_bytes();
    let mut bytes = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' && index + 2 < input.len() {
            if let (Some(high), Some(low)) = (
                (input[index + 1] as char).to_digit(16),
                (input[index + 2] as char).to_digit(16),
            ) {
                bytes.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        bytes.push(input[index]);
        index += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn prepare_transition(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_id: u64,
    from: &str,
    kind: &str,
) -> Result<(), V8BindingError> {
    let finished = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("navigation transition finished"))?;
    let committed = v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation(
        "navigation transition committed",
    ))?;
    // Internal lifecycle promises are exposed to observers but do not report
    // an unhandled rejection merely because an observer never requested them.
    finished.get_promise(scope).mark_as_handled();
    committed.get_promise(scope).mark_as_handled();
    let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
    let object = new_platform_object(scope, state, "NavigationTransition", id)?;
    let mut navigation = state.navigation_api.borrow_mut();
    let to = navigation.events[&event_id].destination;
    navigation.transitions.insert(
        id,
        Transition {
            object: v8::Global::new(scope, object),
            from: from.into(),
            to,
            navigation_type: kind.into(),
            committed: v8::Global::new(scope, committed),
            finished: v8::Global::new(scope, finished),
        },
    );
    navigation
        .pending
        .as_mut()
        .ok_or_else(|| failure("AbortError"))?
        .transition = Some(id);
    Ok(())
}
fn queue_navigation_finish(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_id: u64,
) -> Result<(), V8BindingError> {
    let object = {
        let navigation = state.navigation_api.borrow();
        v8::Local::new(
            scope,
            &navigation
                .events
                .get(&event_id)
                .ok_or_else(|| failure("InvalidStateError"))?
                .object,
        )
    };
    if state
        .navigation_api
        .borrow()
        .queued_finishes
        .contains(&event_id)
    {
        return Ok(());
    }
    let callback = v8::Function::builder(finish_callback)
        .data(object.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("navigation finish microtask"))?;
    reserve_publication(state)?;
    // Finish becomes runnable at its microtask checkpoint. Keeping an unready
    // event in the publication queue would block synchronous fragment events.
    state
        .navigation_api
        .borrow_mut()
        .queued_finishes
        .insert(event_id);
    scope.enqueue_microtask(callback);
    Ok(())
}
fn finish_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(object) = args.data().try_cast::<v8::Object>() else {
        return;
    };
    let Some(state) = binding_state_for_receiver(scope, object) else {
        return;
    };
    let Ok(id) = event_id(scope, object) else {
        return;
    };
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if !navigation.queued_finishes.remove(&id) {
            return;
        }
        if !navigation
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(id))
        {
            return;
        }
        navigation.continuation_requests.insert((id, false));
    }
    if let Err(error) = poll_publications(scope, &state) {
        throw_binding_error(scope, &error);
    }
}
pub(super) fn set_attribute(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    checked_receiver(scope, state, object, interface)?;
    set_platform_attribute(scope, state, object, interface, member, value)
}

pub fn stage_swap_destination(
    scope: &mut v8::PinScope,
    snapshot: NavigationHistorySnapshot,
) -> Result<(), V8BindingError> {
    let state = current_state(scope)?;
    ensure(scope, &state)?;
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.staged_swap = Some(snapshot);
    navigation.staged_swap_activation = None;
    Ok(())
}

fn abort_ongoing(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    let document = AdmissionDocument::capture(state);
    for _ in 0..256 {
        if !document.current(state) {
            return Ok(false);
        }
        if state.navigation_api.borrow().pending.is_none() {
            advance_native_revision(state)?;
            return Ok(true);
        }
        abort_navigation(scope, "Superseded by another navigation")?;
    }
    Err(failure("QuotaExceededError"))
}

fn begin_commit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_id: u64,
    plan: CommitPlan,
) -> Result<(), V8BindingError> {
    reserve_publication(state)?;
    let handlers = state.navigation_api.borrow().events[&event_id]
        .precommit_handlers
        .clone();
    let count = handlers.len();
    let mut batch = publication(state);
    batch.script = Some(ScriptPublication::Handlers(HandlerPublication {
        event: event_id,
        precommit: true,
        handlers: handlers.into(),
        controller: None,
        active: None,
    }));
    {
        let mut navigation = state.navigation_api.borrow_mut();
        let pending = navigation
            .pending
            .as_mut()
            .filter(|pending| pending.event_id == Some(event_id))
            .ok_or_else(|| failure("AbortError"))?;
        pending.plan = Some(plan);
        pending.remaining = count;
        navigation
            .events
            .get_mut(&event_id)
            .unwrap()
            .precommit_handlers
            .clear();
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, state);
    Ok(())
}
fn apply_commit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_id: u64,
) -> Result<(), V8BindingError> {
    reserve_publication(state)?;
    let plan = {
        let navigation = state.navigation_api.borrow();
        let Some(pending) = navigation
            .pending
            .as_ref()
            .filter(|pending| pending.event_id == Some(event_id))
        else {
            return Ok(());
        };
        pending
            .plan
            .clone()
            .ok_or_else(|| failure("InvalidStateError"))?
    };
    let mut batch = publication(state);
    batch.script = Some(ScriptPublication::Commit {
        event: event_id,
        plan,
        stage: 0,
        receipt: None,
    });
    {
        let mut navigation = state.navigation_api.borrow_mut();
        navigation.pending.as_mut().unwrap().plan = None;
        enqueue_publication(&mut navigation, batch);
    }
    let _ = poll_publications(scope, state);
    Ok(())
}

fn precommit_fulfilled(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(state) = current_state(scope) else {
        return;
    };
    let id = args.data().integer_value(scope).unwrap_or(-1) as u64;
    let ready = {
        let mut navigation = state.navigation_api.borrow_mut();
        let Some(pending) = navigation
            .pending
            .as_mut()
            .filter(|pending| pending.event_id == Some(id) && pending.plan.is_some())
        else {
            return;
        };
        pending.remaining = pending.remaining.saturating_sub(1);
        pending.remaining == 0
    };
    if ready {
        {
            let mut navigation = state.navigation_api.borrow_mut();
            navigation.continuation_requests.insert((id, true));
            navigation.publication_retry_at = None;
        }
        if let Err(error) = poll_publications(scope, &state) {
            throw_binding_error(scope, &error);
        }
    }
}
fn precommit_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    let event_id = state
        .navigation_api
        .borrow()
        .controllers
        .get(&id)
        .ok_or_else(|| failure("TypeError"))?
        .event;
    if state.execution_retired()
        || !state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| {
                pending.event_id == Some(event_id)
                    && pending.plan.is_some()
                    && !pending.committed_entry
            })
    {
        return Err(failure("InvalidStateError"));
    }
    if member == "addHandler" {
        let handler = args
            .get(0)
            .try_cast::<v8::Function>()
            .map_err(|_| failure("TypeError"))?;
        let mut navigation = state.navigation_api.borrow_mut();
        let event = navigation.events.get_mut(&event_id).unwrap();
        if event.handlers.len() >= 256 {
            return Err(failure("QuotaExceededError"));
        }
        event.handlers.push(v8::Global::new(scope, handler));
        return Ok(());
    }
    if member != "redirect" {
        return Err(failure("TypeError"));
    }
    let (kind, current) = {
        let navigation = state.navigation_api.borrow();
        (
            navigation.pending.as_ref().unwrap().navigation_type.clone(),
            state
                .navigation_history_snapshot()
                .ok_or_else(|| failure("InvalidStateError"))?,
        )
    };
    if !matches!(kind.as_str(), "push" | "replace") {
        return Err(failure("InvalidStateError"));
    }
    let input = required_string(scope, args, 0)?;
    let base = document_base_url(scope, state, true)?;
    let target = Url::parse(&base)
        .and_then(|base| base.join(&input))
        .map_err(|_| failure("SyntaxError"))?;
    if Url::parse(&current.url)
        .map_err(|_| failure("InvalidStateError"))?
        .origin()
        != target.origin()
    {
        return Err(failure("SecurityError"));
    }
    let value = args.get(1);
    let options = if value.is_null_or_undefined() {
        None
    } else {
        Some(
            value
                .try_cast::<v8::Object>()
                .map_err(|_| failure("TypeError"))?,
        )
    };
    let history =
        event_init_member(scope, options, "history").ok_or(V8BindingError::JavaScriptException)?;
    let history = if history.is_undefined() {
        "auto".into()
    } else {
        history
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope)
    };
    let kind = match history.as_str() {
        "auto" => kind,
        "push" | "replace" => history,
        _ => return Err(failure("TypeError")),
    };
    let info =
        event_init_member(scope, options, "info").ok_or(V8BindingError::JavaScriptException)?;
    let value =
        event_init_member(scope, options, "state").ok_or(V8BindingError::JavaScriptException)?;
    let wire = if value.is_undefined() {
        None
    } else {
        Some(
            structured_clone::serialize_storage(scope, value)
                .ok_or(V8BindingError::JavaScriptException)?,
        )
    };
    let mut navigation = state.navigation_api.borrow_mut();
    let pending = navigation
        .pending
        .as_mut()
        .filter(|pending| pending.event_id == Some(event_id) && pending.plan.is_some())
        .ok_or_else(|| failure("InvalidStateError"))?;
    let mut destination = pending.destination.clone();
    destination.url = target.into();
    destination.state = wire;
    destination.key = if kind == "replace" {
        current.key.clone()
    } else if pending.navigation_type == "replace" {
        token()?
    } else {
        destination.key
    };
    pending.destination = destination.clone();
    pending.navigation_type = kind.clone();
    let plan = pending.plan.as_mut().unwrap();
    let from_index = plan
        .base_entries
        .iter()
        .position(|entry| entry.id == plan.from)
        .ok_or_else(|| failure("InvalidStateError"))?;
    plan.entries = plan.base_entries.clone();
    if kind == "replace" {
        plan.entries[from_index] = destination.clone();
        plan.current = from_index;
    } else {
        plan.entries.truncate(from_index + 1);
        plan.entries.push(destination.clone());
        plan.current = from_index + 1;
    }
    plan.kind = kind.clone();
    let transition = pending.transition;
    let destination_id = navigation.events[&event_id].destination;
    let record = navigation.destinations.get_mut(&destination_id).unwrap();
    record.snapshot.url = destination.url;
    record.snapshot.state = destination.state;
    let event = navigation.events.get_mut(&event_id).unwrap();
    event.navigation_type = kind.clone();
    event.info = v8::Global::new(scope, info);
    if let Some(transition) = transition {
        navigation
            .transitions
            .get_mut(&transition)
            .unwrap()
            .navigation_type = kind;
    }
    Ok(())
}

pub(super) fn history_state_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let snapshot = state
        .navigation_history_snapshot()
        .ok_or_else(|| failure("InvalidStateError"))?;
    if let Some(value) = state
        .navigation_api
        .borrow()
        .history_values
        .get(&snapshot.id)
    {
        return Ok(v8::Local::new(scope, value));
    }
    let value = match snapshot.history_state {
        Some(wire) => structured_clone::deserialize_storage(scope, wire)
            .ok_or(V8BindingError::JavaScriptException)?,
        None => v8::null(scope).into(),
    };
    state
        .navigation_api
        .borrow_mut()
        .history_values
        .insert(snapshot.id, v8::Global::new(scope, value));
    Ok(value)
}

/// Synchronous Location API admission. A false result means the native default
/// was cancelled or handled in this document; no URL/fetch request may follow.
pub(super) fn begin_location_navigation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    url: &Url,
    replace: bool,
    reload: bool,
) -> Result<bool, V8BindingError> {
    begin_location_navigation_with_source(scope, state, url, replace, reload, None)
}
fn native_source_can_fire_cross_document(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    source: &NativeNavigationSource,
) -> Result<bool, V8BindingError> {
    if let Some(key) = &source.source_domain_key {
        return Ok(document_domain_access_key(state, state.document_root())?.as_ref() == Some(key));
    }
    if let Some(key) = &source.source_origin_key {
        return Ok(document_domain_access_key(state, state.document_root())?.as_ref() == Some(key));
    }
    Ok(match &source.source_window {
        Some(window) => {
            let window = v8::Local::new(scope, window);
            crate::v8_context_bridge::can_access_window(scope, window)
        }
        None => true,
    })
}

/// Read a native Document identity without consulting authored properties.
/// Opaque identities retain the authenticated strict key rather than reparsing
/// their serialized `null`; a relaxed tuple uses the separate access namespace.
pub fn document_domain_access_key(
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<Option<String>, V8BindingError> {
    let provenance = state.document_provenance(document)?;
    let domain = state
        .dom()
        .borrow()
        .arena()
        .get(document)?
        .document_metadata
        .as_ref()
        .and_then(|metadata| metadata.relaxed_domain.get());
    if let Some(domain) = domain {
        let origin = provenance
            .as_ref()
            .map(|source| source.origin.clone())
            .unwrap_or_else(|| state.current_location_url());
        let mut identity =
            crate::origin::DomainAccessIdentity::new(crate::origin::Origin::from_url(&origin));
        identity
            .set_domain(&domain)
            .map_err(|_| failure("InvalidStateError"))?;
        return Ok(Some(identity.security_token_key()));
    }
    Ok(provenance.map(|source| source.origin_key.clone()))
}
fn begin_location_navigation_with_source(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    url: &Url,
    replace: bool,
    reload: bool,
    source: Option<NativeNavigationSource>,
) -> Result<bool, V8BindingError> {
    let document = AdmissionDocument::capture(state);
    ensure(scope, state)?;
    if !abort_ongoing(scope, state)? {
        return Ok(false);
    }
    if !document.current(state) {
        return Ok(false);
    }
    let is_download = source
        .as_ref()
        .is_some_and(|source| source.download.is_some());
    if entries_disabled(state) {
        // Navigation API exposure and HTML fragment history are separate.
        // Opaque documents still commit same-document Location changes inline.
        if reload || is_download {
            return Ok(true);
        }
        let current = state
            .navigation_history_snapshot()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let mut previous = Url::parse(&current.url).map_err(|_| failure("InvalidStateError"))?;
        let mut target = url.clone();
        previous.set_fragment(None);
        target.set_fragment(None);
        if url.fragment().is_none() || previous != target {
            return Ok(true);
        }
        record_document_scroll(state, state.document_scroll_offset());
        let mut entries = state.navigation_history_entries();
        let current_index = entries
            .iter()
            .position(|entry| entry.id == current.id)
            .ok_or_else(|| failure("InvalidStateError"))?;
        let destination = NavigationHistorySnapshot {
            scroll: entries[current_index].scroll.clone(),
            key: if replace {
                current.key.clone()
            } else {
                token()?
            },
            id: token()?,
            url: url.to_string(),
            document_id: current.document_id.clone(),
            state: None,
            history_state: None,
        };
        let selected = if replace {
            entries[current_index] = destination;
            current_index
        } else {
            entries.truncate(current_index + 1);
            entries.push(destination);
            entries.len() - 1
        };
        let kind = if replace { "replace" } else { "push" };
        if let Some(observer) = source
            .as_ref()
            .and_then(|source| source.execution_observer.as_ref())
        {
            observer.admitted();
        }
        state.commit_location(url.as_str())?;
        sync_history(scope, entries, selected, kind)?;
        publish_history_update(state, kind);
        perform_navigation_scroll(state)?;
        publish_fragment_events(
            scope,
            state,
            &current.url,
            url.as_str(),
            source
                .as_ref()
                .and_then(|source| source.execution_observer.clone()),
        )?;
        return Ok(false);
    }
    let same_origin_source = source
        .as_ref()
        .map(|source| native_source_can_fire_cross_document(scope, state, source))
        .transpose()?
        .unwrap_or(true);
    state.navigation_api.borrow_mut().native_source = source;
    record_document_scroll(state, state.document_scroll_offset());
    let current = state
        .navigation_history_snapshot()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let entries = state.navigation_history_entries();
    let index = entries
        .iter()
        .position(|entry| entry.id == current.id)
        .ok_or_else(|| failure("InvalidStateError"))?;
    let kind = if reload {
        "reload"
    } else if replace {
        "replace"
    } else {
        "push"
    };
    let mut old = Url::parse(&current.url).map_err(|_| failure("InvalidStateError"))?;
    let mut target = url.clone();
    old.set_fragment(None);
    target.set_fragment(None);
    let same_document = !reload && !is_download && url.fragment().is_some() && old == target;
    // HTML performs the fragment path before its source-origin/fetch-scheme
    // gate. A cross-document navigation from a different origin must not give
    // the destination a cancellable navigate event or expose source data.
    if !same_document
        && !is_download
        && (!same_origin_source
            || !matches!(
                url.scheme(),
                "about" | "blob" | "data" | "file" | "http" | "https"
            ))
    {
        return Ok(true);
    }
    let destination = NavigationHistorySnapshot {
        scroll: if same_document || reload {
            current.scroll.clone()
        } else {
            Default::default()
        },
        key: if replace || reload {
            current.key.clone()
        } else {
            token()?
        },
        id: token()?,
        url: url.to_string(),
        document_id: if same_document {
            current.document_id.clone()
        } else {
            token()?
        },
        state: if reload { current.state.clone() } else { None },
        history_state: if reload {
            current.history_state.clone()
        } else {
            None
        },
    };
    let committed = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("Location navigation committed"))?;
    let finished = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("Location navigation finished"))?;
    committed.get_promise(scope).mark_as_handled();
    finished.get_promise(scope).mark_as_handled();
    let execution_observer = state
        .navigation_api
        .borrow()
        .native_source
        .as_ref()
        .and_then(|source| source.execution_observer.clone());
    state.navigation_api.borrow_mut().pending = Some(PendingNavigation {
        execution_observer,
        committed: v8::Global::new(scope, committed),
        finished: v8::Global::new(scope, finished),
        destination: destination.clone(),
        navigation_type: kind.into(),
        event_id: None,
        transition: None,
        committed_entry: false,
        remaining: 0,
        plan: None,
    });
    let dispatched = dispatch_navigate(scope, state, &destination, kind, index, &current, None);
    if !document.current(state)
        && matches!(&dispatched,Err(V8BindingError::Unsupported{member,..}) if member=="AbortError")
    {
        return Ok(false);
    }
    let event_id = dispatched?;
    if !document.current(state) {
        return Ok(false);
    }
    if !state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .is_some_and(|pending| pending.event_id == Some(event_id))
    {
        return Ok(false);
    }
    if state
        .events
        .borrow()
        .get(&event_id)
        .is_some_and(|event| event.default_prevented)
    {
        abort_navigation(scope, "Location navigation cancelled")?;
        return Ok(false);
    }
    let intercepted = state.navigation_api.borrow().events[&event_id].intercepted;
    if same_document || intercepted {
        if intercepted {
            prepare_transition(scope, state, event_id, &current.id, kind)?;
        }
        let mut destination = destination;
        destination.document_id = current.document_id.clone();
        state
            .navigation_api
            .borrow_mut()
            .pending
            .as_mut()
            .unwrap()
            .destination = destination.clone();
        let base_entries = entries.clone();
        let mut entries = entries;
        let selected = if replace || reload {
            entries[index] = destination;
            index
        } else {
            entries.truncate(index + 1);
            entries.push(destination);
            entries.len() - 1
        };
        if let Some(observer) = state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .and_then(|pending| pending.execution_observer.clone())
        {
            observer.admitted();
        }
        begin_commit(
            scope,
            state,
            event_id,
            CommitPlan {
                base_entries,
                entries,
                current: selected,
                from: current.id,
                kind: kind.into(),
                intercepted,
            },
        )?;
        if same_document && !intercepted {
            perform_navigation_scroll(state)?;
            publish_fragment_events(scope, state, &current.url, url.as_str(), None)?;
        }
        Ok(false)
    } else {
        Ok(true)
    }
}

/// Capture native session history without consulting author-visible properties.
pub fn snapshot_history(
    scope: &mut v8::PinScope,
) -> Result<NavigationHistoryUpdate, V8BindingError> {
    snapshot_history_limited(scope, crate::form_state::MAX_FORM_SNAPSHOT_BYTES)
}
pub fn snapshot_history_limited(
    scope: &mut v8::PinScope,
    max_bytes: usize,
) -> Result<NavigationHistoryUpdate, V8BindingError> {
    let state = current_state(scope)?;
    ensure(scope, &state)?;
    let navigation = state.navigation_api.borrow();
    let current = navigation
        .current
        .as_ref()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let index = navigation
        .order
        .iter()
        .position(|id| id == current)
        .ok_or_else(|| failure("InvalidStateError"))?;
    if navigation.order.len() > 4096 {
        return Err(failure("QuotaExceededError: navigation snapshot entries"));
    }
    let mut bytes = 0usize;
    for id in &navigation.order {
        let entry = &navigation
            .entries
            .get(id)
            .ok_or_else(|| failure("InvalidStateError"))?
            .snapshot;
        let size = std::mem::size_of::<NavigationHistorySnapshot>()
            .saturating_add(entry.key.len())
            .saturating_add(entry.id.len())
            .saturating_add(entry.document_id.len())
            .saturating_add(entry.url.len())
            .saturating_add(
                entry
                    .state
                    .as_ref()
                    .map_or(0, |value| value.retained_bytes()),
            )
            .saturating_add(
                entry
                    .history_state
                    .as_ref()
                    .map_or(0, |value| value.retained_bytes()),
            );
        bytes = bytes
            .checked_add(size)
            .filter(|bytes| *bytes <= max_bytes)
            .ok_or_else(|| failure("QuotaExceededError: navigation snapshot bytes"))?;
    }
    let entries = navigation
        .order
        .iter()
        .map(|id| navigation.entries[id].snapshot.clone())
        .collect();
    Ok(NavigationHistoryUpdate {
        entries,
        current: index,
        navigation_type: "replace".into(),
        truncates_forward: false,
    })
}

fn entries_disabled(state: &V8BindingState) -> bool {
    state.navigation_api.borrow().initial_document
        || state.security_origin.borrow().as_deref() == Some("null")
}

/// Run one already queued frame history task after the JavaScript task ended.
struct JointTarget {
    owner: Rc<V8BindingState>,
    document: AdmissionDocument,
    destination: NavigationHistorySnapshot,
    admitted: bool,
    operation: Option<NativeNavigationToken>,
}
struct JointTraversal {
    targets: std::collections::VecDeque<JointTarget>,
    receipt: crate::joint_history::NativeJointTraversalReceipt,
}
impl Drop for JointTraversal {
    fn drop(&mut self) {
        if !self.targets.is_empty() {
            self.receipt.retire();
        }
    }
}
pub(super) fn joint_document_interactive(state: &V8BindingState) {
    if let Some(entry) = state.navigation_history_snapshot() {
        let group = state.joint_history.borrow().clone();
        group
            .borrow_mut()
            .record_interactive(&state.joint_history_route(), &entry.id);
    }
}
pub(super) fn joint_document_loaded(state: &V8BindingState) {
    if let Some(entry) = state.navigation_history_snapshot() {
        let group = state.joint_history.borrow().clone();
        let route = state.joint_history_route();
        group.borrow_mut().record_publication(&route, &entry.id);
        group.borrow_mut().record_load(&route, &entry.id);
    }
}
/// A push in any navigable truncates the shared forward history. Publish the
/// affected local entry disposal using the same retained native event queue.
pub(super) fn poll_joint_projection(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    state
        .joint_history
        .borrow()
        .borrow_mut()
        .reconcile_executions();
    if state.execution_retired()
        || state.execution_suspended()
        || !state.navigation_api.borrow().publications.is_empty()
    {
        return Ok(0);
    }
    let Some(current) = state.navigation_history_snapshot() else {
        return Ok(0);
    };
    let route = state.joint_history_route();
    let group = state.joint_history.borrow().clone();
    let entries = {
        let timeline = group.borrow();
        if timeline
            .selected(&route)
            .is_none_or(|selected| selected.id != current.id)
        {
            return Ok(0);
        }
        let removed = state.history.borrow().as_ref().is_some_and(|history| {
            history
                .entries
                .iter()
                .any(|entry| !timeline.knows(&route, &entry.navigation_id))
        });
        if !removed {
            return Ok(0);
        }
        state
            .navigation_history_entries()
            .into_iter()
            .filter(|entry| timeline.knows(&route, &entry.id))
            .collect::<Vec<_>>()
    };
    let Some(current) = entries.iter().position(|entry| entry.id == current.id) else {
        return Ok(0);
    };
    let receipt = start_native_history_publication(
        scope,
        NavigationHistoryUpdate {
            entries,
            current,
            navigation_type: "push".into(),
            truncates_forward: true,
        },
    )?;
    state
        .navigation_api
        .borrow_mut()
        .native_receipts
        .remove(&receipt);
    publish_history_update(state, "push");
    Ok(1)
}
pub(super) fn poll_joint_tasks(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    let Some(mut task) = state.navigation_api.borrow_mut().joint_tasks.pop_front() else {
        return Ok(0);
    };
    let mut count = 0;
    let result = (|| {
        while let Some(target) = task.targets.front_mut() {
            if task.receipt.poll().is_some_and(|result| result.is_err()) {
                return Ok(true);
            }
            if !target.document.current(&target.owner) {
                state.joint_history.borrow().borrow_mut().fail_execution(
                    task.receipt.token(),
                    crate::joint_history::JointTraversalFailure::Retired,
                );
                task.targets.pop_front();
                continue;
            }
            if target.owner.execution_suspended() {
                return Ok(false);
            }
            let context = target
                .owner
                .global_object
                .to_local(scope)
                .and_then(|window| window.get_creation_context(scope))
                .ok_or_else(|| failure("InvalidStateError: joint traversal context"))?;
            let scope = &mut v8::ContextScope::new(scope, context);
            if !target.admitted {
                // Native navigation admission retains or terminates its own
                // callback suffix. An error cannot cause a second navigate event.
                target.admitted = true;
                match admit_native_history_traversal(scope, target.destination.clone()) {
                    Ok(NativeNavigationAdmission::Handled) => {
                        if target
                            .owner
                            .navigation_history_snapshot()
                            .is_none_or(|entry| entry.id != target.destination.id)
                            && target
                                .owner
                                .pending_navigation_destination()
                                .is_none_or(|entry| entry.id != target.destination.id)
                        {
                            state.joint_history.borrow().borrow_mut().fail_execution(
                                task.receipt.token(),
                                crate::joint_history::JointTraversalFailure::Canceled,
                            );
                        }
                        task.targets.pop_front();
                        count += 1;
                        continue;
                    }
                    Ok(NativeNavigationAdmission::Proceed {
                        destination: Some(destination),
                        operation,
                    }) => {
                        target.destination = destination;
                        target.operation = operation;
                    }
                    Ok(_) => {
                        task.targets.pop_front();
                        count += 1;
                        continue;
                    }
                    Err(error) => {
                        state.joint_history.borrow().borrow_mut().fail_execution(
                            task.receipt.token(),
                            crate::joint_history::JointTraversalFailure::Failed(error.to_string()),
                        );
                        task.targets.pop_front();
                        return Err(error);
                    }
                }
            }
            if target
                .operation
                .as_ref()
                .is_none_or(|operation| !native_navigation_matches(&target.owner, operation))
            {
                state.joint_history.borrow().borrow_mut().fail_execution(
                    task.receipt.token(),
                    crate::joint_history::JointTraversalFailure::Canceled,
                );
                return Ok(true);
            }
            queue_cross_document_traversal(scope, target.destination.clone())?;
            task.targets.pop_front();
            count += 1;
            if scope.is_execution_terminating() {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    match result {
        Ok(true) => Ok(count),
        other => {
            state
                .navigation_api
                .borrow_mut()
                .joint_tasks
                .push_front(task);
            other.map(|_| count)
        }
    }
}
pub fn run_history_task(
    scope: &mut v8::PinScope,
    mut task: HistoryTraversalTask,
    _destination: Option<NavigationHistorySnapshot>,
) -> Result<(), V8BindingError> {
    let state = current_state(scope)?;
    if !task.is_current() {
        task.acknowledge();
        return Ok(());
    }
    if !task.is_current_for(&state) {
        return Err(failure("InvalidStateError"));
    }
    if !task.is_runnable() {
        return Ok(());
    }
    if task.delta() == 0 {
        let url = state.location_url.borrow().clone();
        task.acknowledge();
        return location::navigate(scope, &state, url, true, true);
    }
    let result = run_joint_history_delta(scope, task.delta());
    if result.is_ok() {
        task.acknowledge();
    }
    result.map(|_| ())
}
/// Browser UI and queued script traversal share one frozen native step plan.
/// The receipt completes only when every selected entry is natively committed.
pub fn run_joint_history_delta(
    scope: &mut v8::PinScope,
    delta: i64,
) -> Result<Option<crate::joint_history::NativeJointTraversalReceipt>, V8BindingError> {
    let state = current_state(scope)?;
    if state.execution_retired() || state.execution_suspended() {
        return Err(failure("InvalidStateError: inactive joint traversal"));
    }
    let Some(plan) = state.plan_joint_history(delta)? else {
        return Ok(None);
    };
    let group = state.joint_history.borrow().clone();
    let owners = scope
        .get_slot::<BindingStateRegistry>()
        .ok_or_else(|| failure("InvalidStateError"))?
        .instances
        .borrow()
        .values()
        .filter_map(RcWeak::upgrade)
        .filter(|owner| {
            !owner.execution_retired() && Rc::ptr_eq(&group, &owner.joint_history.borrow())
        })
        .collect::<Vec<_>>();
    let mut targets = std::collections::VecDeque::new();
    targets
        .try_reserve(plan.destinations.len())
        .map_err(|_| failure("QuotaExceededError: joint traversal"))?;
    for (route, destination) in &plan.destinations {
        let mut matches = owners
            .iter()
            .filter(|owner| owner.joint_history_route() == *route && !owner.execution_suspended());
        if let Some(owner) = matches.next() {
            if matches.next().is_some() {
                return Err(failure("InvalidStateError: duplicate joint route"));
            }
            targets.push_back(JointTarget {
                owner: owner.clone(),
                document: AdmissionDocument::capture(owner),
                destination: destination.clone(),
                admitted: false,
                operation: None,
            });
        }
    }
    {
        let mut navigation = state.navigation_api.borrow_mut();
        if navigation.joint_tasks.len() >= 128 {
            return Err(failure("QuotaExceededError: joint traversals"));
        }
        navigation
            .joint_tasks
            .try_reserve(1)
            .map_err(|_| failure("QuotaExceededError: joint traversals"))?;
    }
    let receipt = group
        .borrow_mut()
        .begin_execution(&plan)
        .map_err(|_| failure("QuotaExceededError: joint receipt"))?;
    if !state.commit_joint_history(&plan)? {
        receipt.cancel();
        return Err(failure("InvalidStateError: joint traversal superseded"));
    }
    state
        .navigation_api
        .borrow_mut()
        .joint_tasks
        .push_back(JointTraversal {
            targets,
            receipt: receipt.clone(),
        });
    let _ = poll_joint_tasks(scope, &state);
    Ok(Some(receipt))
}

/// Convert an accepted native history traversal into an authenticated loader request.
pub fn queue_cross_document_traversal(
    scope: &mut v8::PinScope,
    destination: NavigationHistorySnapshot,
) -> Result<(), V8BindingError> {
    let state = current_state(scope)?;
    ensure(scope, &state)?;
    if state.execution_retired() {
        return Ok(());
    }
    let script_authority = if let Some(backend) = state.location_navigation_backend.borrow().clone()
    {
        let target = state.native_worker_creator()?;
        let authority = backend
            .authorize(
                target.clone(),
                target.clone(),
                destination.url.clone(),
                state.request_initiator_url()?,
                state.document_referrer_policy(),
            )
            .map_err(|_| failure("SecurityError"))?
            .ok_or_else(|| failure("SecurityError"))?;
        Some(crate::navigation::ScriptNavigationAdmission { authority, target })
    } else {
        None
    };
    state
        .pending_location_navigation
        .borrow_mut()
        .replace(LocationNavigation {
            execution_observer: None,
            navigation_operation: capture_native_history_operation(&state, &destination.id),
            download: None,
            url: destination.url.clone(),
            navigation_destination: Some(destination),
            script_authority,
            form_authority: None,
            replace: false,
            reload: false,
            post: None,
            referrer_policy: None,
        });
    Ok(())
}

/// History traversal has the same native navigate dispatch as Navigation.back.
pub(super) fn begin_history_traversal(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: &str,
) -> Result<bool, V8BindingError> {
    let document = AdmissionDocument::capture(state);
    ensure(scope, state)?;
    if !abort_ongoing(scope, state)? {
        return Ok(false);
    }
    let entries = state.navigation_history_entries();
    let Some(index) = entries.iter().position(|entry| entry.id == id) else {
        return Ok(false);
    };
    let destination = entries[index].clone();
    let current = state
        .navigation_history_snapshot()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let committed =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("history committed"))?;
    let finished =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("history finished"))?;
    committed.get_promise(scope).mark_as_handled();
    finished.get_promise(scope).mark_as_handled();
    state.navigation_api.borrow_mut().pending = Some(PendingNavigation {
        execution_observer: None,
        committed: v8::Global::new(scope, committed),
        finished: v8::Global::new(scope, finished),
        destination: destination.clone(),
        navigation_type: "traverse".into(),
        event_id: None,
        transition: None,
        committed_entry: false,
        remaining: 0,
        plan: None,
    });
    let event_id = dispatch_navigate(
        scope,
        state,
        &destination,
        "traverse",
        index,
        &current,
        None,
    )?;
    if !document.current(state)
        || !state
            .navigation_api
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|pending| pending.event_id == Some(event_id))
    {
        return Ok(false);
    }
    if state
        .events
        .borrow()
        .get(&event_id)
        .is_some_and(|event| event.default_prevented)
    {
        abort_navigation(scope, "History traversal cancelled")?;
        return Ok(false);
    }
    if state.navigation_api.borrow().events[&event_id].intercepted {
        prepare_transition(scope, state, event_id, &current.id, "traverse")?;
        begin_commit(
            scope,
            state,
            event_id,
            CommitPlan {
                base_entries: entries.clone(),
                entries,
                current: index,
                from: current.id,
                kind: "traverse".into(),
                intercepted: true,
            },
        )?;
        return Ok(false);
    }
    Ok(true)
}
pub(super) fn finish_history_traversal(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let event = state
        .navigation_api
        .borrow()
        .pending
        .as_ref()
        .and_then(|pending| pending.event_id);
    if let Some(event) = event {
        queue_navigation_finish(scope, state, event)?;
    }
    Ok(())
}

/// Admission for native defaults and browser/automation loads, before resource
/// fetch. Already-dispatched Location/API requests use capture_native_operation.
pub fn admit_native_navigation(
    scope: &mut v8::PinScope,
    url: &str,
    replace: bool,
    reload: bool,
    source: NativeNavigationSource,
) -> Result<NativeNavigationAdmission, V8BindingError> {
    let state = current_state(scope)?;
    if state.execution_retired() {
        if let Some(observer) = source.execution_observer {
            observer.complete_document_operation(
                crate::navigation::NavigationExecutionOutcome::Canceled,
            );
        }
        return Ok(NativeNavigationAdmission::Handled);
    }
    let url = Url::parse(url).map_err(|_| failure("SyntaxError"))?;
    let document = AdmissionDocument::capture(&state);
    let execution_observer = source.execution_observer.clone();
    let proceed =
        begin_location_navigation_with_source(scope, &state, &url, replace, reload, Some(source));
    if document.current(&state) {
        state.navigation_api.borrow_mut().native_source = None;
    }
    if !proceed? || !document.current(&state) {
        return Ok(NativeNavigationAdmission::Handled);
    }
    let destination = state.pending_navigation_destination();
    let operation = if let Some(entry) = destination.as_ref() {
        capture_native_operation(&state, &entry.id)
    } else {
        capture_native_revision(&state)
    };
    if operation.is_none() {
        return Ok(NativeNavigationAdmission::Handled);
    }
    if let Some(observer) = execution_observer {
        observer.admitted();
    }
    // The loader, rather than the abandoned source promises, owns terminal
    // execution after admission. Keeping a clone in pending would suppress
    // failure publication when a later resource preparation is discarded.
    if let Some(pending) = state.navigation_api.borrow_mut().pending.as_mut() {
        if let Some(observer) = pending.execution_observer.take() {
            observer.loader_owned();
        }
    }
    Ok(NativeNavigationAdmission::Proceed {
        operation,
        destination,
    })
}
fn advance_native_revision(state: &V8BindingState) -> Result<(), V8BindingError> {
    let mut navigation = state.navigation_api.borrow_mut();
    navigation.revision = navigation
        .revision
        .checked_add(1)
        .ok_or_else(|| failure("QuotaExceededError"))?;
    Ok(())
}
pub fn capture_native_revision(state: &V8BindingState) -> Option<NativeNavigationToken> {
    if state.execution_retired() {
        return None;
    }
    Some(NativeNavigationToken {
        binding: state.instance_id,
        generation: state.document_generation(),
        epoch: state.document_authority_epoch(),
        revision: state.navigation_api.borrow().revision,
        event: None,
        destination: None,
    })
}
pub fn native_navigation_matches(state: &V8BindingState, token: &NativeNavigationToken) -> bool {
    if state.execution_retired()
        || token.binding != state.instance_id
        || token.generation != state.document_generation()
        || token.epoch != state.document_authority_epoch()
    {
        return false;
    }
    let navigation = state.navigation_api.borrow();
    if token.revision != navigation.revision {
        return false;
    }
    match token.event {
        None => true,
        Some(event) => {
            admission_allows_native(&navigation, event)
                && !state
                    .events
                    .borrow()
                    .get(&event)
                    .is_some_and(|event| event.default_prevented)
                && navigation.pending.as_ref().is_some_and(|pending| {
                    pending.event_id == Some(event)
                        && Some(&pending.destination.id) == token.destination.as_ref()
                })
        }
    }
}
// Hidden session entries remain private, but their admitted native operation
// still needs an exact destination when a loader fails after actor restoration.
fn capture_native_history_operation(
    state: &V8BindingState,
    destination_id: &str,
) -> Option<NativeNavigationToken> {
    let mut token = capture_current_native_operation(state)?;
    if token.event.is_some() && token.destination.as_deref() != Some(destination_id) {
        return None;
    }
    token.destination = Some(destination_id.to_owned());
    Some(token)
}
pub fn capture_native_operation(
    state: &V8BindingState,
    destination_id: &str,
) -> Option<NativeNavigationToken> {
    let mut token = capture_native_revision(state)?;
    let navigation = state.navigation_api.borrow();
    let pending = navigation.pending.as_ref()?;
    if pending.destination.id != destination_id
        || !admission_allows_native(&navigation, pending.event_id?)
        || state
            .events
            .borrow()
            .get(&pending.event_id?)
            .is_some_and(|event| event.default_prevented)
    {
        return None;
    }
    token.event = Some(pending.event_id?);
    token.destination = Some(destination_id.into());
    Some(token)
}
/// Explicit native cancellation only. Successful cross-document navigation,
/// reload, 204/205 and downloads leave source promises unresolved by design.
pub fn abort_native_navigation(
    scope: &mut v8::PinScope,
    token: &NativeNavigationToken,
) -> Result<bool, V8BindingError> {
    let state = current_state(scope)?;
    if !native_navigation_matches(&state, token) {
        return Ok(false);
    }
    stop_navigation(scope, &state)?;
    Ok(true)
}

#[derive(Clone, Copy, Debug)]
pub enum NativeNavigationFailure {
    Abort,
    Network,
    Security,
    QuotaExceeded,
    InvalidState,
}
/// Settle browser/automation custody without settling Navigation API promises:
/// 204/205/download navigation promises deliberately remain pending.
pub fn settle_joint_navigation_without_commit(
    scope: &mut v8::PinScope,
    token: &NativeNavigationToken,
    selected_destination: Option<&str>,
) -> Result<bool, V8BindingError> {
    let state = current_state(scope)?;
    if !native_navigation_matches(&state, token) {
        return Ok(false);
    }
    let pending = state.pending_navigation_destination();
    if let Some(destination) = pending
        .as_ref()
        .map(|entry| entry.id.as_str())
        .or(token.destination.as_deref())
        .or(selected_destination)
    {
        state.joint_history.borrow().borrow_mut().fail_route(
            &state.joint_route.borrow(),
            destination,
            crate::joint_history::JointTraversalFailure::Canceled,
        );
    }
    Ok(true)
}

pub fn fail_native_navigation(
    scope: &mut v8::PinScope,
    token: &NativeNavigationToken,
    failure: NativeNavigationFailure,
) -> Result<bool, V8BindingError> {
    let state = current_state(scope)?;
    if !native_navigation_matches(&state, token) {
        return Ok(false);
    }
    let name = match failure {
        NativeNavigationFailure::Abort => "AbortError",
        NativeNavigationFailure::Network => "NetworkError",
        NativeNavigationFailure::Security => "SecurityError",
        NativeNavigationFailure::QuotaExceeded => "QuotaExceededError",
        NativeNavigationFailure::InvalidState => "InvalidStateError",
    };
    if let Some(destination) = token.destination.as_deref() {
        let failure = match failure {
            NativeNavigationFailure::Abort => crate::joint_history::JointTraversalFailure::Canceled,
            _ => crate::joint_history::JointTraversalFailure::Failed(name.into()),
        };
        state.fail_joint_history_entry(destination, failure);
    }
    if state.navigation_api.borrow().pending.is_some() {
        let reason = dom_exception_value(scope, name)?;
        reject_pending(scope, &state, reason)?;
    } else {
        advance_native_revision(&state)?;
    }
    Ok(true)
}

/// HTML URL rewrite rules compare URL components, not opaque-origin tokens.
pub(super) fn can_rewrite_history_url(current: &Url, target: &Url) -> bool {
    if current.scheme() != target.scheme()
        || current.username() != target.username()
        || current.password() != target.password()
        || current.host() != target.host()
        || current.port() != target.port()
    {
        return false;
    }
    if matches!(current.scheme(), "http" | "https") {
        return true;
    }
    if current.path() != target.path() {
        return false;
    }
    current.scheme() == "file" || current.query() == target.query()
}
#[cfg(test)]
mod history_rewrite_tests {
    use super::*;
    #[test]
    fn opaque_document_history_rewrite_preserves_path_and_query() {
        for (from, to, allowed) in [
            ("about:srcdoc", "about:srcdoc#hash", true),
            ("about:blank", "about:blank?query", false),
            (
                "blob:https://example.test/id",
                "blob:https://example.test/id#hash",
                true,
            ),
            (
                "blob:https://example.test/id",
                "blob:https://example.test/other",
                false,
            ),
            (
                "https://example.test/a",
                "https://user@example.test/a",
                false,
            ),
            ("https://example.test/a", "https://example.test/b?q", true),
        ] {
            assert_eq!(
                can_rewrite_history_url(&Url::parse(from).unwrap(), &Url::parse(to).unwrap()),
                allowed,
                "{from} -> {to}"
            );
        }
    }
}

/// Admit a browser/history traversal using the existing entry identity. Reuse
/// author-dispatched admission rather than firing a second navigate event.
pub fn admit_native_history_traversal(
    scope: &mut v8::PinScope,
    destination: NavigationHistorySnapshot,
) -> Result<NativeNavigationAdmission, V8BindingError> {
    let state = current_state(scope)?;
    if state.execution_retired() {
        return Ok(NativeNavigationAdmission::Handled);
    }
    ensure(scope, &state)?;
    let destination_id = destination.id.as_str();
    if !state
        .navigation_history_entries()
        .iter()
        .any(|entry| entry.id == destination_id)
    {
        // The native session history also contains cross-origin entries hidden
        // from Navigation.entries. Browser traversal must still proceed, but
        // must not publish those private snapshots to this source document.
        if !abort_ongoing(scope, &state)? {
            return Ok(NativeNavigationAdmission::Handled);
        }
        return Ok(NativeNavigationAdmission::Proceed {
            operation: capture_native_history_operation(&state, &destination.id),
            destination: Some(destination),
        });
    }
    let previous = state
        .navigation_history_snapshot()
        .ok_or_else(|| failure("InvalidStateError"))?;
    if entries_disabled(&state) {
        // HTML traversal remains active without exposing Navigation entries or
        // firing navigate in an opaque/initial document.
        if !abort_ongoing(scope, &state)? {
            return Ok(NativeNavigationAdmission::Handled);
        }
        if destination.document_id != previous.document_id {
            return Ok(NativeNavigationAdmission::Proceed {
                operation: capture_native_history_operation(&state, &destination.id),
                destination: Some(destination),
            });
        }
        state.commit_location(&destination.url)?;
        history_traversed(scope, &state, destination_id)?;
        dispatch_history_transition(scope, &state, &previous.url, &destination.url, true)?;
        return Ok(NativeNavigationAdmission::Handled);
    }
    let already_handled = {
        let navigation = state.navigation_api.borrow();
        navigation.pending.as_ref().is_some_and(|pending| {
            pending.destination.id == destination_id
                && (pending.committed_entry
                    || pending
                        .event_id
                        .and_then(|id| navigation.events.get(&id))
                        .is_some_and(|event| event.intercepted))
        })
    };
    if already_handled {
        return Ok(NativeNavigationAdmission::Handled);
    }
    let already_dispatched = {
        let navigation = state.navigation_api.borrow();
        navigation.pending.as_ref().is_some_and(|pending| {
            pending.destination.id == destination_id
                && pending.navigation_type == "traverse"
                && pending
                    .event_id
                    .is_some_and(|event| admission_allows_native(&navigation, event))
        })
    };
    if !already_dispatched && !begin_history_traversal(scope, &state, destination_id)? {
        return Ok(NativeNavigationAdmission::Handled);
    }
    let Some(destination) = state
        .pending_navigation_destination()
        .filter(|entry| entry.id == destination_id)
    else {
        return Ok(NativeNavigationAdmission::Handled);
    };
    if destination.document_id == previous.document_id {
        state.commit_location(&destination.url)?;
        history_traversed(scope, &state, destination_id)?;
        dispatch_history_transition(scope, &state, &previous.url, &destination.url, true)?;
        finish_history_traversal(scope, &state)?;
        return Ok(NativeNavigationAdmission::Handled);
    }
    let operation = capture_native_operation(&state, destination_id);
    if operation.is_none() {
        return Ok(NativeNavigationAdmission::Handled);
    }
    Ok(NativeNavigationAdmission::Proceed {
        operation,
        destination: Some(destination),
    })
}

pub fn capture_native_source(
    scope: &mut v8::PinScope,
    source_element: Option<NodeId>,
    user_initiated: bool,
    form_data: Option<v8::Global<v8::Object>>,
    download: Option<String>,
) -> Result<NativeNavigationSource, V8BindingError> {
    let state = current_state(scope)?;
    let window = state
        .global_object
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?;
    let source_element_object = source_element
        .map(|node| wrap_node(scope, &state, node).map(|object| v8::Global::new(scope, object)))
        .transpose()?;
    Ok(NativeNavigationSource {
        execution_observer: None,
        source_origin_key: None,
        source_domain_key: document_domain_access_key(&state, state.document_root())?,
        source_element,
        source_element_object,
        source_window: Some(v8::Global::new(scope, window)),
        user_initiated,
        form_data,
        download,
    })
}

/// History exposes the full native session cursor; Navigation exposes only its
/// contiguous origin segment. No foreign entry object is created here.
pub fn sync_session_history(
    scope: &mut v8::PinScope,
    update: NavigationHistoryUpdate,
) -> Result<(), V8BindingError> {
    let state = current_state(scope)?;
    if update.entries.is_empty()
        || update.current >= update.entries.len()
        || update.entries.len() > 4096
        || update.entries.iter().any(|entry| !entry.scroll.valid())
    {
        return Err(failure("InvalidStateError"));
    }
    let projection = history_projection::prepare(&state, update.entries.len())?;
    {
        let group = state.joint_history.borrow().clone();
        let route = state.joint_route.borrow().clone();
        if route == "root"
            && group.borrow().route_count() <= 1
            && group.borrow().len() < update.entries.len()
        {
            let mut timeline = crate::joint_history::Timeline::default();
            timeline
                .seed(route, update.entries.clone(), update.current)
                .map_err(|_| failure("InvalidStateError: joint session"))?;
            *group.borrow_mut() = timeline;
        }
    }
    let scroll_restoration = update.entries[update.current]
        .scroll
        .mode
        .as_str()
        .to_owned();

    *state.history.borrow_mut() = Some(HistoryRecord {
        entries: update
            .entries
            .iter()
            .map(|entry| HistoryEntry {
                url: entry.url.clone(),
                state_json: entry.history_state.clone(),
                navigation_id: entry.id.clone(),
                document_id: entry.document_id.clone(),
            })
            .collect(),
        index: update.current,
        scroll_restoration,
    });
    if let Some(projection) = projection {
        projection.commit()?;
    }
    Ok(())
}

pub(super) fn publish_current_native_history_projection(
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let count = state
        .history
        .borrow()
        .as_ref()
        .map(|history| history.entries.len());
    let Some(count) = count else {
        if state
            .navigation_api
            .borrow()
            .history_projection
            .publisher
            .is_none()
        {
            return Ok(());
        }
        return Err(V8BindingError::Allocation(
            "native full history projection unavailable",
        ));
    };
    if let Some(projection) = history_projection::prepare(state, count)? {
        projection.commit()?;
    }
    Ok(())
}

pub(super) fn prepare_initial_history_projection(
    state: &Rc<V8BindingState>,
) -> Result<Option<history_projection::Reservation>, V8BindingError> {
    history_projection::prepare(state, 1)
}

pub fn capture_current_native_operation(state: &V8BindingState) -> Option<NativeNavigationToken> {
    match state.pending_navigation_destination() {
        Some(entry) => capture_native_operation(state, &entry.id),
        None => capture_native_revision(state),
    }
}

#[cfg(test)]
mod history_publication_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::test_eval::eval(scope, source)
    }
    fn entry(id: &str) -> NavigationHistorySnapshot {
        NavigationHistorySnapshot {
            scroll: Default::default(),
            key: format!("key-{id}"),
            id: id.into(),
            url: format!("https://example.test/{id}"),
            document_id: "retained-document".into(),
            state: None,
            history_state: None,
        }
    }
    #[test]
    fn pagehide_visibility_and_unload_share_retained_order_and_ignore_opens_guard() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><p id=kept>retained</p></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.trace=[];addEventListener('pagehide',()=>trace.push('pagehide:'+document.hidden));document.addEventListener('visibilitychange',e=>{trace.push('visibility:'+document.hidden);if(!e.isTrusted||e.target!==document)throw Error('visibility target');if(document.hidden){document.open();if(!document.getElementById('kept'))throw Error('unload guard lost');}});addEventListener('unload',()=>trace.push('unload:'+document.hidden));addEventListener('pageshow',()=>trace.push('pageshow:'+document.hidden));"#,
        );
        let guard = state.begin_unload();
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Page {
                kind: "pagehide".into(),
                persisted: false,
            },
        )
        .unwrap();
        assert!(eval(scope, "trace.join()==='pagehide:false'&&!document.hidden").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Unload {
                timing: Default::default(),
            },
        )
        .unwrap();
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        drop(guard);
        assert!(eval(scope,"trace.join()==='pagehide:false,visibility:true,unload:true'&&!!document.getElementById('kept')").is_true());
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Page {
                kind: "pageshow".into(),
                persisted: true,
            },
        )
        .unwrap();
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(eval(scope,"trace.join()==='pagehide:false,visibility:true,unload:true,visibility:false,pageshow:false'").is_true());
    }
    #[test]
    fn lifecycle_visibility_composes_minimized_prerender_and_parked_publication() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct Messages;
        impl crate::message_wire::MessageTransport for Messages {
            fn request(
                &self,
                request: crate::message_wire::MessageRequest,
            ) -> Result<crate::message_wire::MessageResponse, String> {
                match request {
                    crate::message_wire::MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    } => Ok(crate::message_wire::MessageResponse::Done),
                    other => Err(format!(
                        "unexpected visibility fixture registration: {other:?}"
                    )),
                }
            }
        }
        struct Window(Arc<AtomicBool>);
        impl crate::platform_control::Backend for Window {
            fn request(
                &self,
                _: crate::worker_wire::Creator,
                request: crate::platform_control::Request,
            ) -> Result<crate::platform_control::Response, String> {
                assert!(matches!(
                    request,
                    crate::platform_control::Request::WindowState
                ));
                Ok(crate::platform_control::Response::WindowState {
                    hidden: self.0.load(Ordering::Relaxed),
                    revision: 1,
                })
            }
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let minimized = Arc::new(AtomicBool::new(false));
        state.set_message_transport(Arc::new(Messages));
        state.set_native_platform_backend(Arc::new(Window(minimized.clone())));
        eval(
            scope,
            "globalThis.changes=[];document.addEventListener('visibilitychange',()=>changes.push(document.visibilityState))",
        );
        let hide = start_native_event_publication(
            scope,
            NativePublicationEvent::Page {
                kind: "pagehide".into(),
                persisted: true,
            },
        )
        .unwrap();
        assert!(resume_native_history_publication(scope, hide).unwrap());
        minimized.store(true, Ordering::Relaxed);
        let show = start_native_event_publication(
            scope,
            NativePublicationEvent::Page {
                kind: "pageshow".into(),
                persisted: true,
            },
        )
        .unwrap();
        assert!(resume_native_history_publication(scope, show).unwrap());
        assert!(eval(scope, "document.hidden&&changes.join()==='hidden'").is_true());
        assert!(!native_platform::rendering_visible(&state).unwrap());
        minimized.store(false, Ordering::Relaxed);
        state.document_prerendering.set(true);
        struct PrivateWindow;
        impl crate::platform_control::Backend for PrivateWindow {
            fn request(
                &self,
                _: crate::worker_wire::Creator,
                _: crate::platform_control::Request,
            ) -> Result<crate::platform_control::Response, String> {
                panic!("private hidden document queried active backend")
            }
        }
        state.set_native_platform_backend(Arc::new(PrivateWindow));
        let show = start_native_event_publication(
            scope,
            NativePublicationEvent::Page {
                kind: "pageshow".into(),
                persisted: true,
            },
        )
        .unwrap();
        assert!(resume_native_history_publication(scope, show).unwrap());
        assert!(eval(scope, "document.hidden&&changes.length===1").is_true());
        state.set_native_platform_backend(Arc::new(Window(minimized)));
        state.document_prerendering.set(false);
        state.set_execution_suspended(true);
        let show = start_native_event_publication(
            scope,
            NativePublicationEvent::Page {
                kind: "pageshow".into(),
                persisted: true,
            },
        )
        .unwrap();
        assert!(!resume_native_history_publication(scope, show).unwrap());
        state.set_execution_suspended(false);
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, show).unwrap());
        assert!(eval(scope, "!document.hidden&&changes.join()==='hidden,visible'").is_true());
    }
    #[test]
    fn script_close_uses_full_session_entry_count_not_visible_document_count() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let first = entry("a");
        let second = entry("b");
        sync_history(scope, vec![first.clone()], 0, "push").unwrap();
        sync_session_history(
            scope,
            NavigationHistoryUpdate {
                entries: vec![first.clone(), second.clone()],
                current: 1,
                navigation_type: "push".into(),
                truncates_forward: false,
            },
        )
        .unwrap();
        assert!(
            !state.native_history_has_single_entry(),
            "pushState and fragment entries still count as separate session entries"
        );
        let mut foreign = entry("foreign");
        foreign.url = "https://foreign.test/".into();
        foreign.document_id = "other-document".into();
        sync_session_history(
            scope,
            NavigationHistoryUpdate {
                entries: vec![first.clone(), foreign],
                current: 0,
                navigation_type: "traverse".into(),
                truncates_forward: false,
            },
        )
        .unwrap();
        assert_eq!(state.navigation_history_entries().len(), 1);
        assert!(
            !state.native_history_has_single_entry(),
            "foreign forward history cannot disappear from close eligibility"
        );
        sync_session_history(
            scope,
            NavigationHistoryUpdate {
                entries: vec![first, second],
                current: 1,
                navigation_type: "push".into(),
                truncates_forward: true,
            },
        )
        .unwrap();
        assert!(
            !state.native_history_has_single_entry(),
            "two remaining same-Document entries still prohibit ordinary script close"
        );
        sync_session_history(
            scope,
            NavigationHistoryUpdate {
                entries: vec![entry("only")],
                current: 0,
                navigation_type: "replace".into(),
                truncates_forward: true,
            },
        )
        .unwrap();
        assert!(state.native_history_has_single_entry());
    }
    #[test]
    fn navigate_admission_document_open_cannot_publish_old_destination_or_event_tail() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.trace=[];navigation.onnavigate=()=>{trace.push('old');document.open()};globalThis.result=navigation.navigate('#obsolete');result.committed.catch(e=>trace.push('committed:'+e.name));result.finished.catch(e=>trace.push('finished:'+e.name))",
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"trace.join()==='old,committed:AbortError,finished:AbortError'&&!location.href.includes('obsolete')").is_true());
        assert!(state.navigation_api.borrow().publications.is_empty());
        assert!(state.navigation_api.borrow().pending.is_none());
    }
    #[test]
    fn discard_requested_inside_existing_abort_keeps_listener_suffix_alive() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn request(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            mut rv: v8::ReturnValue,
        ) {
            let state = current_state(scope).unwrap();
            rv.set_bool(abort_navigation_for_discard(scope, &state).unwrap());
        }
        let function = v8::Function::new(scope, request).unwrap();
        let key = v8::String::new(scope, "requestDiscard").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), function.into());
        eval(
            scope,
            "globalThis.trace=[];let controller=new AbortController();controller.signal.addEventListener('abort',()=>{if(requestDiscard())throw Error('retired callback stack');trace.push('first')});controller.signal.addEventListener('abort',()=>trace.push('tail'));controller.abort()",
        );
        assert!(eval(scope, "trace.join()==='first,tail'").is_true());
        assert!(!state.execution_retired());
        assert!(abort_navigation_for_discard(scope, &state).unwrap());
    }
    #[test]
    fn discard_during_navigate_aborts_signal_and_both_promises_before_retirement() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        fn discard(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            let state = current_state(scope).unwrap();
            assert!(abort_navigation_for_discard(scope, &state).unwrap());
            state.retire_execution();
        }
        let function = v8::Function::new(scope, discard).unwrap();
        let key = v8::String::new(scope, "discardNative").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), function.into());
        eval(
            scope,
            "globalThis.trace=[];navigation.onnavigate=e=>{globalThis.signal=e.signal;e.signal.onabort=()=>trace.push('abort');discardNative()};globalThis.result=navigation.navigate('#next');result.committed.catch(e=>trace.push('committed:'+e.name));result.finished.catch(e=>trace.push('finished:'+e.name));if(!signal.aborted||trace.join()!=='abort')throw Error('signal not synchronously aborted')",
        );
        scope.perform_microtask_checkpoint();
        assert!(
            eval(
                scope,
                "trace.join()==='abort,committed:AbortError,finished:AbortError'"
            )
            .is_true()
        );
    }
    #[test]
    fn discard_retains_abort_suffix_and_rejects_reentrant_navigation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.trace=[];navigation.onnavigate=e=>{globalThis.signal=e.signal;e.intercept({handler:()=>new Promise(()=>{})});e.signal.addEventListener('abort',()=>{trace.push('first');try{history.pushState(null,'','#forbidden')}catch(e){trace.push('history:'+e.name)}let next=navigation.navigate('#forbidden');next.committed.catch(e=>trace.push('reentry:'+e.name));next.finished.catch(()=>{})});e.signal.addEventListener('abort',()=>trace.push('tail'))};globalThis.result=navigation.navigate('#next');result.finished.catch(e=>trace.push('finished:'+e.name))",
        );
        scope.perform_microtask_checkpoint();
        native_platform::publication::fail_after_next_callback();
        assert!(abort_navigation_for_discard(scope, &state).is_err());
        assert!(!state.execution_retired());
        assert!(
            eval(
                scope,
                "signal.aborted&&trace.join()==='first,history:SecurityError'"
            )
            .is_true(),
            "{}",
            eval(scope, "JSON.stringify({aborted:signal.aborted,trace})")
                .to_rust_string_lossy(scope)
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(abort_navigation_for_discard(scope, &state).unwrap());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"trace.filter(x=>x==='first').length===1&&trace.includes('tail')&&trace.includes('reentry:InvalidStateError')&&trace.includes('finished:AbortError')").is_true());
        assert!(state.navigation_api.borrow().pending.is_none());
    }

    #[test]
    fn unload_publication_retains_callback_suffix_and_finishes_measurement_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];addEventListener('unload',e=>{globalThis.original=e;trace.push('first')},{once:true});addEventListener('unload',e=>{if(e!==original||!e.isTrusted||e.cancelable)throw Error('unload identity');trace.push('tail')})",
        );
        assert!(state.has_unload_handlers());
        let timing = crate::navigation_timing::UnloadTiming::default();
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Unload {
                timing: timing.clone(),
            },
        )
        .unwrap();
        assert!(timing.interval().is_none());
        assert!(state.is_unloading());
        let epoch = state.document_stream_epoch();
        eval(scope, "document.open();document.write('ignored')");
        assert_eq!(state.document_stream_epoch(), epoch);
        assert!(eval(scope, "trace.join()==='first'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        let interval = timing.interval().unwrap();
        assert!(!state.is_unloading());
        assert!(interval.0 <= interval.1);
        assert!(eval(scope, "trace.join()==='first,tail'").is_true());
        assert!(
            resume_native_history_publication(scope, receipt).is_err(),
            "successful native receipt is consumed"
        );
        assert!(eval(scope, "trace.join()==='first,tail'").is_true());
        assert_eq!(timing.interval(), Some(interval));
    }
    #[test]
    fn print_publication_retains_event_suffix_across_failure_and_suspension() {
        super::super::tests::initialize_v8();
        for kind in ["beforeprint", "afterprint"] {
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            eval(
                scope,
                &format!(
                    "globalThis.trace=[];addEventListener('{kind}',e=>{{globalThis.original=e;trace.push('first')}},{{once:true}});addEventListener('{kind}',e=>{{if(e!==original||!e.isTrusted||e.bubbles||e.cancelable)throw Error('print identity');trace.push('tail')}})"
                ),
            );
            native_platform::publication::fail_after_next_callback();
            let receipt = start_native_event_publication(
                scope,
                NativePublicationEvent::Print { kind: kind.into() },
            )
            .unwrap();
            assert!(eval(scope, "trace.join()==='first'").is_true());
            state.set_execution_suspended(true);
            assert!(!resume_native_history_publication(scope, receipt).unwrap());
            state.set_execution_suspended(false);
            state.navigation_api.borrow_mut().publication_retry_at = None;
            assert!(resume_native_history_publication(scope, receipt).unwrap());
            assert!(eval(scope, "trace.join()==='first,tail'").is_true());
        }
    }

    #[test]
    fn print_callback_suffix_cannot_cross_document_open_stream() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.trace=[];addEventListener('beforeprint',()=>trace.push('first'));addEventListener('beforeprint',()=>trace.push('stale-tail'))",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Print {
                kind: "beforeprint".into(),
            },
        )
        .unwrap();
        assert!(eval(scope, "trace.join()==='first'").is_true());
        assert_eq!(
            state.navigation_api.borrow().parser_receipts.get(&receipt),
            Some(&state.document_stream_epoch())
        );
        state
            .document_stream_epoch
            .set(state.document_stream_epoch().wrapping_add(1));
        retire_parser_publications(&state);
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).is_err());
        poll_publications(scope, &state).unwrap();
        assert!(eval(scope, "trace.join()==='first'").is_true());
    }

    #[test]
    fn reload_swap_uses_current_entry_and_staged_push_survives_preparation_failure() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        let mut replacement = entry("replacement");
        replacement.url = "https://example.test/a".into();
        stage_swap_destination(scope, replacement).unwrap();
        let reload = activation_for_swap(scope, "https://example.test/a", "reload").unwrap();
        let current: v8::Local<v8::Value> =
            v8::Local::new(scope, &state.navigation_api.borrow().entries["a"].object).into();
        assert_eq!(
            attribute(scope, &state, reload, "NavigationActivation", "entry").unwrap(),
            current
        );
        assert_eq!(
            attribute(scope, &state, reload, "NavigationActivation", "from").unwrap(),
            current
        );
        let destination = entry("b");
        let url = destination.url.clone();
        stage_swap_destination(scope, destination).unwrap();
        let before = state.platform_objects.borrow().len();
        FAIL_ENTRY_STAGING.with(|remaining| remaining.set(Some(0)));
        assert!(activation_for_swap(scope, &url, "push").is_err());
        assert_eq!(
            state
                .navigation_api
                .borrow()
                .staged_swap
                .as_ref()
                .unwrap()
                .id,
            "b"
        );
        assert_eq!(state.platform_objects.borrow().len(), before);
        let first = activation_for_swap(scope, &url, "push").unwrap();
        let second = activation_for_swap(scope, &url, "push").unwrap();
        assert_eq!(first, second);
        assert_eq!(
            state
                .navigation_api
                .borrow()
                .staged_swap
                .as_ref()
                .unwrap()
                .id,
            "b"
        );
        let mut receipt =
            page_swap::NativePageSwap::new(scope, &state, &url, "push", true).unwrap();
        assert!(state.navigation_api.borrow().staged_swap.is_none());
        receipt.cancel();
    }

    #[test]
    fn native_execution_loader_transfer_releases_source_pending_observer() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        let outcomes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = outcomes.clone();
        let observer = crate::navigation::NavigationExecutionObserver::new(move |outcome| {
            sink.lock().unwrap().push(outcome)
        });
        let source = NativeNavigationSource {
            execution_observer: Some(observer.clone()),
            ..Default::default()
        };
        assert!(matches!(
            admit_native_navigation(scope, "https://example.test/b", false, false, source).unwrap(),
            NativeNavigationAdmission::Proceed { .. }
        ));
        assert!(
            state
                .navigation_api
                .borrow()
                .pending
                .as_ref()
                .unwrap()
                .execution_observer
                .is_none()
        );
        drop(observer);
        assert!(matches!(
            outcomes.lock().unwrap().as_slice(),
            [crate::navigation::NavigationExecutionOutcome::Failed { .. }]
        ));
    }
    #[test]
    fn entry_staging_failure_leaves_history_and_native_registrations_unchanged() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        let keys = state
            .navigation_api
            .borrow()
            .entries
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let platforms = state.platform_objects.borrow().len();
        let wrappers = state.platform_wrappers.borrow().len();
        let history = state
            .history
            .borrow()
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.navigation_id.clone())
            .collect::<Vec<_>>();
        FAIL_ENTRY_STAGING.with(|remaining| remaining.set(Some(1)));
        assert!(sync_history(scope, vec![entry("a"), entry("b"), entry("c")], 2, "push").is_err());
        assert_eq!(
            state
                .navigation_api
                .borrow()
                .entries
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            keys
        );
        assert_eq!(state.platform_objects.borrow().len(), platforms);
        assert_eq!(state.platform_wrappers.borrow().len(), wrappers);
        assert_eq!(
            state
                .history
                .borrow()
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .map(|entry| entry.navigation_id.clone())
                .collect::<Vec<_>>(),
            history
        );
        assert_eq!(state.navigation_api.borrow().current.as_deref(), Some("a"));
        sync_history(scope, vec![entry("a"), entry("b"), entry("c")], 2, "push").unwrap();
        assert_eq!(state.navigation_api.borrow().current.as_deref(), Some("c"));
    }
    #[test]
    fn committed_history_event_failure_resumes_same_event_and_disposes_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a"), entry("b")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.events=[];globalThis.original=navigation.currentEntry;globalThis.old=navigation.entries()[1];globalThis.change=null;navigation.addEventListener('currententrychange',e=>{change=e;events.push(e.navigationType+':'+e.from.id)},{once:true});globalThis.second=e=>{if(change!==e||!e.isTrusted)throw Error('event identity');events.push('second')};navigation.addEventListener('currententrychange',second);old.addEventListener('dispose',e=>{if(e.target!==old)throw Error('dispose identity');events.push('dispose')})",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(sync_history(scope, vec![entry("a"), entry("c")], 1, "push").is_err());
        assert_eq!(state.navigation_api.borrow().current.as_deref(), Some("c"));
        assert!(eval(scope, "events.join(',')==='push:a'&&old.index===-1").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "events.join(',')==='push:a,second,dispose'&&change.from===original"
            )
            .is_true()
        );
        assert_eq!(poll_publications(scope, &state).unwrap(), 0);
        // A restored same-document/native history selection reuses the entry
        // wrapper and does not recreate or re-dispose the removed entry.
        eval(
            scope,
            "navigation.removeEventListener('currententrychange',second);events.length=0;change=null",
        );
        sync_history(scope, vec![entry("a"), entry("c")], 0, "traverse").unwrap();
        assert!(eval(scope, "navigation.currentEntry===original&&old.index===-1").is_true());
    }
    #[test]
    fn navigation_finish_retains_success_event_after_pending_is_consumed() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        let committed = v8::PromiseResolver::new(scope).unwrap();
        let finished = v8::PromiseResolver::new(scope).unwrap();
        state.navigation_api.borrow_mut().pending = Some(PendingNavigation {
            execution_observer: None,
            committed: v8::Global::new(scope, committed),
            finished: v8::Global::new(scope, finished),
            destination: entry("a"),
            navigation_type: "push".into(),
            event_id: None,
            transition: None,
            committed_entry: true,
            remaining: 0,
            plan: None,
        });
        eval(
            scope,
            "globalThis.calls=[];globalThis.saved=null;navigation.addEventListener('navigatesuccess',e=>{saved=e;calls.push('first')},{once:true});navigation.addEventListener('navigatesuccess',e=>{if(saved!==e)throw Error('identity');calls.push('second')})",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(finish_navigation(scope).is_err());
        assert!(state.navigation_api.borrow().pending.is_none());
        assert!(eval(scope, "calls.join(',')==='first'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "calls.join(',')==='first,second'&&saved.currentTarget===null"
            )
            .is_true()
        );
        assert_eq!(
            finished.get_promise(scope).state(),
            v8::PromiseState::Fulfilled
        );
    }

    #[test]
    fn native_commit_receipt_distinguishes_accepted_callback_failure_from_uncommitted_failure() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.calls=[];navigation.addEventListener('currententrychange',()=>calls.push('first'),{once:true});navigation.addEventListener('currententrychange',()=>calls.push('second'))",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_history_publication(
            scope,
            NavigationHistoryUpdate {
                entries: vec![entry("a"), entry("b")],
                current: 1,
                navigation_type: "push".into(),
                truncates_forward: true,
            },
        )
        .unwrap();
        assert_eq!(state.navigation_api.borrow().current.as_deref(), Some("b"));
        assert!(!resume_native_history_publication(scope, receipt).unwrap());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(eval(scope, "calls.join(',')==='first,second'").is_true());
        assert!(state.navigation_api.borrow().native_receipts.is_empty());
        FAIL_ENTRY_STAGING.with(|remaining| remaining.set(Some(0)));
        assert!(
            start_native_history_publication(
                scope,
                NavigationHistoryUpdate {
                    entries: vec![entry("a"), entry("b"), entry("c")],
                    current: 2,
                    navigation_type: "push".into(),
                    truncates_forward: true
                }
            )
            .is_err()
        );
        assert!(state.navigation_api.borrow().native_receipts.is_empty());
        assert_eq!(state.navigation_api.borrow().current.as_deref(), Some("b"));
    }

    #[test]
    fn intercepted_handler_value_survives_failure_without_calling_handler_twice() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.handlers=[];globalThis.interceptError=null;navigation.addEventListener('navigate',e=>{try{e.intercept({handler(){handlers.push('first');return 1}});e.intercept({handler(){handlers.push('second');return 2}})}catch(error){interceptError=String(error.stack||error);throw error}})",
        );
        FAIL_HANDLER_AFTER_CALL.with(|failure| failure.set(true));
        eval(
            scope,
            "globalThis.result=navigation.navigate('#b');result.finished.catch(()=>{});result.committed.catch(()=>{})",
        );
        assert!(
            eval(scope, "handlers.join(',')==='first'").is_true(),
            "{}",
            eval(scope, "JSON.stringify({handlers,interceptError})").to_rust_string_lossy(scope)
        );
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "handlers.join(',')==='first,second'").is_true());
    }

    #[test]
    fn native_interception_consumes_navigation_without_loader_request() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.events=0;navigation.onnavigate=e=>{events++;e.intercept()}",
        );
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://example.test/b",
                false,
                false,
                Default::default()
            )
            .unwrap(),
            NativeNavigationAdmission::Handled
        ));
        assert!(state.pending_location_navigation.borrow().is_none());
        assert_eq!(state.current_location_url(), "https://example.test/b");
        assert!(eval(scope, "events===1").is_true());
        scope.perform_microtask_checkpoint();
        poll_publications(scope, &state).unwrap();
        assert!(state.pending_location_navigation.borrow().is_none());
        assert!(eval(scope, "events===1").is_true());
    }

    #[test]
    fn failed_native_admission_retains_tail_and_never_grants_a_navigation_token() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.saved=null;navigation.addEventListener('navigate',e=>{saved=e;calls.push('first')},{once:true});navigation.addEventListener('navigate',e=>{if(saved!==e)throw Error('identity');calls.push('cancel');e.preventDefault()},{once:true})",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(
            admit_native_navigation(
                scope,
                "https://example.test/b",
                false,
                false,
                Default::default()
            )
            .is_err()
        );
        let destination = state.pending_navigation_destination().unwrap();
        assert!(capture_native_operation(&state, &destination.id).is_none());
        assert!(state.pending_location_navigation.borrow().is_none());
        assert_eq!(
            state.navigation_history_snapshot().unwrap().url,
            "https://example.test/a"
        );
        assert!(eval(scope, "calls.join(',')==='first'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "calls.join(',')==='first,cancel'&&saved.defaultPrevented&&saved.signal.aborted"
            )
            .is_true()
        );
        assert!(state.pending_location_navigation.borrow().is_none());
        assert!(state.navigation_api.borrow().pending.is_none());
        // An explicit new request gets a new admission. Recovery never issues
        // the failed request in the background.
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://example.test/c",
                false,
                false,
                Default::default()
            )
            .unwrap(),
            NativeNavigationAdmission::Proceed {
                operation: Some(_),
                ..
            }
        ));
    }
    #[test]
    fn native_admission_termination_is_not_success_and_author_exceptions_are_not_cancellation() {
        fn terminate(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        let callback = v8::Function::new(scope, terminate).unwrap();
        let key = v8::String::new(scope, "terminateNative").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), callback.into());
        eval(
            scope,
            "globalThis.calls=[];navigation.addEventListener('navigate',()=>{calls.push('first');terminateNative()},{once:true});navigation.addEventListener('navigate',()=>calls.push('tail'),{once:true})",
        );
        assert!(
            admit_native_navigation(
                scope,
                "https://example.test/b",
                false,
                false,
                Default::default()
            )
            .is_err()
        );
        assert!(scope.is_execution_terminating());
        scope.cancel_terminate_execution();
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(eval(scope, "calls.join(',')==='first,tail'").is_true());
        assert!(state.pending_location_navigation.borrow().is_none());
        eval(
            scope,
            "navigation.addEventListener('navigate',()=>{throw Error('author listener')},{once:true});navigation.addEventListener('navigate',()=>calls.push('author-tail'),{once:true})",
        );
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://example.test/c",
                false,
                false,
                Default::default()
            )
            .unwrap(),
            NativeNavigationAdmission::Proceed {
                operation: Some(_),
                ..
            }
        ));
        assert!(eval(scope, "calls.join(',')==='first,tail,author-tail'").is_true());
    }
    #[test]
    fn native_history_transaction_publishes_session_and_activation_before_reentrant_history() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        eval(
            scope,
            "globalThis.observed=null;navigation.addEventListener('currententrychange',()=>{observed=[history.length,navigation.activation.entry.id];history.replaceState({author:true},'', '#author')},{once:true})",
        );
        let update = NavigationHistoryUpdate {
            entries: vec![entry("a"), entry("b")],
            current: 1,
            navigation_type: "traverse".into(),
            truncates_forward: false,
        };
        let receipt =
            start_native_history_transaction(scope, update.clone(), update, Some(Some(entry("a"))))
                .unwrap();
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"observed[0]===2&&observed[1]==='b'&&history.state.author&&location.hash==='#author'").is_true());
        assert!(
            state.history.borrow().as_ref().unwrap().entries[1]
                .url
                .ends_with("#author")
        );
    }

    #[test]
    fn lifecycle_receipts_resume_dom_load_and_pageshow_with_legacy_targets() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.saved=null;document.addEventListener('DOMContentLoaded',e=>{saved=e;calls.push('dom1')},{once:true});document.addEventListener('DOMContentLoaded',e=>calls.push(e===saved?'dom2':'bad-dom'),{once:true});addEventListener('load',e=>{saved=e;calls.push(e.target===document&&e.currentTarget===window&&e.composedPath()[0]===window?'load1':'bad-load')},{once:true});addEventListener('load',e=>calls.push(e===saved?'load2':'bad-load-identity'),{once:true});addEventListener('pageshow',e=>calls.push(e.persisted?'bad-persisted':'show'),{once:true})",
        );
        assert!(!state.document_completely_loaded());
        native_platform::publication::fail_after_next_callback();
        let dom_receipt = start_native_document_lifecycle_publication(scope, true).unwrap();
        assert!(eval(scope, "calls.join(',')==='dom1'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, dom_receipt).unwrap());
        assert!(eval(scope, "calls.join(',')==='dom1,dom2'").is_true());
        native_platform::publication::fail_after_next_callback();
        let load_receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(!state.document_completely_loaded());
        assert!(eval(scope, "calls.join(',')==='dom1,dom2,load1'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, load_receipt).unwrap());
        assert!(
            eval(
                scope,
                "calls.join(',')==='dom1,dom2,load1,load2,show'&&document.readyState==='complete'"
            )
            .is_true()
        );
        assert!(state.navigation_api.borrow().parser_receipts.is_empty());
        assert!(state.document_completely_loaded());
        eval(scope, "document.open()");
        assert!(!state.document_completely_loaded());
    }
    #[test]
    fn script_error_insertion_suppression_survives_callback_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><p id=before>before</p><script id=s type=application/json></script></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let gate = DocumentParserGate::new(
            oryn_dom::html::StreamingParser::with_document(dom, state.document_root()),
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        *state.document_parser_gate.borrow_mut() = Some(gate);
        let node = state.dom.borrow().find_by_id("s").unwrap();
        eval(
            scope,
            "globalThis.calls=[];const script=document.getElementById('s');script.addEventListener('error',()=>calls.push('first'));script.addEventListener('error',()=>{document.write('replacement');document.close();calls.push(String(!!document.getElementById('before')))})",
        );
        native_platform::publication::fail_after_next_callback();
        let _receipt =
            start_native_script_completion_publication_with_insertion(scope, node, false, false)
                .unwrap();
        assert!(eval(scope, "calls.join(',')==='first'").is_true());
        assert!(state.parser_insertion_suppression.get().is_none());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(eval(scope,"calls.join(',')==='first,false'&&document.documentElement.textContent==='replacement'").is_true());
        assert!(state.parser_insertion_suppression.get().is_none());
    }

    #[test]
    fn retired_child_preserves_started_parent_frame_event_but_cancels_unstarted_event() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<iframe id=f></iframe>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let frame = state
            .dom
            .borrow()
            .query_selector(state.document_root(), "#f")
            .unwrap()
            .unwrap();
        eval(
            scope,
            "globalThis.calls=[];document.getElementById('f').addEventListener('load',()=>calls.push('first'));document.getElementById('f').addEventListener('load',()=>calls.push('second'))",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_frame_load_publication(scope, frame).unwrap();
        assert!(eval(scope, "calls.join(',')==='first'").is_true());
        retire_native_event_receipt(&state, receipt);
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(eval(scope, "calls.join(',')==='first,second'").is_true());
        assert!(
            !state
                .navigation_api
                .borrow()
                .native_receipts
                .contains_key(&receipt)
        );
        native_platform::publication::fail_before_next_prepare();
        let receipt = start_native_frame_load_publication(scope, frame).unwrap();
        retire_native_event_receipt(&state, receipt);
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(eval(scope, "calls.join(',')==='first,second'").is_true());
    }
    #[test]
    fn load_document_open_does_not_consume_initial_pageshow() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.shows=0;addEventListener('load',()=>document.open(),{once:true});addEventListener('pageshow',()=>shows++)",
        );
        let _receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(!state.initial_pageshow_delivered.get());
        assert!(eval(scope, "shows===0&&document.readyState==='loading'").is_true());
        assert!(state.navigation_api.borrow().parser_receipts.is_empty());
    }
    #[test]
    fn parser_script_completion_receipt_resumes_listener_tail_without_restarting_dispatch() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><script id=target type=text/plain></script></body>")
                .into_runtime()
                .unwrap(),
        ));
        let node = dom.borrow().find_by_id("target").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.firstEvent=null;const script=document.getElementById('target');script.addEventListener('load',e=>{firstEvent=e;calls.push('first')});script.addEventListener('load',e=>{if(firstEvent!==e)throw Error('event identity');calls.push('second')})",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_script_completion_publication(scope, node, true).unwrap();
        assert!(eval(scope, "calls.join(',')==='first'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(eval(scope, "calls.join(',')==='first,second'").is_true());
        assert!(state.navigation_api.borrow().parser_receipts.is_empty());
    }
    #[test]
    fn parser_readiness_receipt_retains_event_tail_and_document_open_retires_old_receipts() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.saved=null;document.addEventListener('readystatechange',e=>{saved=e;calls.push('first')},{once:true});document.addEventListener('readystatechange',e=>{if(saved!==e)throw Error('identity');calls.push('second')},{once:true})",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt =
            start_native_readiness_publication(scope, oryn_dom::DocumentReadyState::Interactive)
                .unwrap();
        assert!(
            eval(
                scope,
                "calls.join(',')==='first'&&document.readyState==='interactive'"
            )
            .is_true()
        );
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(eval(scope, "calls.join(',')==='first,second'").is_true());
        assert!(state.navigation_api.borrow().parser_receipts.is_empty());
        eval(
            scope,
            "document.addEventListener('readystatechange',()=>calls.push('old-first'),{once:true});document.addEventListener('readystatechange',()=>calls.push('old-tail'),{once:true})",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt =
            start_native_readiness_publication(scope, oryn_dom::DocumentReadyState::Complete)
                .unwrap();
        assert!(
            state
                .navigation_api
                .borrow()
                .parser_receipts
                .contains_key(&receipt)
        );
        eval(scope, "document.open()");
        assert!(
            !state
                .navigation_api
                .borrow()
                .native_receipts
                .contains_key(&receipt)
        );
        assert!(state.navigation_api.borrow().parser_receipts.is_empty());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "!calls.includes('old-tail')&&document.readyState==='loading'"
            )
            .is_true()
        );
    }
}

#[cfg(test)]
mod native_source_origin_admission_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::test_eval::eval(scope, source)
    }
    fn entry(id: &str) -> NavigationHistorySnapshot {
        NavigationHistorySnapshot {
            scroll: Default::default(),
            key: format!("key-{id}"),
            id: id.into(),
            url: format!("https://example.test/{id}"),
            document_id: "retained-document".into(),
            state: None,
            history_state: None,
        }
    }
    #[test]
    fn relaxed_navigation_identity_requires_bilateral_opt_in_and_preserves_strict_origin() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let strict = crate::origin::Origin::from_url("https://target.example.test:8443");
        state
            .install_document_provenance(oryn_dom::DocumentProvenance {
                capability: vec![1],
                origin: strict.serialize(),
                origin_key: strict.security_token_key(),
                inert: false,
            })
            .unwrap();
        let mut source = crate::origin::DomainAccessIdentity::new(crate::origin::Origin::from_url(
            "https://source.example.test:9443",
        ));
        source.set_domain("example.test").unwrap();
        let navigation = NativeNavigationSource {
            source_origin_key: Some(source.origin().security_token_key()),
            source_domain_key: Some(source.security_token_key()),
            ..Default::default()
        };
        assert!(!native_source_can_fire_cross_document(scope, &state, &navigation).unwrap());
        state
            .dom()
            .borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .relaxed_domain
            .set(Some("example.test".into()));
        assert!(
            native_source_can_fire_cross_document(scope, &state, &navigation).unwrap(),
            "bilateral relaxation permits access across hosts and ports"
        );
        let unrelaxed = NativeNavigationSource {
            source_origin_key: Some(strict.security_token_key()),
            ..Default::default()
        };
        assert!(
            !native_source_can_fire_cross_document(scope, &state, &unrelaxed).unwrap(),
            "matching strict origins do not bypass one-sided relaxation"
        );
        assert_eq!(
            state
                .document_provenance(state.document_root())
                .unwrap()
                .unwrap()
                .origin_key,
            strict.security_token_key()
        );
        let mut http = crate::origin::DomainAccessIdentity::new(crate::origin::Origin::from_url(
            "http://source.example.test",
        ));
        http.set_domain("example.test").unwrap();
        assert!(
            !native_source_can_fire_cross_document(
                scope,
                &state,
                &NativeNavigationSource {
                    source_domain_key: Some(http.security_token_key()),
                    ..Default::default()
                }
            )
            .unwrap()
        );
    }
    fn admission_fixture(scope: &mut v8::PinScope) -> Rc<V8BindingState> {
        let context = scope.get_current_context();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://target.test/start").unwrap();
        sync_history(
            scope,
            vec![NavigationHistorySnapshot {
                scroll: Default::default(),
                key: "key".into(),
                id: "id".into(),
                url: "https://target.test/start".into(),
                document_id: "document".into(),
                state: None,
                history_state: None,
            }],
            0,
            "push",
        )
        .unwrap();
        state
    }
    fn scroll_fixture(scope: &mut v8::PinScope) -> Rc<V8BindingState> {
        let context = scope.get_current_context();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/a").unwrap();
        sync_history(scope, vec![entry("a")], 0, "push").unwrap();
        state.set_layout(BindingLayout {
            viewport: Some((100.0, 100.0)),
            document_scroll_size: Some((2000.0, 2000.0)),
            ..Default::default()
        });
        state
    }
    fn opaque_history_fixture(scope: &mut v8::PinScope) -> Rc<V8BindingState> {
        let context = scope.get_current_context();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("about:blank").unwrap();
        state.set_security_origin("null".into());
        let mut initial = entry("opaque");
        initial.url = "about:blank".into();
        sync_history(scope, vec![initial], 0, "push").unwrap();
        let receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        state
    }
    #[test]
    fn opaque_fragment_history_keeps_navigation_private_and_retains_joint_callback_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = opaque_history_fixture(scope);
        assert!(eval(scope,"globalThis.pops=0;globalThis.hashes=0;globalThis.navigates=0;globalThis.initialLength=history.length;onpopstate=()=>pops++;onhashchange=()=>hashes++;navigation.onnavigate=()=>navigates++;location.hash='#one';location.hash='#two';pops===2&&hashes===0&&history.length===initialLength+2&&navigation.currentEntry===null&&navigation.entries().length===0&&navigates===0").is_true());
        assert!(!state.has_location_navigation());
        assert_eq!(poll_hash_tasks(scope, &state).unwrap(), 2);
        assert!(eval(scope, "hashes===2").is_true());
        eval(
            scope,
            "onpopstate=null;globalThis.trace=[];addEventListener('popstate',()=>trace.push('first'));addEventListener('popstate',()=>trace.push('tail'));onhashchange=()=>trace.push('hash')",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = run_joint_history_delta(scope, -2).unwrap().unwrap();
        assert!(receipt.poll().is_none());
        assert!(eval(scope,"trace.join()==='first'&&location.href==='about:blank'&&navigation.currentEntry===null&&navigates===0").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert_eq!(receipt.poll(), Some(Ok(())));
        assert!(eval(scope,"trace.join()==='first,tail,hash'&&history.length===initialLength+2&&navigation.entries().length===0").is_true());
        assert!(!state.has_location_navigation());
    }
    #[test]
    fn opaque_fragment_native_execution_waits_for_suffix_or_stream_retirement() {
        use std::sync::{Arc, Mutex};
        super::super::tests::initialize_v8();
        for retire in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = opaque_history_fixture(scope);
            eval(
                scope,
                "globalThis.trace=[];addEventListener('popstate',()=>trace.push('first'));addEventListener('popstate',()=>trace.push('tail'))",
            );
            let outcomes = Arc::new(Mutex::new(Vec::new()));
            let sink = outcomes.clone();
            let admissions = Arc::new(Mutex::new(0));
            let admitted = admissions.clone();
            let observer = crate::navigation::NavigationExecutionObserver::with_admission(
                move || *admitted.lock().unwrap() += 1,
                move |value| sink.lock().unwrap().push(value),
            );
            native_platform::publication::fail_after_next_callback();
            assert!(matches!(
                admit_native_navigation(
                    scope,
                    "about:blank#native",
                    false,
                    false,
                    NativeNavigationSource {
                        execution_observer: Some(observer),
                        ..Default::default()
                    }
                )
                .unwrap(),
                NativeNavigationAdmission::Handled
            ));
            assert_eq!(*admissions.lock().unwrap(), 1);
            assert!(outcomes.lock().unwrap().is_empty());
            assert!(eval(scope, "trace.join()==='first'").is_true());
            if retire {
                eval(scope, "document.open()");
            }
            state.navigation_api.borrow_mut().publication_retry_at = None;
            poll_publications(scope, &state).unwrap();
            assert_eq!(
                *outcomes.lock().unwrap(),
                vec![if retire {
                    crate::navigation::NavigationExecutionOutcome::Canceled
                } else {
                    crate::navigation::NavigationExecutionOutcome::Completed
                }]
            );
            assert!(
                eval(
                    scope,
                    if retire {
                        "trace.join()==='first'"
                    } else {
                        "trace.join()==='first,tail'"
                    }
                )
                .is_true()
            );
            poll_publications(scope, &state).unwrap();
            assert_eq!(outcomes.lock().unwrap().len(), 1);
            assert!(!state.has_location_navigation());
        }
    }
    #[test]
    fn joint_receipt_waits_for_retained_popstate_suffix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "history.pushState(null,'','#b');globalThis.calls=[];addEventListener('popstate',()=>calls.push('first'));addEventListener('popstate',()=>calls.push('second'))",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = run_joint_history_delta(scope, -1).unwrap().unwrap();
        assert!(receipt.poll().is_none());
        assert!(eval(scope, "calls.join()==='first'").is_true());
        for _ in 0..8 {
            state.navigation_api.borrow_mut().publication_retry_at = None;
            poll_publications(scope, &state).unwrap();
            if receipt.poll().is_some() {
                break;
            }
        }
        assert_eq!(receipt.poll(), Some(Ok(())));
        assert!(eval(scope, "calls.join()==='first,second'").is_true());
    }
    #[test]
    fn intercepted_traversal_popstate_is_a_retained_task_after_handlerless_finish() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "history.replaceState({value:7},'','#a');history.pushState(null,'','#b');globalThis.calls=[];navigation.onnavigate=e=>e.intercept();addEventListener('popstate',e=>calls.push('pop:'+e.state.value));addEventListener('popstate',()=>calls.push('suffix'))",
        );
        let receipt = run_joint_history_delta(scope, -1).unwrap().unwrap();
        scope.perform_microtask_checkpoint();
        poll_publications(scope, &state).unwrap();
        assert!(receipt.poll().is_none());
        assert!(eval(scope, "calls.length===0").is_true());
        native_platform::publication::fail_after_next_callback();
        let _ = poll_hash_tasks(scope, &state);
        assert!(receipt.poll().is_none());
        assert!(eval(scope, "calls.join()==='pop:7'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        poll_publications(scope, &state).unwrap();
        assert_eq!(receipt.poll(), Some(Ok(())));
        assert!(eval(scope, "calls.join()==='pop:7,suffix'").is_true());
    }
    #[test]
    fn intercepted_traversal_popstate_does_not_finish_a_pending_handler() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "history.pushState(null,'','#b');globalThis.calls=[];navigation.onnavigate=e=>e.intercept({handler:()=>new Promise(r=>globalThis.finishHandler=r)});onpopstate=()=>calls.push('pop')",
        );
        let receipt = run_joint_history_delta(scope, -1).unwrap().unwrap();
        scope.perform_microtask_checkpoint();
        poll_publications(scope, &state).unwrap();
        poll_hash_tasks(scope, &state).unwrap();
        assert!(eval(scope, "calls.join()==='pop'").is_true());
        assert!(receipt.poll().is_none());
        eval(scope, "finishHandler()");
        scope.perform_microtask_checkpoint();
        poll_publications(scope, &state).unwrap();
        assert_eq!(receipt.poll(), Some(Ok(())));
    }
    #[test]
    fn replacing_parser_stream_during_popstate_cancels_joint_publication() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "history.pushState(null,'','#b');addEventListener('popstate',()=>{document.open();document.write('<p>replacement</p>');document.close()})",
        );
        let receipt = run_joint_history_delta(scope, -1).unwrap().unwrap();
        assert!(matches!(
            receipt.poll(),
            Some(Err(crate::joint_history::JointTraversalFailure::Retired))
        ));
    }
    #[test]
    fn joint_back_restores_each_saved_position_between_popstate_and_hashchange() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "history.pushState(null,'','#1');scrollTo(50,100);history.pushState(null,'','#2');scrollTo(100,200);history.pushState(null,'','#done');scrollTo(555,555);globalThis.scrollEvents=[];onpopstate=()=>scrollEvents.push('pop:'+scrollX+':'+scrollY);onhashchange=()=>scrollEvents.push('hash:'+scrollX+':'+scrollY)",
        );
        for (before, after) in [
            ((555., 555.), (100., 200.)),
            ((100., 200.), (50., 100.)),
            ((50., 100.), (0., 0.)),
        ] {
            eval(scope, "scrollEvents=[]");
            let receipt = run_joint_history_delta(scope, -1).unwrap().unwrap();
            for _ in 0..8 {
                state.navigation_api.borrow_mut().publication_retry_at = None;
                poll_publications(scope, &state).unwrap();
                poll_hash_tasks(scope, &state).unwrap();
                if receipt.poll().is_some() {
                    break;
                }
            }
            assert_eq!(receipt.poll(), Some(Ok(())));
            assert_eq!(state.document_scroll_offset(), after);
            let expected = format!("pop:{}:{},hash:{}:{}", before.0, before.1, after.0, after.1);
            assert_eq!(
                eval(scope, "scrollEvents.join(',')").to_rust_string_lossy(scope),
                expected
            );
        }
    }
    #[test]
    fn manual_mode_allows_new_fragment_navigation_but_blocks_history_fragment_restore() {
        super::super::tests::initialize_v8();
        for manual in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = scroll_fixture(scope);
            eval(
                scope,
                if manual {
                    "history.scrollRestoration='manual';scrollTo(300,400);location.hash='#top'"
                } else {
                    "history.scrollRestoration='auto';scrollTo(300,400);location.hash='#top'"
                },
            );
            assert_eq!(
                state.document_scroll_offset(),
                (0., 0.),
                "new fragment navigation ignores restoration mode"
            );
            poll_hash_tasks(scope, &state).unwrap();
            eval(
                scope,
                "history.pushState(null,'','#done');scrollTo(300,400)",
            );
            let receipt = run_joint_history_delta(scope, -1).unwrap().unwrap();
            for _ in 0..8 {
                state.navigation_api.borrow_mut().publication_retry_at = None;
                poll_publications(scope, &state).unwrap();
                poll_hash_tasks(scope, &state).unwrap();
                if receipt.poll().is_some() {
                    break;
                }
            }
            assert_eq!(receipt.poll(), Some(Ok(())));
            assert_eq!(
                state.document_scroll_offset(),
                if manual { (300., 400.) } else { (0., 0.) }
            );
        }
    }
    #[test]
    fn fresh_fragment_scroll_precedes_load_ignores_manual_and_does_not_replay_after_fault() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "history.replaceState(null,'','#top');history.scrollRestoration='manual';scrollTo(100,200);globalThis.trace=[];onpopstate=()=>trace.push('unexpected-pop');addEventListener('load',()=>{trace.push('load:'+scrollY);scrollTo(30,40)},{once:true});addEventListener('load',()=>trace.push('tail:'+scrollY));onpageshow=()=>trace.push('show:'+scrollY)",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(eval(scope, "trace.join()==='load:0'").is_true());
        assert_eq!(state.document_scroll_offset(), (30., 40.));
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(
            eval(
                scope,
                "trace.join()==='load:0,tail:40,show:40'&&history.scrollRestoration==='manual'"
            )
            .is_true()
        );
    }
    #[test]
    fn reload_saved_scroll_precedes_load_without_popstate_or_fragment_override() {
        super::super::tests::initialize_v8();
        for fragment in ["ascii", "%E4%B8%AD%E6%96%87"] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = scroll_fixture(scope);
            eval(
                scope,
                &format!(
                    "history.replaceState(null,'','#{fragment}');scrollTo(0,0);globalThis.trace=[];onpopstate=()=>trace.push('unexpected-pop');onload=()=>trace.push('load:'+scrollY);onpageshow=()=>trace.push('show:'+scrollY)"
                ),
            );
            sync_activation(scope, None, "reload").unwrap();
            state.set_document_scroll_offset((100., 200.)).unwrap();
            let receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
            assert!(resume_native_history_publication(scope, receipt).unwrap());
            assert_eq!(state.document_scroll_offset(), (0., 0.));
            assert!(eval(scope, "trace.join()==='load:0,show:0'").is_true());
        }
    }
    #[test]
    fn scroll_restoration_mode_is_entry_local_and_inherited_by_push_and_replace() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        assert!(eval(scope,"history.scrollRestoration='manual';history.pushState(null,'','#b');history.scrollRestoration==='manual'").is_true());
        eval(
            scope,
            "history.scrollRestoration='auto';history.replaceState(null,'','#c')",
        );
        let entries = state.navigation_history_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].scroll.mode, HistoryScrollMode::Manual);
        assert_eq!(entries[1].scroll.mode, HistoryScrollMode::Auto);
        sync_history(scope, entries, 0, "traverse").unwrap();
        assert!(eval(scope, "history.scrollRestoration==='manual'").is_true());
    }
    #[test]
    fn retained_popstate_restores_saved_scroll_before_hashchange_without_replay() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        state.set_document_scroll_offset((40.0, 80.0)).unwrap();
        eval(
            scope,
            "globalThis.calls=[];onpopstate=()=>{calls.push('pop');scrollTo(300,400)};onhashchange=()=>calls.push('hash:'+scrollX+':'+scrollY)",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::History {
                previous: "https://example.test/#old".into(),
                current: "https://example.test/#new".into(),
                traversed: true,
            },
        )
        .unwrap();
        assert_eq!(state.document_scroll_offset(), (300.0, 400.0));
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert_eq!(state.document_scroll_offset(), (40.0, 80.0));
        assert!(eval(scope, "calls.join(',')==='pop,hash:40:80'").is_true());
    }
    #[test]
    fn popstate_can_disable_restore_and_replacement_cannot_receive_old_scroll() {
        super::super::tests::initialize_v8();
        for replace in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = scroll_fixture(scope);
            state.set_document_scroll_offset((40.0, 80.0)).unwrap();
            let entry = state.navigation_history_snapshot().unwrap();
            eval(
                scope,
                if replace {
                    "onpopstate=()=>{document.open();document.write('<p>replacement</p>');globalThis.replaced=true}"
                } else {
                    "onpopstate=()=>{history.scrollRestoration='manual';scrollTo(300,400)}"
                },
            );
            let receipt = start_native_event_publication(
                scope,
                NativePublicationEvent::HistoryRestore { entry },
            )
            .unwrap();
            let _ = resume_native_history_publication(scope, receipt);
            if replace {
                assert!(eval(scope, "replaced").is_true());
                assert_ne!(state.document_scroll_offset(), (40.0, 80.0));
            } else {
                assert_eq!(state.document_scroll_offset(), (300.0, 400.0));
            }
        }
    }
    #[test]
    fn newly_loaded_history_document_restores_after_pageshow_with_retained_callback_prefix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        state.set_document_scroll_offset((40.0, 80.0)).unwrap();
        sync_activation(scope, None, "traverse").unwrap();
        state.set_document_scroll_offset((0.0, 0.0)).unwrap();
        eval(
            scope,
            "globalThis.restore=[];onload=()=>restore.push('load');onpageshow=()=>{restore.push('show');scrollTo(300,400)};onpopstate=()=>restore.push('pop:'+scrollX+':'+scrollY)",
        );
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(state.navigation_api.borrow().load_restore.is_none());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(eval(scope, "restore.join(',')==='load,show,pop:300:400'").is_true());
        assert_eq!(state.document_scroll_offset(), (40.0, 80.0));
    }
    #[test]
    fn unknown_saved_scroll_uses_fragment_fallback_only_for_auto_mode() {
        super::super::tests::initialize_v8();
        for (mode, fragment, expected) in [
            (HistoryScrollMode::Auto, "top", (0.0, 0.0)),
            (HistoryScrollMode::Manual, "top", (300.0, 400.0)),
            (HistoryScrollMode::Auto, "missing", (300.0, 400.0)),
        ] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = scroll_fixture(scope);
            state.set_document_scroll_offset((300.0, 400.0)).unwrap();
            let mut entry = state.navigation_history_snapshot().unwrap();
            entry.url = format!("https://example.test/a#{fragment}");
            entry.scroll = HistoryScrollState {
                mode,
                position: None,
            };
            sync_history(scope, vec![entry.clone()], 0, "traverse").unwrap();
            let receipt = start_native_event_publication(
                scope,
                NativePublicationEvent::HistoryRestore { entry },
            )
            .unwrap();
            assert!(resume_native_history_publication(scope, receipt).unwrap());
            assert_eq!(state.document_scroll_offset(), expected);
        }
    }
    #[test]
    fn first_native_checkpoint_captures_real_zero_scroll_without_changing_wire_defaults() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        assert_eq!(
            state.navigation_history_snapshot().unwrap().scroll.position,
            None
        );
        let update = state.take_navigation_history_update().unwrap();
        assert_eq!(
            update.entries[update.current].scroll.position,
            Some((0.0, 0.0))
        );
        assert!(state.take_navigation_history_update().is_none());
    }
    #[test]
    fn saved_scroll_retries_on_layout_growth_without_replaying_history_events() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        state.set_document_scroll_offset((600.0, 800.0)).unwrap();
        let entry = state.navigation_history_snapshot().unwrap();
        state.set_document_scroll_offset((0.0, 0.0)).unwrap();
        state.set_layout(BindingLayout {
            viewport: Some((100.0, 100.0)),
            document_scroll_size: Some((200.0, 200.0)),
            ..Default::default()
        });
        eval(scope, "globalThis.pops=0;onpopstate=()=>pops++");
        let receipt =
            start_native_event_publication(scope, NativePublicationEvent::HistoryRestore { entry })
                .unwrap();
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert_eq!(state.document_scroll_offset(), (100.0, 100.0));
        assert!(state.navigation_api.borrow().growing_scroll.is_some());
        let deadline = state
            .navigation_api
            .borrow()
            .growing_scroll
            .as_ref()
            .unwrap()
            .1;
        state.set_layout(BindingLayout {
            viewport: Some((100.0, 100.0)),
            document_scroll_size: Some((400.0, 500.0)),
            ..Default::default()
        });
        retry_scroll_restoration(&state).unwrap();
        assert_eq!(state.document_scroll_offset(), (300.0, 400.0));
        assert_eq!(
            state
                .navigation_api
                .borrow()
                .growing_scroll
                .as_ref()
                .unwrap()
                .1,
            deadline
        );
        state.set_layout(BindingLayout {
            viewport: Some((100.0, 100.0)),
            document_scroll_size: Some((2000.0, 2000.0)),
            ..Default::default()
        });
        retry_scroll_restoration(&state).unwrap();
        assert_eq!(state.document_scroll_offset(), (600.0, 800.0));
        assert!(state.navigation_api.borrow().growing_scroll.is_none());
        assert!(eval(scope, "pops===1").is_true());
    }
    #[test]
    fn author_scroll_and_manual_mode_cancel_pending_layout_growth_restore() {
        super::super::tests::initialize_v8();
        for manual in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = scroll_fixture(scope);
            state.set_document_scroll_offset((600.0, 800.0)).unwrap();
            let entry = state.navigation_history_snapshot().unwrap();
            state.set_document_scroll_offset((0.0, 0.0)).unwrap();
            state.set_layout(BindingLayout {
                viewport: Some((100.0, 100.0)),
                document_scroll_size: Some((200.0, 200.0)),
                ..Default::default()
            });
            let receipt = start_native_event_publication(
                scope,
                NativePublicationEvent::HistoryRestore { entry },
            )
            .unwrap();
            assert!(resume_native_history_publication(scope, receipt).unwrap());
            assert!(state.navigation_api.borrow().growing_scroll.is_some());
            eval(
                scope,
                if manual {
                    "history.scrollRestoration='manual'"
                } else {
                    "scrollTo(50,60)"
                },
            );
            assert!(state.navigation_api.borrow().growing_scroll.is_none());
            let position = state.document_scroll_offset();
            state.set_layout(BindingLayout {
                viewport: Some((100.0, 100.0)),
                document_scroll_size: Some((2000.0, 2000.0)),
                ..Default::default()
            });
            retry_scroll_restoration(&state).unwrap();
            assert_eq!(state.document_scroll_offset(), position);
        }
    }
    #[test]
    fn fragment_navigation_dispatches_popstate_inline_and_hashchange_as_fifo_tasks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        assert!(eval(scope,"globalThis.order=[];onpopstate=()=>order.push('pop:'+location.hash);onhashchange=e=>order.push('hash:'+new URL(e.newURL).hash);location.hash='#one';location.hash='#two';order.join(',')==='pop:#one,pop:#two'").is_true());
        assert_eq!(poll_hash_tasks(scope, &state).unwrap(), 2);
        assert!(
            eval(
                scope,
                "order.join(',')==='pop:#one,pop:#two,hash:#one,hash:#two'"
            )
            .is_true()
        );
    }
    #[test]
    fn pending_navigation_finish_does_not_block_native_load_publication() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        assert!(eval(scope,"globalThis.order=[];onpopstate=()=>order.push('pop');onload=()=>order.push('load');globalThis.result=navigation.navigate('#pending');onpageshow=()=>order.push('show');result.finished.then(()=>order.push('finished'));result.finished instanceof Promise").is_true());
        assert_eq!(state.navigation_api.borrow().queued_finishes.len(), 1);
        assert!(state.navigation_api.borrow().publications.is_empty());
        let receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(eval(scope, "order.join(',')==='load,show'").is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "order.join(',')==='load,show,finished'").is_true());
        assert!(state.navigation_api.borrow().queued_finishes.is_empty());
        assert!(state.navigation_api.borrow().native_receipts.is_empty());
    }
    #[test]
    fn execution_retirement_retires_queued_navigation_finish() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        eval(
            scope,
            "globalThis.success=0;navigation.onnavigatesuccess=()=>success++;navigation.navigate('#pending').finished.catch(()=>{});",
        );
        assert_eq!(state.navigation_api.borrow().queued_finishes.len(), 1);
        state.retire_execution();
        assert!(state.navigation_api.borrow().queued_finishes.is_empty());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "success===0").is_true());
    }
    #[test]
    fn fragment_popstate_can_push_state_and_queued_hash_is_stream_owned() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = scroll_fixture(scope);
        assert!(eval(scope,"globalThis.popped=false;globalThis.hashes=0;onpopstate=e=>{popped=true;history.pushState(2,'','#two');if(history.state===e.state)throw Error('state')};onhashchange=()=>hashes++;location.hash='#one';popped&&location.hash==='#two'&&hashes===0").is_true());
        eval(scope, "document.open()");
        assert_eq!(poll_hash_tasks(scope, &state).unwrap(), 0);
        assert!(eval(scope, "hashes===0").is_true());
    }
    #[test]
    fn history_scroll_wire_defaults_do_not_invent_a_saved_position() {
        let mut wire = serde_json::to_value(entry("a")).unwrap();
        wire.as_object_mut().unwrap().remove("scroll");
        let decoded: NavigationHistorySnapshot = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded.scroll.mode, HistoryScrollMode::Auto);
        assert_eq!(decoded.scroll.position, None);
        assert!(
            !HistoryScrollState {
                mode: HistoryScrollMode::Auto,
                position: Some((f64::INFINITY, 0.0))
            }
            .valid()
        );
    }
    #[test]
    fn history_traversal_tasks_are_fifo_and_callbacks_follow_script_completion() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        let entries = vec![entry("a"), entry("b"), entry("c")];
        sync_history(scope, entries.clone(), 2, "push").unwrap();
        sync_session_history(
            scope,
            NavigationHistoryUpdate {
                entries: entries.clone(),
                current: 2,
                navigation_type: "push".into(),
                truncates_forward: false,
            },
        )
        .unwrap();
        state.commit_location(&entries[2].url).unwrap();
        assert!(eval(scope,"globalThis.traversals=[];onpopstate=()=>traversals.push(location.pathname);history.back();history.back();location.pathname==='/c'&&traversals.length===0").is_true());
        let first = state.take_history_traversal().unwrap();
        assert_eq!(first.delta(), -1);
        assert!(state.take_history_traversal().is_none());
        assert!(!state.has_history_traversal());
        drop(first);
        assert!(state.has_history_traversal());
        let first = state.take_history_traversal().unwrap();
        run_history_task(scope, first, Some(entries[1].clone())).unwrap();
        assert_eq!(state.current_location_url(), entries[1].url);
        let second = state.take_history_traversal().unwrap();
        run_history_task(scope, second, Some(entries[0].clone())).unwrap();
        assert_eq!(state.current_location_url(), entries[0].url);
        assert!(state.take_history_traversal().is_none());
        assert!(eval(scope, "traversals.join(',')==='/b,/a'").is_true());
    }
    #[test]
    fn history_task_claim_release_suspension_and_document_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        eval(scope, "history.go(1);history.go(-2)");
        let mut old = state.take_history_traversal().unwrap();
        assert_eq!(old.delta(), 1);
        state.set_execution_suspended(true);
        assert!(old.is_current());
        assert!(!old.is_runnable());
        assert!(state.take_history_traversal().is_none());
        state.set_execution_suspended(false);
        assert!(old.is_current());
        eval(scope, "document.open();history.go(7)");
        assert!(!old.is_current());
        old.acknowledge();
        let mut fresh = state.take_history_traversal().unwrap();
        assert_eq!(fresh.delta(), 7);
        fresh.acknowledge();
        assert!(state.take_history_traversal().is_none());
    }
    #[test]
    fn location_history_policy_uses_complete_load_and_unconsumed_native_activation() {
        struct Messages;
        impl crate::message_wire::MessageTransport for Messages {
            fn request(
                &self,
                _: crate::message_wire::MessageRequest,
            ) -> Result<crate::message_wire::MessageResponse, String> {
                Ok(crate::message_wire::MessageResponse::Done)
            }
        }
        struct Activation(bool);
        impl crate::user_activation::ActivationBackend for Activation {
            fn notify(&self, _: crate::worker_wire::Creator) -> Result<(), String> {
                Ok(())
            }
            fn transient(
                &self,
                _: crate::worker_wire::Creator,
                consume: bool,
            ) -> Result<bool, String> {
                assert!(
                    !consume,
                    "Location history choice must not consume activation"
                );
                Ok(self.0)
            }
        }
        super::super::tests::initialize_v8();
        for (loaded, activated, explicit_replace, expected) in [
            (false, false, false, true),
            (true, false, false, false),
            (false, true, false, false),
            (true, true, true, true),
        ] {
            let mut isolate = v8::Isolate::new(Default::default());
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = admission_fixture(scope);
            state.document_completely_loaded.set(loaded);
            state.set_message_transport(std::sync::Arc::new(Messages));
            state.set_activation_backend(std::sync::Arc::new(Activation(activated)));
            eval(
                scope,
                "globalThis.kind='';navigation.onnavigate=e=>{kind=e.navigationType}",
            );
            eval(
                scope,
                if explicit_replace {
                    "location.replace('https://target.test/next')"
                } else {
                    "location.assign('https://target.test/next')"
                },
            );
            assert_eq!(
                state
                    .pending_location_navigation
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .replace,
                expected
            );
            assert!(
                eval(
                    scope,
                    if expected {
                        "kind==='replace'"
                    } else {
                        "kind==='push'"
                    }
                )
                .is_true()
            );
        }
    }
    #[test]
    fn hidden_history_network_failure_retains_native_destination_after_queueing() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        let destination = NavigationHistorySnapshot {
            scroll: Default::default(),
            key: "foreign-key".into(),
            id: "foreign-id".into(),
            url: "https://foreign.test/failed".into(),
            document_id: "foreign-document".into(),
            state: None,
            history_state: None,
        };
        let timeline = state.joint_history.borrow().clone();
        let plan = {
            let timeline = timeline.borrow();
            crate::joint_history::Traversal {
                revision: timeline.revision(),
                from: timeline.cursor(),
                to: timeline.cursor(),
                destinations: vec![(state.joint_route.borrow().clone(), destination.clone())],
            }
        };
        let receipt = timeline.borrow_mut().begin_execution(&plan).unwrap();
        let NativeNavigationAdmission::Proceed {
            operation: Some(admitted),
            ..
        } = admit_native_history_traversal(scope, destination.clone()).unwrap()
        else {
            panic!("hidden history admission")
        };
        assert_eq!(
            admitted.destination.as_deref(),
            Some(destination.id.as_str())
        );
        assert!(state.pending_navigation_destination().is_none());
        queue_cross_document_traversal(scope, destination.clone()).unwrap();
        let queued = state
            .pending_location_navigation
            .borrow_mut()
            .take()
            .unwrap();
        let token = queued.navigation_operation.unwrap();
        assert_eq!(token.destination, admitted.destination);
        assert!(fail_native_navigation(scope, &token, NativeNavigationFailure::Network).unwrap());
        assert!(matches!(
            receipt.poll(),
            Some(Err(crate::joint_history::JointTraversalFailure::Failed(_)))
        ));
        assert!(!native_navigation_matches(&state, &token));
        assert!(state.pending_navigation_destination().is_none());
        assert!(
            !state
                .navigation_history_entries()
                .iter()
                .any(|entry| entry.id == destination.id)
        );
    }

    #[test]
    fn hidden_no_commit_uses_selected_native_destination_and_rejects_stale_token() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        let destination = NavigationHistorySnapshot {
            scroll: Default::default(),
            key: "hidden-key".into(),
            id: "hidden-id".into(),
            url: "https://other.test/empty".into(),
            document_id: "hidden-document".into(),
            state: None,
            history_state: None,
        };
        let timeline = state.joint_history.borrow().clone();
        let plan = {
            let timeline = timeline.borrow();
            crate::joint_history::Traversal {
                revision: timeline.revision(),
                from: timeline.cursor(),
                to: timeline.cursor(),
                destinations: vec![(state.joint_route.borrow().clone(), destination.clone())],
            }
        };
        let receipt = timeline.borrow_mut().begin_execution(&plan).unwrap();
        assert!(state.pending_navigation_destination().is_none());
        let stale = capture_native_revision(&state).unwrap();
        advance_native_revision(&state).unwrap();
        assert!(
            !settle_joint_navigation_without_commit(scope, &stale, Some(&destination.id)).unwrap()
        );
        assert_eq!(receipt.poll(), None);
        let current = capture_native_revision(&state).unwrap();
        assert!(
            settle_joint_navigation_without_commit(scope, &current, Some(&destination.id)).unwrap()
        );
        assert_eq!(
            receipt.poll(),
            Some(Err(crate::joint_history::JointTraversalFailure::Canceled))
        );
        assert!(native_navigation_matches(&state, &current));
        assert!(state.pending_navigation_destination().is_none());
    }
    #[test]
    fn location_during_load_replaces_but_timer_after_load_pushes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        eval(
            scope,
            "addEventListener('load',()=>{location.href='https://target.test/in-load'},{once:true})",
        );
        let receipt = start_native_document_lifecycle_publication(scope, false).unwrap();
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(state.document_completely_loaded());
        assert!(
            state
                .pending_location_navigation
                .borrow()
                .as_ref()
                .unwrap()
                .replace
        );
        // A later task sees the completed load; no replacement is inherited
        // from the pending earlier request or from readyState alone.
        eval(scope, "location.href='https://target.test/after-load'");
        assert!(
            !state
                .pending_location_navigation
                .borrow()
                .as_ref()
                .unwrap()
                .replace
        );
    }
    #[test]
    fn location_admission_cannot_publish_into_document_open_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        let before = AdmissionDocument::capture(&state);
        eval(
            scope,
            "globalThis.calls=0;navigation.onnavigate=e=>{calls++;document.open()};location.href='https://target.test/stale'",
        );
        assert!(!before.current(&state));
        assert!(state.pending_location_navigation.borrow().is_none());
        assert_eq!(state.current_location_url(), "https://target.test/start");
        assert!(eval(scope, "calls===1").is_true());
    }
    #[test]
    fn abort_callback_replacement_preserves_new_document_navigation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        eval(
            scope,
            "globalThis.aborts=0;navigation.onnavigate=e=>e.signal.addEventListener('abort',()=>{aborts++;document.open();location.href='https://target.test/fresh'},{once:true})",
        );
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://target.test/first",
                false,
                false,
                NativeNavigationSource::default()
            )
            .unwrap(),
            NativeNavigationAdmission::Proceed { .. }
        ));
        let before = AdmissionDocument::capture(&state);
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://target.test/stale",
                false,
                false,
                NativeNavigationSource::default()
            )
            .unwrap(),
            NativeNavigationAdmission::Handled
        ));
        assert!(!before.current(&state));
        assert!(eval(scope, "aborts===1").is_true());
        assert_eq!(
            state
                .pending_location_navigation
                .borrow()
                .as_ref()
                .unwrap()
                .url,
            "https://target.test/fresh"
        );
        assert_eq!(state.current_location_url(), "https://target.test/fresh");
    }
    #[test]
    fn history_mutation_stops_after_navigate_replaces_document() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = admission_fixture(scope);
        let before = AdmissionDocument::capture(&state);
        eval(scope, "navigation.onnavigate=()=>document.open()");
        assert!(
            !history_mutated(scope, &state, "https://target.test/stale", "push", None).unwrap()
        );
        assert!(!before.current(&state));
        assert_eq!(state.current_location_url(), "https://target.test/start");
        assert!(state.take_navigation_history_update().is_none_or(|update| {
            !update
                .entries
                .iter()
                .any(|entry| entry.url == "https://target.test/stale")
        }));
    }
    #[test]
    fn cross_document_source_gate_preserves_fragment_exception_and_fetch_schemes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://target.test/start").unwrap();
        state
            .install_document_provenance(oryn_dom::DocumentProvenance {
                capability: vec![1],
                origin: "https://target.test".into(),
                origin_key: "target-origin".into(),
                inert: false,
            })
            .unwrap();
        sync_history(
            scope,
            vec![NavigationHistorySnapshot {
                scroll: Default::default(),
                key: "key".into(),
                id: "id".into(),
                url: "https://target.test/start".into(),
                document_id: "document".into(),
                state: None,
                history_state: None,
            }],
            0,
            "push",
        )
        .unwrap();
        let source=v8::String::new(scope,"globalThis.events=[];navigation.onnavigate=e=>{events.push(e.sourceElement===null);e.preventDefault()}").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let foreign = NativeNavigationSource {
            source_origin_key: Some("foreign-origin".into()),
            ..Default::default()
        };
        let admission = admit_native_navigation(
            scope,
            "https://target.test/next",
            false,
            false,
            foreign.clone(),
        )
        .unwrap();
        let NativeNavigationAdmission::Proceed {
            operation: Some(operation),
            destination: None,
        } = admission
        else {
            panic!("cross-origin load must proceed without an exposed navigation event")
        };
        assert!(native_navigation_matches(&state, &operation));
        assert!(state.navigation_api.borrow().pending.is_none());
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://target.test/start#fragment",
                false,
                false,
                foreign
            )
            .unwrap(),
            NativeNavigationAdmission::Handled
        ));
        assert!(
            !native_navigation_matches(&state, &operation),
            "later fragment admission supersedes the previous hidden operation"
        );
        let same = NativeNavigationSource {
            source_origin_key: Some("target-origin".into()),
            ..Default::default()
        };
        assert!(matches!(
            admit_native_navigation(
                scope,
                "https://target.test/next",
                false,
                false,
                same.clone()
            )
            .unwrap(),
            NativeNavigationAdmission::Handled
        ));
        assert!(matches!(
            admit_native_navigation(scope, "mailto:recipient@example.test", false, false, same)
                .unwrap(),
            NativeNavigationAdmission::Proceed {
                operation: Some(_),
                destination: None
            }
        ));
        let source = v8::String::new(scope, "events.length===2&&events.every(Boolean)").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod suspended_unload_publication_regressions {
    use super::*;

    fn fixture(scope: &mut v8::PinScope) -> Rc<V8BindingState> {
        let context = scope.get_current_context();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let callback = v8::Function::new(scope, park).unwrap();
        let key = v8::String::new(scope, "parkNative").unwrap();
        assert_eq!(
            context
                .global(scope)
                .create_data_property(scope, key.into(), callback.into()),
            Some(true)
        );
        super::test_eval::eval(
            scope,
            "globalThis.trace=[];addEventListener('unload',e=>{globalThis.original=e;trace.push('first');parkNative()},{once:true});addEventListener('unload',e=>{if(e!==original||!e.isTrusted||e.cancelable)throw Error('unload identity');trace.push('tail')})",
        );
        state
    }
    fn park(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
        current_state(scope).unwrap().set_execution_suspended(true);
    }

    #[test]
    fn suspended_unload_resumes_immediately_with_same_event_and_no_replay() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        let timing = crate::navigation_timing::UnloadTiming::default();
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Unload {
                timing: timing.clone(),
            },
        )
        .unwrap();
        assert!(state.execution_suspended());
        assert!(state.is_unloading());
        assert!(timing.interval().is_none());
        assert!(state.navigation_api.borrow().publication_retry_at.is_none());
        assert!(super::test_eval::eval(scope, "trace.join()==='first'").is_true());
        state.set_execution_suspended(false);
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(!state.is_unloading());
        let interval = timing.interval().unwrap();
        assert!(interval.0 <= interval.1);
        assert!(super::test_eval::eval(scope, "trace.join()==='first,tail'").is_true());
        assert!(resume_native_history_publication(scope, receipt).is_err());
        assert_eq!(timing.interval(), Some(interval));
        assert!(super::test_eval::eval(scope, "trace.join()==='first,tail'").is_true());
    }

    #[test]
    fn failed_callback_keeps_backoff_even_when_owner_is_resumed() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        native_platform::publication::fail_after_next_callback();
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Unload {
                timing: Default::default(),
            },
        )
        .unwrap();
        assert!(
            state.navigation_api.borrow().publication_retry_at.is_some(),
            "an actual injected callback failure still creates backoff"
        );
        // Extend the real failure's deadline to make the no-early-retry assertion
        // deterministic even when this test process is descheduled.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        state.navigation_api.borrow_mut().publication_retry_at = Some(deadline);
        state.set_execution_suspended(false);
        assert!(!resume_native_history_publication(scope, receipt).unwrap());
        assert_eq!(
            state.navigation_api.borrow().publication_retry_at,
            Some(deadline)
        );
        assert!(state.is_unloading());
        assert!(super::test_eval::eval(scope, "trace.join()==='first'").is_true());
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(!state.is_unloading());
        assert!(super::test_eval::eval(scope, "trace.join()==='first,tail'").is_true());
    }

    #[test]
    fn suspension_preserves_failed_nested_publication_deadline_and_receipts() {
        #[derive(Clone, Copy)]
        struct Nested {
            deadline: std::time::Instant,
            receipt: u64,
        }
        fn nested(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            let state = current_state(scope).unwrap();
            native_platform::publication::fail_after_next_callback();
            let receipt = start_native_event_publication(
                scope,
                NativePublicationEvent::Print {
                    kind: "beforeprint".into(),
                },
            )
            .unwrap();
            assert!(state.navigation_api.borrow().publication_retry_at.is_some());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            state.navigation_api.borrow_mut().publication_retry_at = Some(deadline);
            scope.set_slot(Nested { deadline, receipt });
            state.set_execution_suspended(true);
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        let callback = v8::Function::new(scope, nested).unwrap();
        let key = v8::String::new(scope, "parkNative").unwrap();
        assert_eq!(
            context
                .global(scope)
                .create_data_property(scope, key.into(), callback.into()),
            Some(true)
        );
        super::test_eval::eval(
            scope,
            "addEventListener('beforeprint',e=>{globalThis.originalPrint=e;trace.push('child-first')},{once:true});addEventListener('beforeprint',e=>{if(e!==originalPrint||!e.isTrusted)throw Error('nested identity');trace.push('child-tail')})",
        );
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Unload {
                timing: Default::default(),
            },
        )
        .unwrap();
        let saved = *scope.get_slot::<Nested>().unwrap();
        assert_eq!(
            state.navigation_api.borrow().publication_retry_at,
            Some(saved.deadline)
        );
        assert!(state.is_unloading());
        assert!(super::test_eval::eval(scope, "trace.join()==='first,child-first'").is_true());
        state.set_execution_suspended(false);
        assert!(!resume_native_history_publication(scope, receipt).unwrap());
        assert!(!resume_native_history_publication(scope, saved.receipt).unwrap());
        assert_eq!(
            state.navigation_api.borrow().publication_retry_at,
            Some(saved.deadline)
        );
        state.navigation_api.borrow_mut().publication_retry_at = None;
        assert!(resume_native_history_publication(scope, receipt).unwrap());
        assert!(resume_native_history_publication(scope, saved.receipt).unwrap());
        assert!(!state.is_unloading());
        assert!(super::test_eval::eval(scope, "trace.length===4&&['first','tail','child-first','child-tail'].every(label=>trace.filter(x=>x===label).length===1)").is_true());
    }

    #[test]
    fn retiring_suspended_unload_cancels_suffix_and_releases_guard() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = fixture(scope);
        let receipt = start_native_event_publication(
            scope,
            NativePublicationEvent::Unload {
                timing: Default::default(),
            },
        )
        .unwrap();
        assert!(state.is_unloading());
        state.retire_execution();
        assert!(state.execution_retired());
        assert!(!state.is_unloading());
        assert!(resume_native_history_publication(scope, receipt).is_err());
        assert!(super::test_eval::eval(scope, "trace.join()==='first'").is_true());
    }
}
