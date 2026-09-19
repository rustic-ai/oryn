//! Owned execution thread and bounded queues for an independent worker isolate.
//! The runtime factory must allocate native state only: author code starts in
//! `run`, after its interrupt handle has been published to the parent.
use crate::agent_registry::AgentProcess;
use oryn_js::worker_wire::{AgentBootstrap, AgentCommand, AgentEvent, WorkerError};
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub trait AgentRuntime: 'static {
    /// Must interrupt execution without entering the isolate or waiting for its
    /// event loop. V8 implementations use the isolate's thread-safe handle.
    fn interrupt(&self) -> Arc<dyn Fn() + Send + Sync>;
    fn cleanup(&self) -> Arc<dyn Fn(Duration) -> Result<(), WorkerError> + Send + Sync> {
        Arc::new(|_| Ok(()))
    }
    fn run(self, endpoint: AgentEndpoint) -> Result<(), WorkerError>;
}

struct Queue<T> {
    values: VecDeque<(T, usize)>,
    bytes: usize,
}
impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            values: VecDeque::new(),
            bytes: 0,
        }
    }
}
struct State {
    commands: Queue<AgentCommand>,
    events: Queue<AgentEvent>,
    admitted: bool,
    stopping: bool,
    finished: bool,
    failure: Option<WorkerError>,
    suspend_requested: bool,
    suspend_acknowledged: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    max_bytes: usize,
    max_messages: usize,
}
fn invalid(message: &str) -> WorkerError {
    WorkerError::new("InvalidStateError", message)
}
fn quota() -> WorkerError {
    WorkerError::new("QuotaExceededError", "worker mailbox limit")
}

// Count the actual wire encoding without creating another attacker-sized copy.
struct Counter {
    bytes: usize,
    limit: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= self.limit)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "worker wire limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) fn wire_size(value: &impl Serialize, limit: usize) -> Result<usize, WorkerError> {
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| quota())?;
    Ok(counter.bytes)
}
fn enqueue<T>(
    queue: &mut Queue<T>,
    value: T,
    bytes: usize,
    shared: &Shared,
) -> Result<(), WorkerError> {
    let total = queue.bytes.checked_add(bytes).ok_or_else(quota)?;
    if queue.values.len() >= shared.max_messages || total > shared.max_bytes {
        return Err(quota());
    }
    queue.values.push_back((value, bytes));
    queue.bytes = total;
    Ok(())
}
fn dequeue<T>(queue: &mut Queue<T>) -> Option<T> {
    let (value, bytes) = queue.values.pop_front()?;
    queue.bytes -= bytes;
    Some(value)
}

/// Owned by the isolate thread. No author-visible capability can manufacture it.
#[derive(Clone)]
pub struct AgentEndpoint {
    shared: Arc<Shared>,
}
impl AgentEndpoint {
    /// A timeout permits autonomous timers/network work even with no commands.
    /// Stop is out of band and takes precedence over previously queued messages.
    pub fn receive(&self, timeout: Duration) -> Result<Option<AgentCommand>, WorkerError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        let (mut state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |state| {
                !state.stopping && state.commands.values.is_empty()
            })
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        if state.stopping {
            return Ok(Some(AgentCommand::Stop));
        }
        Ok(dequeue(&mut state.commands))
    }
    /// Safe task boundary: hold no V8 scope or parent broker lock while parked.
    /// Termination wakes this condition and remains independent of resume.
    pub fn suspension_checkpoint(&self) -> Result<bool, WorkerError> {
        self.suspension_checkpoint_with_elapsed()
            .map(|result| result.is_some())
    }
    pub fn suspension_checkpoint_with_elapsed(&self) -> Result<Option<Duration>, WorkerError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        let paused_at = state.suspend_requested.then(std::time::Instant::now);
        while state.suspend_requested && !state.stopping {
            state.suspend_acknowledged = true;
            self.shared.changed.notify_all();
            state = self
                .shared
                .changed
                .wait(state)
                .map_err(|_| invalid("worker mailbox poisoned"))?;
        }
        state.suspend_acknowledged = false;
        self.shared.changed.notify_all();
        Ok(
            (!state.stopping)
                .then(|| paused_at.map_or(Duration::ZERO, |started| started.elapsed())),
        )
    }
    pub fn stopped(&self) -> bool {
        self.shared
            .state
            .lock()
            .map(|state| state.stopping)
            .unwrap_or(true)
    }
    pub fn emit(&self, event: AgentEvent) -> Result<(), WorkerError> {
        if let AgentEvent::Message { value, .. } = &event {
            value
                .validate()
                .map_err(|error| WorkerError::new("DataCloneError", error))?;
        }
        let bytes = wire_size(&event, self.shared.max_bytes)?;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        if state.stopping || state.finished {
            return Err(invalid("worker has stopped"));
        }
        enqueue(&mut state.events, event, bytes, &self.shared)?;
        self.shared.changed.notify_all();
        Ok(())
    }
}

pub struct AgentThreadProcess {
    shared: Arc<Shared>,
    interrupt: Arc<dyn Fn() + Send + Sync>,
    cleanup: Arc<dyn Fn(Duration) -> Result<(), WorkerError> + Send + Sync>,
    thread: Option<JoinHandle<()>>,
}
impl AgentThreadProcess {
    pub fn spawn<R: AgentRuntime>(
        bootstrap: AgentBootstrap,
        factory: impl FnOnce(AgentBootstrap) -> Result<R, WorkerError> + Send + 'static,
    ) -> Result<Self, WorkerError> {
        let mut process = Self::prepare(bootstrap, factory)?;
        process.start()?;
        Ok(process)
    }
    /// Allocate the runtime and publish its interrupt, but do not execute author
    /// source until the registry has committed the principal and client routes.
    pub fn prepare<R: AgentRuntime>(
        bootstrap: AgentBootstrap,
        factory: impl FnOnce(AgentBootstrap) -> Result<R, WorkerError> + Send + 'static,
    ) -> Result<Self, WorkerError> {
        if bootstrap.limits.queue_bytes == 0 || bootstrap.limits.queue_messages == 0 {
            return Err(quota());
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                commands: Queue::default(),
                events: Queue::default(),
                admitted: false,
                stopping: false,
                finished: false,
                failure: None,
                suspend_requested: false,
                suspend_acknowledged: false,
            }),
            changed: Condvar::new(),
            max_bytes: bootstrap.limits.queue_bytes,
            max_messages: bootstrap.limits.queue_messages,
        });
        let owner = shared.clone();
        let (publish, ready) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name(format!("oryn-agent-{}", bootstrap.id.0))
            .spawn(move || {
                let runtime = match factory(bootstrap) {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = publish.send(Err(error));
                        return;
                    }
                };
                if publish
                    .send(Ok((runtime.interrupt(), runtime.cleanup())))
                    .is_err()
                {
                    return;
                }
                {
                    let state = match owner.state.lock() {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                    let state = match owner
                        .changed
                        .wait_while(state, |state| !state.admitted && !state.stopping)
                    {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                    if state.stopping {
                        return;
                    }
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.run(AgentEndpoint {
                        shared: owner.clone(),
                    })
                }));
                if let Ok(mut state) = owner.state.lock() {
                    state.finished = true;
                    state.failure = match result {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(error),
                        Err(_) => Some(invalid("worker runtime panicked")),
                    };
                    if state.failure.is_none() && !state.stopping {
                        let closed = AgentEvent::Closed;
                        let queued = wire_size(&closed, owner.max_bytes)
                            .and_then(|bytes| enqueue(&mut state.events, closed, bytes, &owner));
                        if let Err(error) = queued {
                            state.failure = Some(error);
                        }
                    }
                    owner.changed.notify_all();
                }
            })
            .map_err(|error| invalid(&format!("worker thread launch failed: {error}")))?;
        match ready.recv() {
            Ok(Ok((interrupt, cleanup))) => Ok(Self {
                shared,
                interrupt,
                cleanup,
                thread: Some(thread),
            }),
            result => {
                let _ = thread.join();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => invalid("worker initialization failed"),
                })
            }
        }
    }
}
impl AgentProcess for AgentThreadProcess {
    fn execution_placement(&self) -> Option<crate::execution_placement::ExecutionPlacement> {
        Some(crate::execution_placement::ExecutionPlacement::thread())
    }
    fn start(&mut self) -> Result<(), WorkerError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        if state.stopping || state.finished {
            return Err(invalid("worker has stopped"));
        }
        state.admitted = true;
        self.shared.changed.notify_all();
        Ok(())
    }
    fn set_suspended(&mut self, suspended: bool) -> Result<(), WorkerError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        if state.stopping || state.finished {
            return Err(invalid("worker retired"));
        }
        state.suspend_requested = suspended;
        if !suspended {
            state.suspend_acknowledged = false;
        }
        self.shared.changed.notify_all();
        Ok(())
    }
    fn suspension_status(&mut self) -> Result<bool, WorkerError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        if state.stopping || state.finished {
            return Err(invalid("worker retired"));
        }
        Ok(state.suspend_requested && state.suspend_acknowledged)
    }
    fn send(&mut self, command: AgentCommand) -> Result<(), WorkerError> {
        if matches!(command, AgentCommand::Stop) {
            return self.terminate();
        }
        if let AgentCommand::Message { value, .. }
        | AgentCommand::Worklet(oryn_js::worker_wire::WorkletCommand::Construct {
            value,
            ..
        }) = &command
        {
            value
                .validate()
                .map_err(|error| WorkerError::new("DataCloneError", error))?;
        }
        let bytes = wire_size(&command, self.shared.max_bytes)?;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        if state.stopping || state.finished {
            return Err(invalid("worker has stopped"));
        }
        enqueue(&mut state.commands, command, bytes, &self.shared)?;
        self.shared.changed.notify_all();
        Ok(())
    }
    fn poll(&mut self, max: usize) -> Result<Vec<AgentEvent>, WorkerError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| invalid("worker mailbox poisoned"))?;
        // Accepted events precede the terminal runtime failure. The queue is
        // bounded and the failure remains available after its final receipt.
        if state.events.values.is_empty() {
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
        }
        let mut events = Vec::new();
        while events.len() < max {
            match dequeue(&mut state.events) {
                Some(event) => events.push(event),
                None => break,
            }
        }
        Ok(events)
    }
    fn terminate(&mut self) -> Result<(), WorkerError> {
        if self.thread.is_none() {
            return (self.cleanup)(Duration::from_secs(5));
        }
        let poisoned = {
            let (mut state, poisoned) = match self.shared.state.lock() {
                Ok(state) => (state, false),
                Err(error) => (error.into_inner(), true),
            };
            state.stopping = true;
            self.shared.changed.notify_all();
            poisoned
        };
        // Never invoke an isolate callback while holding its mailbox lock.
        (self.interrupt)();
        self.thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| invalid("worker teardown panicked"))?;
        let cleanup = (self.cleanup)(Duration::from_secs(5));
        if poisoned {
            Err(invalid("worker mailbox poisoned during teardown"))
        } else {
            cleanup
        }
    }
}
impl Drop for AgentThreadProcess {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::worker_wire::*;

    fn shared(messages: usize, bytes: usize) -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State {
                commands: Queue::default(),
                events: Queue::default(),
                admitted: false,
                stopping: false,
                finished: false,
                failure: None,
                suspend_requested: false,
                suspend_acknowledged: false,
            }),
            changed: Condvar::new(),
            max_bytes: bytes,
            max_messages: messages,
        })
    }
    #[test]
    fn suspension_acknowledges_only_checkpoint_and_stop_wakes_without_resume() {
        let shared = shared(4, 4096);
        {
            let mut state = shared.state.lock().unwrap();
            state.suspend_requested = true;
        }
        assert!(!shared.state.lock().unwrap().suspend_acknowledged);
        let endpoint = AgentEndpoint {
            shared: shared.clone(),
        };
        let thread = std::thread::spawn(move || endpoint.suspension_checkpoint());
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !shared.state.lock().unwrap().suspend_acknowledged {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        {
            let mut state = shared.state.lock().unwrap();
            state.stopping = true;
            shared.changed.notify_all();
        }
        assert!(!thread.join().unwrap().unwrap());
    }

    #[test]
    fn suspension_preserves_queued_commands_until_native_resume() {
        let shared = shared(4, 4096);
        {
            let mut state = shared.state.lock().unwrap();
            state.suspend_requested = true;
            enqueue(
                &mut state.commands,
                AgentCommand::CancelTask { id: 71 },
                32,
                &shared,
            )
            .unwrap();
        }
        let endpoint = AgentEndpoint {
            shared: shared.clone(),
        };
        let thread = std::thread::spawn(move || {
            assert!(endpoint.suspension_checkpoint().unwrap());
            endpoint.receive(Duration::ZERO)
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !shared.state.lock().unwrap().suspend_acknowledged {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        {
            let mut state = shared.state.lock().unwrap();
            assert_eq!(state.commands.values.len(), 1);
            state.suspend_requested = false;
            shared.changed.notify_all();
        }
        assert!(matches!(
            thread.join().unwrap().unwrap(),
            Some(AgentCommand::CancelTask { id: 71 })
        ));
    }

    #[test]
    fn mailbox_capacity_rejection_preserves_pending_events() {
        let shared = shared(1, 1024);
        let endpoint = AgentEndpoint {
            shared: shared.clone(),
        };
        endpoint.emit(AgentEvent::Ready).unwrap();
        assert_eq!(
            endpoint.emit(AgentEvent::Closed).unwrap_err().name,
            "QuotaExceededError"
        );
        let mut state = shared.state.lock().unwrap();
        assert_eq!(state.events.values.len(), 1);
        assert!(matches!(
            dequeue(&mut state.events),
            Some(AgentEvent::Ready)
        ));
        assert_eq!(state.events.bytes, 0);
    }
    #[test]
    fn cancellation_preempts_queued_messages() {
        let shared = shared(1, 1024);
        {
            let mut state = shared.state.lock().unwrap();
            enqueue(
                &mut state.commands,
                AgentCommand::Lifecycle {
                    id: 1,
                    event: LifecycleEvent::Install,
                    payload: None,
                },
                1,
                &shared,
            )
            .unwrap();
            state.stopping = true;
        }
        let endpoint = AgentEndpoint { shared };
        assert!(matches!(
            endpoint.receive(Duration::ZERO).unwrap(),
            Some(AgentCommand::Stop)
        ));
        assert!(endpoint.emit(AgentEvent::Ready).is_err());
    }
    #[test]
    fn wire_budget_counts_escaped_utf16_and_metadata() {
        let value = AgentEvent::Error {
            message: vec![0xd800; 32],
            filename: "worker.js".into(),
            line: 1,
            column: 1,
        };
        let encoded = serde_json::to_vec(&value).unwrap();
        assert!(wire_size(&value, encoded.len() - 1).is_err());
        assert_eq!(wire_size(&value, encoded.len()).unwrap(), encoded.len());
    }
    #[test]
    fn terminal_failure_follows_already_accepted_error_events() {
        struct FailedRuntime;
        impl AgentRuntime for FailedRuntime {
            fn interrupt(&self) -> Arc<dyn Fn() + Send + Sync> {
                Arc::new(|| {})
            }
            fn run(self, endpoint: AgentEndpoint) -> Result<(), WorkerError> {
                for line in [1, 2] {
                    endpoint.emit(AgentEvent::Error {
                        message: "source failure".encode_utf16().collect(),
                        filename: "worker.js".into(),
                        line,
                        column: 1,
                    })?;
                }
                Err(WorkerError::new("TypeError", "original startup failure"))
            }
        }
        let mut process =
            AgentThreadProcess::spawn(admission_bootstrap(), |_| Ok(FailedRuntime)).unwrap();
        process.thread.take().unwrap().join().unwrap();
        assert!(process.poll(0).unwrap().is_empty());
        for expected in [1, 2] {
            assert!(
                matches!(process.poll(1).unwrap().as_slice(),[AgentEvent::Error{line,..}] if *line==expected)
            );
        }
        assert_eq!(process.shared.state.lock().unwrap().events.bytes, 0);
        for _ in 0..2 {
            let error = process.poll(1).unwrap_err();
            assert_eq!(error.name, "TypeError");
            assert_eq!(error.message, "original startup failure");
        }
        process.terminate().unwrap();
    }

    struct BlockedRuntime {
        release: Arc<(Mutex<bool>, Condvar)>,
    }
    impl AgentRuntime for BlockedRuntime {
        fn interrupt(&self) -> Arc<dyn Fn() + Send + Sync> {
            let release = self.release.clone();
            Arc::new(move || {
                *release.0.lock().unwrap() = true;
                release.1.notify_all();
            })
        }
        fn run(self, _: AgentEndpoint) -> Result<(), WorkerError> {
            let released = self.release.0.lock().unwrap();
            drop(
                self.release
                    .1
                    .wait_while(released, |released| !*released)
                    .unwrap(),
            );
            Ok(())
        }
    }
    struct AdmissionRuntime {
        ran: Arc<std::sync::atomic::AtomicBool>,
    }
    impl AgentRuntime for AdmissionRuntime {
        fn interrupt(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn run(self, _: AgentEndpoint) -> Result<(), WorkerError> {
            self.ran.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }
    fn admission_bootstrap() -> AgentBootstrap {
        AgentBootstrap {
            cookie_ancestor_urls: Vec::new(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            policy: oryn_js::worker_wire::AgentPolicy::default(),
            top_level_url: None,
            id: AgentId(7),
            kind: AgentKind::Dedicated,
            partition: "context/site".into(),
            origin: AgentOrigin {
                serialized: "https://a.test".into(),
                authority: "origin:a".into(),
                opaque: false,
            },
            script_url: "https://a.test/w.js".into(),
            source: Vec::new(),
            modules: Default::default(),
            options: WorkerOptions::default(),
            referrer_policy: "no-referrer".into(),
            limits: AgentLimits::default(),
        }
    }
    #[test]
    fn prepared_runtime_cannot_execute_before_parent_admission() {
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = ran.clone();
        let mut process = AgentThreadProcess::prepare(admission_bootstrap(), move |_| {
            Ok(AdmissionRuntime { ran: probe })
        })
        .unwrap();
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!process.shared.state.lock().unwrap().admitted);
        process.start().unwrap();
        process.start().unwrap();
        process.thread.take().unwrap().join().unwrap();
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[test]
    fn rejected_admission_drops_runtime_without_author_execution() {
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = ran.clone();
        let mut process = AgentThreadProcess::prepare(admission_bootstrap(), move |_| {
            Ok(AdmissionRuntime { ran: probe })
        })
        .unwrap();
        process.terminate().unwrap();
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        assert!(process.start().is_err());
    }
    #[test]
    fn shutdown_interrupts_busy_runtime_without_command_dispatch() {
        let bootstrap = AgentBootstrap {
            cookie_ancestor_urls: Vec::new(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            policy: oryn_js::worker_wire::AgentPolicy::default(),
            top_level_url: None,
            id: AgentId(1),
            kind: AgentKind::Dedicated,
            partition: "context/site".into(),
            origin: AgentOrigin {
                serialized: "https://a.test".into(),
                authority: "origin:a".into(),
                opaque: false,
            },
            script_url: "https://a.test/worker.js".into(),
            source: Vec::new(),
            modules: Default::default(),
            options: WorkerOptions::default(),
            referrer_policy: "strict-origin-when-cross-origin".into(),
            limits: AgentLimits::default(),
        };
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let runtime_release = release.clone();
        let mut process = AgentThreadProcess::spawn(bootstrap, move |_| {
            Ok(BlockedRuntime {
                release: runtime_release,
            })
        })
        .unwrap();
        process.terminate().unwrap();
        assert!(*release.0.lock().unwrap());
        assert!(process.thread.is_none());
        process.terminate().unwrap();
    }
}
