//! Opt-in parent process evidence, independent of network activity.
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PROCESSES: usize = 16_384;
const MAX_LEASES: usize = 65_536;
static ACTIVE: OnceLock<Mutex<Weak<Mutex<State>>>> = OnceLock::new();
#[derive(Default)]
struct State {
    processes: BTreeMap<u64, ProcessRecord>,
    overflow: bool,
    gaps: BTreeSet<String>,
    last_unix_ns: u128,
    lease_count: usize,
}
impl State {
    fn reserve_lease(&mut self) -> bool {
        if self.lease_count >= MAX_LEASES {
            self.overflow = true;
            return false;
        }
        self.lease_count += 1;
        true
    }
    fn timestamp(&mut self) -> u128 {
        self.observe_time(now())
    }
    fn observe_time(&mut self, time: u128) -> u128 {
        if time == 0 {
            self.gaps.insert("native wall clock unavailable".into());
        }
        if time < self.last_unix_ns {
            self.gaps.insert("native wall clock moved backwards".into());
        }
        self.last_unix_ns = self.last_unix_ns.max(time);
        time
    }
}
#[derive(Clone)]
pub struct WorkerLifecycleAudit(Arc<Mutex<State>>);
#[derive(Clone, Serialize)]
pub struct WorkerLeaseRecord {
    pub context_id: u64,
    pub page_id: u64,
    pub generation: u64,
    pub started_unix_ns: u128,
    pub ended_unix_ns: Option<u128>,
}
#[derive(Clone, Serialize)]
pub struct ProcessRecord {
    pub launch_id: u64,
    pub worker_pid: u32,
    pub parent_pid: u32,
    pub role: String,
    pub binary_sha256: Option<String>,
    pub spawned_unix_ns: u128,
    pub reaped_unix_ns: Option<u128>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub leases: Vec<WorkerLeaseRecord>,
}
#[derive(Serialize)]
pub struct WorkerLifecycleSnapshot {
    pub schema_version: u32,
    pub complete: bool,
    pub overflow: bool,
    pub gaps: Vec<String>,
    pub processes: Vec<ProcessRecord>,
}
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or(0)
}
impl WorkerLifecycleAudit {
    /// Hold this guard before runtime verification and until final worker reap.
    /// Concurrent qualification owners must share the same guard explicitly.
    pub fn start() -> Result<Self, String> {
        let mut active = ACTIVE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "lifecycle audit registry poisoned")?;
        if active.upgrade().is_some() {
            return Err("worker lifecycle audit already active".into());
        }
        let state = Arc::new(Mutex::new(State::default()));
        *active = Arc::downgrade(&state);
        Ok(Self(state))
    }
    pub fn snapshot(&self) -> WorkerLifecycleSnapshot {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let mut gaps = state.gaps.clone();
        if self.0.is_poisoned() || ACTIVE.get().is_some_and(Mutex::is_poisoned) {
            gaps.insert("native lifecycle audit lock poisoned".into());
        }
        WorkerLifecycleSnapshot {
            schema_version: 1,
            complete: !state.overflow && gaps.is_empty(),
            overflow: state.overflow,
            gaps: gaps.into_iter().collect(),
            processes: state.processes.values().cloned().collect(),
        }
    }
}
pub(crate) struct ProcessObservation {
    state: Arc<Mutex<State>>,
    launch_id: u64,
}
pub(crate) fn spawned(
    launch_id: u64,
    pid: u32,
    context: u64,
    page: u64,
    generation: u64,
    binary_sha256: Option<String>,
) -> Option<ProcessObservation> {
    let state = ACTIVE.get()?.lock().ok()?.upgrade()?;
    let mut records = state.lock().unwrap_or_else(|error| error.into_inner());
    if records.processes.len() >= MAX_PROCESSES {
        records.overflow = true;
        return None;
    }
    if records.processes.contains_key(&launch_id) {
        records
            .gaps
            .insert("duplicate native launch identity".into());
        return None;
    }
    let binary_sha256 = binary_sha256.filter(|value| {
        let valid = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !valid {
            records.gaps.insert("invalid native binary digest".into());
        }
        valid
    });
    if !records.reserve_lease() {
        return None;
    }
    let time = records.timestamp();
    records.processes.insert(
        launch_id,
        ProcessRecord {
            launch_id,
            worker_pid: pid,
            parent_pid: std::process::id(),
            role: "page".into(),
            binary_sha256,
            spawned_unix_ns: time,
            reaped_unix_ns: None,
            exit_code: None,
            signal: None,
            leases: vec![WorkerLeaseRecord {
                context_id: context,
                page_id: page,
                generation,
                started_unix_ns: time,
                ended_unix_ns: None,
            }],
        },
    );
    drop(records);
    Some(ProcessObservation { state, launch_id })
}
impl ProcessObservation {
    pub(crate) fn begin_lease(&self, context: u64, page: u64, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(record) = state.processes.get(&self.launch_id) else {
            state
                .gaps
                .insert("lease references missing native launch".into());
            return;
        };
        if record.reaped_unix_ns.is_some() {
            state
                .gaps
                .insert("lease admitted after native process reap".into());
            return;
        }
        if record.leases.iter().any(|lease| {
            lease.context_id == context && lease.page_id == page && lease.ended_unix_ns.is_none()
        }) {
            state.gaps.insert("overlapping native page leases".into());
            return;
        }
        if !state.reserve_lease() {
            return;
        }
        let time = state.timestamp();
        if let Some(record) = state.processes.get_mut(&self.launch_id) {
            record.leases.push(WorkerLeaseRecord {
                context_id: context,
                page_id: page,
                generation,
                started_unix_ns: time,
                ended_unix_ns: None,
            });
        }
    }
    pub(crate) fn end_leases(&self, page: Option<u64>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let time = state.timestamp();
        if let Some(record) = state.processes.get_mut(&self.launch_id) {
            for lease in &mut record.leases {
                if lease.ended_unix_ns.is_none() && page.is_none_or(|page| lease.page_id == page) {
                    lease.ended_unix_ns = Some(time);
                }
            }
        }
    }
    pub(crate) fn role(&self, role: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !matches!(role, "page" | "prewarmed" | "verification") {
            state.gaps.insert("invalid native process role".into());
            return;
        }
        if let Some(record) = state.processes.get_mut(&self.launch_id) {
            record.role = role.into();
        }
    }
    pub(crate) fn binary(&self, sha256: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            state.gaps.insert("invalid native binary digest".into());
            return;
        }
        if let Some(record) = state.processes.get_mut(&self.launch_id) {
            record.binary_sha256 = Some(sha256.into());
        }
    }
    #[cfg(unix)]
    pub(crate) fn reaped(&self, status: std::process::ExitStatus) {
        use std::os::unix::process::ExitStatusExt;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let time = state.timestamp();
        if let Some(record) = state.processes.get_mut(&self.launch_id) {
            if record.reaped_unix_ns.is_some() {
                return;
            }
            record.reaped_unix_ns = Some(time);
            record.exit_code = status.code();
            record.signal = status.signal();
            for lease in &mut record.leases {
                if lease.ended_unix_ns.is_none() {
                    lease.ended_unix_ns = Some(time);
                }
            }
        }
    }
}
impl Drop for ProcessObservation {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .processes
            .get(&self.launch_id)
            .is_some_and(|record| record.reaped_unix_ns.is_none())
        {
            state.gaps.insert(format!(
                "launch {} lost without checked reap evidence",
                self.launch_id
            ));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    fn empty_process(id: u64) -> ProcessRecord {
        ProcessRecord {
            launch_id: id,
            worker_pid: 42,
            parent_pid: 7,
            role: "page".into(),
            binary_sha256: None,
            spawned_unix_ns: 1,
            reaped_unix_ns: None,
            exit_code: None,
            signal: None,
            leases: Vec::new(),
        }
    }
    #[test]
    fn lease_history_budget_is_shared_across_processes_and_never_wraps() {
        let state = Arc::new(Mutex::new(State {
            lease_count: MAX_LEASES - 1,
            ..Default::default()
        }));
        state.lock().unwrap().processes.insert(1, empty_process(1));
        state.lock().unwrap().processes.insert(2, empty_process(2));
        let first = ProcessObservation {
            state: state.clone(),
            launch_id: 1,
        };
        let second = ProcessObservation {
            state: state.clone(),
            launch_id: 2,
        };
        first.begin_lease(1, 1, 0);
        second.begin_lease(2, 2, 0);
        let snapshot = WorkerLifecycleAudit(state.clone()).snapshot();
        assert!(snapshot.overflow);
        assert!(!snapshot.complete);
        assert_eq!(snapshot.processes[0].leases.len(), 1);
        assert!(snapshot.processes[1].leases.is_empty());
        assert_eq!(state.lock().unwrap().lease_count, MAX_LEASES);
    }
    #[test]
    fn overlapping_or_post_reap_leases_are_not_recorded_as_valid_ownership() {
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().unwrap().processes.insert(1, empty_process(1));
        let observation = ProcessObservation {
            state: state.clone(),
            launch_id: 1,
        };
        observation.begin_lease(1, 1, 0);
        observation.begin_lease(1, 1, 1);
        observation.reaped(std::process::ExitStatus::from_raw(0));
        observation.begin_lease(2, 2, 0);
        let snapshot = WorkerLifecycleAudit(state).snapshot();
        assert_eq!(snapshot.processes[0].leases.len(), 1);
        assert!(!snapshot.complete);
        assert!(snapshot.gaps.iter().any(|gap| gap.contains("overlapping")));
        assert!(
            snapshot
                .gaps
                .iter()
                .any(|gap| gap.contains("after native process reap"))
        );
    }
    #[test]
    fn wall_clock_regression_remains_an_explicit_evidence_gap() {
        let mut state = State::default();
        assert_eq!(state.observe_time(100), 100);
        assert_eq!(state.observe_time(90), 90);
        assert_eq!(state.last_unix_ns, 100);
        state.observe_time(120);
        let snapshot = WorkerLifecycleAudit(Arc::new(Mutex::new(state))).snapshot();
        assert!(!snapshot.complete);
        assert!(
            snapshot
                .gaps
                .iter()
                .any(|gap| gap.contains("moved backwards"))
        );
    }

    #[test]
    fn poisoned_history_cannot_produce_complete_evidence() {
        let state = Arc::new(Mutex::new(State::default()));
        let poisoned = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("injected history failure");
        })
        .join();
        let snapshot = WorkerLifecycleAudit(state).snapshot();
        assert!(!snapshot.complete);
        assert!(snapshot.gaps.iter().any(|gap| gap.contains("poisoned")));
    }

    #[test]
    fn observation_retains_checked_exit_and_marks_missing_reap() {
        let state = Arc::new(Mutex::new(State::default()));
        let make = |id| ProcessRecord {
            launch_id: id,
            worker_pid: 42,
            parent_pid: 7,
            role: "verification".into(),
            binary_sha256: None,
            spawned_unix_ns: 1,
            reaped_unix_ns: None,
            exit_code: None,
            signal: None,
            leases: Vec::new(),
        };
        state.lock().unwrap().processes.insert(1, make(1));
        state.lock().unwrap().processes.insert(2, make(2));
        let first = ProcessObservation {
            state: state.clone(),
            launch_id: 1,
        };
        first.reaped(std::process::ExitStatus::from_raw(9));
        drop(first);
        let second = ProcessObservation {
            state: state.clone(),
            launch_id: 2,
        };
        second.begin_lease(10, 11, 1);
        second.begin_lease(10, 12, 1);
        second.end_leases(Some(11));
        {
            let state = state.lock().unwrap();
            let leases = &state.processes[&2].leases;
            assert!(leases[0].ended_unix_ns.is_some());
            assert!(leases[1].ended_unix_ns.is_none());
        }
        second.end_leases(None);
        second.begin_lease(20, 21, 2);
        drop(second);
        let snapshot = WorkerLifecycleAudit(state).snapshot();
        assert_eq!(snapshot.processes[0].signal, Some(9));
        assert!(snapshot.processes[0].reaped_unix_ns.is_some());
        assert!(!snapshot.complete);
        assert_eq!(snapshot.gaps.len(), 1);
    }
}
