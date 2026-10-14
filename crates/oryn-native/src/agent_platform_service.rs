//! Parent-side asynchronous operation ownership for one admitted AgentId.
//! Drivers close over native authority and must cancel underlying I/O before
//! returning a terminal cancellation; operation IDs are only correlations.
use oryn_js::{platform::*, worker_wire::*};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
const MAX_OPERATIONS: usize = 32;
const MAX_BYTES: usize = 32 * 1024 * 1024;
pub trait AgentOperationExecutor: Send + Sync + 'static {
    /// Freeze any retained native authority before the asynchronous launch.
    fn prepare(&self, _id: HostOperationId, _request: &PlatformHostRequest) -> Result<(), String> {
        Ok(())
    }
    fn retains_keepalive(&self) -> bool {
        false
    }
    fn execute(
        &self,
        id: HostOperationId,
        request: PlatformHostRequest,
        cancel: Arc<AtomicBool>,
    ) -> Result<PlatformHostResult, String>;
    fn cancel(&self, id: HostOperationId);
    /// Dispose a result whose consumer retired while the operation completed.
    /// HttpStarted/WebSocketOpened ownership must not escape this path.
    /// This must only revoke local ownership or signal retained cleanup; it may
    /// run while unwinding an unacknowledged response and must not wait for IO.
    fn dispose(&self, result: PlatformHostResult);
    /// Explicit cancellation differs from losing a retired JS consumer: it
    /// must cancel even a keepalive response rather than orphan-drain it.
    fn dispose_canceled(&self, result: PlatformHostResult) {
        self.dispose(result)
    }
    fn retire(&self);
    /// Called after all accepted operations have joined. Failures retain ownership for retry.
    fn finish_retirement(&self) -> Result<(), String> {
        Ok(())
    }
}
struct State {
    idb_order: VecDeque<HostOperationId>,
    keepalive: std::collections::BTreeSet<HostOperationId>,
    retired: bool,
    launching: usize,
    pending: BTreeMap<HostOperationId, Arc<AtomicBool>>,
    ready: VecDeque<(PlatformHostCompletion, usize)>,
    ready_bytes: usize,
}
struct Shared {
    state: Mutex<State>,
    idb_changed: Condvar,
    threads: Mutex<Vec<JoinHandle<()>>>,
}
struct IdbTurn(Option<(Arc<Shared>, HostOperationId)>);
impl IdbTurn {
    fn enter(shared: &Arc<Shared>, id: HostOperationId, indexed_db: bool) -> Self {
        if !indexed_db {
            return Self(None);
        }
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while state.idb_order.front() != Some(&id) {
            state = shared
                .idb_changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        Self(Some((shared.clone(), id)))
    }
}
impl Drop for IdbTurn {
    fn drop(&mut self) {
        if let Some((shared, id)) = self.0.take() {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.idb_order.retain(|candidate| *candidate != id);
            shared.idb_changed.notify_all();
        }
    }
}
pub struct AgentPlatformService {
    driver: Arc<dyn AgentOperationExecutor>,
    shared: Arc<Shared>,
}
fn error(message: impl Into<String>) -> WorkerError {
    WorkerError::new("InvalidStateError", message)
}
impl AgentPlatformService {
    pub fn new(driver: Arc<dyn AgentOperationExecutor>) -> Self {
        Self {
            driver,
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    idb_order: VecDeque::new(),
                    keepalive: Default::default(),
                    retired: false,
                    launching: 0,
                    pending: BTreeMap::new(),
                    ready: VecDeque::new(),
                    ready_bytes: 0,
                }),
                threads: Mutex::new(Vec::new()),
                idb_changed: Condvar::new(),
            }),
        }
    }
    fn reap_finished(&self) -> Result<(), WorkerError> {
        let finished = {
            let mut threads = self
                .shared
                .threads
                .lock()
                .map_err(|_| error("agent operation threads poisoned"))?;
            let mut finished = Vec::new();
            let mut index = 0;
            while index < threads.len() {
                if threads[index].is_finished() {
                    finished.push(threads.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            finished
        };
        let mut panicked = false;
        for thread in finished {
            panicked |= thread.join().is_err();
        }
        if panicked {
            Err(error("agent operation thread panicked"))
        } else {
            Ok(())
        }
    }
    pub fn submit(
        &self,
        id: HostOperationId,
        request: PlatformHostRequest,
    ) -> Result<(), WorkerError> {
        self.reap_finished()?;
        crate::agent_thread::wire_size(&request, MAX_BYTES)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let indexed_db = matches!(&request, PlatformHostRequest::IndexedDb { .. });
        let keepalive = self.driver.retains_keepalive()
            && matches!(&request,PlatformHostRequest::HttpStart{request}if request.fetch.as_ref().is_some_and(|fetch|fetch.keepalive));
        {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| error("agent operation registry poisoned"))?;
            if state.retired {
                return Err(error("agent platform retired"));
            }
            if state.pending.contains_key(&id) {
                return Err(error("duplicate agent operation"));
            }
            if state.pending.len() >= MAX_OPERATIONS {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "agent operation limit",
                ));
            }
            if keepalive {
                state.keepalive.insert(id);
            }
            state.pending.insert(id, cancel.clone());
            if indexed_db {
                state.idb_order.push_back(id);
            }
            state.launching += 1;
        }
        if let Err(failure) = self.driver.prepare(id, &request) {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.pending.remove(&id);
            state.keepalive.remove(&id);
            state.launching -= 1;
            state.idb_order.retain(|candidate| *candidate != id);
            self.shared.idb_changed.notify_all();
            return Err(error(failure));
        }
        let shared = self.shared.clone();
        let driver = self.driver.clone();
        let spawned = std::thread::Builder::new()
            .name("oryn-agent-platform-operation".into())
            .spawn(move || {
                // Mutex acquisition inside execute is not a FIFO: record the
                // admitted order before launching concurrent operation threads.
                // Canceled work still takes/releases its turn, so no successor
                // can overtake an earlier request or strand a retired queue.
                let _turn = IdbTurn::enter(&shared, id, indexed_db);
                let mut result = if cancel.load(Ordering::Acquire) {
                    Err("AbortError: agent operation cancelled".into())
                } else {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        driver.execute(id, request, cancel.clone())
                    }))
                    .unwrap_or_else(|_| Err("agent operation panicked".into()))
                };
                if cancel.load(Ordering::Acquire) {
                    if let Ok(value) = result {
                        driver.dispose_canceled(value);
                    }
                    result = Err("AbortError: agent operation cancelled".into());
                }
                let mut completion = PlatformHostCompletion {
                    operation_id: id,
                    result,
                };
                let mut bytes = crate::agent_thread::wire_size(&completion, MAX_BYTES).ok();
                if bytes.is_none() {
                    if let Ok(value) = std::mem::replace(
                        &mut completion.result,
                        Err("QuotaExceededError: agent result budget".into()),
                    ) {
                        driver.dispose(value);
                    }
                    bytes = crate::agent_thread::wire_size(&completion, MAX_BYTES).ok();
                }
                let mut state = shared
                    .state
                    .lock()
                    .unwrap_or_else(|failure| failure.into_inner());
                if state.retired || !state.pending.contains_key(&id) {
                    drop(state);
                    if let Ok(value) = completion.result {
                        driver.dispose(value);
                    }
                    return;
                }
                if state
                    .pending
                    .get(&id)
                    .is_some_and(|cancel| cancel.load(Ordering::Acquire))
                {
                    drop(state);
                    if let Ok(value) = std::mem::replace(
                        &mut completion.result,
                        Err("AbortError: agent operation cancelled".into()),
                    ) {
                        driver.dispose_canceled(value);
                    }
                    bytes = crate::agent_thread::wire_size(&completion, MAX_BYTES).ok();
                    state = shared
                        .state
                        .lock()
                        .unwrap_or_else(|failure| failure.into_inner());
                    if state.retired {
                        return;
                    }
                }
                let bytes = bytes.unwrap_or(256);
                if state
                    .ready_bytes
                    .checked_add(bytes)
                    .is_none_or(|total| total > MAX_BYTES - MAX_OPERATIONS * 1024)
                {
                    // No driver callback runs under the operation registry lock.
                    drop(state);
                    if let Ok(value) = std::mem::replace(
                        &mut completion.result,
                        Err("QuotaExceededError: agent completion queue".into()),
                    ) {
                        driver.dispose(value);
                    }
                    let bytes =
                        crate::agent_thread::wire_size(&completion, MAX_BYTES).unwrap_or(256);
                    state = shared
                        .state
                        .lock()
                        .unwrap_or_else(|failure| failure.into_inner());
                    if state.retired {
                        return;
                    }
                    state.ready_bytes += bytes;
                    state.ready.push_back((completion, bytes));
                } else {
                    state.ready_bytes += bytes;
                    state.ready.push_back((completion, bytes));
                }
            });
        match spawned {
            Ok(thread) => {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(|failure| failure.into_inner());
                self.shared
                    .threads
                    .lock()
                    .unwrap_or_else(|failure| failure.into_inner())
                    .push(thread);
                state.launching -= 1;
                Ok(())
            }
            Err(failure) => {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(|failure| failure.into_inner());
                state.launching -= 1;
                state.pending.remove(&id);
                state.keepalive.remove(&id);
                state.idb_order.retain(|candidate| *candidate != id);
                self.shared.idb_changed.notify_all();
                drop(state);
                self.driver.cancel(id);
                Err(error(failure.to_string()))
            }
        }
    }
    pub fn poll(&self, max: usize) -> Result<Vec<PlatformHostCompletion>, WorkerError> {
        self.reap_finished()?;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| error("agent operation registry poisoned"))?;
        if state.retired {
            return Err(error("agent platform retired"));
        }
        let mut results = Vec::new();
        for _ in 0..max.min(MAX_OPERATIONS) {
            let Some((completion, bytes)) = state.ready.pop_front() else {
                break;
            };
            state.ready_bytes -= bytes;
            state.pending.remove(&completion.operation_id);
            state.keepalive.remove(&completion.operation_id);
            results.push(completion);
        }
        Ok(results)
    }
    pub(crate) fn reclaim_completions(&self, completions: &[PlatformHostCompletion]) {
        for completion in completions {
            if let Ok(
                value @ (PlatformHostResult::HttpStarted { .. }
                | PlatformHostResult::WebSocketOpened { .. }),
            ) = &completion.result
            {
                self.driver.dispose(value.clone());
            }
        }
    }
    pub fn cancel(&self, id: HostOperationId) -> Result<(), WorkerError> {
        let (pending, dispose) = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| error("agent operation registry poisoned"))?;
            if state.retired {
                return Err(error("agent platform retired"));
            }
            let pending = state.pending.get(&id).cloned();
            if let Some(cancel) = &pending {
                cancel.store(true, Ordering::Release);
            }
            let mut dispose = None;
            if let Some(index) = state
                .ready
                .iter()
                .position(|(completion, _)| completion.operation_id == id)
            {
                let (mut completion, old_bytes) = state.ready.remove(index).unwrap();
                state.ready_bytes -= old_bytes;
                dispose = std::mem::replace(
                    &mut completion.result,
                    Err("AbortError: agent operation cancelled".into()),
                )
                .ok();
                let bytes = crate::agent_thread::wire_size(&completion, MAX_BYTES).unwrap_or(256);
                state.ready_bytes += bytes;
                state.ready.insert(index, (completion, bytes));
            }
            (pending, dispose)
        };
        if pending.is_some() {
            self.driver.cancel(id);
        }
        if let Some(value) = dispose {
            self.driver.dispose_canceled(value);
        }
        Ok(())
    }
    pub fn retire(&self) {
        let (pending, ready, keepalive) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|failure| failure.into_inner());
            if state.retired {
                return;
            }
            state.retired = true;
            state.ready_bytes = 0;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.ready),
                std::mem::take(&mut state.keepalive),
            )
        };
        for (id, cancel) in pending {
            if !keepalive.contains(&id) {
                cancel.store(true, Ordering::Release);
                self.driver.cancel(id);
            }
        }
        for (completion, _) in ready {
            if let Ok(value) = completion.result {
                self.driver.dispose(value);
            }
        }
        self.driver.retire();
    }
    /// Call after retire outside broker/registry locks. An incomplete drain
    /// retains the join handles, so the owner can retry and report the failure.
    pub fn drain(&self, timeout: Duration) -> Result<(), WorkerError> {
        let deadline = Instant::now() + timeout;
        let mut panicked = false;
        loop {
            let finished = {
                let mut threads = self
                    .shared
                    .threads
                    .lock()
                    .map_err(|_| error("agent operation threads poisoned"))?;
                let mut finished = Vec::new();
                let mut index = 0;
                while index < threads.len() {
                    if threads[index].is_finished() {
                        finished.push(threads.swap_remove(index));
                    } else {
                        index += 1;
                    }
                }
                finished
            };
            for thread in finished {
                panicked |= thread.join().is_err();
            }
            let drained = {
                let state = self
                    .shared
                    .state
                    .lock()
                    .map_err(|_| error("agent operation registry poisoned"))?;
                state.launching == 0
                    && self
                        .shared
                        .threads
                        .lock()
                        .map_err(|_| error("agent operation threads poisoned"))?
                        .is_empty()
            };
            if drained {
                if panicked {
                    return Err(error("agent operation thread panicked"));
                }
                if self
                    .shared
                    .state
                    .lock()
                    .map_err(|_| error("agent operation registry poisoned"))?
                    .retired
                {
                    self.driver.finish_retirement().map_err(error)?;
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(error("agent operations have not drained"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn idb_turn_follows_admission_order_even_when_successor_runs_first() {
        struct Noop;
        impl AgentOperationExecutor for Noop {
            fn execute(
                &self,
                _: HostOperationId,
                _: PlatformHostRequest,
                _: Arc<AtomicBool>,
            ) -> Result<PlatformHostResult, String> {
                Ok(PlatformHostResult::HttpAborted)
            }
            fn cancel(&self, _: HostOperationId) {}
            fn dispose(&self, _: PlatformHostResult) {}
            fn retire(&self) {}
        }
        let service = AgentPlatformService::new(Arc::new(Noop));
        let first = HostOperationId(1);
        let second = HostOperationId(2);
        service
            .shared
            .state
            .lock()
            .unwrap()
            .idb_order
            .extend([first, second]);
        let shared = service.shared.clone();
        let (started, starting) = std::sync::mpsc::channel();
        let (entered, received) = std::sync::mpsc::channel();
        let successor = std::thread::spawn(move || {
            started.send(()).unwrap();
            let turn = IdbTurn::enter(&shared, second, true);
            entered.send(second).unwrap();
            drop(turn);
        });
        starting.recv_timeout(Duration::from_secs(2)).unwrap();
        let predecessor = IdbTurn::enter(&service.shared, first, true);
        assert!(matches!(
            received.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        // Non-IDB work remains independent of the held IDB head.
        drop(IdbTurn::enter(&service.shared, HostOperationId(3), false));
        drop(predecessor);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            second
        );
        successor.join().unwrap();
        assert!(service.shared.state.lock().unwrap().idb_order.is_empty());
    }
    #[test]
    fn agent_idb_successor_waits_for_predecessor_terminal_disposal_and_publication() {
        struct Publication {
            executed: std::sync::mpsc::Sender<HostOperationId>,
            execute_release: Mutex<std::sync::mpsc::Receiver<()>>,
            disposing: std::sync::mpsc::Sender<()>,
            dispose_release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl AgentOperationExecutor for Publication {
            fn execute(
                &self,
                id: HostOperationId,
                _: PlatformHostRequest,
                _: Arc<AtomicBool>,
            ) -> Result<PlatformHostResult, String> {
                self.executed.send(id).unwrap();
                if id == HostOperationId(1) {
                    self.execute_release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                }
                Ok(PlatformHostResult::IndexedDb {
                    response: IdbHostResponse::default(),
                })
            }
            fn cancel(&self, _: HostOperationId) {}
            fn dispose(&self, _: PlatformHostResult) {}
            fn dispose_canceled(&self, _: PlatformHostResult) {
                self.disposing.send(()).unwrap();
                self.dispose_release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
            }
            fn retire(&self) {}
        }
        let (executed, observed) = std::sync::mpsc::channel();
        let (execute_release, execution) = std::sync::mpsc::channel();
        let (disposing, disposal) = std::sync::mpsc::channel();
        let (dispose_release, release) = std::sync::mpsc::channel();
        let service = AgentPlatformService::new(Arc::new(Publication {
            executed,
            execute_release: Mutex::new(execution),
            disposing,
            dispose_release: Mutex::new(release),
        }));
        let request = || PlatformHostRequest::IndexedDb {
            origin: "https://example.test".into(),
            command: IdbHostCommand::ListDatabases,
        };
        service.submit(HostOperationId(1), request()).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(2)).unwrap(),
            HostOperationId(1)
        );
        service.submit(HostOperationId(2), request()).unwrap();
        service.cancel(HostOperationId(1)).unwrap();
        execute_release.send(()).unwrap();
        disposal.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            observed.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(service.poll(32).unwrap().is_empty());
        dispose_release.send(()).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(2)).unwrap(),
            HostOperationId(2)
        );
        // Successor executes only after predecessor's AbortError entered ready.
        let first = service.poll(1).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].operation_id, HostOperationId(1));
        service.retire();
        service.drain(Duration::from_secs(2)).unwrap();
        assert!(service.shared.state.lock().unwrap().idb_order.is_empty());
    }
    #[test]
    fn agent_idb_cancelled_waiter_releases_turn_and_retirement_joins() {
        struct Blocking {
            entered: std::sync::mpsc::Sender<HostOperationId>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl AgentOperationExecutor for Blocking {
            fn execute(
                &self,
                id: HostOperationId,
                _: PlatformHostRequest,
                _: Arc<AtomicBool>,
            ) -> Result<PlatformHostResult, String> {
                self.entered.send(id).unwrap();
                if id == HostOperationId(1) {
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                }
                Ok(PlatformHostResult::IndexedDb {
                    response: IdbHostResponse::default(),
                })
            }
            fn cancel(&self, _: HostOperationId) {}
            fn dispose(&self, _: PlatformHostResult) {}
            fn retire(&self) {}
        }
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let service = AgentPlatformService::new(Arc::new(Blocking {
            entered,
            release: Mutex::new(wait),
        }));
        let request = || PlatformHostRequest::IndexedDb {
            origin: "https://example.test".into(),
            command: IdbHostCommand::ListDatabases,
        };
        service.submit(HostOperationId(1), request()).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(2)).unwrap(),
            HostOperationId(1)
        );
        service.submit(HostOperationId(2), request()).unwrap();
        service.cancel(HostOperationId(2)).unwrap();
        service.submit(HostOperationId(3), request()).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(2)).unwrap(),
            HostOperationId(3)
        );
        service.retire();
        service.drain(Duration::from_secs(2)).unwrap();
        assert!(service.shared.state.lock().unwrap().idb_order.is_empty());
    }
    use super::*;
    struct Executor {
        release: Mutex<bool>,
        changed: std::sync::Condvar,
        disposed: std::sync::atomic::AtomicUsize,
    }
    impl AgentOperationExecutor for Executor {
        fn execute(
            &self,
            _: HostOperationId,
            _: PlatformHostRequest,
            cancel: Arc<AtomicBool>,
        ) -> Result<PlatformHostResult, String> {
            let guard = self.release.lock().unwrap();
            drop(
                self.changed
                    .wait_while(guard, |release| {
                        !*release && !cancel.load(Ordering::Acquire)
                    })
                    .unwrap(),
            );
            Ok(PlatformHostResult::WebSocketOpened {
                socket_id: 7,
                protocol: String::new(),
            })
        }
        fn cancel(&self, _: HostOperationId) {
            self.changed.notify_all();
        }
        fn dispose(&self, _: PlatformHostResult) {
            self.disposed.fetch_add(1, Ordering::SeqCst);
        }
        fn retire(&self) {
            self.changed.notify_all();
        }
    }
    fn executor() -> Arc<Executor> {
        Arc::new(Executor {
            release: Mutex::new(false),
            changed: std::sync::Condvar::new(),
            disposed: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    fn request() -> PlatformHostRequest {
        PlatformHostRequest::WebSocketOpen {
            url: "wss://a.test/".into(),
            protocols: Vec::new(),
        }
    }
    #[test]
    fn keepalive_is_prepared_before_launch_and_retirement_does_not_abort_it() {
        struct Retained {
            prepared: AtomicBool,
            started: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
            completed: AtomicBool,
            canceled: AtomicBool,
        }
        impl AgentOperationExecutor for Retained {
            fn retains_keepalive(&self) -> bool {
                true
            }
            fn prepare(&self, _: HostOperationId, _: &PlatformHostRequest) -> Result<(), String> {
                self.prepared.store(true, Ordering::Release);
                Ok(())
            }
            fn execute(
                &self,
                _: HostOperationId,
                _: PlatformHostRequest,
                cancel: Arc<AtomicBool>,
            ) -> Result<PlatformHostResult, String> {
                assert!(self.prepared.load(Ordering::Acquire));
                self.started.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                assert!(!cancel.load(Ordering::Acquire));
                self.completed.store(true, Ordering::Release);
                Ok(PlatformHostResult::HttpAborted)
            }
            fn cancel(&self, _: HostOperationId) {
                self.canceled.store(true, Ordering::Release);
            }
            fn dispose(&self, _: PlatformHostResult) {}
            fn retire(&self) {}
        }
        let (started, entered) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let driver = Arc::new(Retained {
            prepared: AtomicBool::new(false),
            started,
            release: Mutex::new(wait),
            completed: AtomicBool::new(false),
            canceled: AtomicBool::new(false),
        });
        let service = AgentPlatformService::new(driver.clone());
        service
            .submit(
                HostOperationId(9),
                PlatformHostRequest::HttpStart {
                    request: PlatformHttpRequest {
                        upgrade_mixed_image: false,
                        fetch: Some(FetchOptions {
                            keepalive: true,
                            privileged_range: false,
                            cache: Default::default(),
                            mode: "cors".into(),
                            redirect: "follow".into(),
                            integrity: String::new(),
                        }),
                        use_cors_preflight: false,
                        method: "POST".into(),
                        url: "https://a.test/report".into(),
                        headers: Vec::new(),
                        body: vec![1],
                        credentials: "omit".into(),
                        initiator_url: None,
                        referrer_policy: None,
                    },
                },
            )
            .unwrap();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        service.retire();
        release.send(()).unwrap();
        service.drain(Duration::from_secs(2)).unwrap();
        assert!(driver.completed.load(Ordering::Acquire));
        assert!(!driver.canceled.load(Ordering::Acquire));
    }
    #[test]
    fn lost_polled_reply_reclaims_resources_from_original_driver_after_retirement() {
        let driver = executor();
        *driver.release.lock().unwrap() = true;
        let other = executor();
        let service = AgentPlatformService::new(driver.clone());
        service.submit(HostOperationId(2), request()).unwrap();
        service.drain(Duration::from_secs(1)).unwrap();
        let completions = service.poll(1).unwrap();
        assert_eq!(completions.len(), 1);
        service.retire();
        assert_eq!(driver.disposed.load(Ordering::SeqCst), 0);
        service.reclaim_completions(&completions);
        assert_eq!(driver.disposed.load(Ordering::SeqCst), 1);
        assert_eq!(other.disposed.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn explicit_cancel_uses_cancellation_disposal_for_a_ready_keepalive_result() {
        struct Ready {
            aborted: AtomicBool,
            orphaned: AtomicBool,
        }
        impl AgentOperationExecutor for Ready {
            fn retains_keepalive(&self) -> bool {
                true
            }
            fn execute(
                &self,
                _: HostOperationId,
                _: PlatformHostRequest,
                _: Arc<AtomicBool>,
            ) -> Result<PlatformHostResult, String> {
                Ok(PlatformHostResult::HttpAborted)
            }
            fn cancel(&self, _: HostOperationId) {}
            fn dispose(&self, _: PlatformHostResult) {
                self.orphaned.store(true, Ordering::Release);
            }
            fn dispose_canceled(&self, _: PlatformHostResult) {
                self.aborted.store(true, Ordering::Release);
            }
            fn retire(&self) {}
        }
        let driver = Arc::new(Ready {
            aborted: AtomicBool::new(false),
            orphaned: AtomicBool::new(false),
        });
        let service = AgentPlatformService::new(driver.clone());
        service
            .submit(
                HostOperationId(10),
                PlatformHostRequest::HttpStart {
                    request: PlatformHttpRequest {
                        upgrade_mixed_image: false,
                        fetch: Some(FetchOptions {
                            keepalive: true,
                            privileged_range: false,
                            cache: Default::default(),
                            mode: "cors".into(),
                            redirect: "follow".into(),
                            integrity: String::new(),
                        }),
                        use_cors_preflight: false,
                        method: "POST".into(),
                        url: "https://a.test/report".into(),
                        headers: Vec::new(),
                        body: vec![1],
                        credentials: "omit".into(),
                        initiator_url: None,
                        referrer_policy: None,
                    },
                },
            )
            .unwrap();
        service.drain(Duration::from_secs(2)).unwrap();
        service.cancel(HostOperationId(10)).unwrap();
        assert!(driver.aborted.load(Ordering::Acquire));
        assert!(!driver.orphaned.load(Ordering::Acquire));
        service.retire();
    }
    #[test]
    fn admission_returns_before_io_and_cancel_keeps_one_terminal_completion() {
        let executor = executor();
        let service = AgentPlatformService::new(executor.clone());
        service.submit(HostOperationId(1), request()).unwrap();
        assert!(service.poll(1).unwrap().is_empty());
        assert!(service.submit(HostOperationId(1), request()).is_err());
        service.cancel(HostOperationId(1)).unwrap();
        service.drain(Duration::from_secs(1)).unwrap();
        let completion = service.poll(1).unwrap();
        assert_eq!(completion.len(), 1);
        assert!(completion[0].result.is_err());
        assert!(service.poll(1).unwrap().is_empty());
        service.retire();
    }
    #[test]
    fn cancel_after_io_before_delivery_disposes_undelivered_resource() {
        let executor = executor();
        *executor.release.lock().unwrap() = true;
        let service = AgentPlatformService::new(executor.clone());
        service.submit(HostOperationId(2), request()).unwrap();
        service.drain(Duration::from_secs(1)).unwrap();
        service.cancel(HostOperationId(2)).unwrap();
        assert!(service.poll(1).unwrap()[0].result.is_err());
        assert_eq!(executor.disposed.load(Ordering::SeqCst), 1);
        service.retire();
    }
    #[test]
    fn retirement_cancels_accepted_io_and_discards_late_completions() {
        let executor = executor();
        let service = AgentPlatformService::new(executor.clone());
        service.submit(HostOperationId(3), request()).unwrap();
        service.retire();
        service.drain(Duration::from_secs(1)).unwrap();
        assert!(service.poll(1).is_err());
        assert!(service.submit(HostOperationId(4), request()).is_err());
        assert!(service.shared.state.lock().unwrap().pending.is_empty());
    }
}

#[cfg(test)]
mod retirement_cleanup_tests {
    use super::*;
    struct Cleanup {
        attempts: std::sync::atomic::AtomicUsize,
    }
    impl AgentOperationExecutor for Cleanup {
        fn execute(
            &self,
            _: HostOperationId,
            _: PlatformHostRequest,
            _: Arc<AtomicBool>,
        ) -> Result<PlatformHostResult, String> {
            Ok(PlatformHostResult::HttpAborted)
        }
        fn cancel(&self, _: HostOperationId) {}
        fn dispose(&self, _: PlatformHostResult) {}
        fn retire(&self) {}
        fn finish_retirement(&self) -> Result<(), String> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err("retained native cleanup".into())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn retirement_cleanup_failure_is_reported_and_retried_after_operation_join() {
        let driver = Arc::new(Cleanup {
            attempts: std::sync::atomic::AtomicUsize::new(0),
        });
        let service = AgentPlatformService::new(driver.clone());
        service.drain(Duration::from_secs(1)).unwrap();
        assert_eq!(driver.attempts.load(Ordering::SeqCst), 0);
        service.retire();
        assert!(service.drain(Duration::from_secs(1)).is_err());
        service.drain(Duration::from_secs(1)).unwrap();
        assert_eq!(driver.attempts.load(Ordering::SeqCst), 2);
    }
}
