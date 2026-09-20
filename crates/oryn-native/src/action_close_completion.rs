//! Action-owned close receipts. Actors publish custody and reply before callers
//! wait for physical cleanup, including cleanup that needs another worker turn.
use super::*;
#[cfg(feature = "v8-host")]
use crate::context_service::{ContextServiceRequest, ContextServiceResponse, SharedContextClient};

#[cfg(feature = "v8-host")]
pub(super) struct Scope {
    client: Option<SharedContextClient>,
    page: PageId,
    action: u64,
}
#[cfg(feature = "v8-host")]
impl Scope {
    pub fn begin(
        client: Option<SharedContextClient>,
        page: PageId,
        action: u64,
    ) -> Result<Self, RuntimeError> {
        if let Some(client) = &client {
            match client.request(ContextServiceRequest::BeginActionCloseScope {
                page_id: page,
                action_id: action,
            })? {
                ContextServiceResponse::ActionCloseScopeUpdated => {}
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "action close scope admission mismatch".into(),
                    ));
                }
            }
        }
        Ok(Self {
            client,
            page,
            action,
        })
    }
    pub fn finish(mut self, retain: bool) -> Result<(), RuntimeError> {
        if let Some(client) = &self.client {
            match client.request(ContextServiceRequest::EndActionCloseScope {
                page_id: self.page,
                action_id: self.action,
                retain,
            })? {
                ContextServiceResponse::ActionCloseScopeUpdated => {}
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "action close scope completion mismatch".into(),
                    ));
                }
            }
        }
        self.client = None;
        Ok(())
    }
}
#[cfg(feature = "v8-host")]
impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.request(ContextServiceRequest::EndActionCloseScope {
                page_id: self.page,
                action_id: self.action,
                retain: false,
            });
        }
    }
}
impl BrowserContext {
    pub(crate) fn begin_action_close_scope(
        &self,
        page: PageId,
        action: u64,
    ) -> Result<(), RuntimeError> {
        self.inner
            .script_close_admissions
            .lock()
            .map_err(|_| RuntimeError::ContextService("close admission registry poisoned".into()))?
            .begin_action(page, action)
    }
    pub(crate) fn end_action_close_scope(
        &self,
        page: PageId,
        action: u64,
        retain: bool,
    ) -> Result<(), RuntimeError> {
        self.inner
            .script_close_admissions
            .lock()
            .map_err(|_| RuntimeError::ContextService("close admission registry poisoned".into()))?
            .end_action(page, action, retain)
    }
}
impl PageHandle {
    pub(super) async fn complete_action_closes(
        &self,
        mut result: ActionResult,
    ) -> Result<ActionResult, RuntimeError> {
        let Some(context) = self.context.upgrade() else {
            return Ok(result);
        };
        // A worker must release its response before the parent can send Close.
        // The real parent completes Action, Actions and OIL responses below.
        #[cfg(feature = "v8-host")]
        if context.context_client.is_some() {
            return Ok(result);
        }
        let deadline = tokio::time::Instant::now() + context.config.command_timeout;
        loop {
            let closed = context
                .script_close_admissions
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("close admission registry poisoned".into())
                })?
                .poll_action(result.page_id, result.action_id.0);
            if let Some(closed) = closed {
                for page_id in closed {
                    if !result.effects.iter().any(|effect| matches!(effect, Effect::PageClosed { page_id: existing } if *existing == page_id)) {
                        result.effects.push(Effect::PageClosed { page_id });
                    }
                    if page_id == self.page_id() {
                        self.publish_closed();
                    }
                }
                return Ok(result);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RuntimeError::ActionExecutionFailed {
                    action_id: result.action_id,
                    cause: Box::new(RuntimeError::ContextService(
                        "action close receipts remain pending after command deadline".into(),
                    )),
                });
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    pub(super) async fn complete_worker_action_error(
        &self,
        mut error: RuntimeError,
    ) -> RuntimeError {
        if let RuntimeError::BatchExecutionFailed { completed, .. } = &mut error {
            for result in completed {
                if let Ok(completed) = self.complete_action_closes(result.clone()).await {
                    *result = completed;
                }
            }
        }
        error
    }
    pub(super) async fn complete_worker_action_closes(
        &self,
        mut response: crate::worker::WorkerResponse,
    ) -> Result<crate::worker::WorkerResponse, RuntimeError> {
        use crate::worker::WorkerResponse;
        match &mut response {
            WorkerResponse::Action { result } => {
                *result = self.complete_action_closes(result.clone()).await?
            }
            WorkerResponse::Actions { results } => {
                for result in results {
                    *result = self.complete_action_closes(result.clone()).await?;
                }
            }
            WorkerResponse::Oil { outputs } => {
                for output in outputs {
                    let result = match output {
                        crate::oil::NativeOilOutput::Action { result }
                        | crate::oil::NativeOilOutput::Intent {
                            result: crate::intent::IntentResult::Action { result },
                        } => Some(result),
                        _ => None,
                    };
                    if let Some(result) = result {
                        *result = self.complete_action_closes(result.clone()).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(response)
    }
}
