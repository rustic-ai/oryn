//! Original service event ownership survives rejected replies, lost body readers,
//! cancellation transport errors and failed process retirement.
use super::*;
use std::sync::{Condvar, OnceLock};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Active,
    Wait,
    Cancel,
    Complete,
}
struct Job {
    registry: Arc<ServiceRegistry>,
    lease: ServiceEventLease,
    slot: ServiceTaskSlot,
    deadline: Instant,
    finished: AtomicBool,
    state: Mutex<(Mode, bool, Instant)>,
}
struct Pool {
    jobs: Mutex<Vec<Arc<Job>>>,
    changed: Condvar,
}
static POOL: OnceLock<Result<Arc<Pool>, String>> = OnceLock::new();
fn pool() -> Result<&'static Arc<Pool>, WorkerError> {
    POOL.get_or_init(|| {
        let pool = Arc::new(Pool {
            jobs: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        });
        // One retained worker suffices for bounded cleanup; no per-Drop thread or IO.
        let worker = pool.clone();
        std::thread::Builder::new()
            .name("oryn-service-stream-cleanup".into())
            .spawn(move || worker.run())
            .map_err(|failure| failure.to_string())?;
        Ok(pool)
    })
    .as_ref()
    .map_err(|failure| error(failure.clone()))
}
impl Pool {
    fn run(&self) {
        loop {
            let job = {
                let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(job) = jobs.iter().find_map(|job| {
                        let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                        if state.0 != Mode::Active && !state.1 && state.2 <= Instant::now() {
                            state.1 = true;
                            Some(job.clone())
                        } else {
                            None
                        }
                    }) {
                        break job;
                    }
                    jobs = self
                        .changed
                        .wait_timeout(jobs, Duration::from_millis(20))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            };
            let mode = job.state.lock().unwrap_or_else(|e| e.into_inner()).0;
            let acknowledged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if mode == Mode::Complete {
                    return true;
                }
                let agent = job.lease.agent;
                let id = job.slot.id;
                if job.registry.retired.load(Ordering::Acquire) || Instant::now() >= job.deadline {
                    return job.registry.retire_version(agent).is_ok();
                }
                let Ok(context) = job.registry.context() else {
                    return false;
                };
                if mode == Mode::Cancel {
                    // A successful send is not the cancellation ACK: retain the
                    // lease until task completion, or acknowledged process stop.
                    let _ = context.dispatch_worker_registry(|agents, _| {
                        agents.send_service_control(agent, AgentCommand::CancelTask { id })
                    });
                }
                match job.registry.poll_service_task(
                    &context,
                    agent,
                    &job.lease.serial,
                    &job.slot.inbox,
                    id,
                ) {
                    Ok((_, finished)) => {
                        if finished.is_some() {
                            job.finished.store(true, Ordering::Release);
                        }
                        let body_settled = {
                            let inbox = job
                                .slot
                                .inbox
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            !inbox.started.contains(&id) || inbox.ended.contains(&id)
                        };
                        job.finished.load(Ordering::Acquire) && (mode == Mode::Wait || body_settled)
                    }
                    Err(_) => job.registry.retire_version(agent).is_ok(),
                }
            }))
            .unwrap_or(false);
            if acknowledged {
                self.jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|entry| !Arc::ptr_eq(entry, &job));
            } else {
                let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
                state.1 = false;
                state.2 = Instant::now() + Duration::from_millis(20);
            }
            self.changed.notify_all();
            // The retained worker, never the foreground destructor, releases the
            // slot and event lease (which may activate another service version).
            drop(job);
        }
    }
}
pub(super) struct Ticket {
    job: Arc<Job>,
    armed: bool,
}
impl Ticket {
    pub(super) fn reserve(
        registry: &ServiceRegistry,
        lease: ServiceEventLease,
        slot: ServiceTaskSlot,
        deadline: Instant,
    ) -> Result<Self, WorkerError> {
        let pool = pool()?;
        let registry = registry
            .weak_self
            .upgrade()
            .ok_or_else(|| error("service registry retired"))?;
        let mut jobs = pool
            .jobs
            .lock()
            .map_err(|_| error("service stream custody poisoned"))?;
        if registry.retired.load(Ordering::Acquire) {
            return Err(error("service registry retired"));
        }
        if jobs.len() >= 1024 {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "service stream cleanup capacity",
            ));
        }
        let job = Arc::new(Job {
            registry,
            lease,
            slot,
            deadline,
            finished: AtomicBool::new(false),
            state: Mutex::new((Mode::Active, false, Instant::now())),
        });
        jobs.push(job.clone());
        Ok(Self { job, armed: true })
    }
    pub(super) fn observe_lifetime(&self, finished: bool) {
        if finished {
            self.job.finished.store(true, Ordering::Release);
        }
    }
    pub(super) fn finish(mut self, finished: bool) {
        self.armed = false;
        self.set(if finished { Mode::Complete } else { Mode::Wait });
    }
    fn set(&self, mode: Mode) {
        self.job.state.lock().unwrap_or_else(|e| e.into_inner()).0 = mode;
        if let Some(Ok(pool)) = POOL.get() {
            pool.changed.notify_all();
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if self.armed {
            self.set(Mode::Cancel);
        }
    }
}
impl ServiceRegistry {
    pub(super) fn drain_stream_cleanup(&self, budget: Duration) -> Result<(), WorkerError> {
        let Some(Ok(pool)) = POOL.get() else {
            return Ok(());
        };
        let deadline = Instant::now() + budget;
        let mut jobs = pool
            .jobs
            .lock()
            .map_err(|_| error("service stream custody poisoned"))?;
        while jobs
            .iter()
            .any(|job| std::ptr::eq(job.registry.as_ref(), self))
        {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Err(error("service stream cancellation acknowledgement pending"));
            };
            jobs = pool
                .changed
                .wait_timeout(jobs, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropped_stream_ticket_retains_exact_event_without_waiting_for_registry_io() {
        let context = crate::Browser::new().new_context();
        let registry = ServiceRegistry::new(&context);
        let inbox = Arc::new(Mutex::new(ServiceControlInbox::default()));
        inbox.lock().unwrap().active.insert(41);
        let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let lease = ServiceEventLease {
            check_soft_update: false,
            activity: None,
            registry: Arc::downgrade(&registry),
            agent: AgentId(u64::MAX),
            serial: Arc::new(Mutex::new(())),
            inbox: inbox.clone(),
            inflight: inflight.clone(),
        };
        let slot = ServiceTaskSlot {
            registry: Arc::downgrade(&registry),
            inbox: inbox.clone(),
            id: 41,
        };
        let ticket = Ticket::reserve(&registry, lease, slot, Instant::now()).unwrap();
        // Exact version retirement needs this lock. Dropping the ticket must
        // publish ownership without trying to execute that retirement inline.
        let held = registry.state.lock().unwrap();
        drop(ticket);
        assert_eq!(inflight.load(Ordering::Acquire), 1);
        assert!(registry.drain_stream_cleanup(Duration::ZERO).is_err());
        let other = ServiceRegistry::new(&context);
        other.drain_stream_cleanup(Duration::ZERO).unwrap();
        drop(held);
        registry
            .drain_stream_cleanup(Duration::from_secs(2))
            .unwrap();
    }
}
