//! Bounded observation only. No lock waits, cancellation, scheduling or authority.
use super::Identity;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const ROOT_LIMIT: usize = 256;
const TRANSITION_LIMIT: u64 = 64;
type Key = (u64, u64);

#[derive(Clone, serde::Serialize)]
struct Frame {
    scope_id: u64,
    parent_scope_id: Option<u64>,
    phase: &'static str,
    start_monotonic_ns: Option<u128>,
    // A retained navigation owner is explanatory, never Close authority.
    retained_navigation_owner: Option<Identity>,
}
#[derive(Default)]
struct State {
    next_scope: u64,
    current: Option<Frame>,
    close: Option<Identity>,
    transitions: u64,
}
struct Tracker {
    profile: crate::native_timing::TurnCoalescer,
    key: Key,
    root_id: u64,
    state: Mutex<State>,
    incoherent: AtomicBool,
    missed_updates: AtomicU64,
    retired: AtomicBool,
}
impl Tracker {
    fn snapshot(&self) -> serde_json::Value {
        match self.state.try_lock() {
            Ok(state) => serde_json::json!({
                "available": !self.incoherent.load(Ordering::Relaxed) && !self.retired.load(Ordering::Relaxed),
                "root_id": self.root_id, "page_id": self.key.0, "route_generation": self.key.1,
                "frame": state.current, "close_request": state.close,
                "transition_count": state.transitions,
                "missed_updates": self.missed_updates.load(Ordering::Relaxed),
                "retired": self.retired.load(Ordering::Relaxed),
                "incoherent": self.incoherent.load(Ordering::Relaxed),
                "phase_clock": "clock_monotonic"
            }),
            Err(_) => {
                serde_json::json!({"available":false,"root_id":self.root_id,"reason":"snapshot_lock_unavailable"})
            }
        }
    }
    fn missed(&self) {
        self.incoherent.store(true, Ordering::Relaxed);
        self.missed_updates.fetch_add(1, Ordering::Relaxed);
    }
    fn activate(&self, identity: Identity) {
        let Ok(mut state) = self.state.try_lock() else {
            self.missed();
            return;
        };
        // Keep the original observation identity through cancellation/retry.
        if state.close.is_none() {
            state.close = Some(identity);
        }
    }
    fn notification(&self, state: &mut State, boundary: &'static str) -> Option<serde_json::Value> {
        let close = state.close?;
        let sequence = state.transitions;
        state.transitions = state.transitions.saturating_add(1);
        if sequence > TRANSITION_LIMIT {
            return None;
        }
        let capped = sequence == TRANSITION_LIMIT;
        Some(serde_json::json!({
            "close_request": close, "root_id":self.root_id,
            "page_id":self.key.0,"route_generation":self.key.1,
            "boundary":if capped { "transition_cap" } else { boundary },
            "sequence":sequence,"transition_limit":TRANSITION_LIMIT,
            "frame":if capped { None } else { state.current.clone() },
            "incoherent":self.incoherent.load(Ordering::Relaxed),
            "retired":self.retired.load(Ordering::Relaxed),
            "missed_updates":self.missed_updates.load(Ordering::Relaxed),
            "completion_inferred":false
        }))
    }
    fn enter(self: &Arc<Self>, phase: &'static str, owner: Option<Identity>) -> PhaseGuard {
        let Ok(mut state) = self.state.try_lock() else {
            self.missed();
            return PhaseGuard::empty();
        };
        let Some(id) = state.next_scope.checked_add(1) else {
            self.missed();
            return PhaseGuard::empty();
        };
        state.next_scope = id;
        let previous = state.current.clone();
        let owner = owner.or_else(|| {
            previous
                .as_ref()
                .and_then(|frame| frame.retained_navigation_owner)
        });
        state.current = Some(Frame {
            scope_id: id,
            parent_scope_id: previous.as_ref().map(|frame| frame.scope_id),
            phase,
            start_monotonic_ns: monotonic(),
            retained_navigation_owner: owner,
        });
        let event = self.notification(&mut state, "enter");
        drop(state);
        emit_transition(event);
        PhaseGuard {
            tracker: Some(self.clone()),
            scope_id: id,
            previous,
            _actor_thread: std::marker::PhantomData,
        }
    }
}
fn monotonic() -> Option<u128> {
    #[cfg(unix)]
    {
        let mut stamp = std::mem::MaybeUninit::<libc::timespec>::zeroed();
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, stamp.as_mut_ptr()) } == 0 {
            let stamp = unsafe { stamp.assume_init() };
            if stamp.tv_sec >= 0 && (0..1_000_000_000).contains(&stamp.tv_nsec) {
                return Some(stamp.tv_sec as u128 * 1_000_000_000 + stamp.tv_nsec as u128);
            }
        }
    }
    None
}
fn emit_transition(event: Option<serde_json::Value>) {
    if let Some(event) = event {
        crate::native_timing::point("close_actor_phase_transition", event);
    }
}
fn registry() -> &'static Mutex<BTreeMap<Key, Weak<Tracker>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<Key, Weak<Tracker>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
pub(crate) struct Registration(Arc<Tracker>);
impl Drop for Registration {
    fn drop(&mut self) {
        self.0.retired.store(true, Ordering::Relaxed);
        self.0.profile.retire();
        if let Ok(mut registry) = registry().try_lock()
            && registry
                .get(&self.0.key)
                .is_some_and(|weak| weak.ptr_eq(&Arc::downgrade(&self.0)))
        {
            registry.remove(&self.0.key);
        }
    }
}
pub(crate) fn register(page: u64, generation: u64) -> Option<Registration> {
    register_enabled(page, generation, crate::native_timing::enabled())
}
fn register_enabled(page: u64, generation: u64, enabled: bool) -> Option<Registration> {
    if !enabled {
        return None;
    }
    let mut registry = registry().try_lock().ok()?;
    registry.retain(|_, weak| {
        weak.upgrade()
            .is_some_and(|value| !value.retired.load(Ordering::Relaxed))
    });
    let key = (page, generation);
    if registry.len() >= ROOT_LIMIT || registry.contains_key(&key) {
        return None;
    }
    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ROOT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .ok()?;
    let tracker = Arc::new(Tracker {
        profile: Default::default(),
        key,
        root_id: id,
        state: Default::default(),
        incoherent: AtomicBool::new(false),
        missed_updates: AtomicU64::new(0),
        retired: AtomicBool::new(false),
    });
    registry.insert(key, Arc::downgrade(&tracker));
    Some(Registration(tracker))
}
thread_local! {
    static CURRENT: RefCell<Option<Arc<Tracker>>> = const { RefCell::new(None) };
}
pub(crate) struct PhaseGuard {
    tracker: Option<Arc<Tracker>>,
    scope_id: u64,
    previous: Option<Frame>,
    _actor_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl PhaseGuard {
    fn empty() -> Self {
        Self {
            tracker: None,
            scope_id: 0,
            previous: None,
            _actor_thread: std::marker::PhantomData,
        }
    }
}
impl Drop for PhaseGuard {
    fn drop(&mut self) {
        let Some(tracker) = &self.tracker else {
            return;
        };
        let Ok(mut state) = tracker.state.try_lock() else {
            tracker.missed();
            return;
        };
        if state.current.as_ref().map(|frame| frame.scope_id) != Some(self.scope_id) {
            tracker.missed();
            return;
        }
        state.current = self.previous.take();
        let event = tracker.notification(&mut state, "leave_restore_parent");
        drop(state);
        emit_transition(event);
    }
}
pub(crate) struct TurnGuard {
    profile: Option<crate::native_timing::CommandScope>,
    profile_owner: Option<Arc<Tracker>>,
    phase: Option<PhaseGuard>,
    previous: Option<Arc<Tracker>>,
    installed: bool,
}
impl Drop for TurnGuard {
    fn drop(&mut self) {
        self.phase.take();
        if let Some(mut profile) = self.profile.take() {
            // A returned actor turn is not a successful WebDriver command.
            // Protocol errors and pending commands retain their own evidence.
            if let Some(owner) = &self.profile_owner {
                profile.finish_into(!std::thread::panicking(), &owner.profile);
            } else {
                profile.finish(!std::thread::panicking());
            }
        }
        if self.installed {
            CURRENT.with(|current| {
                if let Ok(mut current) = current.try_borrow_mut() {
                    *current = self.previous.take();
                }
            });
        }
    }
}
pub(crate) fn turn(
    registration: Option<&Registration>,
    phase: &'static str,
    owner: Option<Identity>,
) -> TurnGuard {
    let tracker = registration.map(|registration| registration.0.clone());
    let profile = crate::native_timing::enabled().then(|| {
        crate::native_timing::CommandScope::start(
            "native_actor_turn",
            serde_json::json!({
                "turn": phase, "retained_navigation_owner": owner,
                "page_id": tracker.as_ref().map(|tracker| tracker.key.0),
                "route_generation": tracker.as_ref().map(|tracker| tracker.key.1),
                "scope": "synchronous actor turn; not command completion"
            }),
        )
    });
    let installed = CURRENT.with(|current| {
        current
            .try_borrow_mut()
            .ok()
            .map(|mut current| std::mem::replace(&mut *current, tracker.clone()))
    });
    let Some(previous) = installed else {
        if let Some(tracker) = &tracker {
            tracker.missed();
        }
        return TurnGuard {
            profile_owner: tracker,
            profile,
            phase: None,
            previous: None,
            installed: false,
        };
    };
    let phase = tracker.as_ref().map(|tracker| tracker.enter(phase, owner));
    TurnGuard {
        profile_owner: tracker,
        profile,
        phase,
        previous,
        installed: true,
    }
}
pub(crate) fn phase(label: &'static str) -> PhaseGuard {
    #[cfg(all(test, feature = "v8-host"))]
    TEST_PHASE_LABELS.with(|labels| {
        if let Some(labels) = labels.borrow_mut().as_mut() {
            labels.push(label);
        }
    });
    #[cfg(all(test, feature = "v8-host"))]
    {
        let mut hook = TEST_PHASE_HOOK.with(|slot| slot.borrow_mut().take());
        if let Some(hook) = hook.as_mut() {
            hook(label);
        }
        TEST_PHASE_HOOK.with(|slot| *slot.borrow_mut() = hook);
    }
    let tracker = CURRENT.with(|current| current.try_borrow().ok().and_then(|value| value.clone()));
    tracker.map_or_else(PhaseGuard::empty, |tracker| tracker.enter(label, None))
}

// Test-only observation of the real instrumented callers; no sink or global
// environment mutation is required. The production tracker remains unchanged.
#[cfg(all(test, feature = "v8-host"))]
type TestPhaseHook = Box<dyn FnMut(&'static str)>;
#[cfg(all(test, feature = "v8-host"))]
thread_local! {
    static TEST_PHASE_LABELS: RefCell<Option<Vec<&'static str>>> = const { RefCell::new(None) };
    static TEST_PHASE_HOOK: RefCell<Option<TestPhaseHook>> = const { RefCell::new(None) };
}
#[cfg(all(test, feature = "v8-host"))]
pub(crate) fn capture_test_phase_labels<T>(
    operation: impl FnOnce() -> T,
) -> (T, Vec<&'static str>) {
    capture_test_phase_labels_with_hook(operation, |_| {})
}

#[cfg(all(test, feature = "v8-host"))]
pub(crate) fn capture_test_phase_labels_with_hook<T>(
    operation: impl FnOnce() -> T,
    hook: impl FnMut(&'static str) + 'static,
) -> (T, Vec<&'static str>) {
    struct Restore {
        labels: Option<Vec<&'static str>>,
        hook: Option<Box<dyn FnMut(&'static str)>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PHASE_LABELS.with(|labels| *labels.borrow_mut() = self.labels.take());
            TEST_PHASE_HOOK.with(|hook| *hook.borrow_mut() = self.hook.take());
        }
    }
    let _restore = Restore {
        labels: TEST_PHASE_LABELS.with(|labels| labels.replace(Some(Vec::new()))),
        hook: TEST_PHASE_HOOK.with(|slot| slot.replace(Some(Box::new(hook)))),
    };
    let result = operation();
    let labels = TEST_PHASE_LABELS.with(|labels| labels.borrow_mut().take().unwrap());
    (result, labels)
}

pub(crate) struct CloseTrace {
    identity: Option<Identity>,
    tracker: Option<Arc<Tracker>>,
    unavailable: Option<&'static str>,
}
impl CloseTrace {
    pub(crate) fn begin(identity: Option<Identity>, boundary: &'static str) -> Self {
        let identity =
            identity.filter(|id| id.command == Some("close") && crate::native_timing::enabled());
        let mut trace = Self {
            identity,
            tracker: None,
            unavailable: None,
        };
        if let Some(identity) = identity {
            match registry().try_lock() {
                Ok(registry) => {
                    trace.tracker = registry
                        .get(&(identity.page_id, identity.route_generation))
                        .and_then(Weak::upgrade);
                    if trace.tracker.is_none() {
                        trace.unavailable = Some("tracker_absent_or_retired");
                    }
                }
                Err(_) => trace.unavailable = Some("registry_lock_unavailable"),
            }
            if let Some(tracker) = &trace.tracker {
                tracker.activate(identity);
            }
        }
        trace.point(boundary, None);
        trace
    }
    pub(crate) fn point(&self, boundary: &'static str, observed_success: Option<bool>) {
        if let Some(identity) = self.identity {
            crate::native_timing::point(
                "close_admission_boundary",
                serde_json::json!({
                    "close_request":identity,"boundary":boundary,"observed_success":observed_success,
                    "actor_snapshot":self.tracker.as_ref().map(|tracker|tracker.snapshot()),
                    "snapshot_unavailable":self.unavailable,"completion_inferred":false
                }),
            );
        }
    }
}
impl Drop for CloseTrace {
    fn drop(&mut self) {
        self.point("path_scope_dropped_outcome_unspecified", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(message: u64, command: &'static str) -> Identity {
        Identity {
            process_session: 80,
            page_id: 811,
            route_generation: 900,
            message_id: Some(message),
            command: Some(command),
            context_id: None,
            peer_pid: None,
            association: "request",
        }
    }
    fn tracker() -> Arc<Tracker> {
        Arc::new(Tracker {
            profile: Default::default(),
            key: (811, 900),
            root_id: 31,
            state: Default::default(),
            incoherent: AtomicBool::new(false),
            missed_updates: AtomicU64::new(0),
            retired: AtomicBool::new(false),
        })
    }
    #[test]
    fn absent_sink_has_no_registration_or_phase_owner() {
        assert!(register_enabled(811, 900, false).is_none());
        let _turn = turn(None, "disabled", None);
        let phase = phase("disabled_child");
        assert!(phase.tracker.is_none());
    }
    #[test]
    fn held_actor_phase_snapshot_keeps_navigation_and_close_identities_separate() {
        let tracker = tracker();
        let child = tracker.clone();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let a = entered.clone();
        let b = release.clone();
        let thread = std::thread::spawn(move || {
            let _phase = child.enter(
                "timers_and_control_tasks",
                Some(identity(14, "controlled_navigation")),
            );
            a.wait();
            b.wait();
        });
        entered.wait();
        tracker.activate(identity(15, "close"));
        let snapshot = tracker.snapshot();
        release.wait();
        thread.join().unwrap();
        assert_eq!(snapshot["frame"]["phase"], "timers_and_control_tasks");
        assert_eq!(
            snapshot["frame"]["retained_navigation_owner"]["message_id"],
            14
        );
        assert_eq!(snapshot["close_request"]["message_id"], 15);
        #[cfg(unix)]
        assert!(snapshot["frame"]["start_monotonic_ns"].is_number());
        assert!(tracker.snapshot()["frame"].is_null());
    }
    #[test]
    fn nested_scope_restores_exact_parent_start_after_unwind() {
        let tracker = tracker();
        let outer = tracker.enter("page_poll", None);
        let before = tracker.snapshot()["frame"].clone();
        let result = std::panic::catch_unwind(|| {
            let _inner = tracker.enter("parser_resume", None);
            let current = tracker.snapshot();
            assert_eq!(current["frame"]["parent_scope_id"], before["scope_id"]);
            panic!("injected actor operation failure");
        });
        assert!(result.is_err());
        assert_eq!(tracker.snapshot()["frame"], before);
        drop(outer);
        assert!(tracker.snapshot()["frame"].is_null());
    }
    #[test]
    fn nested_animation_checkpoint_snapshot_keeps_close_and_navigation_owners() {
        let tracker = tracker();
        let animation = tracker.enter(
            "animation_frame",
            Some(identity(14, "controlled_navigation")),
        );
        let animation_snapshot = tracker.snapshot()["frame"].clone();
        let checkpoint = tracker.enter("executable_checkpoint", None);
        let checkpoint_snapshot = tracker.snapshot()["frame"].clone();
        let child = tracker.clone();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let a = entered.clone();
        let b = release.clone();
        let thread = std::thread::spawn(move || {
            let _phase = child.enter("checkpoint_dynamic_frames", None);
            a.wait();
            b.wait();
            Err::<(), _>("retained host failure")
        });
        entered.wait();
        tracker.activate(identity(15, "close"));
        let snapshot = tracker.snapshot();
        release.wait();
        assert_eq!(thread.join().unwrap(), Err("retained host failure"));
        assert_eq!(snapshot["frame"]["phase"], "checkpoint_dynamic_frames");
        assert_eq!(
            snapshot["frame"]["parent_scope_id"],
            checkpoint_snapshot["scope_id"]
        );
        assert_eq!(
            snapshot["frame"]["retained_navigation_owner"]["message_id"],
            14
        );
        assert_eq!(snapshot["close_request"]["message_id"], 15);
        assert_eq!(snapshot["incoherent"], false);
        assert_eq!(tracker.snapshot()["frame"], checkpoint_snapshot);
        drop(checkpoint);
        assert_eq!(tracker.snapshot()["frame"], animation_snapshot);
        drop(animation);
        assert!(tracker.snapshot()["frame"].is_null());
    }

    #[test]
    fn diagnostic_lock_contention_marks_unknown_without_waiting() {
        let tracker = tracker();
        let held = tracker.state.lock().unwrap();
        let phase = tracker.enter("contended", None);
        assert!(phase.tracker.is_none());
        assert_eq!(tracker.snapshot()["available"], false);
        tracker.activate(identity(15, "close"));
        drop(held);
        assert_eq!(tracker.snapshot()["incoherent"], true);
        assert_eq!(tracker.snapshot()["missed_updates"], 2);
    }
    #[test]
    fn transitions_are_silent_before_close_and_have_one_noncompletion_cap() {
        let tracker = tracker();
        let mut state = tracker.state.lock().unwrap();
        for _ in 0..1000 {
            assert!(tracker.notification(&mut state, "enter").is_none());
        }
        assert_eq!(state.transitions, 0);
        state.close = Some(identity(15, "close"));
        for sequence in 0..TRANSITION_LIMIT {
            let row = tracker.notification(&mut state, "enter").unwrap();
            assert_eq!(row["sequence"], sequence);
            assert_eq!(row["boundary"], "enter");
        }
        let cap = tracker.notification(&mut state, "leave").unwrap();
        assert_eq!(cap["boundary"], "transition_cap");
        assert_eq!(cap["completion_inferred"], false);
        assert!(cap["frame"].is_null());
        for _ in 0..10 {
            assert!(tracker.notification(&mut state, "enter").is_none());
        }
        assert_eq!(state.transitions, TRANSITION_LIMIT + 11);
    }
    #[test]
    fn retired_registration_cannot_be_adopted_by_reused_page_generation() {
        let first = register_enabled(911_701, 900, true).unwrap();
        let retained = first.0.clone();
        let old_id = retained.root_id;
        assert!(register_enabled(911_701, 900, true).is_none());
        let next_generation = register_enabled(911_701, 901, true).unwrap();
        assert_ne!(next_generation.0.root_id, old_id);
        drop(first);
        assert_eq!(retained.snapshot()["available"], false);
        assert_eq!(retained.snapshot()["retired"], true);
        let replacement = register_enabled(911_701, 900, true).unwrap();
        assert_ne!(replacement.0.root_id, old_id);
    }
    #[test]
    fn nested_root_and_missing_registration_do_not_leak_phase_attribution() {
        let a = Registration(tracker());
        let b = Registration(tracker());
        let outer = turn(Some(&a), "root_a", None);
        {
            let _inner = turn(Some(&b), "root_b", None);
            let _phase = phase("root_b_child");
            assert_eq!(b.0.snapshot()["frame"]["phase"], "root_b_child");
            assert_eq!(a.0.snapshot()["frame"]["phase"], "root_a");
        }
        {
            let _missing = turn(None, "unregistered_root", None);
            assert!(phase("unregistered_child").tracker.is_none());
        }
        {
            let _phase = phase("root_a_child");
            assert_eq!(a.0.snapshot()["frame"]["phase"], "root_a_child");
        }
        drop(outer);
        assert!(phase("no_root").tracker.is_none());
    }
    #[test]
    fn later_close_cannot_relabel_existing_observation_owner() {
        let tracker = tracker();
        tracker.activate(identity(15, "close"));
        tracker.activate(identity(16, "close"));
        assert_eq!(tracker.snapshot()["close_request"]["message_id"], 15);
    }
}
