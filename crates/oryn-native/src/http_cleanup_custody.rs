//! Retained cancellation ownership for parent HTTP operations. Reservations are
//! bounded before IO starts; cancellation never runs in a ticket destructor.
use crate::{
    network::{NetworkBroker, SharedNetworkBroker},
    runtime::RuntimeError,
};
use std::sync::{Condvar, OnceLock};
use std::time::Instant;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct Scope(Arc<ScopeState>);
struct ScopeState {
    retired: AtomicBool,
    forced: AtomicBool,
}
impl Default for Scope {
    fn default() -> Self {
        Self(Arc::new(ScopeState {
            retired: AtomicBool::new(false),
            forced: AtomicBool::new(false),
        }))
    }
}
struct State {
    id: Option<u64>,
    cancel: bool,
    running: bool,
    done: bool,
    retry: Instant,
}
struct Job {
    protected_keepalive: bool,
    scope: Scope,
    public: u64,
    network: SharedNetworkBroker,
    state: Mutex<State>,
    cancelled: AtomicBool,
    keepalive: Mutex<Option<Arc<crate::keepalive::Lease>>>,
}
struct Queue {
    jobs: Vec<Arc<Job>>,
    stopping: bool,
}
struct Pool {
    queue: Mutex<Queue>,
    changed: Condvar,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
static POOL: OnceLock<Result<Arc<Pool>, String>> = OnceLock::new();
fn pool() -> Result<&'static Arc<Pool>, String> {
    POOL.get_or_init(|| {
        let pool = Arc::new(Pool {
            queue: Mutex::new(Queue {
                jobs: Vec::new(),
                stopping: false,
            }),
            changed: Condvar::new(),
            threads: Mutex::new(Vec::new()),
        });
        for index in 0..4 {
            let worker = pool.clone();
            match std::thread::Builder::new()
                .name(format!("oryn-http-cleanup-{index}"))
                .spawn(move || worker.run())
            {
                Ok(thread) => pool
                    .threads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(thread),
                Err(error) => {
                    pool.queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .stopping = true;
                    pool.changed.notify_all();
                    let threads = std::mem::take(
                        &mut *pool.threads.lock().unwrap_or_else(|e| e.into_inner()),
                    );
                    for thread in threads {
                        let _ = thread.join();
                    }
                    return Err(error.to_string());
                }
            }
        }
        Ok(pool)
    })
    .as_ref()
    .map_err(Clone::clone)
}
pub(crate) fn ready() -> Result<(), RuntimeError> {
    let pool = pool().map_err(RuntimeError::ContextService)?;
    if pool
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stopping
    {
        Err(RuntimeError::Closed)
    } else {
        Ok(())
    }
}
impl Pool {
    fn run(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if queue.stopping {
                        return;
                    }
                    if let Some(job) = queue.jobs.iter().find_map(|job| {
                        let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                        if state.id.is_some()
                            && state.cancel
                            && !state.running
                            && !state.done
                            && state.retry <= Instant::now()
                        {
                            state.running = true;
                            Some(job.clone())
                        } else {
                            None
                        }
                    }) {
                        break job;
                    }
                    queue = self
                        .changed
                        .wait_timeout(queue, Duration::from_millis(100))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            };
            let id = job
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .id
                .expect("selected attached HTTP job");
            let acknowledged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                job.network.cancel_operation(id).is_ok()
            }))
            .unwrap_or(false);
            {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                state.running = false;
                state.done = acknowledged;
                state.retry = Instant::now() + Duration::from_millis(100);
            }
            if acknowledged {
                job.keepalive
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                self.remove(&job);
            }
            self.changed.notify_all();
        }
    }
    fn remove(&self, job: &Arc<Job>) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .retain(|other| !Arc::ptr_eq(other, job));
        self.changed.notify_all();
    }
}
pub(crate) struct Ticket {
    job: Arc<Job>,
}
impl Ticket {
    pub(crate) fn reserve(
        scope: &Scope,
        public: u64,
        network: SharedNetworkBroker,
    ) -> Result<Self, RuntimeError> {
        Self::reserve_inner(scope, public, network, false)
    }
    /// Only a natively frozen, previously accepted keepalive may use this after
    /// the document scope seals. Explicit request cancellation still applies.
    pub(crate) fn reserve_keepalive(
        scope: &Scope,
        public: u64,
        network: SharedNetworkBroker,
    ) -> Result<Self, RuntimeError> {
        Self::reserve_inner(scope, public, network, true)
    }
    fn reserve_inner(
        scope: &Scope,
        public: u64,
        network: SharedNetworkBroker,
        protected_keepalive: bool,
    ) -> Result<Self, RuntimeError> {
        let pool = pool().map_err(RuntimeError::ContextService)?;
        let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
        if scope.0.forced.load(Ordering::Acquire)
            || (scope.0.retired.load(Ordering::Acquire) && !protected_keepalive)
        {
            return Err(RuntimeError::Closed);
        }
        if queue.jobs.len() >= 1024 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "HTTP cancellation custody".into(),
            });
        }
        let job = Arc::new(Job {
            protected_keepalive,
            scope: scope.clone(),
            public,
            network,
            state: Mutex::new(State {
                id: None,
                cancel: false,
                running: false,
                done: false,
                retry: Instant::now(),
            }),
            cancelled: AtomicBool::new(false),
            keepalive: Mutex::new(None),
        });
        queue.jobs.push(job.clone());
        Ok(Self { job })
    }
    pub(crate) fn attach(&self, id: u64) {
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            state.id.is_none() && !state.done,
            "one HTTP operation per ticket"
        );
        state.id = Some(id);
        drop(state);
        if let Some(Ok(pool)) = POOL.get() {
            pool.changed.notify_all();
        }
    }
    pub(crate) fn retain_keepalive(&self, lease: Arc<crate::keepalive::Lease>) {
        *self
            .job
            .keepalive
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(lease);
    }
    pub(crate) fn keepalive(&self) -> Option<Arc<crate::keepalive::Lease>> {
        self.job
            .keepalive
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    /// Only call after the complete response has reached EOF. A failed or
    /// canceled operation instead retains this owner until cancellation ACK.
    pub(crate) fn finish_keepalive(&self) {
        self.job
            .keepalive
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
    pub(crate) fn cancel(&self) {
        self.job.cancelled.store(true, Ordering::Release);
        self.job
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel = true;
        if let Some(Ok(pool)) = POOL.get() {
            pool.changed.notify_all();
        }
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.job.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn cancellation(&self) -> &AtomicBool {
        &self.job.cancelled
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.job.cancelled.store(true, Ordering::Release);
        let unattached = {
            let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
            state.cancel = true;
            state.id.is_none()
        };
        if let Some(Ok(pool)) = POOL.get() {
            if unattached {
                pool.remove(&self.job);
            }
            pool.changed.notify_all();
        }
    }
}
impl Scope {
    /// Observe retained ownership without creating workers, cancelling work, or
    /// waiting for a broker acknowledgement. Unattached admissions count too.
    pub(crate) fn is_drained(&self) -> Result<bool, RuntimeError> {
        let pool = match POOL.get() {
            None => return Ok(true),
            Some(Ok(pool)) => pool,
            Some(Err(error)) => return Err(RuntimeError::ContextService(error.clone())),
        };
        let queue = pool.queue.lock().map_err(|_| {
            RuntimeError::ContextService("HTTP cancellation custody poisoned".into())
        })?;
        Ok(!queue
            .jobs
            .iter()
            .any(|job| Arc::ptr_eq(&self.0, &job.scope.0)))
    }
    pub(crate) fn retire_preserving_keepalive(&self) {
        self.0.retired.store(true, Ordering::Release);
        if let Some(Ok(pool)) = POOL.get() {
            let queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
            for job in &queue.jobs {
                if Arc::ptr_eq(&self.0, &job.scope.0) && !job.protected_keepalive {
                    job.cancelled.store(true, Ordering::Release);
                    job.state.lock().unwrap_or_else(|e| e.into_inner()).cancel = true;
                }
            }
            pool.changed.notify_all();
        }
    }
    pub(crate) fn cancel(&self, public: Option<u64>) -> bool {
        if public.is_none() {
            self.0.forced.store(true, Ordering::Release);
            self.0.retired.store(true, Ordering::Release);
        }
        let mut found = false;
        if let Some(Ok(pool)) = POOL.get() {
            let queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
            for job in &queue.jobs {
                if Arc::ptr_eq(&self.0, &job.scope.0) && public.is_none_or(|id| id == job.public) {
                    found = true;
                    job.cancelled.store(true, Ordering::Release);
                    job.state.lock().unwrap_or_else(|e| e.into_inner()).cancel = true;
                }
            }
            pool.changed.notify_all();
        }
        found
    }
    pub(crate) fn drain(&self, budget: Duration) -> Result<(), RuntimeError> {
        let Some(Ok(pool)) = POOL.get() else {
            return Ok(());
        };
        let deadline = Instant::now() + budget;
        let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
        while queue
            .jobs
            .iter()
            .any(|job| Arc::ptr_eq(&self.0, &job.scope.0))
        {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(RuntimeError::ContextService(
                    "HTTP cancellation acknowledgement pending".into(),
                ));
            };
            queue = pool
                .changed
                .wait_timeout(queue, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{
        NetworkBroker, NetworkError, NetworkOperation, NetworkRequest, NetworkResponse,
    };
    struct BlockingCancel {
        attempts: Arc<std::sync::atomic::AtomicUsize>,
        started: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        senders: Mutex<Vec<tokio::sync::mpsc::Sender<crate::network::NetworkEvent>>>,
    }
    impl NetworkBroker for BlockingCancel {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!()
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(1);
            self.senders.lock().unwrap().push(sender);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }
        fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
            if self.attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                return Err(NetworkError::Transport("injected cleanup failure".into()));
            }
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            Ok(())
        }
    }
    #[test]
    fn drop_retains_failed_cancellation_without_waiting_and_exact_scope_drains_after_ack() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (started, entered) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let network = SharedNetworkBroker::new(BlockingCancel {
            attempts: attempts.clone(),
            started,
            release: Mutex::new(wait),
            senders: Mutex::new(Vec::new()),
        });
        let scope = Scope::default();
        let other = Scope::default();
        assert!(scope.is_drained().unwrap());
        assert!(other.is_drained().unwrap());
        let pending = Ticket::reserve(&other, 41, network.clone()).unwrap();
        let ticket = Ticket::reserve(&scope, 41, network.clone()).unwrap();
        let budget = crate::keepalive::Budget::default();
        let owner = crate::agent_registry::AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        ticket.retain_keepalive(Arc::new(budget.reserve(owner.clone(), 64 * 1024).unwrap()));
        let operation = network
            .start_operation(NetworkRequest {
                request_id: 41,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: false,
                security: Default::default(),
                resolved_addrs: Vec::new(),
            })
            .unwrap();
        ticket.attach(operation.request_id);
        drop(ticket);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            budget.reserve(owner.clone(), 1).is_err(),
            "failed cancellation retains the body charge"
        );
        assert!(!scope.is_drained().unwrap());
        assert!(!other.is_drained().unwrap());
        assert!(scope.drain(Duration::ZERO).is_err());
        assert!(other.drain(Duration::ZERO).is_err());
        release.send(()).unwrap();
        scope.drain(Duration::from_secs(2)).unwrap();
        assert!(
            budget.reserve(owner, 64 * 1024).is_ok(),
            "typed cancellation ACK releases the body charge"
        );
        assert!(scope.is_drained().unwrap());
        assert!(!other.is_drained().unwrap());
        assert!(attempts.load(Ordering::Acquire) >= 2);
        assert!(other.drain(Duration::ZERO).is_err());
        drop(pending);
        other.drain(Duration::from_secs(1)).unwrap();
        assert!(other.is_drained().unwrap());
        drop(operation);
    }
    #[test]
    fn document_retirement_preserves_only_prepared_keepalive_and_explicit_abort_still_cancels() {
        let scope = Scope::default();
        let network = SharedNetworkBroker::new(crate::network::UnavailableBroker);
        let ordinary = Ticket::reserve(&scope, 101, network.clone()).unwrap();
        let keepalive = Ticket::reserve_keepalive(&scope, 102, network.clone()).unwrap();
        scope.retire_preserving_keepalive();
        assert!(ordinary.cancelled());
        assert!(!keepalive.cancelled());
        assert!(Ticket::reserve(&scope, 103, network.clone()).is_err());
        let redirect = Ticket::reserve_keepalive(&scope, 102, network).unwrap();
        assert!(!redirect.cancelled());
        assert!(scope.cancel(Some(102)));
        assert!(keepalive.cancelled());
        assert!(redirect.cancelled());
        scope.cancel(None);
        assert!(
            Ticket::reserve_keepalive(
                &scope,
                104,
                SharedNetworkBroker::new(crate::network::UnavailableBroker)
            )
            .is_err(),
            "forced cancellation cannot reopen a protected admission"
        );
        drop((ordinary, keepalive, redirect));
        scope.drain(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn cancellation_before_attachment_remains_owned_without_a_premature_ack() {
        let scope = Scope::default();
        let network = SharedNetworkBroker::new(crate::network::UnavailableBroker);
        let ticket = Ticket::reserve(&scope, 7, network.clone()).unwrap();
        scope.cancel(Some(7));
        assert!(ticket.cancelled());
        assert!(scope.drain(Duration::ZERO).is_err());
        drop(ticket);
        scope.drain(Duration::from_secs(1)).unwrap();
        scope.cancel(None);
        assert!(Ticket::reserve(&scope, 8, network).is_err());
    }
}
