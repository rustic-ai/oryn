//! Readiness waits retain their reply and absolute deadline between actor turns.
//! They never run a nested event loop that owns a related root's unique host.
use super::*;

pub(super) struct Progress {
    predicate: WaitPredicate,
    timeout: Option<Duration>,
    deadline: Option<Instant>,
    started: bool,
}
impl Progress {
    pub(super) fn new(predicate: WaitPredicate, timeout: Option<Duration>) -> Self {
        Self {
            predicate,
            timeout,
            deadline: None,
            started: false,
        }
    }
    fn expired(&self) -> bool {
        self.started
            && self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }
    fn unsatisfied(&self) -> RuntimeError {
        RuntimeError::WaitUnsatisfied(format!("{:?}", self.predicate))
    }
    pub(super) fn next_delay(&self) -> Duration {
        self.deadline
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(1))
            })
            .unwrap_or(Duration::from_millis(1))
    }
    pub(super) fn poll(&mut self, state: &mut PageState) -> Result<bool, RuntimeError> {
        // A zero-duration wait still gets its original initial predicate check.
        // Later turns never evaluate an authored predicate after its deadline.
        if self.expired() {
            return Err(self.unsatisfied());
        }
        let network_idle = matches!(
            &self.predicate,
            WaitPredicate::Lifecycle(LifecycleState::NetworkIdle)
        );
        if !self.started {
            self.deadline = self
                .timeout
                .map(|duration| {
                    Instant::now().checked_add(duration).ok_or_else(|| {
                        RuntimeError::WaitUnsatisfied("wait deadline overflow".into())
                    })
                })
                .transpose()?;
            self.started = true;
            #[cfg(all(test, feature = "v8-host"))]
            if network_idle {
                loading_stage_diagnostics::wait(state, "wait-enter", || {
                    format!("timeout={:?}", self.timeout)
                });
            }
            #[cfg(feature = "v8-host")]
            {
                if network_idle && state.executable.is_some() {
                    state.poll_page_tasks()?;
                    #[cfg(test)]
                    loading_stage_diagnostics::wait(state, "wait-after-initial-poll", String::new);
                }
                if matches!(
                    &self.predicate,
                    WaitPredicate::SelectorVisible(_) | WaitPredicate::SelectorHidden(_)
                ) {
                    state.refresh_layout()?;
                }
            }
        }
        if state.wait_satisfied(&self.predicate)? {
            if network_idle && !state.lifecycle.contains(&LifecycleState::NetworkIdle) {
                state.lifecycle.push(LifecycleState::NetworkIdle);
                state.trace.push(
                    Revision(state.revision),
                    None,
                    TraceEventKind::LifecycleChanged {
                        state: LifecycleState::NetworkIdle,
                    },
                );
            }
            return Ok(true);
        }
        if self.expired() {
            return Err(self.unsatisfied());
        }
        #[cfg(feature = "v8-host")]
        if state.executable.is_none() && !network_idle {
            return Err(self.unsatisfied());
        }
        #[cfg(not(feature = "v8-host"))]
        if !network_idle {
            return Err(self.unsatisfied());
        }
        Ok(false)
    }
    #[cfg(feature = "v8-host")]
    pub(super) fn pump(&self, state: &mut PageState) -> Result<(), RuntimeError> {
        #[cfg(feature = "v8-host")]
        if state.executable.is_some() {
            state.poll_page_tasks()?;
            if matches!(
                &self.predicate,
                WaitPredicate::SelectorVisible(_) | WaitPredicate::SelectorHidden(_)
            ) {
                state.refresh_layout()?;
            }
        }
        Ok(())
    }
    fn turn(&mut self, state: &mut PageState) -> Result<bool, RuntimeError> {
        let done = self.poll(state)?;
        #[cfg(feature = "v8-host")]
        if !done {
            self.pump(state)?;
        }
        Ok(done)
    }
}

enum Reply {
    Page(oneshot::Sender<Result<(), RuntimeError>>),
    #[cfg(all(unix, feature = "v8-host"))]
    Scoped(oneshot::Sender<Result<crate::worker::WorkerResponse, RuntimeError>>),
}
impl Reply {
    fn closed(&self) -> bool {
        match self {
            Self::Page(reply) => reply.is_closed(),
            #[cfg(all(unix, feature = "v8-host"))]
            Self::Scoped(reply) => reply.is_closed(),
        }
    }
    fn complete(self, result: Result<(), RuntimeError>) {
        match self {
            Self::Page(reply) => {
                let _ = reply.send(result);
            }
            #[cfg(all(unix, feature = "v8-host"))]
            Self::Scoped(reply) => {
                let _ = reply.send(result.map(|()| crate::worker::WorkerResponse::Satisfied));
            }
        }
    }
}

pub(super) struct Pending {
    progress: Progress,
    reply: Reply,
    cancellation: Option<Arc<oryn_js::prompts::PromptController>>,
    #[cfg(all(unix, feature = "v8-host"))]
    scope: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
}
impl Pending {
    pub(super) fn from_command(
        command: &mut Option<PageCommand>,
        state: &PageState,
    ) -> Option<Self> {
        let incoming = command.take()?;
        let (predicate, timeout, reply) = match incoming {
            PageCommand::WaitFor { predicate, reply } => (
                predicate,
                Some(state.config.command_timeout),
                Reply::Page(reply),
            ),
            PageCommand::WaitForUntil {
                predicate,
                timeout,
                reply,
            } => (predicate, timeout, Reply::Page(reply)),
            #[cfg(all(unix, feature = "v8-host"))]
            PageCommand::ScopedWorker {
                scope,
                request,
                reply,
            } => {
                let (predicate, timeout) = match request {
                    crate::worker::WorkerRequest::WaitFor { predicate } => {
                        (predicate, Some(state.config.command_timeout))
                    }
                    crate::worker::WorkerRequest::WaitForUntil {
                        predicate,
                        timeout_ms,
                    } => (predicate, timeout_ms.map(Duration::from_millis)),
                    request => {
                        *command = Some(PageCommand::ScopedWorker {
                            scope,
                            request,
                            reply,
                        });
                        return None;
                    }
                };
                return Some(Self {
                    progress: Progress::new(predicate, timeout),
                    reply: Reply::Scoped(reply),
                    cancellation: None,
                    scope: Some(scope),
                });
            }
            incoming => {
                *command = Some(incoming);
                return None;
            }
        };
        Some(Self {
            progress: Progress::new(predicate, timeout),
            reply,
            cancellation: None,
            #[cfg(all(unix, feature = "v8-host"))]
            scope: None,
        })
    }
    pub(super) fn with_cancellation(
        mut self,
        cancellation: Arc<oryn_js::prompts::PromptController>,
    ) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
    pub(super) fn next_delay(&self) -> Duration {
        self.progress.next_delay()
    }
    pub(super) fn reject(self, error: RuntimeError) {
        self.reply.complete(Err(error));
    }
    pub(super) fn poll(mut self, state: &mut PageState) -> Option<Self> {
        if self.reply.closed() {
            return None;
        }
        if self
            .cancellation
            .as_ref()
            .is_some_and(|controller| controller.inspect().is_err())
        {
            self.reject(RuntimeError::Closed);
            return None;
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if state.script_close_departure_completed() {
            self.reject(RuntimeError::Closed);
            return None;
        }
        // Check before entering a frame scope: no scoped setup or predicate is
        // allowed to start another wait probe after the absolute deadline.
        let result = if self.progress.expired() {
            Err(self.progress.unsatisfied())
        } else {
            #[cfg(all(unix, feature = "v8-host"))]
            if let Some(scope) = self.scope.clone() {
                state.with_scoped_frame(scope, false, None, false, true, |frame| {
                    self.progress.turn(frame)
                })
            } else {
                self.progress.turn(state)
            }
            #[cfg(not(all(unix, feature = "v8-host")))]
            {
                self.progress.turn(state)
            }
        };
        match result {
            Ok(false) => Some(self),
            Ok(true) => {
                self.reply.complete(Ok(()));
                None
            }
            Err(error) => {
                self.reply.complete(Err(error));
                None
            }
        }
    }
}
