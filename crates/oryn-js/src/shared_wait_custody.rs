//! Bounded native wait RPC custody. Isolate threads never perform broker IO.
//! A fixed executor retains each admission, including abandoned late replies,
//! until its waiter and optional receiver lease have been acknowledged cleaned.
use crate::{shared_memory::*, worker_wire::*};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
const CAPACITY: usize = 256;
const LANES: usize = 4;
struct State {
    initial: Option<SharedWaitResult>,
    result: Option<SharedWaitResult>,
    waiter: Option<SharedWaitId>,
    abandoned: bool,
    queued: bool,
    running: bool,
    done: bool,
    cleanup_error: bool,
    retry_after: Option<Instant>,
}
struct Job {
    transport: Arc<dyn WorkerTransport>,
    creator: Creator,
    descriptor: SharedMemoryDescriptor,
    backing: Mutex<Option<Arc<dyn Send + Sync>>>,
    acquire: bool,
    offset: u64,
    width64: bool,
    expected: u64,
    admission_id: WaitAdmissionId,
    deadline: Option<Instant>,
    state: Mutex<State>,
    changed: Condvar,
}
struct Source {
    transport: std::sync::Weak<dyn WorkerTransport>,
    creator: Creator,
    lane: [u8; 32],
    next: u64,
}
struct Queue {
    jobs: Vec<Arc<Job>>,
    ready: VecDeque<Arc<Job>>,
    sources: Vec<Source>,
}
struct Executor {
    queue: Mutex<Queue>,
    ready: Condvar,
    stopping: std::sync::atomic::AtomicBool,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
static EXECUTOR: OnceLock<Result<Arc<Executor>, String>> = OnceLock::new();
fn executor() -> Result<&'static Arc<Executor>, String> {
    EXECUTOR
        .get_or_init(|| {
            let pool = Arc::new(Executor {
                queue: Mutex::new(Queue {
                    jobs: Vec::new(),
                    ready: VecDeque::new(),
                    sources: Vec::new(),
                }),
                ready: Condvar::new(),
                stopping: std::sync::atomic::AtomicBool::new(false),
                threads: Mutex::new(Vec::new()),
            });
            for index in 0..LANES {
                let worker = pool.clone();
                // Fixed process-owned lanes, never one thread per author request.
                match std::thread::Builder::new()
                    .name(format!("oryn-wait-custody-{index}"))
                    .spawn(move || worker.run())
                {
                    Ok(thread) => pool
                        .threads
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(thread),
                    Err(error) => {
                        pool.stopping
                            .store(true, std::sync::atomic::Ordering::Release);
                        pool.ready.notify_all();
                        for thread in pool
                            .threads
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .drain(..)
                        {
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
impl Executor {
    fn schedule(&self, job: &Arc<Job>) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.queued && !state.running && !state.done {
            state.queued = true;
            queue.ready.push_back(job.clone());
            self.ready.notify_one();
        }
    }
    fn run(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                while queue.ready.is_empty() {
                    if self.stopping.load(std::sync::atomic::Ordering::Acquire) {
                        return;
                    }
                    queue = self
                        .ready
                        .wait_timeout(queue, Duration::from_millis(100))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                    let retries: Vec<_> = queue
                        .jobs
                        .iter()
                        .filter_map(|job| {
                            let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                            if state.abandoned
                                && state.cleanup_error
                                && !state.running
                                && !state.queued
                                && state.retry_after.is_none_or(|at| Instant::now() >= at)
                            {
                                state.cleanup_error = false;
                                state.queued = true;
                                Some(job.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                    queue.ready.extend(retries);
                }
                let job = queue.ready.pop_front().unwrap();
                {
                    let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.queued = false;
                    state.running = true;
                }
                job
            };
            // Transport panics do not discard the registered cleanup owner.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.step())).is_err() {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                state.result = Some(SharedWaitResult::Cancelled);
                state.initial.get_or_insert(SharedWaitResult::Cancelled);
                state.cleanup_error = true;
            }
            let (done, again) = {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                state.running = false;
                if state.cleanup_error {
                    state.retry_after = Instant::now().checked_add(Duration::from_millis(100));
                }
                job.changed.notify_all();
                (
                    state.done,
                    state.abandoned && !state.cleanup_error && !state.done,
                )
            };
            if done {
                self.queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .jobs
                    .retain(|entry| !Arc::ptr_eq(entry, &job));
            } else if again {
                self.schedule(&job);
            }
        }
    }
}
impl Job {
    fn request(&self, request: SharedMemoryRequest) -> Result<SharedMemoryResponse, String> {
        match self
            .transport
            .request(AgentServiceRequest::SharedMemory {
                creator: self.creator.clone(),
                request,
            })
            .map_err(|e| e.message)?
        {
            AgentServiceResponse::SharedMemory(response) => Ok(response),
            _ => Err("wait custody response mismatch".into()),
        }
    }
    fn step(&self) {
        let (initial, abandoned, waiter) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.initial, state.abandoned, state.waiter)
        };
        if abandoned {
            // A client-known native lane sequence makes cancellation meaningful
            // even if the admission reply was lost before its waiter ID arrived.
            if self
                .request(SharedMemoryRequest::CancelAdmission {
                    id: self.admission_id,
                })
                .is_err()
            {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .cleanup_error = true;
                return;
            }
            // Release the retained physical mapping on this owned IO lane,
            // never as a side effect of dropping the final V8 ticket handle.
            let backing = self
                .backing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            drop(backing);
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.waiter = None;
            state.done = true;
            return;
        }
        if initial.is_none() {
            let timeout_ns = self.deadline.map(|deadline| {
                u64::try_from(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .as_nanos(),
                )
                .unwrap_or(u64::MAX)
            });
            let result = match self.request(SharedMemoryRequest::WaitAdmission {
                id: self.admission_id,
                descriptor: self.descriptor,
                acquire: self.acquire,
                offset: self.offset,
                width64: self.width64,
                expected: self.expected,
                timeout_ns,
            }) {
                Ok(SharedMemoryResponse {
                    outcome: SharedMemoryOutcome::Wait(result),
                    ..
                }) => result,
                _ => SharedWaitResult::Cancelled,
            };
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.initial = Some(result);
            state.result = Some(result);
            if let SharedWaitResult::Pending(id) = result {
                state.waiter = Some(id);
            }
        } else if let Some(id) = waiter {
            let result = match self.request(SharedMemoryRequest::WaitPoll { id }) {
                Ok(SharedMemoryResponse {
                    outcome: SharedMemoryOutcome::Wait(result),
                    ..
                }) => Some(result),
                _ => None,
            };
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.result = Some(result.unwrap_or(SharedWaitResult::Cancelled));
            // Lost replies retain the token for an explicit cancel acknowledgement.
            if result.is_some_and(|result| !matches!(result, SharedWaitResult::Pending(_))) {
                state.waiter = None;
            }
        }
    }
}
/// Dropping this handle marks abandonment; the fixed executor retains custody.
pub struct WaitTicket {
    job: Arc<Job>,
}
impl WaitTicket {
    pub fn begin(
        backing: Arc<dyn Send + Sync>,
        transport: Arc<dyn WorkerTransport>,
        creator: Creator,
        descriptor: SharedMemoryDescriptor,
        acquire: bool,
        offset: u64,
        width64: bool,
        expected: u64,
        timeout_ns: Option<u64>,
    ) -> Result<Self, String> {
        let pool = executor()?;
        let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.jobs.len() >= CAPACITY {
            return Err("shared wait custody capacity exhausted".into());
        }
        queue
            .sources
            .retain(|source| source.transport.strong_count() > 0);
        let index = match queue.sources.iter().position(|source| {
            source.creator == creator
                && source
                    .transport
                    .upgrade()
                    .is_some_and(|existing| Arc::ptr_eq(&existing, &transport))
        }) {
            Some(index) => index,
            None => {
                if queue.sources.len() >= 4096 {
                    return Err("native wait source capacity exhausted".into());
                }
                let mut lane = [0; 32];
                getrandom::fill(&mut lane).map_err(|e| e.to_string())?;
                if lane == [0; 32] || queue.sources.iter().any(|source| source.lane == lane) {
                    return Err("native wait lane identity collision".into());
                }
                queue.sources.push(Source {
                    transport: Arc::downgrade(&transport),
                    creator: creator.clone(),
                    lane,
                    next: 1,
                });
                queue.sources.len() - 1
            }
        };
        let source = &mut queue.sources[index];
        let sequence = source.next;
        source.next = source
            .next
            .checked_add(1)
            .ok_or("wait lane sequence exhausted")?;
        let admission_id = WaitAdmissionId {
            lane: source.lane,
            sequence,
        };
        let job = Arc::new(Job {
            backing: Mutex::new(Some(backing)),
            transport,
            creator,
            descriptor,
            acquire,
            admission_id,
            offset,
            width64,
            expected,
            deadline: timeout_ns
                .and_then(|ns| Instant::now().checked_add(Duration::from_nanos(ns))),
            state: Mutex::new(State {
                initial: None,
                result: None,
                waiter: None,
                abandoned: false,
                queued: true,
                running: false,
                done: false,
                cleanup_error: false,
                retry_after: None,
            }),
            changed: Condvar::new(),
        });
        queue.jobs.push(job.clone());
        queue.ready.push_back(job.clone());
        pool.ready.notify_one();
        Ok(Self { job })
    }
    pub fn admission(&self, budget: Duration) -> Option<SharedWaitResult> {
        self.read(budget, true)
    }
    pub fn poll(&self, budget: Duration) -> Option<SharedWaitResult> {
        if let Ok(pool) = executor() {
            pool.schedule(&self.job);
        }
        self.read(budget, false)
    }
    fn read(&self, budget: Duration, initial: bool) -> Option<SharedWaitResult> {
        let mut state = self.job.state.lock().unwrap_or_else(|e| e.into_inner());
        let budget = budget.min(Duration::from_millis(50));
        if (initial && state.initial.is_none()) || (!initial && (state.running || state.queued)) {
            state = self
                .job
                .changed
                .wait_timeout(state, budget)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if initial { state.initial } else { state.result }
    }
}
impl Drop for WaitTicket {
    fn drop(&mut self) {
        self.job
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .abandoned = true;
        if let Ok(pool) = executor() {
            pool.schedule(&self.job);
        }
    }
}
/// After revoking a route, callers can retry cleanup without losing failed
/// ownership. A timeout keeps all jobs registered and returns a real failure.
pub fn drain(transport: &Arc<dyn WorkerTransport>, budget: Duration) -> Result<(), String> {
    let Some(Ok(pool)) = EXECUTOR.get() else {
        return Ok(());
    };
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    loop {
        let jobs: Vec<_> = pool
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .iter()
            .filter(|job| Arc::ptr_eq(&job.transport, transport))
            .cloned()
            .collect();
        if jobs.is_empty() {
            return Ok(());
        }
        for job in &jobs {
            {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                state.abandoned = true;
                state.cleanup_error = false;
            }
            pool.schedule(job);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err("shared wait custody drain is still pending".into());
        };
        let state = jobs[0].state.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            jobs[0]
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    struct Broker {
        calls: Mutex<Vec<SharedMemoryRequest>>,
        entered: mpsc::SyncSender<()>,
        unblock: Mutex<mpsc::Receiver<()>>,
        fail_cancel: Mutex<bool>,
    }
    impl WorkerTransport for Broker {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            let AgentServiceRequest::SharedMemory { request, .. } = request else {
                panic!("wrong request")
            };
            self.calls.lock().unwrap().push(request.clone());
            let outcome = match request {
                SharedMemoryRequest::WaitAdmission { .. } => {
                    self.entered.send(()).unwrap();
                    self.unblock.lock().unwrap().recv().unwrap();
                    SharedMemoryOutcome::Wait(SharedWaitResult::Pending(SharedWaitId([4; 32])))
                }
                SharedMemoryRequest::WaitPoll { .. } => {
                    SharedMemoryOutcome::Wait(SharedWaitResult::Ok)
                }
                SharedMemoryRequest::CancelAdmission { .. } => {
                    if std::mem::take(&mut *self.fail_cancel.lock().unwrap()) {
                        return Err(WorkerError::new(
                            "NetworkError",
                            "injected cancellation reply loss",
                        ));
                    }
                    SharedMemoryOutcome::None
                }
                _ => SharedMemoryOutcome::None,
            };
            Ok(AgentServiceResponse::SharedMemory(SharedMemoryResponse {
                isolated: true,
                outcome,
                descriptor: None,
                native_descriptor: None,
            }))
        }
    }
    fn descriptor() -> SharedMemoryDescriptor {
        SharedMemoryDescriptor {
            token: SharedMemoryToken([1; 32]),
            byte_length: 4,
            mapped_length: 8192,
            maximum_length: 4,
            data_offset: 4096,
            reserved_length: 4096,
            guard_length: 0,
            wasm: false,
            memory64: false,
            growable: false,
        }
    }
    fn setup(
        fail_cancel: bool,
    ) -> (
        Arc<Broker>,
        Arc<dyn WorkerTransport>,
        mpsc::Receiver<()>,
        mpsc::SyncSender<()>,
    ) {
        let (entered, rx) = mpsc::sync_channel(1);
        let (tx, unblock) = mpsc::sync_channel(1);
        let broker = Arc::new(Broker {
            calls: Mutex::new(Vec::new()),
            entered,
            unblock: Mutex::new(unblock),
            fail_cancel: Mutex::new(fail_cancel),
        });
        (broker.clone(), broker, rx, tx)
    }
    #[test]
    fn interrupted_admission_retains_late_waiter_until_cancel_acknowledgement() {
        let (broker, transport, entered, unblock) = setup(true);
        let ticket = WaitTicket::begin(
            Arc::new(()),
            transport.clone(),
            Creator::Agent(AgentId(19)),
            descriptor(),
            false,
            0,
            false,
            0,
            None,
        )
        .unwrap();
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(ticket.admission(Duration::ZERO), None);
        drop(ticket); // does not wait for the blocked RPC and cannot lose it
        assert!(drain(&transport, Duration::ZERO).is_err());
        unblock.send(()).unwrap();
        drain(&transport, Duration::from_secs(1)).unwrap();
        let calls = broker.calls.lock().unwrap();
        assert!(matches!(
            calls.first(),
            Some(SharedMemoryRequest::WaitAdmission { .. })
        ));
        assert_eq!(
            calls
                .iter()
                .filter(|request| matches!(request, SharedMemoryRequest::CancelAdmission { .. }))
                .count(),
            2
        );
    }
    #[test]
    fn asynchronous_classification_survives_notification_before_first_poll() {
        let (broker, transport, entered, unblock) = setup(false);
        let ticket = WaitTicket::begin(
            Arc::new(()),
            transport.clone(),
            Creator::Agent(AgentId(20)),
            descriptor(),
            false,
            0,
            false,
            0,
            None,
        )
        .unwrap();
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        unblock.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while ticket.admission(Duration::from_millis(10)).is_none() {
            assert!(Instant::now() < deadline);
        }
        assert!(matches!(
            ticket.admission(Duration::ZERO),
            Some(SharedWaitResult::Pending(_))
        ));
        while ticket.poll(Duration::from_millis(10)) != Some(SharedWaitResult::Ok) {
            assert!(Instant::now() < deadline);
        }
        assert!(matches!(
            ticket.admission(Duration::ZERO),
            Some(SharedWaitResult::Pending(_))
        ));
        drop(ticket);
        drain(&transport, Duration::from_secs(1)).unwrap();
        assert_eq!(
            broker
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|r| matches!(r, SharedMemoryRequest::CancelAdmission { .. }))
                .count(),
            1,
            "completed admission still releases its native receiver alias"
        );
    }
}
