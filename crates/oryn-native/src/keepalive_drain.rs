//! Reserved orphan-response consumers. Polling is nonblocking and independent
//! of the retired JS agent; no new queue admission is needed during retirement.
use std::sync::{
    Arc, Condvar, Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
type Poll = Box<dyn FnMut() -> bool + Send + 'static>;
struct Job {
    owners: AtomicUsize,
    poll: Mutex<Option<Poll>>,
}
struct Pool {
    jobs: Mutex<Vec<Arc<Job>>>,
    changed: Condvar,
}
static POOL: OnceLock<Result<Arc<Pool>, String>> = OnceLock::new();
fn pool() -> Result<&'static Arc<Pool>, String> {
    POOL.get_or_init(|| {
        let pool = Arc::new(Pool {
            jobs: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        });
        let worker = pool.clone();
        std::thread::Builder::new()
            .name("oryn-keepalive-drain".into())
            .spawn(move || worker.run())
            .map_err(|error| error.to_string())?;
        Ok(pool)
    })
    .as_ref()
    .map_err(Clone::clone)
}
impl Pool {
    fn run(&self) {
        loop {
            let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner()).clone();
            for job in jobs {
                let poll = job.poll.lock().unwrap_or_else(|e| e.into_inner()).take();
                let done = if let Some(mut poll) = poll {
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll()))
                        .unwrap_or(true);
                    if !done {
                        *job.poll.lock().unwrap_or_else(|e| e.into_inner()) = Some(poll);
                    }
                    done
                } else {
                    job.owners.load(Ordering::Acquire) == 0
                };
                if done {
                    self.jobs
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .retain(|other| !Arc::ptr_eq(other, &job));
                }
            }
            let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            drop(
                self.changed
                    .wait_timeout(jobs, Duration::from_millis(10))
                    .unwrap_or_else(|e| e.into_inner()),
            );
        }
    }
}
pub(crate) struct Ticket {
    job: Arc<Job>,
}
impl Clone for Ticket {
    fn clone(&self) -> Self {
        self.job.owners.fetch_add(1, Ordering::Relaxed);
        Self {
            job: self.job.clone(),
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.job.owners.fetch_sub(1, Ordering::AcqRel);
        if let Some(Ok(pool)) = POOL.get() {
            pool.changed.notify_all();
        }
    }
}
impl Ticket {
    pub(crate) fn reserve() -> Result<Self, String> {
        let pool = pool()?;
        let mut jobs = pool.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.len() >= 128 {
            return Err("keepalive response custody capacity".into());
        }
        jobs.try_reserve(1)
            .map_err(|_| "keepalive response custody allocation".to_string())?;
        let job = Arc::new(Job {
            owners: AtomicUsize::new(1),
            poll: Mutex::new(None),
        });
        jobs.push(job.clone());
        Ok(Self { job })
    }
    /// Returns true only when the response has reached EOF or terminal failure.
    pub(crate) fn orphan(&self, poll: impl FnMut() -> bool + Send + 'static) {
        let mut slot = self.job.poll.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(Box::new(poll));
        }
        drop(slot);
        if let Some(Ok(pool)) = POOL.get() {
            pool.changed.notify_all();
        }
    }
}
