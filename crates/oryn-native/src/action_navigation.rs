//! An acknowledged action can own root or embedded navigation. The actor waits
//! for those exact terminal receipts before running the next document operation.
//! No action/request is stored here, retried, or reconstructed.
use super::*;
use std::task::Poll;

pub(super) struct ScopedAction {
    pub(super) action_id: ActionId,
    pub(super) deadline: Option<Instant>,
    #[cfg(unix)]
    pub(super) ipc_identity: Option<crate::ipc_progress::Identity>,
}

pub(super) struct Barrier {
    pub(super) action_id: ActionId,
    #[cfg(unix)]
    pub(super) ipc_identity: Option<crate::ipc_progress::Identity>,
    completion: Option<oneshot::Receiver<Result<NavigationResult, RuntimeError>>>,
    frame: Option<crate::executable::ActionFrameNavigationReceipt>,
    frame_deadline: Option<Instant>,
    selected_commit: Option<crate::executable::ActionFrameCommittedSelection>,
    failure: Option<RuntimeError>,
}
impl Barrier {
    fn new(action_id: ActionId) -> Self {
        Self {
            action_id,
            #[cfg(unix)]
            ipc_identity: None,
            completion: None,
            frame: None,
            frame_deadline: None,
            selected_commit: None,
            failure: None,
        }
    }
    fn fail(&mut self, error: RuntimeError) {
        if self.failure.is_none() {
            self.failure = Some(RuntimeError::ActionExecutionFailed {
                action_id: self.action_id,
                cause: Box::new(error),
            });
        }
    }
    fn poll(&mut self) -> Poll<Result<(), RuntimeError>> {
        if let Some(error) = &self.failure {
            return Poll::Ready(Err(error.clone()));
        }
        if let Some(completion) = &mut self.completion {
            match completion.try_recv() {
                Ok(Ok(_)) => self.completion = None,
                Ok(Err(error)) => self.fail(error),
                Err(oneshot::error::TryRecvError::Empty) => {}
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.fail(RuntimeError::ProtocolViolation(
                        "action navigation lost its terminal receipt".into(),
                    ))
                }
            }
        }
        if let Some(frame) = &self.frame {
            use oryn_js::navigation::NavigationExecutionOutcome;
            match frame.poll() {
                Some(
                    NavigationExecutionOutcome::Completed
                    | NavigationExecutionOutcome::Committed { .. },
                ) => {
                    self.selected_commit = frame.committed_selection();
                    self.frame = None;
                }
                Some(NavigationExecutionOutcome::Canceled) => {
                    self.fail(RuntimeError::NavigationCancelled)
                }
                Some(NavigationExecutionOutcome::Failed { message }) => {
                    self.fail(RuntimeError::Script(message))
                }
                None if self
                    .frame_deadline
                    .is_none_or(|deadline| Instant::now() >= deadline) =>
                {
                    self.fail(RuntimeError::WorkerLimitExceeded {
                        resource: "action frame navigation wall time".into(),
                    });
                }
                None => {}
            }
        }
        if let Some(error) = &self.failure {
            Poll::Ready(Err(error.clone()))
        } else if self.completion.is_none() && self.frame.is_none() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

impl PageState {
    pub(super) fn begin_action_history_navigation(
        &mut self,
        mut task: oryn_js::v8_bindings::HistoryTraversalTask,
    ) -> Result<Option<NavigationProgress>, RuntimeError> {
        if !task.is_current() {
            task.acknowledge();
            return Ok(None);
        }
        if !task.is_runnable() {
            return Ok(None);
        }
        if task.delta() == 0 {
            task.acknowledge();
            return self.begin_refresh_navigation().map(Some);
        }
        let receipt = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::EvaluationUnavailable)?
            .run_history_task_with_receipt(task)
            .map_err(map_host_error)?;
        Ok(receipt.map(|receipt| {
            NavigationProgress::Joint(
                receipt,
                NavigationResult {
                    page_id: self.page_id,
                    requested_url: self.url.clone(),
                    final_url: self.url.clone(),
                    title: self
                        .document
                        .as_ref()
                        .map(document_title)
                        .unwrap_or_default(),
                    status: 200,
                    document_generation: self.document_generation,
                    revision: Revision(self.revision),
                    lifecycle: self.lifecycle.clone(),
                    challenge: self.last_challenge.clone(),
                },
            )
        }))
    }

    pub(super) fn retain_action_navigation(
        &mut self,
        action_id: ActionId,
        source_generation: u64,
        admitted_deadline: Option<Instant>,
        progress: NavigationProgress,
    ) -> Result<(), RuntimeError> {
        let deadline = match &progress {
            NavigationProgress::Pending(id) => self
                .pending_navigation_transaction
                .as_mut()
                .filter(|transaction| transaction.id == *id)
                .map(|transaction| {
                    if let Some(deadline) = admitted_deadline {
                        transaction.deadline = transaction.deadline.min(deadline);
                    }
                    transaction.deadline
                }),
            _ => admitted_deadline,
        }
        .ok_or_else(|| {
            RuntimeError::ProtocolViolation(
                "action navigation has no admitted deadline or transaction".into(),
            )
        })?;
        let (completion, receiver) = oneshot::channel();
        let reply = NavigationReply::ActionNavigation {
            completion,
            previous: None,
        };
        if let Some(pending) =
            self.retain_document_reply(reply, source_generation, deadline, Ok(progress))
        {
            self.pending_execution_replies.push(pending);
        }
        let mut barrier = Barrier::new(action_id);
        #[cfg(unix)]
        {
            barrier.ipc_identity = self
                .navigation_progress_identity
                .or_else(|| crate::ipc_progress::capture(self.page_id.0));
        }
        barrier.completion = Some(receiver);
        self.adopt_action_navigation(barrier)
    }

    pub(super) fn retain_action_frame_navigation(
        &mut self,
        action_id: ActionId,
        deadline: Option<Instant>,
        receipt: crate::executable::ActionFrameNavigationReceipt,
    ) -> Result<(), RuntimeError> {
        let mut barrier = Barrier::new(action_id);
        #[cfg(unix)]
        {
            barrier.ipc_identity = self
                .navigation_progress_identity
                .or_else(|| crate::ipc_progress::capture(self.page_id.0));
        }
        barrier.frame = Some(receipt);
        barrier.frame_deadline = deadline;
        self.adopt_action_navigation(barrier)
    }

    pub(super) fn adopt_action_navigation(
        &mut self,
        mut incoming: Barrier,
    ) -> Result<(), RuntimeError> {
        if let Some(current) = &mut self.action_navigation {
            if current.action_id != incoming.action_id
                || current.completion.is_some() && incoming.completion.is_some()
                || current.frame.is_some() && incoming.frame.is_some()
            {
                return Err(RuntimeError::ProtocolViolation(
                    "action navigation receipt owner collision".into(),
                ));
            }
            if incoming.completion.is_some() {
                current.completion = incoming.completion.take();
            }
            if incoming.frame.is_some() {
                current.frame = incoming.frame.take();
                current.frame_deadline = incoming.frame_deadline;
            }
            if current.failure.is_none() {
                current.failure = incoming.failure.take();
            }
            #[cfg(unix)]
            if current.ipc_identity.is_none() {
                current.ipc_identity = incoming.ipc_identity;
            }
        } else {
            self.action_navigation = Some(incoming);
        }
        Ok(())
    }
    pub(super) fn retain_failed_action_navigation(
        &mut self,
        action_id: ActionId,
        error: RuntimeError,
    ) {
        let barrier = self
            .action_navigation
            .get_or_insert_with(|| Barrier::new(action_id));
        barrier.fail(error);
    }
    pub(super) fn fail_action_navigation(&mut self, error: RuntimeError) {
        if let Some(barrier) = &mut self.action_navigation {
            barrier.fail(error);
        }
    }
    pub(super) fn resolve_action_frame_selection(
        &mut self,
        source: &crate::host::webdriver_script::AutomationRealmSnapshot,
        action_id: ActionId,
    ) -> Result<
        crate::host::webdriver_script::AutomationRealmSnapshot,
        crate::host::webdriver_script::ScriptError,
    > {
        use crate::host::webdriver_script::ScriptError;
        let inventory = self
            .executable
            .as_ref()
            .ok_or_else(ScriptError::retired)?
            .automation_realms();
        if let Some(current) = inventory
            .iter()
            .find(|entry| entry.same_document_as(source))
        {
            return Ok(current.clone());
        }
        let (_, selection) = self
            .completed_action_frame_selection
            .as_ref()
            .filter(|(owner, selection)| {
                *owner == action_id
                    && selection.source.same_document_as(source)
                    && selection.destination.same_navigable_as(source)
            })
            .ok_or_else(ScriptError::retired)?;
        let destination = inventory
            .into_iter()
            .find(|entry| entry.same_document_as(&selection.destination))
            .ok_or_else(ScriptError::retired)?;
        // A single exact native transition, consumed only after currentness
        // succeeds. Old handles and unrelated later documents are never mapped.
        self.completed_action_frame_selection = None;
        Ok(destination)
    }

    pub(super) fn poll_action_navigation(&mut self) -> Poll<Result<(), RuntimeError>> {
        let Some(barrier) = &mut self.action_navigation else {
            return Poll::Ready(Ok(()));
        };
        let result = barrier.poll();
        if matches!(&result, Poll::Ready(Ok(()))) {
            #[cfg(unix)]
            let released = (
                barrier.ipc_identity,
                barrier
                    .selected_commit
                    .as_ref()
                    .map_or(self.document_generation, |selection| {
                        selection.destination.document_generation
                    }),
            );
            self.completed_action_frame_selection = barrier
                .selected_commit
                .take()
                .map(|selection| (barrier.action_id, selection));
            self.action_navigation = None;
            #[cfg(unix)]
            crate::ipc_progress::result(
                released.0,
                crate::ipc_progress::Phase::ActionNavigationReleased,
                None,
                Some(released.1),
                Some(true),
            );
        }
        // Failure remains fenced until explicit navigation recovers the page.
        result
    }
}

pub(super) fn replaces_document(command: &PageCommand) -> bool {
    matches!(
        command,
        PageCommand::LoadHtml { .. }
            | PageCommand::ControlledNavigate { .. }
            | PageCommand::Goto { .. }
            | PageCommand::GotoResource { .. }
            | PageCommand::Back { .. }
            | PageCommand::Forward { .. }
            | PageCommand::Refresh { .. }
            | PageCommand::Close { .. }
    )
}

/// One retained document command; later commands stay in the existing bounded
/// mailbox. Internal completion and physical retirement still use their existing
/// control paths. An explicit wait keeps its original elapsed timeout.
pub(super) struct Pending {
    command: PageCommand,
    started: Instant,
    wait_deadline: Option<Instant>,
    wait_overflow: bool,
}
impl Pending {
    pub(super) fn from_command(
        command: &mut Option<PageCommand>,
        state: &PageState,
    ) -> Option<Self> {
        if !targets_document(command.as_ref()?) {
            return None;
        }
        let started = Instant::now();
        let incoming = command.take().unwrap();
        let wait_timeout = match &incoming {
            PageCommand::WaitFor { .. } => Some(state.config.command_timeout),
            PageCommand::WaitForUntil { timeout, .. } => *timeout,
            _ => None,
        };
        // Wait construction validates overflow in the existing wait machinery.
        let wait_deadline = wait_timeout.and_then(|timeout| started.checked_add(timeout));
        Some(Self {
            command: incoming,
            started,
            wait_deadline,
            wait_overflow: wait_timeout.is_some() && wait_deadline.is_none(),
        })
    }
    pub(super) fn next_delay(&self) -> Duration {
        self.wait_deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_millis(5))
            .min(Duration::from_millis(5))
    }
    pub(super) fn expired(&self) -> Option<RuntimeError> {
        if self.wait_overflow {
            return Some(RuntimeError::WaitUnsatisfied(
                "wait deadline overflow".into(),
            ));
        }
        if self
            .wait_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            let predicate = match &self.command {
                PageCommand::WaitFor { predicate, .. }
                | PageCommand::WaitForUntil { predicate, .. } => predicate,
                _ => unreachable!("only explicit waits have a predicate deadline"),
            };
            Some(RuntimeError::WaitUnsatisfied(format!("{predicate:?}")))
        } else {
            None
        }
    }
    pub(super) fn resume(mut self) -> Option<PageCommand> {
        if let Some(error) = self.expired() {
            self.reject(error);
            return None;
        }
        self.command = match self.command {
            PageCommand::WaitFor { predicate, reply } => PageCommand::WaitForUntil {
                predicate,
                reply,
                timeout: self
                    .wait_deadline
                    .map(|deadline| deadline.saturating_duration_since(Instant::now())),
            },
            PageCommand::WaitForUntil {
                predicate,
                timeout,
                reply,
            } => PageCommand::WaitForUntil {
                predicate,
                reply,
                timeout: timeout.map(|timeout| timeout.saturating_sub(self.started.elapsed())),
            },
            command => command,
        };
        Some(self.command)
    }
    pub(super) fn closed(&self) -> bool {
        reply_closed(&self.command)
    }
    pub(super) fn reject(self, error: RuntimeError) {
        reject(self.command, error);
    }
}

fn targets_document(command: &PageCommand) -> bool {
    match command {
        PageCommand::WaitFor { .. } => true,
        PageCommand::WaitForUntil { .. } => true,
        PageCommand::Evaluate { .. } => true,
        PageCommand::ResolveFocusedForm { .. } => true,
        PageCommand::ResolveSelector { .. } => true,
        PageCommand::ResolveTarget { .. } => true,
        PageCommand::ResolveSemantic { .. } => true,
        PageCommand::Extract { .. } => true,
        PageCommand::Observe { .. } => true,
        PageCommand::ObserveWith { .. } => true,
        PageCommand::ObserveSemantic { .. } => true,
        PageCommand::ObserveSemanticV6 { .. } => true,
        PageCommand::ComputedStyle { .. } => true,
        PageCommand::AccessibilitySnapshot { .. } => true,
        PageCommand::AccessibilityText { .. } => true,
        PageCommand::BoxModel { .. } => true,
        PageCommand::PaintSnapshot { .. } => true,
        PageCommand::PrintSnapshot { .. } => true,
        PageCommand::SetViewport { .. } => true,
        PageCommand::SetMedia { .. } => true,
        PageCommand::Scroll { .. } => true,
        PageCommand::ScrollIntoView { .. } => true,
        PageCommand::ObserveDelta { .. } => true,
        PageCommand::ObserveSemanticDelta { .. } => true,
        PageCommand::Execute { .. } => true,
        PageCommand::ExecuteForced { .. } => true,
        PageCommand::ExecuteSensitive { .. } => true,
        PageCommand::SetFiles { .. } => true,
        PageCommand::WebdriverScript { .. } => true,
        PageCommand::WebdriverFrames { .. } => true,
        PageCommand::WebdriverActions { .. } => true,
        #[cfg(unix)]
        PageCommand::ScopedWorker { .. } => true,
        #[cfg(unix)]
        PageCommand::ScopedOwnedActions { .. } => true,
        PageCommand::ControlledNavigate { .. } => false,
        PageCommand::LoadHtml { .. } => false,
        PageCommand::Goto { .. } => false,
        PageCommand::GotoResource { .. } => false,
        PageCommand::Back { .. } => false,
        PageCommand::Forward { .. } => false,
        PageCommand::Refresh { .. } => false,
        PageCommand::Capabilities { .. } => false,
        PageCommand::ChallengeAssessment { .. } => false,
        PageCommand::Trace { .. } => false,
        PageCommand::Close { .. } => false,
        PageCommand::PaintSnapshotCompleted { .. } => false,
        PageCommand::AccessibilitySnapshotCompleted { .. } => false,
        #[cfg(unix)]
        PageCommand::ScriptCloseHistory { .. } | PageCommand::PrepareScriptClose { .. } => false,
        PageCommand::PollContextEvents => false,
    }
}

fn reply_closed(command: &PageCommand) -> bool {
    match command {
        PageCommand::WaitFor { reply, .. } => reply.is_closed(),
        PageCommand::WaitForUntil { reply, .. } => reply.is_closed(),
        PageCommand::Evaluate { reply, .. } => reply.is_closed(),
        PageCommand::ResolveFocusedForm { reply, .. } => reply.is_closed(),
        PageCommand::ResolveSelector { reply, .. } => reply.is_closed(),
        PageCommand::ResolveTarget { reply, .. } => reply.is_closed(),
        PageCommand::ResolveSemantic { reply, .. } => reply.is_closed(),
        PageCommand::Extract { reply, .. } => reply.is_closed(),
        PageCommand::Observe { reply, .. } => reply.is_closed(),
        PageCommand::ObserveWith { reply, .. } => reply.is_closed(),
        PageCommand::ObserveSemantic { reply, .. } => reply.is_closed(),
        PageCommand::ObserveSemanticV6 { reply, .. } => reply.is_closed(),
        PageCommand::ComputedStyle { reply, .. } => reply.is_closed(),
        PageCommand::AccessibilitySnapshot { reply, .. } => reply.is_closed(),
        PageCommand::AccessibilityText { reply, .. } => reply.is_closed(),
        PageCommand::BoxModel { reply, .. } => reply.is_closed(),
        PageCommand::PaintSnapshot { reply, .. } => reply.is_closed(),
        PageCommand::PrintSnapshot { reply, .. } => reply.is_closed(),
        PageCommand::SetViewport { reply, .. } => reply.is_closed(),
        PageCommand::SetMedia { reply, .. } => reply.is_closed(),
        PageCommand::Scroll { reply, .. } => reply.is_closed(),
        PageCommand::ScrollIntoView { reply, .. } => reply.is_closed(),
        PageCommand::ObserveDelta { reply, .. } => reply.is_closed(),
        PageCommand::ObserveSemanticDelta { reply, .. } => reply.is_closed(),
        PageCommand::Execute { reply, .. } => reply.is_closed(),
        PageCommand::ExecuteForced { reply, .. } => reply.is_closed(),
        PageCommand::ExecuteSensitive { reply, .. } => reply.is_closed(),
        PageCommand::SetFiles { reply, .. } => reply.is_closed(),
        PageCommand::WebdriverScript { reply, .. } => reply.is_closed(),
        PageCommand::WebdriverFrames { reply, .. } => reply.is_closed(),
        PageCommand::WebdriverActions { reply, .. } => reply.is_closed(),
        #[cfg(unix)]
        PageCommand::ScopedWorker { reply, .. } => reply.is_closed(),
        #[cfg(unix)]
        PageCommand::ScopedOwnedActions { reply, .. } => reply.is_closed(),
        _ => unreachable!("retained command requires a document reply"),
    }
}

fn reject(command: PageCommand, error: RuntimeError) {
    match command {
        PageCommand::WaitFor { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::WaitForUntil { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::Evaluate { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ResolveFocusedForm { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ResolveSelector { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ResolveTarget { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ResolveSemantic { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::Extract { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::Observe { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ObserveWith { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ObserveSemantic { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ObserveSemanticV6 { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ComputedStyle { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::AccessibilitySnapshot { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::AccessibilityText { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::BoxModel { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::PaintSnapshot { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::PrintSnapshot { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::SetViewport { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::SetMedia { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::Scroll { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ScrollIntoView { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ObserveDelta { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ObserveSemanticDelta { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::Execute { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ExecuteForced { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::ExecuteSensitive { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::SetFiles { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        #[cfg(unix)]
        PageCommand::ScopedWorker { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        #[cfg(unix)]
        PageCommand::ScopedOwnedActions { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        PageCommand::WebdriverScript { reply, .. } => {
            let _ = reply.send(Err(script_error(error)));
        }
        PageCommand::WebdriverFrames { reply, .. } => {
            let _ = reply.send(Err(script_error(error)));
        }
        PageCommand::WebdriverActions { reply, .. } => {
            let _ = reply.send((Err(script_error(error)), Default::default(), Vec::new()));
        }
        _ => unreachable!("retained command requires a document reply"),
    }
}
fn script_error(error: RuntimeError) -> crate::host::webdriver_script::ScriptError {
    crate::host::webdriver_script::ScriptError {
        code: if matches!(
            error.root_cause(),
            RuntimeError::Closed | RuntimeError::PageTaskYield
        ) {
            "no such window"
        } else {
            "unknown error"
        }
        .into(),
        message: error.to_string(),
    }
}

#[cfg(all(test, unix))]
mod readiness_release_tests {
    use super::*;

    #[test]
    fn readiness_trace_keeps_failed_action_fenced_and_does_not_rearm_deadlines() {
        let config = BrowserConfig::default();
        let command_timeout = config.command_timeout;
        let navigation_timeout = config.navigation_timeout;
        let mut state = PageState::new(
            PageId(890_303),
            None,
            config,
            ContextOptions::default(),
            None,
            None,
        );
        let (sender, receiver) = oneshot::channel();
        let mut barrier = Barrier::new(ActionId(71));
        barrier.completion = Some(receiver);
        state.action_navigation = Some(barrier);
        assert!(state.poll_action_navigation().is_pending());
        sender.send(Err(RuntimeError::NavigationCancelled)).unwrap();
        for _ in 0..3 {
            assert!(matches!(
                state.poll_action_navigation(),
                Poll::Ready(Err(RuntimeError::ActionExecutionFailed {
                    action_id: ActionId(71),
                    ..
                }))
            ));
            assert!(
                state.action_navigation.is_some(),
                "failed ownership must not emit a release"
            );
        }
        assert_eq!(state.config.command_timeout, command_timeout);
        assert_eq!(state.config.navigation_timeout, navigation_timeout);
    }
}
