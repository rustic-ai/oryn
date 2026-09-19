//! Retained physical close work, separate from already published Window.closed.
use crate::runtime::{PageHandle, RuntimeError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
#[derive(Default)]
struct State {
    running: bool,
    complete: bool,
    failure: Option<String>,
    thread: Option<std::thread::JoinHandle<()>>,
}
pub(crate) struct Job {
    page: PageHandle,
    state: Mutex<State>,
    changed: Condvar,
}
impl std::fmt::Debug for Job {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Job").finish_non_exhaustive()
    }
}
impl Job {
    pub(crate) fn new(page: PageHandle) -> Arc<Self> {
        Arc::new(Self {
            page,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        })
    }
    pub(crate) fn start(self: &Arc<Self>) -> Result<(), RuntimeError> {
        let previous = {
            let mut state = self.state.lock().map_err(|_| {
                RuntimeError::ContextService("script close custody poisoned".into())
            })?;
            if state.running || state.complete {
                return Ok(());
            }
            state.running = true;
            state.failure = None;
            state.thread.take()
        };
        if let Some(previous) = previous {
            let _ = previous.join();
        }
        let job = self.clone();
        // Publish the thread handle before its completion can make a retry
        // eligible, so a fast completion cannot overwrite a newer owner.
        let mut admission = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match std::thread::Builder::new()
            .name("oryn-script-close".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| error.to_string())?;
                    runtime
                        .block_on(job.page.close_ordered())
                        .map_err(|error| error.to_string())
                }))
                .unwrap_or_else(|_| Err("script close worker panicked".into()));
                let mut state = job.state.lock().unwrap_or_else(|error| error.into_inner());
                state.running = false;
                state.complete = result.is_ok();
                state.failure = result.err();
                job.changed.notify_all();
            }) {
            Ok(thread) => {
                admission.thread = Some(thread);
                Ok(())
            }
            Err(error) => {
                let message = format!("script close worker spawn failed: {error}");
                admission.running = false;
                admission.failure = Some(message.clone());
                self.changed.notify_all();
                Err(RuntimeError::ContextService(message))
            }
        }
    }
    pub(crate) fn drain(self: &Arc<Self>, budget: Duration) -> Result<(), RuntimeError> {
        self.start()?;
        let deadline = Instant::now() + budget;
        let mut state = self
            .state
            .lock()
            .map_err(|_| RuntimeError::ContextService("script close custody poisoned".into()))?;
        while state.running {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RuntimeError::ContextService(
                    "script close teardown deadline".into(),
                ));
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| RuntimeError::ContextService("script close custody poisoned".into()))?
                .0;
        }
        if let Some(failure) = &state.failure {
            return Err(RuntimeError::ContextService(failure.clone()));
        }
        let thread = state.thread.take();
        drop(state);
        if let Some(thread) = thread {
            thread
                .join()
                .map_err(|_| RuntimeError::ContextService("script close worker panicked".into()))?;
        }
        Ok(())
    }
}
