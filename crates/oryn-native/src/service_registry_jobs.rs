//! Profile-local equivalent-job subscribers. Registration IDs are translated by
//! the recipient registry; only native lineage crosses this internal boundary.
use super::{ServiceResult, WorkerError, error};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type ResultValue = Result<(ServiceResult, Option<String>), WorkerError>;
type Recipient = Box<dyn FnOnce(ResultValue) + Send>;
#[derive(Default)]
pub(crate) struct ProfileJobs {
    scopes: Mutex<BTreeMap<String, Vec<Weak<SharedJob>>>>,
}
struct Pending {
    settled: bool,
    recipients: Vec<Recipient>,
}
pub(crate) struct SharedJob {
    key: String,
    predecessor: Option<Arc<SharedJob>>,
    finished: AtomicBool,
    pending: Mutex<Pending>,
}
impl ProfileJobs {
    pub(super) fn admit(
        &self,
        scope: String,
        key: String,
        recipient: Recipient,
    ) -> Result<Option<Arc<SharedJob>>, WorkerError> {
        let retained_tail;
        let mut scopes = self
            .scopes
            .lock()
            .map_err(|_| error("profile equivalent job directory poisoned"))?;
        scopes.retain(|_, jobs| {
            jobs.retain(|job| job.strong_count() != 0);
            !jobs.is_empty()
        });
        if scopes.len() >= 4096 && !scopes.contains_key(&scope) {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "profile equivalent scope capacity",
            ));
        }
        let jobs = scopes.entry(scope).or_default();
        // Only the scope's tail may merge. A different queued job is an ordering
        // barrier even when an older equivalent job is still running.
        retained_tail = jobs.last().and_then(Weak::upgrade);
        if let Some(tail) = &retained_tail {
            let mut pending = tail
                .pending
                .lock()
                .map_err(|_| error("profile equivalent job poisoned"))?;
            if tail.key == key && !pending.settled {
                if pending.recipients.len() >= 256 {
                    return Err(WorkerError::new(
                        "QuotaExceededError",
                        "profile equivalent recipient capacity",
                    ));
                }
                pending.recipients.push(recipient);
                return Ok(None);
            }
        }
        if jobs.len() >= 256 {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "profile equivalent queue capacity",
            ));
        }
        let job = Arc::new(SharedJob {
            key,
            predecessor: retained_tail.clone(),
            finished: AtomicBool::new(false),
            pending: Mutex::new(Pending {
                settled: false,
                recipients: vec![recipient],
            }),
        });
        jobs.push(Arc::downgrade(&job));
        Ok(Some(job))
    }
}
impl SharedJob {
    pub(super) fn wait_turn(&self, cancel: &AtomicBool) -> Result<(), WorkerError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(WorkerError::new(
                    "AbortError",
                    "profile service job owner retired",
                ));
            }
            let mut predecessor = self.predecessor.as_deref();
            let mut blocked = false;
            while let Some(prior) = predecessor {
                if !prior.finished.load(Ordering::Acquire) {
                    blocked = true;
                    break;
                }
                predecessor = prior.predecessor.as_deref();
            }
            if !blocked {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(WorkerError::new(
                    "TimeoutError",
                    "profile service job ordering deadline",
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    pub(super) fn finish(&self) {
        self.publish(Err(error("native equivalent job ended before publication")));
        self.finished.store(true, Ordering::Release);
    }
    pub(super) fn publish(&self, result: ResultValue) {
        let recipients = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if pending.settled {
                return;
            }
            pending.settled = true;
            std::mem::take(&mut pending.recipients)
        };
        // Callers may lock another registry or re-enter this directory.
        for recipient in recipients {
            recipient(result.clone());
        }
    }
}
impl Drop for SharedJob {
    fn drop(&mut self) {
        self.finish();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equivalent_tail_shares_once_and_distinct_job_is_a_barrier() {
        let directory = ProfileJobs::default();
        let results = Arc::new(Mutex::new(Vec::new()));
        let subscriber = |id| {
            let results = results.clone();
            Box::new(move |result: ResultValue| results.lock().unwrap().push((id, result.is_ok())))
                as Recipient
        };
        let first = directory
            .admit("scope".into(), "update".into(), subscriber(1))
            .unwrap()
            .unwrap();
        assert!(
            directory
                .admit("scope".into(), "update".into(), subscriber(2))
                .unwrap()
                .is_none()
        );
        let other = directory
            .admit("scope".into(), "unregister".into(), subscriber(3))
            .unwrap()
            .unwrap();
        let after = directory
            .admit("scope".into(), "update".into(), subscriber(4))
            .unwrap()
            .unwrap();
        first.publish(Ok((ServiceResult::Bool(true), None)));
        first.publish(Err(error("late failure")));
        assert_eq!(*results.lock().unwrap(), vec![(1, true), (2, true)]);
        other.finish();
        after.finish();
        drop(other);
        drop(after);
        assert_eq!(
            *results.lock().unwrap(),
            vec![(1, true), (2, true), (3, false), (4, false)]
        );
    }
    #[test]
    fn settlement_prevents_late_join_and_callbacks_run_outside_locks() {
        let directory = Arc::new(ProfileJobs::default());
        let weak = Arc::downgrade(&directory);
        let job = directory
            .admit(
                "scope".into(),
                "update".into(),
                Box::new(move |_| {
                    assert!(weak.upgrade().unwrap().scopes.try_lock().is_ok());
                }),
            )
            .unwrap()
            .unwrap();
        job.publish(Ok((ServiceResult::Bool(true), None)));
        assert!(
            directory
                .admit("scope".into(), "update".into(), Box::new(|_| {}))
                .unwrap()
                .is_some()
        );
        assert!(
            directory
                .admit(
                    "other-partition-scope".into(),
                    "update".into(),
                    Box::new(|_| {})
                )
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn settled_result_keeps_ordering_lease_and_cancelled_middle_cannot_overtake() {
        let directory = ProfileJobs::default();
        let first = directory
            .admit("scope".into(), "register".into(), Box::new(|_| {}))
            .unwrap()
            .unwrap();
        let middle = directory
            .admit("scope".into(), "unregister".into(), Box::new(|_| {}))
            .unwrap()
            .unwrap();
        let last = directory
            .admit("scope".into(), "update".into(), Box::new(|_| {}))
            .unwrap()
            .unwrap();
        first.publish(Ok((ServiceResult::Bool(true), None)));
        middle.finish();
        drop(middle);
        let done = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(AtomicBool::new(false));
        let result = done.clone();
        let started = entered.clone();
        let waiter = std::thread::spawn(move || {
            started.store(true, Ordering::Release);
            last.wait_turn(&AtomicBool::new(false)).unwrap();
            result.store(true, Ordering::Release);
            last.finish();
        });
        while !entered.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        assert!(
            !done.load(Ordering::Acquire),
            "promise settlement does not release install ordering"
        );
        first.finish();
        waiter.join().unwrap();
        assert!(done.load(Ordering::Acquire));
    }
    #[test]
    fn one_recipient_retirement_does_not_cancel_shared_result() {
        let directory = ProfileJobs::default();
        let retired = Arc::new(AtomicBool::new(false));
        let deliveries = Arc::new(Mutex::new(Vec::new()));
        let live = retired.clone();
        let output = deliveries.clone();
        let job = directory
            .admit(
                "scope".into(),
                "update".into(),
                Box::new(move |_| {
                    if !live.load(Ordering::Acquire) {
                        output.lock().unwrap().push(1);
                    }
                }),
            )
            .unwrap()
            .unwrap();
        let output = deliveries.clone();
        assert!(
            directory
                .admit(
                    "scope".into(),
                    "update".into(),
                    Box::new(move |result| {
                        assert!(result.is_ok());
                        output.lock().unwrap().push(2);
                    })
                )
                .unwrap()
                .is_none()
        );
        retired.store(true, Ordering::Release);
        job.publish(Ok((ServiceResult::Bool(true), None)));
        job.finish();
        assert_eq!(*deliveries.lock().unwrap(), vec![2]);
    }
}
