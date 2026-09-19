//! Driver-owned native scripts can outlive their HTTP response when a modal
//! prompt suspends execution. Their futures and admission permits remain owned
//! until completion; session revocation closes the page before draining them.
use super::{CommandError, registry::SessionCancellation};
use oryn_native::{
    PageHandle, host::webdriver_script::ScriptValue, user_prompts::PromptController,
};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, oneshot},
    task::JoinSet,
};

pub struct ScriptTasks {
    tasks: Mutex<JoinSet<Result<(), CommandError>>>,
    capacity: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    closing: AtomicBool,
    failure: Mutex<Option<CommandError>>,
}
impl ScriptTasks {
    pub fn new(capacity: usize) -> Self {
        Self {
            tasks: Mutex::new(JoinSet::new()),
            capacity: Arc::new(Semaphore::new(capacity)),
            bytes: Arc::new(Semaphore::new(64 * 1024 * 1024)),
            closing: AtomicBool::new(false),
            failure: Mutex::new(None),
        }
    }
    #[cfg(test)]
    pub(crate) fn active_count(&self) -> usize {
        self.tasks.lock().expect("script task lock").len()
    }
    fn record(&self, result: Result<Result<(), CommandError>, tokio::task::JoinError>) {
        let error = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(error) => CommandError {
                code: "unknown error",
                message: format!("native script task failed: {error}"),
            },
        };
        self.failure
            .lock()
            .expect("script failure lock")
            .get_or_insert(error);
    }
    pub async fn execute<F>(
        &self,
        page: PageHandle,
        body_bytes: usize,
        cancellation: Arc<SessionCancellation>,
        execution: F,
    ) -> Result<ScriptValue, CommandError>
    where
        F: Future<Output = Result<ScriptValue, CommandError>> + Send + 'static,
    {
        let controller = page.prompt_controller().map_err(|error| CommandError {
            code: "no such window",
            message: error.to_string(),
        })?;
        self.execute_owned(
            controller,
            body_bytes,
            cancellation,
            execution,
            async move {
                page.close().await.map_err(|error| CommandError {
                    code: "unknown error",
                    message: error.to_string(),
                })
            },
            None,
        )
        .await
    }
    pub async fn navigate(
        &self,
        page: PageHandle,
        body_bytes: usize,
        cancellation: Arc<SessionCancellation>,
        job: Arc<oryn_native::navigation_control::NavigationJob>,
    ) -> Result<ScriptValue, CommandError> {
        let controller = page.prompt_controller().map_err(|error| CommandError {
            code: "no such window",
            message: error.to_string(),
        })?;
        let execute_page = page.clone();
        let execute_job = job.clone();
        let result = self
            .execute_owned(
                controller,
                body_bytes,
                cancellation,
                async move {
                    execute_page
                        .run_controlled_navigation(execute_job)
                        .await
                        .map_err(|error| CommandError {
                            code: "unknown error",
                            message: error.to_string(),
                        })?;
                    Ok(ScriptValue::Null)
                },
                async move {
                    page.close().await.map_err(|error| CommandError {
                        code: "unknown error",
                        message: error.to_string(),
                    })
                },
                Some(job.clone()),
            )
            .await;
        if let Err(error) = &result {
            let _ = job.stop(oryn_native::navigation_control::NavigationStop::Failed(
                error.message.clone(),
            ));
        }
        result
    }
    async fn execute_owned<F, C>(
        &self,
        controller: Arc<PromptController>,
        body_bytes: usize,
        cancellation: Arc<SessionCancellation>,
        execution: F,
        close: C,
        navigation: Option<Arc<oryn_native::navigation_control::NavigationJob>>,
    ) -> Result<ScriptValue, CommandError>
    where
        F: Future<Output = Result<ScriptValue, CommandError>> + Send + 'static,
        C: Future<Output = Result<(), CommandError>> + Send + 'static,
    {
        let (reply, response) = oneshot::channel();
        {
            let mut tasks = self.tasks.lock().expect("script task lock");
            if self.closing.load(Ordering::Acquire) {
                return Err(CommandError {
                    code: "invalid session id",
                    message: "driver is shutting down".into(),
                });
            }
            while let Some(result) = tasks.try_join_next() {
                self.record(result);
            }
            let permit = self
                .capacity
                .clone()
                .try_acquire_owned()
                .map_err(|_| CommandError {
                    code: "unknown error",
                    message: "native script capacity exhausted".into(),
                })?;
            let weight = u32::try_from(body_bytes.max(1))
                .map_err(|_| CommandError::invalid("script body too large"))?;
            let byte_permit = self
                .bytes
                .clone()
                .try_acquire_many_owned(weight)
                .map_err(|_| CommandError {
                    code: "unknown error",
                    message: "native script byte capacity exhausted".into(),
                })?;
            tasks.spawn(async move {
                let _permit=permit;let _byte_permit=byte_permit;let mut reply=Some(reply);
                tokio::pin!(execution);
                let mut execution_finished=false;
                let readiness=async {
                    match &navigation {
                        Some(job)=>job.wait_ready().await.map(|_|ScriptValue::Null).map_err(super::navigation::error),
                        None=>std::future::pending::<Result<ScriptValue,CommandError>>().await,
                    }
                };
                tokio::pin!(readiness);
                let mut interval=tokio::time::interval(Duration::from_millis(1));
                loop {
                    tokio::select! {
                        biased;
                        _=cancellation.cancelled()=>{
                            if let Some(reply)=reply.take() {let _=reply.send(Err(CommandError {code:"invalid session id",message:"session revoked during script".into()}));}
                            // Release the lease through the reply first. Native close
                            // must bypass or interrupt the suspended actor.
                            let cleanup=close.await;
                            if !execution_finished {let _=execution.await;}
                            return cleanup;
                        }
                        result=&mut execution, if !execution_finished=>{
                            execution_finished=true;
                            if result.is_err() || navigation.is_none() {
                                // Prefer the authoritative timeout/stop code over
                                // the loader's generic cancellation error.
                                let result=if let Some(job)=&navigation {
                                    match job.progress().ok().and_then(|progress|progress.stopped) {
                                        Some(stop)=>Err(super::navigation::error(stop)),None=>result,
                                    }
                                } else {result};
                                if let Some(reply)=reply.take() {let _=reply.send(result);}
                                return Ok(());
                            }
                            if reply.is_none() {return Ok(());}
                        }
                        result=&mut readiness, if navigation.is_some() && reply.is_some()=>{
                            if let Some(reply)=reply.take() {let _=reply.send(result);}
                            if execution_finished {return Ok(());}
                        }
                        _=interval.tick()=>{
                            if let Some(prompt)=controller.inspect().ok().flatten() {
                                if prompt.request.kind==oryn_native::user_prompts::PromptKind::BeforeUnload {
                                    let _=controller.accept(prompt.request.token);
                                } else if let Some(reply)=reply.take() {
                                    let _=reply.send(Ok(ScriptValue::Null));
                                    if execution_finished {return Ok(());}
                                }
                            }
                        }
                    }
                }
            });
        }
        response.await.map_err(|_| CommandError {
            code: "unknown error",
            message: "native script response channel closed".into(),
        })?
    }
    /// Stop admission before revoking sessions; drain after native page cleanup.
    pub fn close_admission(&self) {
        let _tasks = self.tasks.lock().expect("script task lock");
        self.closing.store(true, Ordering::Release);
    }
    pub async fn drain(&self) -> Result<(), CommandError> {
        self.close_admission();
        let mut tasks = std::mem::take(&mut *self.tasks.lock().expect("script task lock"));
        while let Some(result) = tasks.join_next().await {
            self.record(result);
        }
        match self.failure.lock().expect("script failure lock").clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_native::user_prompts::{PromptKind, PromptRequest, PromptResponse, PromptToken};
    #[tokio::test]
    async fn modal_response_keeps_native_execution_and_capacity_owned_until_resolution() {
        let tasks = ScriptTasks::new(1);
        let controller = Arc::new(PromptController::new());
        let cancellation = Arc::new(SessionCancellation::default());
        let native = controller.clone();
        let token: PromptToken =
            serde_json::from_value(serde_json::json!({"id":1,"realm":1,"document_generation":1}))
                .unwrap();
        let execution = async move {
            let result = tokio::task::spawn_blocking(move || {
                native.wait_for_request(PromptRequest {
                    token,
                    kind: PromptKind::Prompt,
                    text: "name".into(),
                    default_value: "default".into(),
                })
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result, PromptResponse::Accepted("typed".into()));
            Ok(ScriptValue::Null)
        };
        let result = tasks
            .execute_owned(
                controller.clone(),
                1,
                cancellation,
                execution,
                async { Ok(()) },
                None,
            )
            .await
            .unwrap();
        assert_eq!(result, ScriptValue::Null);
        assert_eq!(tasks.capacity.available_permits(), 0);
        controller.set_input(token, "typed".into()).unwrap();
        controller.accept(token).unwrap();
        tasks.drain().await.unwrap();
        assert_eq!(tasks.capacity.available_permits(), 1);
    }
    #[tokio::test]
    async fn revocation_releases_response_then_closes_and_drains_native_execution() {
        let tasks = Arc::new(ScriptTasks::new(1));
        let controller = Arc::new(PromptController::new());
        let cancellation = Arc::new(SessionCancellation::default());
        let (closed, wait_closed) = oneshot::channel();
        let runner = tasks.clone();
        let signal = cancellation.clone();
        let task = tokio::spawn(async move {
            runner
                .execute_owned(
                    controller,
                    1,
                    signal,
                    async move {
                        wait_closed.await.unwrap();
                        Ok(ScriptValue::Null)
                    },
                    async move {
                        closed.send(()).unwrap();
                        Ok(())
                    },
                    None,
                )
                .await
        });
        cancellation.cancel();
        assert_eq!(task.await.unwrap().unwrap_err().code, "invalid session id");
        tasks.drain().await.unwrap();
        assert_eq!(tasks.capacity.available_permits(), 1);
    }
    #[tokio::test]
    async fn navigation_none_replies_at_start_but_keeps_native_future_owned() {
        use oryn_native::navigation_control::*;
        let tasks = ScriptTasks::new(1);
        let control = NavigationControl::default();
        let job = control
            .begin(NavigationRequest {
                command: NavigationCommand::Refresh,
                strategy: PageLoadStrategy::None,
                timeout_ms: Some(0),
            })
            .unwrap();
        let native = job.clone();
        let (complete, wait_complete) = oneshot::channel();
        let response = tasks
            .execute_owned(
                Arc::new(PromptController::new()),
                1,
                Arc::new(SessionCancellation::default()),
                async move {
                    native.started().unwrap();
                    wait_complete.await.unwrap();
                    native.complete_without_commit().unwrap();
                    Ok(ScriptValue::Null)
                },
                async { Ok(()) },
                Some(job),
            )
            .await
            .unwrap();
        assert_eq!(response, ScriptValue::Null);
        assert_eq!(tasks.capacity.available_permits(), 0);
        complete.send(()).unwrap();
        tasks.drain().await.unwrap();
        assert_eq!(tasks.capacity.available_permits(), 1);
    }
    #[tokio::test]
    async fn normal_navigation_waits_for_load_after_native_command_returns() {
        use oryn_native::navigation_control::*;
        let tasks = Arc::new(ScriptTasks::new(1));
        let control = NavigationControl::default();
        let job = control
            .begin(NavigationRequest {
                command: NavigationCommand::Refresh,
                strategy: PageLoadStrategy::Normal,
                timeout_ms: None,
            })
            .unwrap();
        let native = job.clone();
        let ready_job = job.clone();
        let runner = tasks.clone();
        let (parsed, parse_done) = oneshot::channel();
        let response = tokio::spawn(async move {
            runner
                .execute_owned(
                    Arc::new(PromptController::new()),
                    1,
                    Arc::new(SessionCancellation::default()),
                    async move {
                        native.started().unwrap();
                        native.committed(1, "about:blank".into()).unwrap();
                        native.interactive(1).unwrap();
                        parsed.send(()).unwrap();
                        Ok(ScriptValue::Null)
                    },
                    async { Ok(()) },
                    Some(ready_job),
                )
                .await
        });
        parse_done.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!response.is_finished());
        job.complete(1).unwrap();
        assert_eq!(response.await.unwrap().unwrap(), ScriptValue::Null);
        tasks.drain().await.unwrap();
    }

    #[tokio::test]
    async fn closed_admission_never_polls_native_execution() {
        let tasks = ScriptTasks::new(1);
        tasks.close_admission();
        let result = tasks
            .execute_owned(
                Arc::new(PromptController::new()),
                1,
                Arc::new(SessionCancellation::default()),
                async {
                    panic!("closed driver polled native execution");
                    #[allow(unreachable_code)]
                    Ok(ScriptValue::Null)
                },
                async { Ok(()) },
                None,
            )
            .await;
        assert_eq!(result.unwrap_err().code, "invalid session id");
        tasks.drain().await.unwrap();
    }
}
