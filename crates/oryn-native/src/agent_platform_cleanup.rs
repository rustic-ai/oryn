//! Bound agent platform revocation custody. Drop only signals a reserved job;
//! the original transport is retained through a parent revocation acknowledgement.
use oryn_js::worker_wire::{AgentServiceRequest, AgentServiceResponse, WorkerTransport};
use std::{
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
struct State {
    request: Option<AgentServiceRequest>,
    requested: bool,
    running: bool,
    done: bool,
    retry: Instant,
}
struct Job {
    transport: Arc<dyn WorkerTransport>,
    state: Mutex<State>,
}
struct Executor {
    jobs: Mutex<Vec<Arc<Job>>>,
    changed: Condvar,
}
static EXECUTOR: OnceLock<Result<Arc<Executor>, String>> = OnceLock::new();
fn executor() -> Result<&'static Arc<Executor>, String> {
    EXECUTOR
        .get_or_init(|| {
            let executor = Arc::new(Executor {
                jobs: Mutex::new(Vec::new()),
                changed: Condvar::new(),
            });
            // A stalled original route must not monopolize unrelated cleanup.
            // A fixed lane count bounds threads while each job retains one owner.
            for lane in 0..4 {
                let worker = executor.clone();
                std::thread::Builder::new()
                    .name(format!("oryn-agent-platform-revoke-{lane}"))
                    .spawn(move || worker.run())
                    .map_err(|error| error.to_string())?;
            }
            Ok(executor)
        })
        .as_ref()
        .map_err(Clone::clone)
}
impl Executor {
    fn run(&self) {
        loop {
            let job = {
                let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
                loop {
                    let next = jobs.iter().find_map(|job| {
                        let mut state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                        if state.requested
                            && !state.running
                            && !state.done
                            && state.retry <= Instant::now()
                        {
                            state.running = true;
                            Some(job.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(job) = next {
                        break job;
                    }
                    jobs = self
                        .changed
                        .wait_timeout(jobs, Duration::from_millis(100))
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                }
            };
            let request = job
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .request
                .clone();
            let acknowledged = request.is_none()
                || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    matches!(
                        job.transport.request(request.unwrap()),
                        Ok(AgentServiceResponse::Done)
                    )
                }))
                .unwrap_or(false);
            {
                let mut state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                state.running = false;
                state.done = acknowledged;
                state.retry = Instant::now() + Duration::from_millis(100);
            }
            if acknowledged {
                self.jobs
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .retain(|entry| !Arc::ptr_eq(entry, &job));
            }
            self.changed.notify_all();
        }
    }
}
pub(super) struct Ticket {
    job: Arc<Job>,
}
impl Ticket {
    pub(super) fn reserve(transport: Arc<dyn WorkerTransport>) -> Result<Self, String> {
        let ticket = Self::reserve_result(transport)?;
        ticket.attach(AgentServiceRequest::PlatformRetire);
        Ok(ticket)
    }
    pub(super) fn reserve_result(transport: Arc<dyn WorkerTransport>) -> Result<Self, String> {
        let executor = executor()?;
        let mut jobs = executor
            .jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if jobs.len() >= 1024 {
            return Err("agent platform revocation custody capacity".into());
        }
        let job = Arc::new(Job {
            transport,
            state: Mutex::new(State {
                request: None,
                requested: false,
                running: false,
                done: false,
                retry: Instant::now(),
            }),
        });
        jobs.push(job.clone());
        Ok(Self { job })
    }
}
impl Ticket {
    pub(super) fn attach(&self, request: AgentServiceRequest) {
        assert!(matches!(
            &request,
            AgentServiceRequest::PlatformRetire | AgentServiceRequest::PlatformReleaseResult { .. }
        ));
        let mut state = self
            .job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(state.request.is_none() && !state.requested);
        state.request = Some(request);
    }
    pub(super) fn accept(self) {
        self.job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .done = true;
        if let Some(Ok(executor)) = EXECUTOR.get() {
            executor
                .jobs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|job| !Arc::ptr_eq(job, &self.job));
            executor.changed.notify_all();
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.job
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested = true;
        if let Some(Ok(executor)) = EXECUTOR.get() {
            executor.changed.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::worker_wire::WorkerError;
    #[test]
    fn dropped_platform_owner_does_not_wait_for_parent_and_failed_revocation_remains_owned() {
        struct Parent {
            entered: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
            first: Mutex<bool>,
        }
        impl WorkerTransport for Parent {
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                assert!(matches!(request, AgentServiceRequest::PlatformRetire));
                self.entered.send(()).unwrap();
                let mut first = self.first.lock().unwrap();
                if *first {
                    *first = false;
                    self.release.lock().unwrap().recv().unwrap();
                    Err(WorkerError::new(
                        "InvalidStateError",
                        "injected revoke failure",
                    ))
                } else {
                    Ok(AgentServiceResponse::Done)
                }
            }
        }
        let (entered, seen) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let ticket = Ticket::reserve(Arc::new(Parent {
            entered,
            release: Mutex::new(gate),
            first: Mutex::new(true),
        }))
        .unwrap();
        let job = ticket.job.clone();
        drop(ticket);
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!job.state.lock().unwrap().done);
        release.send(()).unwrap();
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !job.state.lock().unwrap().done {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
