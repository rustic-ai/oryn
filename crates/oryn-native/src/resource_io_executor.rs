//! Process-owned, bounded IO lanes for parser, dynamic and module resources. Admission never waits
//! for another resource's network operation to finish.
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, OnceLock},
};

type Job = Box<dyn FnOnce() + Send + 'static>;
struct Queue {
    jobs: VecDeque<Job>,
    outstanding: usize,
    stopping: bool,
}
struct Executor {
    queue: Mutex<Queue>,
    changed: Condvar,
    capacity: usize,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
static EXECUTOR: OnceLock<Result<Arc<Executor>, String>> = OnceLock::new();

/// Validate permanent worker availability before admitting deferred work.
pub(crate) fn ready() -> Result<(), String> {
    let pool = EXECUTOR
        .get_or_init(|| Executor::new(8, 1024))
        .as_ref()
        .map_err(Clone::clone)?;
    if pool
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stopping
    {
        Err("resource IO executor stopped".into())
    } else {
        Ok(())
    }
}

pub(crate) fn submit(job: impl FnOnce() + Send + 'static) -> Result<(), String> {
    EXECUTOR
        .get_or_init(|| Executor::new(8, 1024))
        .as_ref()
        .map_err(Clone::clone)?
        .submit(Box::new(job))
}

impl Executor {
    fn new(lanes: usize, capacity: usize) -> Result<Arc<Self>, String> {
        if lanes == 0 || capacity == 0 {
            return Err("invalid resource executor limits".into());
        }
        let pool = Arc::new(Self {
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                outstanding: 0,
                stopping: false,
            }),
            changed: Condvar::new(),
            capacity,
            threads: Mutex::new(Vec::new()),
        });
        for index in 0..lanes {
            let worker = pool.clone();
            match std::thread::Builder::new()
                .name(format!("oryn-resource-io-{index}"))
                .spawn(move || worker.run())
            {
                Ok(thread) => pool
                    .threads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(thread),
                Err(error) => {
                    pool.stop();
                    pool.join();
                    return Err(format!("resource IO executor could not start: {error}"));
                }
            }
        }
        Ok(pool)
    }

    fn submit(&self, job: Job) -> Result<(), String> {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.stopping {
            return Err("resource IO executor stopped".into());
        }
        if queue.outstanding >= self.capacity {
            return Err("pending resource IO jobs exceeded".into());
        }
        queue.outstanding += 1;
        queue.jobs.push_back(job);
        self.changed.notify_one();
        Ok(())
    }

    fn run(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(job) = queue.jobs.pop_front() {
                        break job;
                    }
                    if queue.stopping {
                        return;
                    }
                    queue = self.changed.wait(queue).unwrap_or_else(|e| e.into_inner());
                }
            };
            // Resource wrappers publish their own panic failure. A final guard
            // keeps executor capacity and the IO lane alive if a wrapper fails.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.outstanding -= 1;
            self.changed.notify_all();
        }
    }

    fn stop(&self) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stopping = true;
        self.changed.notify_all();
    }

    fn join(&self) {
        let threads = std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        for thread in threads {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn capacity_includes_running_work_and_rejection_drops_owned_job() {
        let pool = Executor::new(1, 2).unwrap();
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        pool.submit(Box::new(move || {
            entered.send(()).unwrap();
            wait.recv().unwrap();
        }))
        .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (done, completed) = mpsc::channel();
        pool.submit(Box::new(move || {
            done.send(()).unwrap();
        }))
        .unwrap();
        let owner = Arc::new(());
        let captured = owner.clone();
        assert!(pool.submit(Box::new(move || drop(captured))).is_err());
        assert_eq!(Arc::strong_count(&owner), 1);
        release.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        pool.stop();
        pool.join();
        assert_eq!(pool.queue.lock().unwrap().outstanding, 0);
        assert!(pool.submit(Box::new(|| {})).is_err());
    }

    #[test]
    fn panicked_work_does_not_discard_queued_work_or_executor_capacity() {
        let pool = Executor::new(1, 2).unwrap();
        pool.submit(Box::new(|| panic!("simulated parser job panic")))
            .unwrap();
        let (done, completed) = mpsc::channel();
        pool.submit(Box::new(move || {
            done.send(()).unwrap();
        }))
        .unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        pool.stop();
        pool.join();
        assert_eq!(pool.queue.lock().unwrap().outstanding, 0);
    }
}
