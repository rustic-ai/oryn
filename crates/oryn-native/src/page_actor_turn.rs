//! A root-local command turn. The surrounding actor owns scheduling and must
//! destroy root state before acknowledging Close. Navigation and capture reply
//! queues travel with this root, never with whichever root ran previously.
use super::*;

pub(super) fn dispatch(
    state: &mut PageState,
    command: PageCommand,
    _context_owner: &BrowserContext,
    #[cfg(feature = "v8-host")] pending_navigations: &mut Vec<PendingNavigationReply>,
    #[cfg(feature = "v8-host")] pending_captures: &mut VecDeque<capture_readiness::Pending>,
    #[cfg(feature = "v8-host")] paint_completion_sender: &mpsc::WeakSender<PageCommand>,
) -> Option<oneshot::Sender<()>> {
    #[cfg(all(unix, feature = "v8-host"))]
    let command = if state.script_close_departure_completed() {
        reject_after_departure(command)?
    } else {
        command
    };
    match command {
        PageCommand::LoadHtml {
            #[cfg(all(unix, feature = "v8-host"))]
            policy_token,
            url,
            html,
            reply,
            ..
        } => {
            #[cfg(feature = "v8-host")]
            state.cancel_navigation_transaction(RuntimeError::NavigationCancelled);
            state.trace.push(
                Revision(state.revision),
                None,
                TraceEventKind::NavigationStarted { url: url.clone() },
            );
            #[cfg(not(all(unix, feature = "v8-host")))]
            if let Err(error) =
                _context_owner.install_document_security(state.page_id, url.clone(), &[])
            {
                let _ = reply.send(Err(error));
                return None;
            }
            #[cfg(feature = "v8-host")]
            if let Some(pending) =
                state.dispatch_document_reply(NavigationReply::Html(reply), |state| {
                    #[cfg(all(unix, feature = "v8-host"))]
                    {
                        state.load_html_policy = Some((
                            policy_token.ok_or_else(|| {
                                RuntimeError::ProtocolViolation(
                                    "load HTML policy grant missing".into(),
                                )
                            })?,
                            format!("{:x}", Sha256::digest(html.as_bytes())),
                        ));
                    }
                    let loaded = state.load_document_with_redirect_history(
                        url.clone(),
                        &html,
                        None,
                        None,
                        HistoryUpdate::Push,
                        &BTreeMap::new(),
                        Vec::new(),
                        false,
                        Some(Vec::new()),
                    );
                    #[cfg(all(unix, feature = "v8-host"))]
                    {
                        state.load_html_policy = None;
                    }
                    loaded?;
                    state.set_challenge_assessment(assess_challenge(200, &[], &html));
                    Ok(NavigationProgress::Ready(NavigationResult {
                        page_id: state.page_id,
                        requested_url: url,
                        final_url: state.url.clone(),
                        title: state
                            .document
                            .as_ref()
                            .map(document_title)
                            .unwrap_or_default(),
                        status: 200,
                        document_generation: state.document_generation,
                        revision: Revision(state.revision),
                        lifecycle: state.lifecycle.clone(),
                        challenge: state.last_challenge.clone(),
                    }))
                })
            {
                pending_navigations.push(pending);
            }
            #[cfg(not(feature = "v8-host"))]
            {
                if let Err(error) = state.load_document_with_redirect_history(
                    url.clone(),
                    &html,
                    None,
                    None,
                    HistoryUpdate::Push,
                    &BTreeMap::new(),
                    #[cfg(feature = "v8-host")]
                    Vec::new(),
                    false,
                    Some(Vec::new()),
                ) {
                    let _ = reply.send(Err(error));
                    return None;
                }
                state.set_challenge_assessment(assess_challenge(200, &[], &html));
                let observation = state.observation(ProjectionProfile::Compact);
                let _ = reply.send(Ok((Revision(state.revision), observation.page)));
            }
        }
        PageCommand::ControlledNavigate {
            request,
            emitter,
            reply,
        } => {
            #[cfg(feature = "v8-host")]
            let strategy = request.strategy;
            let result = state.controlled_navigation(request, emitter.clone());
            #[cfg(feature="v8-host")]
                        if result.is_ok()&&let Some(transaction)=state.pending_navigation_transaction.as_mut()&&!state.pending_execution_replies.iter().any(|pending|matches!(&pending.reply,NavigationReply::JointEvents(owner,_)if owner.same_job(&emitter))){
                            if strategy==crate::navigation_control::PageLoadStrategy::None{transaction.admission_reply=Some(reply);}
                            else{transaction.reply=Some(NavigationReply::Controlled(reply));}
                        }else if result.is_ok(){
                            if let Some(pending)=state.pending_execution_replies.iter_mut().rev().find(|pending|matches!(&pending.reply,NavigationReply::JointEvents(owner,_)if owner.same_job(&emitter))){
                                if strategy==crate::navigation_control::PageLoadStrategy::None{let _=reply.send(Ok(()));}
                                else if let NavigationReply::JointEvents(_,slot)=&mut pending.reply{*slot=Some(reply);}
                            }else{let _=reply.send(result);}
                        }else{let _=reply.send(result);}
            #[cfg(not(feature = "v8-host"))]
            {
                let _ = reply.send(result);
            }
        }
        PageCommand::Goto { url, reply } => {
            #[cfg(feature = "v8-host")]
            if let Some(pending) = state.dispatch_navigation_reply(reply, |state| {
                state.begin_browser_navigation(url, HistoryUpdate::Push, None, None, None)
            }) {
                pending_navigations.push(pending);
            }
            #[cfg(not(feature = "v8-host"))]
            let _ = reply.send(state.navigate(url, HistoryUpdate::Push));
        }
        PageCommand::GotoResource {
            service_authority,
            form_authority,
            url,
            post,
            referrer_policy,
            reply,
        } => {
            #[cfg(feature = "v8-host")]
            if let Some(pending) = state.dispatch_navigation_reply(reply, |state| {
                state.begin_command_navigation(
                    url,
                    post,
                    referrer_policy,
                    form_authority,
                    service_authority,
                )
            }) {
                pending_navigations.push(pending);
            }
            #[cfg(not(feature = "v8-host"))]
            let _ = reply.send(state.navigate_command_resource(
                url,
                post,
                referrer_policy,
                form_authority,
                service_authority,
            ));
        }
        PageCommand::Back { reply } => {
            #[cfg(feature = "v8-host")]
            if let Some(pending) =
                state.dispatch_navigation_reply(reply, |state| state.begin_history_navigation(-1))
            {
                pending_navigations.push(pending);
            }
            #[cfg(not(feature = "v8-host"))]
            let _ = reply.send(state.traverse_history(-1));
        }
        PageCommand::Forward { reply } => {
            #[cfg(feature = "v8-host")]
            if let Some(pending) =
                state.dispatch_navigation_reply(reply, |state| state.begin_history_navigation(1))
            {
                pending_navigations.push(pending);
            }
            #[cfg(not(feature = "v8-host"))]
            let _ = reply.send(state.traverse_history(1));
        }
        PageCommand::Refresh { reply } => {
            #[cfg(feature = "v8-host")]
            if let Some(pending) =
                state.dispatch_navigation_reply(reply, |state| state.begin_refresh_navigation())
            {
                pending_navigations.push(pending);
            }
            #[cfg(not(feature = "v8-host"))]
            let _ = reply.send(state.refresh());
        }
        PageCommand::WaitFor { predicate, reply } => {
            let _ = reply.send(state.wait_for(&predicate));
        }
        PageCommand::WaitForUntil {
            predicate,
            timeout,
            reply,
        } => {
            let _ = reply.send(state.wait_for_timeout(&predicate, timeout));
        }
        #[cfg(unix)]
        PageCommand::ScopedWorker {
            scope,
            request,
            reply,
        } => {
            #[cfg(feature = "v8-host")]
            if let crate::worker::WorkerRequest::AccessibilityText { target, command } = request {
                state.enqueue_accessibility(
                    Some(scope),
                    target,
                    command,
                    AccessibilityReply::Worker(reply),
                );
                return None;
            }
            let _ = reply.send(state.scoped_worker_request(scope, request));
        }
        PageCommand::WebdriverFrames { command, reply } => {
            let _ = reply.send(state.webdriver_frames(command));
        }
        PageCommand::WebdriverActions {
            command,
            context,
            cancellation,
            reply,
        } => {
            let result = state.execute_webdriver_actions(command, context, cancellation);
            let feedback = state.take_drag_feedback();
            #[cfg(feature = "v8-host")]
            let receipts = state.webdriver_input.take_os_receipts();
            #[cfg(not(feature = "v8-host"))]
            let receipts = Vec::new();
            let _ = reply.send((result, feedback, receipts));
        }
        PageCommand::WebdriverScript {
            request,
            context,
            cancellation,
            reply,
        } => {
            let _ = reply.send(state.execute_webdriver_script(request, context, cancellation));
        }
        PageCommand::Evaluate { source, reply } => {
            let _ = reply.send(state.evaluate(&source));
        }
        PageCommand::PollContextEvents => {
            if let Err(error) = state.poll_page_tasks() {
                #[cfg(feature = "v8-host")]
                for pending in pending_navigations.drain(..) {
                    if let Some(receipt) = &pending.joint {
                        receipt.cancel();
                    }
                    pending
                        .reply
                        .complete(state, Err(RuntimeError::Network(error.to_string())));
                }
                state.trace.push(
                    Revision(state.revision),
                    None,
                    TraceEventKind::ScriptDiagnostic {
                        capability: "scheduler.page_task".into(),
                        detail: error.to_string(),
                    },
                );
            }
        }
        #[cfg(unix)]
        PageCommand::ScriptCloseHistory { reply } => {
            #[cfg(feature = "v8-host")]
            let result = state
                .executable
                .as_ref()
                .ok_or(RuntimeError::EvaluationUnavailable)
                .and_then(|document| document.script_close_history().map_err(map_host_error));
            #[cfg(not(feature = "v8-host"))]
            let result = Err(RuntimeError::EvaluationUnavailable);
            let _ = reply.send(result);
        }
        #[cfg(unix)]
        PageCommand::PrepareScriptClose {
            #[cfg(feature = "v8-host")]
            token,
            #[cfg(feature = "v8-host")]
            expected,
            reply,
            ..
        } => {
            #[cfg(feature = "v8-host")]
            let result = state.poll_script_close_preparation(token, expected);
            #[cfg(not(feature = "v8-host"))]
            let result = Err(RuntimeError::EvaluationUnavailable);
            let _ = reply.send(result);
        }
        PageCommand::Capabilities { reply } => {
            let _ = reply.send(state.capabilities());
        }
        PageCommand::ChallengeAssessment { reply } => {
            let _ = reply.send(state.last_challenge.clone());
        }
        PageCommand::ResolveFocusedForm { reply } => {
            let _ = reply.send(state.resolve_focused_form());
        }
        PageCommand::ResolveSelector {
            selector,
            action,
            reply,
        } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.resolve_selector(&selector, action)),
            );
        }
        PageCommand::ResolveTarget {
            target,
            action,
            reply,
        } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.resolve_target(&target, action)),
            );
        }
        PageCommand::ResolveSemantic {
            target,
            action,
            reply,
        } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.resolve_semantic(&target, action)),
            );
        }
        PageCommand::Extract {
            kind,
            selector,
            reply,
        } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.extract(kind, selector.as_deref())),
            );
        }
        PageCommand::Observe { profile, reply } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .map(|_| state.observation(profile)),
            );
        }
        PageCommand::ObserveWith { options, reply } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.observation_v4(options)),
            );
        }
        PageCommand::ObserveSemantic { profile, reply } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .map(|_| state.semantic_observation(profile)),
            );
        }
        PageCommand::ObserveSemanticV6 { options, reply } => {
            let result = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.semantic_observation_v6(options)),
            );
            if let Err(Ok(observation)) = result {
                crate::semantic::dispose_semantic_observation(observation);
            }
        }
        PageCommand::ComputedStyle { target, reply } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.computed_style(target)),
            );
        }
        PageCommand::AccessibilitySnapshot { reply } => {
            #[cfg(feature = "v8-host")]
            {
                let pending = capture_readiness::Pending::new(
                    state,
                    capture_readiness::Reply::Accessibility(reply),
                );
                if pending_captures.len() >= 32 {
                    pending.reject(
                        state,
                        RuntimeError::PaintUnavailable("capture readiness queue is full".into()),
                    );
                } else {
                    pending_captures.push_back(pending);
                }
            }
            #[cfg(not(feature = "v8-host"))]
            {
                let _ = reply.send(state.accessibility_snapshot());
            }
        }
        #[cfg(feature = "v8-host")]
        PageCommand::AccessibilitySnapshotCompleted {
            generation,
            stamp,
            result,
            reply,
        } => {
            let current = generation == state.document_generation
                && state
                    .executable
                    .as_ref()
                    .and_then(|page| page.root_paint_inventory_stamp())
                    == stamp;
            let _ = reply.send(if current {
                *result
            } else {
                Err(RuntimeError::StaleSemanticRef)
            });
        }
        PageCommand::AccessibilityText {
            target,
            command,
            reply,
        } => {
            #[cfg(feature = "v8-host")]
            state.enqueue_accessibility(None, target, command, AccessibilityReply::Text(reply));
            #[cfg(not(feature = "v8-host"))]
            {
                let _ = reply.send(state.accessibility_text(target, command));
            }
        }
        PageCommand::BoxModel { target, reply } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.box_model(target)),
            );
        }
        PageCommand::PaintSnapshot { reply } => {
            #[cfg(feature = "v8-host")]
            {
                let pending =
                    capture_readiness::Pending::new(state, capture_readiness::Reply::Paint(reply));
                if pending_captures.len() >= 32 {
                    pending.reject(
                        state,
                        RuntimeError::PaintUnavailable("capture readiness queue is full".into()),
                    );
                } else {
                    pending_captures.push_back(pending);
                }
            }
            #[cfg(not(feature = "v8-host"))]
            let _ = reply.send(state.paint_snapshot());
        }
        #[cfg(feature = "v8-host")]
        PageCommand::PaintSnapshotCompleted {
            generation,
            stamp,
            result,
            reply,
        } => {
            if generation == state.document_generation
                && let (Some(page), Some(stamp), Ok(snapshot)) =
                    (state.executable.as_ref(), stamp, result.as_ref())
            {
                // Stale native cursor/epoch is discarded by the
                // executable without replacing the last inventory.
                let _ = page.install_root_paint_capture_inventory(stamp, snapshot);
            }
            let _ = reply.send(result);
        }
        PageCommand::PrintSnapshot { options, reply } => {
            #[cfg(feature = "v8-host")]
            {
                let pending = capture_readiness::Pending::new(
                    state,
                    capture_readiness::Reply::Print(options, reply),
                );
                if pending_captures.len() >= 32 {
                    pending.reject(
                        state,
                        RuntimeError::PaintUnavailable("capture readiness queue is full".into()),
                    );
                } else {
                    pending_captures.push_back(pending);
                }
            }
            #[cfg(not(feature = "v8-host"))]
            {
                let _ = options;
                let _ = reply.send(Err(RuntimeError::PaintUnavailable(
                    "print requires native V8 layout".into(),
                )));
            }
        }
        PageCommand::SetViewport { viewport, reply } => {
            let _ = reply.send(state.set_viewport(viewport));
        }
        PageCommand::SetMedia { media, reply } => {
            let _ = reply.send(state.set_media(media));
        }
        PageCommand::Scroll { request, reply } => {
            let _ = reply.send(state.scroll(request));
        }
        PageCommand::ScrollIntoView { target, reply } => {
            let _ = reply.send(state.scroll_into_view(target));
        }
        PageCommand::ObserveDelta { from, reply } => {
            let _ = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.delta_from(from)),
            );
        }
        PageCommand::ObserveSemanticDelta { from, reply } => {
            let result = reply.send(
                state
                    .refresh_semantic_observation()
                    .and_then(|_| state.semantic_delta_from(from)),
            );
            if let Err(Ok(delta)) = result {
                drop(crate::semantic::OwnedSemanticChanges::new(delta.changes));
            }
        }
        PageCommand::Trace { reply } => {
            #[cfg(feature = "v8-host")]
            if let Some(executable) = state.executable.as_mut() {
                // Trace capture must retain diagnostics even when semantic
                // projection failed. The document cursor prevents duplicates.
                for diagnostic in executable.take_unpublished_diagnostics() {
                    state.trace.push(
                        Revision(state.revision),
                        None,
                        TraceEventKind::ScriptDiagnostic {
                            capability: diagnostic.capability,
                            detail: diagnostic.detail.unwrap_or_default(),
                        },
                    );
                }
            }
            let _ = reply.send(state.trace.snapshot());
        }
        PageCommand::Execute { action, reply } => {
            let _ = reply.send(state.execute(action));
        }
        PageCommand::ExecuteForced { action, reply } => {
            let _ = reply.send(state.execute_forced(action));
        }
        PageCommand::ExecuteSensitive { action, reply } => {
            let _ = reply.send(state.execute_sensitive(action));
        }
        PageCommand::SetFiles {
            target,
            files,
            reply,
        } => {
            let _ = reply.send(state.set_files(target, files));
        }
        PageCommand::Close { reply } => {
            // ResetReady must mean the isolate, retained history,
            // timers and page services are destroyed, not merely
            // that their destruction has been queued.
            #[cfg(feature = "v8-host")]
            for pending in pending_navigations.drain(..) {
                if let Some(receipt) = &pending.joint {
                    receipt.cancel();
                }
                pending.reply.complete(state, Err(RuntimeError::Closed));
            }
            #[cfg(feature = "v8-host")]
            state.cancel_navigation_transaction(RuntimeError::Closed);
            #[cfg(feature = "v8-host")]
            for pending in std::mem::take(&mut state.pending_execution_replies) {
                if let Some(receipt) = &pending.joint {
                    receipt.cancel();
                }
                pending.reply.complete(state, Err(RuntimeError::Closed));
            }
            return Some(reply);
        }
    }
    #[cfg(feature = "v8-host")]
    finish_turn(
        state,
        pending_navigations,
        pending_captures,
        paint_completion_sender,
    );
    None
}

// Shared by ordinary commands and retained wait/WebDriver turns. Page tasks
// are pumped by the command; this bounded suffix owns reply/capture progress.
#[cfg(feature = "v8-host")]
fn finish_turn(
    state: &mut PageState,
    pending_navigations: &mut Vec<PendingNavigationReply>,
    pending_captures: &mut VecDeque<capture_readiness::Pending>,
    paint_completion_sender: &mpsc::WeakSender<PageCommand>,
) {
    if state.terminal_close_requested() {
        return;
    }
    // Retain preexisting reply/capture custody for Close teardown;
    // no author work may run between departure ACK and destruction.
    #[cfg(all(unix, feature = "v8-host"))]
    if state.script_close_departure_completed() {
        return;
    }
    #[cfg(feature = "v8-host")]
    if let Some(pending) = state.poll_navigation_transaction() {
        pending_navigations.push(pending);
    }
    if state.terminal_close_requested() {
        return;
    }
    #[cfg(feature = "v8-host")]
    if let Some(pending) = state.pending_committed_navigation_reply.take() {
        pending_navigations.push(pending);
    }
    #[cfg(feature = "v8-host")]
    {
        let mut waiting = Vec::new();
        for pending in std::mem::take(pending_navigations) {
            if state.terminal_close_requested() {
                waiting.push(pending);
            } else if let Some(pending) = state.poll_navigation_reply(pending) {
                waiting.push(pending);
            }
        }
        *pending_navigations = waiting;
    }
    if state.terminal_close_requested() {
        return;
    }
    #[cfg(feature = "v8-host")]
    {
        // Only the front owns capture/print lifecycle state. Retire
        // expired or cancelled later requests without starting any
        // callbacks or acquiring shared CSS capture custody.
        retire_queued_captures(pending_captures, state);
        if let Some(pending) = pending_captures.pop_front()
            && let Some(pending) = pending.poll(state, paint_completion_sender)
        {
            pending_captures.push_front(pending);
        }
    }
}

/// All root-local pending state stays together when an agent selects a root.
/// No reply queue or receiver is carried implicitly by the previously active
/// document. A shared actor can therefore rotate roots at task boundaries.
pub(super) struct Root {
    close_observer: Option<crate::ipc_progress::close_admission::Registration>,
    // A retained root owns its document state on the heap. Moving a root between
    // agent queues must not copy the full document/prerender state onto every
    // scheduler frame (including unoptimized builds).
    pub(super) state: Box<PageState>,
    #[cfg(all(unix, feature = "v8-host"))]
    close_progress: Option<crate::ipc_progress::Identity>,
    #[cfg(all(unix, feature = "v8-host"))]
    close_progress_reported: u8,
    pending_wait: Option<wait_fairness::Pending>,
    #[cfg(feature = "v8-host")]
    pending_action_command: Option<action_navigation::Pending>,
    #[cfg(feature = "v8-host")]
    pending_webdriver: Option<webdriver_fairness::Pending>,
    #[cfg(feature = "v8-host")]
    terminal_cleanup: Vec<webdriver_fairness::PaintCleanup>,
    receiver: mpsc::Receiver<PageCommand>,
    #[cfg(feature = "v8-host")]
    pending_navigations: Vec<PendingNavigationReply>,
    #[cfg(feature = "v8-host")]
    pending_captures: VecDeque<capture_readiness::Pending>,
    #[cfg(feature = "v8-host")]
    paint_completion_sender: mpsc::WeakSender<PageCommand>,
    #[cfg(feature = "v8-host")]
    _paint_completion_owner: mpsc::Sender<PageCommand>,
    #[cfg(feature = "v8-host")]
    paint_completions: mpsc::Receiver<PageCommand>,
}
impl Root {
    pub(super) fn new(
        state: PageState,
        receiver: mpsc::Receiver<PageCommand>,
        #[cfg(feature = "v8-host")] _paint_completion_sender: mpsc::WeakSender<PageCommand>,
    ) -> Self {
        #[cfg(feature = "v8-host")]
        let (paint_completion_owner, paint_completions) = mpsc::channel(32);
        #[cfg(feature = "v8-host")]
        let paint_completion_sender = paint_completion_owner.downgrade();
        Self {
            close_observer: crate::ipc_progress::close_admission::register(
                state.page_id.0,
                state.config.document_generation_base,
            ),
            #[cfg(all(unix, feature = "v8-host"))]
            close_progress: None,
            #[cfg(all(unix, feature = "v8-host"))]
            close_progress_reported: 0,
            pending_wait: None,
            #[cfg(feature = "v8-host")]
            pending_action_command: None,
            #[cfg(feature = "v8-host")]
            pending_webdriver: None,
            #[cfg(feature = "v8-host")]
            terminal_cleanup: Vec::new(),
            state: Box::new(state),
            receiver,
            #[cfg(feature = "v8-host")]
            pending_navigations: Vec::new(),
            #[cfg(feature = "v8-host")]
            pending_captures: VecDeque::new(),
            #[cfg(feature = "v8-host")]
            paint_completion_sender,
            #[cfg(feature = "v8-host")]
            _paint_completion_owner: paint_completion_owner,
            #[cfg(feature = "v8-host")]
            paint_completions,
        }
    }
    fn try_actor_command(&mut self) -> Result<PageCommand, tokio::sync::mpsc::error::TryRecvError> {
        #[cfg(feature = "v8-host")]
        if let Ok(completion) = self.paint_completions.try_recv() {
            return Ok(completion);
        }
        if self.has_pending_command() {
            if self.receiver.is_closed() && self.receiver.is_empty() {
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            } else {
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            }
        } else {
            self.receiver.try_recv()
        }
    }
    fn poll_terminal_cleanup(&mut self) -> bool {
        #[cfg(feature = "v8-host")]
        {
            let state = &mut self.state;
            self.terminal_cleanup
                .retain_mut(|cleanup| !cleanup.poll_with_trace(state));
            self.terminal_cleanup.is_empty()
        }
        #[cfg(not(feature = "v8-host"))]
        {
            true
        }
    }
    fn finish_pending_turn(&mut self, context: &BrowserContext) {
        let can_finish = context
            .page_prompt_controller(self.state.page_id)
            .is_ok_and(|controller| controller.inspect().is_ok());
        #[cfg(feature = "v8-host")]
        if can_finish {
            finish_turn(
                &mut self.state,
                &mut self.pending_navigations,
                &mut self.pending_captures,
                &self.paint_completion_sender,
            );
        }
        #[cfg(not(feature = "v8-host"))]
        let _ = can_finish;
    }
    fn has_pending_command(&self) -> bool {
        let pending = self.pending_wait.is_some();
        #[cfg(feature = "v8-host")]
        let pending = pending
            || self.pending_action_command.is_some()
            || self.pending_webdriver.is_some()
            || !self.terminal_cleanup.is_empty();
        pending
    }
    fn pending_command_delay(&self) -> Option<Duration> {
        let delay = self
            .pending_wait
            .as_ref()
            .map(|pending| pending.next_delay());
        #[cfg(feature = "v8-host")]
        let delay = delay
            .into_iter()
            .chain(
                self.pending_webdriver
                    .as_ref()
                    .map(|pending| pending.next_delay()),
            )
            .chain(
                self.pending_action_command
                    .as_ref()
                    .map(|pending| pending.next_delay()),
            )
            .chain((!self.terminal_cleanup.is_empty()).then_some(Duration::from_millis(1)))
            .min();
        delay
    }
    fn has_pending_work(&mut self) -> bool {
        if self.has_pending_command() {
            return true;
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if self.state.script_close_departure_completed() {
            return false;
        }
        #[cfg(feature = "v8-host")]
        if !self.pending_captures.is_empty() || !self.pending_navigations.is_empty() {
            return true;
        }
        self.state.has_pending_page_work()
    }
    fn next_work_delay(&mut self) -> Option<Duration> {
        #[cfg(all(unix, feature = "v8-host"))]
        if self.state.script_close_departure_completed() {
            return None;
        }
        let delay = self
            .state
            .next_timer_delay()
            .into_iter()
            .chain(self.pending_command_delay())
            .min();
        #[cfg(feature = "v8-host")]
        {
            delay
                .into_iter()
                .chain(
                    self.pending_captures
                        .iter()
                        .enumerate()
                        .map(|(index, pending)| {
                            if index == 0 {
                                pending.next_delay(&self.state)
                            } else {
                                pending.queued_delay(&self.state)
                            }
                        }),
                )
                .chain((!self.pending_navigations.is_empty()).then_some(Duration::from_millis(10)))
                .min()
        }
        #[cfg(not(feature = "v8-host"))]
        delay
    }
    fn has_runnable_work(&mut self) -> bool {
        if self
            .pending_command_delay()
            .is_some_and(|delay| delay.is_zero())
        {
            return true;
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if self.state.script_close_departure_completed() {
            return false;
        }
        #[cfg(feature = "v8-host")]
        if self
            .pending_captures
            .iter()
            .enumerate()
            .any(|(index, pending)| {
                if index == 0 {
                    pending.next_delay(&self.state).is_zero()
                } else {
                    pending.queued_delay(&self.state).is_zero()
                }
            })
        {
            return true;
        }
        self.state.has_runnable_page_work()
    }
    fn poll_delay(&mut self, timer_delay: Option<Duration>) -> Duration {
        // Document work includes external completions that still need the
        // regular polling cadence. A queue containing only retained actor
        // operations already supplies its own retry/cleanup deadline.
        actor_poll_delay(
            timer_delay,
            self.has_runnable_work(),
            self.state.has_pending_page_work(),
        )
    }
    pub(super) fn next_command(
        &mut self,
        scheduler: &Option<tokio::runtime::Runtime>,
    ) -> Option<PageCommand> {
        #[cfg(feature = "v8-host")]
        if let Ok(completion) = self.paint_completions.try_recv() {
            return Some(completion);
        }
        if self.has_pending_command() {
            if self.receiver.is_closed() && self.receiver.is_empty() {
                return None;
            }
            // Keep later same-page commands in the bounded receiver. Only the
            // retained command advances; related roots still get actor turns.
            let delay = self
                .pending_command_delay()
                .unwrap_or(Duration::from_millis(1));
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            return Some(PageCommand::PollContextEvents);
        }
        if self.has_pending_work() {
            let timer_delay = self.next_work_delay();
            let delay = self.poll_delay(timer_delay);
            // A due timer is runnable now. Passing it through Tokio's
            // timer wheel adds a tick even for a zero-duration sleep.
            // Keep commands ahead of timer work, including shutdown.
            if timer_delay.is_some_and(|delay| delay.is_zero()) {
                match self.try_actor_command() {
                    Ok(command) => Some(command),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        Some(PageCommand::PollContextEvents)
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => None,
                }
            } else {
                scheduler.as_ref().and_then(|runtime| {
                    runtime.block_on(async {
                        #[cfg(feature = "v8-host")]
                        let command = tokio::select! {
                            biased;
                            completion = self.paint_completions.recv() => completion,
                            command = self.receiver.recv() => command,
                            _ = tokio::time::sleep(delay) => Some(PageCommand::PollContextEvents),
                        };
                        #[cfg(not(feature = "v8-host"))]
                        let command = tokio::select! {
                            biased;
                            command = self.receiver.recv() => command,
                            _ = tokio::time::sleep(delay) => Some(PageCommand::PollContextEvents),
                        };
                        command
                    })
                })
            }
        } else {
            #[cfg(feature = "v8-host")]
            {
                if scheduler.is_none() {
                    loop {
                        match self.try_actor_command() {
                            Ok(command) => return Some(command),
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                                return None;
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                                std::thread::sleep(Duration::from_millis(1))
                            }
                        }
                    }
                }
                scheduler.as_ref().and_then(|runtime| {
                    runtime.block_on(async {
                        tokio::select! {
                            biased;
                            completion = self.paint_completions.recv() => completion,
                            command = self.receiver.recv() => command,
                        }
                    })
                })
            }
            #[cfg(not(feature = "v8-host"))]
            {
                self.receiver.blocking_recv()
            }
        }
    }
    /// Fallback actors use the same command-first fairness as shared actors.
    pub(super) fn dispatch_fallback(
        &mut self,
        command: PageCommand,
        context: &BrowserContext,
        next_poll: &mut std::time::Instant,
    ) -> Option<oneshot::Sender<()>> {
        let polled = matches!(&command, PageCommand::PollContextEvents);
        let close = self.dispatch(command, context);
        if close.is_some() {
            return close;
        }
        if polled || (self.has_pending_work() && std::time::Instant::now() >= *next_poll) {
            if !polled {
                self.dispatch(PageCommand::PollContextEvents, context);
            }
            let timer_delay = self.next_work_delay();
            let delay = self.poll_delay(timer_delay);
            let finished = std::time::Instant::now();
            *next_poll = finished.checked_add(delay).unwrap_or(finished);
        }
        None
    }
    pub(super) fn dispatch(
        &mut self,
        command: PageCommand,
        context: &BrowserContext,
    ) -> Option<oneshot::Sender<()>> {
        #[cfg(feature = "v8-host")]
        if self.state.page_task_terminal_control.controller().is_none() {
            match context.page_prompt_controller(self.state.page_id) {
                Ok(controller) => {
                    self.state.page_task_terminal_control.bind(controller);
                }
                Err(_) => self.state.page_task_terminal_control.lookup_failed(),
            }
        }
        #[cfg(all(unix, feature = "v8-host"))]
        let navigation_owner = if matches!(&command, PageCommand::PollContextEvents) {
            self.state
                .pending_navigation_transaction
                .as_ref()
                .and_then(|pending| pending.ipc_identity)
                .or_else(|| {
                    self.state
                        .committed_navigation_progress
                        .as_ref()
                        .map(|progress| progress.owner)
                })
        } else {
            None
        };
        #[cfg(not(all(unix, feature = "v8-host")))]
        let navigation_owner = None;
        let _close_turn = crate::ipc_progress::close_admission::turn(
            self.close_observer.as_ref(),
            if matches!(&command, PageCommand::PollContextEvents) {
                "actor_poll_context_events"
            } else {
                "actor_dispatch"
            },
            navigation_owner,
        );
        #[cfg(all(unix, feature = "v8-host"))]
        if matches!(&command, PageCommand::Close { .. }) && self.close_progress.is_none() {
            self.close_progress = crate::ipc_progress::capture(self.state.page_id.0);
            if let Some(identity) = self.close_progress {
                crate::ipc_progress::record(
                    Some(identity),
                    crate::ipc_progress::Phase::CloseActorEntered,
                    None,
                    Some(self.state.document_generation),
                );
            }
        }
        self.poll_terminal_cleanup();
        #[cfg(all(unix, feature = "v8-host"))]
        let command = if self.state.script_close_departure_completed() {
            self.cancel_pending_commands();
            reject_after_departure(command)?
        } else {
            command
        };
        if matches!(&command, PageCommand::PollContextEvents) && self.has_pending_command() {
            #[cfg(feature = "v8-host")]
            if self.pending_action_command.is_some() {
                self.poll_action_command(context);
                return None;
            }
            if let Some(pending) = self.pending_wait.take() {
                self.pending_wait = pending.poll(&mut self.state);
            }
            #[cfg(feature = "v8-host")]
            if let Some(pending) = self.pending_webdriver.take() {
                self.pending_webdriver = pending.poll(&mut self.state);
            }
            self.finish_pending_turn(context);
            return None;
        }
        #[cfg(feature = "v8-host")]
        if action_navigation::replaces_document(&command) {
            // Explicit recovery owns the next navigation. Dropping this receiver
            // retires the previous fence; ordinary navigation cancellation retains
            // the old transaction's terminal publication and transport cleanup.
            self.state.action_navigation = None;
        }
        #[cfg(feature = "v8-host")]
        let _ = self.state.poll_action_navigation();
        let mut command = Some(command);
        #[cfg(feature = "v8-host")]
        if self.state.action_navigation.is_some()
            && let Some(pending) =
                action_navigation::Pending::from_command(&mut command, &self.state)
        {
            self.pending_action_command = Some(pending);
            self.poll_action_command(context);
            return None;
        }
        if let Some(pending) = wait_fairness::Pending::from_command(&mut command, &self.state) {
            match context.page_prompt_controller(self.state.page_id) {
                Ok(controller) => {
                    self.pending_wait = pending.with_cancellation(controller).poll(&mut self.state)
                }
                Err(error) => pending.reject(error),
            }
            self.finish_pending_turn(context);
            return None;
        }
        #[cfg(feature = "v8-host")]
        if let Some(pending) = webdriver_fairness::Pending::from_command(&mut command, &self.state)
        {
            match context.page_prompt_controller(self.state.page_id) {
                Ok(controller) => {
                    self.pending_webdriver =
                        pending.with_cancellation(controller).poll(&mut self.state)
                }
                Err(error) => {
                    if let Some(cleanup) = pending.reject(&mut self.state, error) {
                        self.terminal_cleanup.push(cleanup);
                    }
                }
            }
            self.finish_pending_turn(context);
            return None;
        }
        let command = command.expect("unclaimed command retained");
        dispatch(
            &mut self.state,
            command,
            context,
            #[cfg(feature = "v8-host")]
            &mut self.pending_navigations,
            #[cfg(feature = "v8-host")]
            &mut self.pending_captures,
            #[cfg(feature = "v8-host")]
            &self.paint_completion_sender,
        )
    }
}
impl Root {
    #[cfg(all(unix, feature = "v8-host"))]
    pub(super) fn close_progress(&self) -> Option<crate::ipc_progress::Identity> {
        self.close_progress
    }
    #[cfg(all(unix, feature = "v8-host"))]
    fn report_close_pending(&mut self, bit: u8, phase: crate::ipc_progress::Phase) {
        if self.close_progress_reported & bit == 0
            && let Some(identity) = self.close_progress
        {
            self.close_progress_reported |= bit;
            crate::ipc_progress::record(
                Some(identity),
                phase,
                None,
                Some(self.state.document_generation),
            );
        }
    }
    #[cfg(feature = "v8-host")]
    fn poll_action_command(&mut self, context: &BrowserContext) {
        let Some(pending) = self.pending_action_command.take() else {
            return;
        };
        if pending.closed() {
            return;
        }
        if context
            .page_prompt_controller(self.state.page_id)
            .map_or(true, |controller| controller.inspect().is_err())
        {
            pending.reject(RuntimeError::Closed);
            return;
        }
        if let Some(error) = pending.expired() {
            pending.reject(error);
            return;
        }
        // One bounded native task turn, then the existing transaction/readiness
        // suffix. Never await network I/O while owning this root's unique host.
        if let Err(error) = self.state.poll_page_tasks() {
            self.state.fail_action_navigation(error.clone());
            self.state.cancel_navigation_transaction(error);
        }
        self.finish_pending_turn(context);
        match self.state.poll_action_navigation() {
            std::task::Poll::Pending => self.pending_action_command = Some(pending),
            std::task::Poll::Ready(Err(error)) => pending.reject(error),
            std::task::Poll::Ready(Ok(())) => {
                // Only the next command is resumed; the admitted action is never
                // retained here and cannot be submitted for a second time.
                if let Some(command) = pending.resume() {
                    let _ = self.dispatch(command, context);
                }
            }
        }
    }

    fn cancel_pending_commands(&mut self) {
        #[cfg(feature = "v8-host")]
        if let Some(pending) = self.pending_action_command.take() {
            pending.reject(RuntimeError::Closed);
        }
        if let Some(pending) = self.pending_wait.take() {
            pending.reject(RuntimeError::Closed);
        }
        #[cfg(feature = "v8-host")]
        if let Some(pending) = self.pending_webdriver.take()
            && let Some(cleanup) = pending.reject(&mut self.state, RuntimeError::Closed)
        {
            self.terminal_cleanup.push(cleanup);
        }
    }
    fn shutdown(&mut self) {
        self.cancel_pending_commands();
        #[cfg(feature = "v8-host")]
        {
            self.paint_completions.close();
            while let Ok(completion) = self.paint_completions.try_recv() {
                match completion {
                    PageCommand::PaintSnapshotCompleted { reply, .. } => {
                        let _ = reply.send(Err(RuntimeError::Closed));
                    }
                    PageCommand::AccessibilitySnapshotCompleted { reply, .. } => {
                        let _ = reply.send(Err(RuntimeError::Closed));
                    }
                    _ => unreachable!("private capture completion channel"),
                }
            }
        }
        #[cfg(feature = "v8-host")]
        {
            // A rejected/retired root may no longer acquire the shared host.
            // Terminal reply cleanup must not dereference a detached driver or
            // run its author callbacks in a sibling root's active context.
            let detached = if self
                .state
                .executable
                .as_ref()
                .is_some_and(|page| !page.has_agent_host())
            {
                self.state.executable.take()
            } else {
                None
            };
            self.state
                .cancel_navigation_transaction(RuntimeError::Closed);
            for pending in std::mem::take(&mut self.state.pending_execution_replies) {
                if let Some(receipt) = &pending.joint {
                    receipt.cancel();
                }
                pending
                    .reply
                    .complete(&mut self.state, Err(RuntimeError::Closed));
            }
            for pending in self.pending_navigations.drain(..) {
                if let Some(receipt) = &pending.joint {
                    receipt.cancel();
                }
                pending
                    .reply
                    .complete(&mut self.state, Err(RuntimeError::Closed));
            }
            for pending in self.pending_captures.drain(..) {
                pending.reject(&mut self.state, RuntimeError::Closed);
            }
            drop(detached);
        }
    }
}

#[cfg(feature = "v8-host")]
fn retire_queued_captures(queue: &mut VecDeque<capture_readiness::Pending>, state: &mut PageState) {
    let mut index = 1;
    while index < queue.len() {
        if let Some(error) = queue[index].queued_error(state) {
            queue.remove(index).unwrap().reject(state, error);
        } else {
            index += 1;
        }
    }
}

fn actor_poll_delay(
    timer_delay: Option<Duration>,
    runnable: bool,
    document_pending: bool,
) -> Duration {
    if document_pending || runnable {
        page_poll_delay(timer_delay, runnable)
    } else {
        timer_delay.unwrap_or(Duration::from_millis(5))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        self.shutdown();
        // Shared actors reach Drop only after their nonblocking retirement
        // check joins cleanup. Fallback/direct ownership still cannot detach
        // native workers or acknowledge Close before their cancellation ends.
        while !self.poll_terminal_cleanup() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use super::*;
    include!("terminal_close_yield_tests.rs");
    include!("page_terminal_control_tests.rs");
    fn root(page: u64) -> (Root, mpsc::Sender<PageCommand>) {
        let (sender, receiver) = mpsc::channel(4);
        let state = PageState::new(
            PageId(page),
            None,
            Default::default(),
            Default::default(),
            None,
            #[cfg(unix)]
            None,
        );
        (Root::new(state, receiver, sender.downgrade()), sender)
    }
    fn timer_root(page: u64) -> (Root, mpsc::Sender<PageCommand>) {
        use crate::host::JavaScriptHost;
        let (mut root, sender) = root(page);
        root.state
            .load_document(
                "https://scheduler.test/".into(),
                "<body>timer</body>",
                None,
                None,
                HistoryUpdate::Push,
                &BTreeMap::new(),
                Vec::new(),
            )
            .unwrap();
        let executable = root.state.executable.as_mut().unwrap();
        let mut host = executable.take_agent_host().unwrap();
        host.execute("globalThis.fallbackTicks=0;setTimeout(()=>fallbackTicks++,0)")
            .unwrap();
        assert!(matches!(
            executable.return_agent_host_after_turn(host),
            Ok(None)
        ));
        (root, sender)
    }
    fn timer_ticks(root: &mut Root) -> String {
        use crate::host::JavaScriptHost;
        let executable = root.state.executable.as_mut().unwrap();
        let mut host = executable.take_agent_host().unwrap();
        let ticks = host.evaluate_string("String(fallbackTicks)").unwrap();
        assert!(matches!(
            executable.return_agent_host_after_turn(host),
            Ok(None)
        ));
        ticks
    }
    #[test]
    fn fallback_command_traffic_cannot_starve_due_timer() {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let (mut root, sender) = timer_root(430);
        let (reply, mut first) = oneshot::channel();
        sender
            .try_send(PageCommand::Capabilities { reply })
            .unwrap();
        let (reply, mut second) = oneshot::channel();
        sender
            .try_send(PageCommand::Capabilities { reply })
            .unwrap();
        assert_eq!(timer_ticks(&mut root), "0");
        let mut next_poll = std::time::Instant::now();
        let command = root.next_command(&None).unwrap();
        assert!(matches!(&command, PageCommand::Capabilities { .. }));
        assert!(
            root.dispatch_fallback(command, &context, &mut next_poll)
                .is_none()
        );
        assert!(first.try_recv().is_ok());
        assert!(matches!(
            second.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(
            timer_ticks(&mut root),
            "1",
            "due work progresses while commands remain queued"
        );
    }
    #[test]
    fn fallback_close_does_not_run_due_author_timer() {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let (mut root, sender) = timer_root(431);
        let (reply, mut closed) = oneshot::channel();
        sender.try_send(PageCommand::Close { reply }).unwrap();
        let command = root.next_command(&None).unwrap();
        let acknowledgement = root
            .dispatch_fallback(command, &context, &mut std::time::Instant::now())
            .unwrap();
        assert_eq!(timer_ticks(&mut root), "0");
        assert!(matches!(
            closed.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(root);
        acknowledgement.send(()).unwrap();
        assert!(closed.try_recv().is_ok());
    }

    #[test]
    fn moving_actor_roots_keeps_document_state_at_one_heap_address() {
        let (first, first_sender) = root(401);
        let state = std::ptr::from_ref(first.state.as_ref());
        let mut roots = Vec::new();
        roots.push(first);
        // Force queue growth and a root move without moving its retained state.
        for page in 402..418 {
            let (next, sender) = root(page);
            roots.push(next);
            drop(sender);
        }
        assert_eq!(std::ptr::from_ref(roots[0].state.as_ref()), state);
        let moved = roots.remove(0);
        assert_eq!(std::ptr::from_ref(moved.state.as_ref()), state);
        assert_eq!(moved.state.page_id, PageId(401));
        // The scheduler envelope must not contain an inline document owner.
        assert!(std::mem::size_of::<Root>() < std::mem::size_of::<PageState>());
        drop(moved);
        assert!(
            first_sender.is_closed(),
            "root drop still retires its command receiver"
        );
    }
    #[test]
    fn retained_actor_deadlines_do_not_spin_without_document_work() {
        let parked = Duration::from_secs(30);
        assert_eq!(actor_poll_delay(Some(parked), false, false), parked);
        assert_eq!(
            actor_poll_delay(Some(parked), false, true),
            Duration::from_millis(5)
        );
        assert_eq!(actor_poll_delay(Some(parked), true, false), Duration::ZERO);
        assert_eq!(
            actor_poll_delay(Some(Duration::ZERO), false, false),
            Duration::ZERO
        );
        assert_eq!(
            actor_poll_delay(Some(Duration::from_millis(2)), false, true),
            Duration::from_millis(2)
        );
    }

    #[test]
    fn cancelled_queued_capture_does_not_wait_for_or_consume_front() {
        let (mut root, _sender) = root(8);
        let (first, mut first_result) = oneshot::channel();
        let (cancelled, cancelled_result) = oneshot::channel();
        let (last, mut last_result) = oneshot::channel();
        for reply in [first, cancelled, last] {
            root.pending_captures
                .push_back(capture_readiness::Pending::new(
                    &root.state,
                    capture_readiness::Reply::Paint(reply),
                ));
        }
        drop(cancelled_result);
        retire_queued_captures(&mut root.pending_captures, &mut root.state);
        assert_eq!(root.pending_captures.len(), 2);
        assert!(matches!(
            first_result.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            last_result.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        root.shutdown();
        assert!(matches!(
            first_result.try_recv(),
            Ok(Err(RuntimeError::Closed))
        ));
        assert!(matches!(
            last_result.try_recv(),
            Ok(Err(RuntimeError::Closed))
        ));
    }

    #[test]
    fn capture_queue_remains_runnable_without_document_tasks() {
        let (mut root, _sender) = root(7);
        assert!(!root.state.has_pending_page_work());
        let (reply, receiver) = oneshot::channel();
        root.pending_captures
            .push_back(capture_readiness::Pending::new(
                &root.state,
                capture_readiness::Reply::Paint(reply),
            ));
        assert!(root.has_pending_work());
        assert!(root.has_runnable_work());
        assert_eq!(root.next_work_delay(), Some(Duration::ZERO));
        drop(receiver);
        assert!(root.has_pending_work());
        assert_eq!(root.next_work_delay(), Some(Duration::ZERO));
        root.shutdown();
        assert!(root.pending_captures.is_empty());
    }

    #[test]
    fn retiring_one_root_rejects_its_capture_without_consuming_sibling_reply() {
        let (mut first, _) = root(1);
        let (mut second, _) = root(2);
        let (a, mut a_result) = oneshot::channel();
        let (b, mut b_result) = oneshot::channel();
        first
            .pending_captures
            .push_back(capture_readiness::Pending::new(
                &first.state,
                capture_readiness::Reply::Paint(a),
            ));
        second
            .pending_captures
            .push_back(capture_readiness::Pending::new(
                &second.state,
                capture_readiness::Reply::Paint(b),
            ));
        drop(first);
        assert!(matches!(a_result.try_recv(), Ok(Err(RuntimeError::Closed))));
        assert!(matches!(
            b_result.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(second);
        assert!(matches!(b_result.try_recv(), Ok(Err(RuntimeError::Closed))));
    }
    #[cfg(unix)]
    #[test]
    fn completed_departure_fences_author_commands_but_retains_cleanup_replies() {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let handle = context.new_page();
        let id = handle.page_id();
        let (mut root, _) = root(id.0);
        root.state.context_client = Some(crate::context_service::SharedContextClient::direct(
            context.clone(),
            id,
        ));
        context
            .install_document_security(id, "https://close.test/".into(), &[])
            .unwrap();
        root.state
            .load_document(
                "https://close.test/".into(),
                "<body>original</body>",
                None,
                None,
                HistoryUpdate::Push,
                &BTreeMap::new(),
                Vec::new(),
            )
            .unwrap();
        let creator = root
            .state
            .executable
            .as_ref()
            .unwrap()
            .script_close_history()
            .unwrap()
            .creator;
        let (capture, mut captured) = oneshot::channel();
        root.pending_captures
            .push_back(capture_readiness::Pending::new(
                &root.state,
                capture_readiness::Reply::Paint(capture),
            ));
        assert!(matches!(
            root.state
                .poll_script_close_preparation(1, creator.clone())
                .unwrap(),
            crate::context_service::ScriptClosePreparation::Allowed(_)
        ));
        let (reply, mut result) = oneshot::channel();
        assert!(
            root.dispatch(
                PageCommand::Evaluate {
                    source: "throw new Error('must never execute')".into(),
                    reply
                },
                &context
            )
            .is_none()
        );
        assert!(matches!(result.try_recv(), Ok(Err(RuntimeError::Closed))));
        let (reply, mut result) = oneshot::channel();
        root.dispatch(
            PageCommand::LoadHtml {
                policy_token: None,
                url: "https://close.test/new".into(),
                html: "<body>must not install</body>".into(),
                reply,
            },
            &context,
        );
        assert!(matches!(result.try_recv(), Ok(Err(RuntimeError::Closed))));
        let (reply, mut result) = oneshot::channel();
        root.dispatch(
            PageCommand::PrepareScriptClose {
                token: 1,
                expected: creator,
                reply,
            },
            &context,
        );
        assert!(matches!(
            result.try_recv(),
            Ok(Ok(crate::context_service::ScriptClosePreparation::Allowed(
                _
            )))
        ));
        root.dispatch(PageCommand::PollContextEvents, &context);
        assert!(!root.has_pending_work());
        assert!(!root.has_runnable_work());
        assert_eq!(root.next_work_delay(), None);
        assert_eq!(root.pending_captures.len(), 1);
        assert!(matches!(
            captured.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let (reply, mut closed) = oneshot::channel();
        let acknowledgement = root
            .dispatch(PageCommand::Close { reply }, &context)
            .unwrap();
        drop(root);
        assert!(matches!(captured.try_recv(), Ok(Err(RuntimeError::Closed))));
        assert!(matches!(
            closed.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        acknowledgement.send(()).unwrap();
        assert!(matches!(closed.try_recv(), Ok(())));
    }
    #[test]
    fn close_ack_remains_owned_by_actor_until_root_destruction() {
        let browser = Browser::in_process_with_config(Default::default());
        let context = browser.new_context();
        let (mut root, sender) = root(1);
        let (reply, mut result) = oneshot::channel();
        let reply = root
            .dispatch(PageCommand::Close { reply }, &context)
            .expect("close acknowledgement custody");
        assert!(matches!(
            result.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(root);
        assert!(sender.is_closed());
        reply.send(()).unwrap();
        assert!(matches!(result.try_recv(), Ok(())));
    }
}

#[cfg(all(unix, feature = "v8-host"))]
mod related_actor {
    use super::*;
    use crate::worker_route::WorkerRoute;
    use std::sync::{Mutex, OnceLock};
    struct Published {
        actor: WorkerRoute,
        sender: mpsc::WeakSender<PageCommand>,
        control: std::sync::mpsc::SyncSender<Attach>,
        claimed: bool,
    }
    struct Attach {
        route: WorkerRoute,
        context: WeakBrowserContext,
    }
    struct DirectReservation {
        actor: WorkerRoute,
        receiver: mpsc::Receiver<PageCommand>,
        context: WeakBrowserContext,
    }
    fn direct_reservations() -> &'static Mutex<BTreeMap<WorkerRoute, DirectReservation>> {
        static DIRECT: OnceLock<Mutex<BTreeMap<WorkerRoute, DirectReservation>>> = OnceLock::new();
        DIRECT.get_or_init(|| Mutex::new(BTreeMap::new()))
    }
    pub(crate) fn reserve_direct(
        actor: WorkerRoute,
        route: WorkerRoute,
        context: WeakBrowserContext,
        capacity: usize,
    ) -> Result<mpsc::Sender<PageCommand>, RuntimeError> {
        let mut reservations = direct_reservations().lock().map_err(|_| {
            RuntimeError::ProtocolViolation("direct actor reservation lock poisoned".into())
        })?;
        if reservations.contains_key(&route)
            || reservations
                .values()
                .filter(|entry| entry.actor == actor)
                .count()
                >= 256
        {
            return Err(RuntimeError::ProtocolViolation(
                "direct root reservation capacity".into(),
            ));
        }
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        reservations.insert(
            route,
            DirectReservation {
                actor,
                receiver,
                context,
            },
        );
        Ok(sender)
    }
    pub(crate) fn cancel_direct(route: &WorkerRoute) {
        direct_reservations()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(route);
    }

    fn registry() -> &'static Mutex<BTreeMap<WorkerRoute, Published>> {
        static REGISTRY: OnceLock<Mutex<BTreeMap<WorkerRoute, Published>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
    }
    fn published() -> &'static std::sync::Condvar {
        static PUBLISHED: std::sync::Condvar = std::sync::Condvar::new();
        &PUBLISHED
    }
    pub(crate) fn claim(
        actor: &WorkerRoute,
        route: &WorkerRoute,
        context: WeakBrowserContext,
    ) -> Result<mpsc::Sender<PageCommand>, RuntimeError> {
        let mut entries = registry().lock().map_err(|_| {
            RuntimeError::ProtocolViolation("related root registry poisoned".into())
        })?;
        // A command reply can wake the worker reader just before the actor
        // adopts callback-created roots. Wait only for publication, never while
        // holding a host or while asking that actor to run another command.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !entries.contains_key(route) {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (next, timeout) = published().wait_timeout(entries, remaining).map_err(|_| {
                RuntimeError::ProtocolViolation("related root publication poisoned".into())
            })?;
            entries = next;
            if timeout.timed_out() {
                break;
            }
        }
        let entry = entries
            .get_mut(route)
            .filter(|entry| &entry.actor == actor && !entry.claimed)
            .ok_or_else(|| {
                RuntimeError::ProtocolViolation(
                    "related root was not admitted by this agent".into(),
                )
            })?;
        let sender = entry.sender.upgrade().ok_or(RuntimeError::Closed)?;
        entry
            .control
            .try_send(Attach {
                route: route.clone(),
                context,
            })
            .map_err(|_| RuntimeError::Closed)?;
        entry.claimed = true;
        Ok(sender)
    }
    struct Entry {
        retiring: bool,
        close_reply: Option<oneshot::Sender<()>>,
        retire_realms: Vec<oryn_js::realm::RealmId>,
        root: Root,
        context: WeakBrowserContext,
        context_guard: Option<BrowserContext>,
        route: Option<WorkerRoute>,
        reserved_sender: Option<mpsc::Sender<PageCommand>>,
        next_poll: std::time::Instant,
    }
    struct PublicationGuard(WorkerRoute);
    impl Drop for PublicationGuard {
        fn drop(&mut self) {
            registry()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|_, entry| entry.actor != self.0);
            direct_reservations()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|_, entry| entry.actor != self.0);
        }
    }
    #[cfg(test)]
    mod registration_tests {
        use super::*;
        #[test]
        fn auxiliary_claim_requires_exact_actor_and_route_and_is_one_use() {
            let browser = Browser::in_process_with_config(Default::default());
            let context = browser.new_context();
            let actor = WorkerRoute::issue(701, 1, 0).unwrap();
            let child = WorkerRoute::issue(701, 2, 0).unwrap();
            let forged_actor = WorkerRoute::issue(701, 1, 0).unwrap();
            let forged_child = WorkerRoute::issue(701, 2, 0).unwrap();
            let (sender, mut receiver) = mpsc::channel(4);
            let (control, updates) = std::sync::mpsc::sync_channel(2);
            registry().lock().unwrap().insert(
                child.clone(),
                Published {
                    actor: actor.clone(),
                    sender: sender.downgrade(),
                    control,
                    claimed: false,
                },
            );
            let _guard = PublicationGuard(actor.clone());
            assert!(claim(&forged_actor, &child, context.downgrade()).is_err());
            assert!(claim(&actor, &forged_child, context.downgrade()).is_err());
            assert!(matches!(
                updates.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            let admitted = claim(&actor, &child, context.downgrade()).unwrap();
            assert_eq!(updates.try_recv().unwrap().route, child);
            assert!(claim(&actor, &child, context.downgrade()).is_err());
            assert!(admitted.try_send(PageCommand::PollContextEvents).is_ok());
            assert!(matches!(
                receiver.try_recv(),
                Ok(PageCommand::PollContextEvents)
            ));
        }
        #[test]
        fn failed_registration_delivery_does_not_consume_admission() {
            let browser = Browser::in_process_with_config(Default::default());
            let context = browser.new_context();
            let actor = WorkerRoute::issue(702, 1, 0).unwrap();
            let child = WorkerRoute::issue(702, 2, 0).unwrap();
            let (sender, _receiver) = mpsc::channel(1);
            let (control, updates) = std::sync::mpsc::sync_channel(1);
            assert!(
                control
                    .try_send(Attach {
                        route: child.clone(),
                        context: context.downgrade()
                    })
                    .is_ok()
            );
            registry().lock().unwrap().insert(
                child.clone(),
                Published {
                    actor: actor.clone(),
                    sender: sender.downgrade(),
                    control,
                    claimed: false,
                },
            );
            let guard = PublicationGuard(actor.clone());
            assert!(claim(&actor, &child, context.downgrade()).is_err());
            assert!(!registry().lock().unwrap().get(&child).unwrap().claimed);
            updates.try_recv().unwrap();
            assert!(claim(&actor, &child, context.downgrade()).is_ok());
            drop(guard);
            assert!(!registry().lock().unwrap().contains_key(&child));
        }
    }
    include!("prerender_blank_actor.rs");
    // Keep by-value auxiliary document construction out of the scheduler frame.
    // This helper returns before any subsequent root dispatch can enter V8.
    #[inline(never)]
    fn construct_related_roots(construct: impl FnOnce()) {
        construct();
    }

    pub(crate) fn run(root: Root, context: WeakBrowserContext, actor: WorkerRoute) {
        let _publications = PublicationGuard(actor.clone());
        let (control_sender, control) = std::sync::mpsc::sync_channel::<Attach>(256);
        let mut entries = vec![Entry {
            retiring: false,
            close_reply: None,
            retire_realms: Vec::new(),
            root,
            context,
            context_guard: None,
            route: Some(actor.clone()),
            reserved_sender: None,
            next_poll: std::time::Instant::now(),
        }];
        let mut host: Option<crate::host::RawV8Host> = None;
        let mut host_identity: Option<std::rc::Rc<()>> = None;
        let mut cursor = 0usize;
        let mut blank_transfers: Vec<BlankTransfer> = Vec::new();
        loop {
            while let Ok(attach) = control.try_recv() {
                if let Some(entry) = entries
                    .iter_mut()
                    .find(|entry| entry.route.as_ref() == Some(&attach.route))
                {
                    entry.context = attach.context;
                    // Remote claims can attach before the destination's local
                    // PageRecord is registered. Never inherit source-page control
                    // or silently replace an already bound destination controller.
                    if let Some(context) = entry.context.upgrade() {
                        match context.page_prompt_controller(entry.root.state.page_id) {
                            Ok(controller) => {
                                entry.root.state.page_task_terminal_control.bind(controller);
                            }
                            Err(_) => entry.root.state.page_task_terminal_control.lookup_failed(),
                        }
                    }
                    entry.context_guard.take();
                    entry.reserved_sender.take();
                }
            }
            if entries.is_empty() {
                let mut index = 0;
                while index < blank_transfers.len() {
                    match blank_transfers[index].poll(
                        &mut host,
                        &mut entries,
                        &actor,
                        &control_sender,
                    ) {
                        Ok(true) => {
                            blank_transfers.remove(index);
                        }
                        Err(
                            RuntimeError::Closed
                            | RuntimeError::NavigationCancelled
                            | RuntimeError::PageNotFound(_),
                        ) => {
                            blank_transfers[index].phase = 255;
                            index += 1;
                        }
                        Err(_) => {
                            blank_transfers[index].retry =
                                Some(Instant::now() + Duration::from_millis(25));
                            index += 1;
                        }
                        Ok(false) => index += 1,
                    }
                }
                if entries.is_empty() {
                    if blank_transfers.is_empty() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
            }
            cursor %= entries.len();
            let entry = &mut entries[cursor];
            let mut close = None;
            let mut retire = entry.retiring;
            if host_identity.is_none() {
                host_identity = entry
                    .root
                    .state
                    .executable
                    .as_ref()
                    .map(|page| page.agent_host_identity())
                    .or_else(|| {
                        entry
                            .root
                            .state
                            .pending_agent_host
                            .as_ref()
                            .map(|host| host.agent_host_identity())
                    });
            }
            if host.is_none()
                && let Ok(Some(owned)) = entry.root.state.extract_agent_host(host_identity.as_ref())
            {
                if host_identity.is_none() {
                    host_identity = Some(owned.agent_host_identity());
                }
                host = Some(owned);
            }
            if entry.retiring {
                if entry.root.state.pending_agent_host.is_none() {
                    entry.root.state.pending_agent_host = host.take();
                }
            } else if let Some(executable) = entry.root.state.executable.as_mut() {
                if !executable.has_agent_host() {
                    if let Some(owned) = host.take() {
                        if let Err((_error, owned)) = executable.install_agent_host(owned) {
                            host = Some(*owned);
                            retire = true;
                        }
                    } else {
                        retire = true;
                    }
                }
            } else if entry.root.state.pending_agent_host.is_none() {
                entry.root.state.pending_agent_host = host.take();
            }
            if !retire {
                let command = entry.root.try_actor_command();
                match command {
                    Ok(command) => {
                        if let Some(context) = entry.context.upgrade() {
                            close = entry.root.dispatch(command, &context);
                        } else {
                            retire = true;
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => retire = true,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
                // A busy command queue must not starve parser, timer, or
                // lifecycle work. Keep commands (especially Close) first, then
                // grant one bounded work turn when this root's deadline is due.
                // Preserve a future deadline across commands instead of moving
                // it forward on every observation.
                if !retire && close.is_none() && entry.root.has_pending_work() {
                    let now = std::time::Instant::now();
                    let timer_delay = entry.root.next_work_delay();
                    let delay = entry.root.poll_delay(timer_delay);
                    entry.next_poll = entry.next_poll.min(now.checked_add(delay).unwrap_or(now));
                    if now >= entry.next_poll {
                        if let Some(context) = entry.context.upgrade() {
                            close = entry
                                .root
                                .dispatch(PageCommand::PollContextEvents, &context);
                        } else {
                            retire = true;
                        }
                        let timer_delay = entry.root.next_work_delay();
                        let delay = entry.root.poll_delay(timer_delay);
                        let finished = std::time::Instant::now();
                        entry.next_poll = finished.checked_add(delay).unwrap_or(finished);
                    }
                }
            }
            let mut created = Vec::new();
            // Detach accepted jobs before source close/navigation cleanup.
            if let Ok(Some(mut owned)) = entry.root.state.extract_agent_host(host_identity.as_ref())
            {
                if host_identity.is_none() {
                    host_identity = Some(owned.agent_host_identity());
                }
                created = owned.take_created_auxiliary_roots();
                for (source, receipt) in owned.take_blank_prerender_activations() {
                    if let Some(transfer) = BlankTransfer::take(
                        &mut entry.root.state,
                        entry.context.clone(),
                        source,
                        receipt,
                    ) {
                        blank_transfers.push(transfer);
                    }
                }
                host = Some(owned);
            }
            if (retire || close.is_some()) && !entry.retiring {
                entry.root.state.clear_cached_documents();
                entry.retire_realms = Vec::new();
                entry.retire_realms.extend(
                    entry
                        .root
                        .state
                        .executable
                        .as_ref()
                        .map(|page| page.agent_root_realm()),
                );
                entry.retire_realms.sort();
                entry.retire_realms.dedup();
                entry.root.shutdown();
                entry.retiring = true;
            }
            if close.is_some() {
                entry.close_reply = close;
            }
            if host.is_none() {
                match entry.root.state.extract_agent_host(host_identity.as_ref()) {
                    Ok(Some(mut owned)) => {
                        if host_identity.is_none() {
                            host_identity = Some(owned.agent_host_identity());
                        }
                        created = owned.take_created_auxiliary_roots();
                        host = Some(owned);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let revision = entry.root.state.revision;
                        entry.root.state.trace.push(
                            Revision(revision),
                            None,
                            TraceEventKind::ScriptDiagnostic {
                                capability: "agent-host-custody".into(),
                                detail: error.to_string(),
                            },
                        );
                    }
                }
            }
            let source_context = entry.context.clone();
            let config = entry.root.state.config.clone();
            let options = source_context
                .upgrade()
                .map(|context| context.inner.options.clone())
                .unwrap_or_default();
            if let Some(owned) = host.as_mut() {
                let _ = entry.root.state.drain_retired_agent_documents(owned);
            }
            if entry.retiring {
                if !entry.retire_realms.is_empty() {
                    entry
                        .root
                        .report_close_pending(1, crate::ipc_progress::Phase::CloseRealmsPending);
                }
                if entry.root.state.has_retired_agent_documents() {
                    entry
                        .root
                        .report_close_pending(2, crate::ipc_progress::Phase::CloseDocumentsPending);
                }
                if !entry.root.terminal_cleanup.is_empty() {
                    entry.root.report_close_pending(
                        4,
                        crate::ipc_progress::Phase::CloseNativeCleanupPending,
                    );
                }
                if let Some(owned) = host.as_mut() {
                    entry
                        .retire_realms
                        .retain(|realm| owned.retire_agent_root(*realm).is_err());
                }
                if entry.retire_realms.is_empty()
                    && !entry.root.state.has_retired_agent_documents()
                    && entry.root.poll_terminal_cleanup()
                {
                    if let Some(route) = entry.route.as_ref() {
                        registry()
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .remove(route);
                    }
                    let mut removed = entries.remove(cursor);
                    let reply = removed.close_reply.take();
                    let ipc_identity = removed.root.close_progress();
                    drop(removed);
                    if let Some(identity) = ipc_identity {
                        crate::ipc_progress::record(
                            Some(identity),
                            crate::ipc_progress::Phase::CloseActorReleased,
                            None,
                            None,
                        );
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(());
                    }
                } else {
                    cursor += 1;
                }
            } else {
                cursor += 1;
            }
            construct_related_roots(|| {
                for created in created {
                    if created.actor_route != actor || entries.len() >= 256 {
                        if let Some(host) = host.as_mut() {
                            let _ = host.discard_created_auxiliary_root(created.realm);
                        }
                        continue;
                    }
                    let created_realm = created.realm;
                    let route = created.route.clone();
                    let page = created.page_id;
                    let opener = created.opener_page_id;
                    let broker = created.broker.clone();
                    let endpoint = created.context.clone();
                    let direct = endpoint.current_route().is_none();
                    let url = created.url.to_string();
                    let Some(owned) = host.as_mut() else {
                        break;
                    };
                    let generation = owned
                        .realm_descriptor(created.realm)
                        .map(|realm| realm.document_generation)
                        .unwrap_or(0);
                    let origin = owned
                        .realm_descriptor(created.realm)
                        .map(|realm| realm.origin.clone());
                    let policy = created.policy.clone();
                    let referrer = created.referrer_policy;
                    let isolation = created.isolation;
                    let executable =
                        match crate::executable::RustExecutableDocument::adopt_initial_root(
                            owned, created,
                        ) {
                            Ok(value) => value,
                            Err(_) => {
                                let _ = owned.discard_created_auxiliary_root(created_realm);
                                continue;
                            }
                        };
                    let direct_reservation = if direct {
                        direct_reservations()
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .remove(&route)
                    } else {
                        None
                    };
                    if direct
                        && !direct_reservation
                            .as_ref()
                            .is_some_and(|entry| entry.actor == actor)
                    {
                        let _ = owned.discard_created_auxiliary_root(created_realm);
                        continue;
                    }
                    let (sender, receiver, reserved_context) =
                        if let Some(reservation) = direct_reservation {
                            (None, reservation.receiver, reservation.context)
                        } else {
                            let (sender, receiver) = mpsc::channel(config.worker_queue_capacity);
                            (Some(sender), receiver, source_context.clone())
                        };
                    let mut state = PageState::new(
                        page,
                        opener,
                        config.clone(),
                        options.clone(),
                        broker,
                        Some(endpoint),
                    );
                    let history = match owned.navigation_history_snapshot() {
                        Ok(history) => history,
                        Err(_) => {
                            let _ = owned.discard_created_auxiliary_root(created_realm);
                            continue;
                        }
                    };
                    let url = history
                        .entries
                        .get(history.current)
                        .map(|entry| entry.url.clone())
                        .unwrap_or(url);
                    state.adopt_agent_document(crate::runtime::AgentDocumentInputs {
                        executable,
                        url,
                        generation,
                        origin: origin.unwrap(),
                        history,
                        policy,
                        referrer,
                        isolation,
                    });
                    let weak = if let Some(sender) = sender.as_ref() {
                        sender.downgrade()
                    } else {
                        let Some(context) = reserved_context.upgrade() else {
                            let _ = owned.discard_created_auxiliary_root(created_realm);
                            continue;
                        };
                        let Some(sender) = context
                            .inner
                            .group
                            .handle(page)
                            .and_then(|handle| handle.sender)
                        else {
                            let _ = owned.discard_created_auxiliary_root(created_realm);
                            continue;
                        };
                        sender.downgrade()
                    };
                    let mut registry = registry().lock().unwrap_or_else(|error| error.into_inner());
                    if registry.contains_key(&route) {
                        let _ = owned.discard_created_auxiliary_root(
                            state.executable.as_ref().unwrap().agent_root_realm(),
                        );
                        continue;
                    }
                    registry.insert(
                        route.clone(),
                        Published {
                            actor: actor.clone(),
                            sender: weak.clone(),
                            control: control_sender.clone(),
                            claimed: direct,
                        },
                    );
                    drop(registry);
                    published().notify_all();
                    entries.push(Entry {
                        retiring: false,
                        close_reply: None,
                        retire_realms: Vec::new(),
                        root: Root::new(state, receiver, weak),
                        context: reserved_context.clone(),
                        context_guard: if direct {
                            None
                        } else {
                            reserved_context.upgrade()
                        },
                        route: Some(route),
                        reserved_sender: sender,
                        next_poll: std::time::Instant::now(),
                    });
                }
            });
            let mut index = 0;
            while index < blank_transfers.len() {
                match blank_transfers[index].poll(&mut host, &mut entries, &actor, &control_sender)
                {
                    Ok(true) => {
                        blank_transfers.remove(index);
                    }
                    Err(
                        RuntimeError::Closed
                        | RuntimeError::NavigationCancelled
                        | RuntimeError::PageNotFound(_),
                    ) => {
                        blank_transfers[index].phase = 255;
                        index += 1;
                    }
                    Err(_) => {
                        blank_transfers[index].retry =
                            Some(Instant::now() + Duration::from_millis(25));
                        index += 1;
                    }
                    Ok(false) => index += 1,
                }
            }
            // Receivers are polled fairly; no root's blocking recv or due timer
            // monopolizes the agent. The bounded wait also services registrations.
            if cursor == 0 || cursor >= entries.len() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
#[cfg(all(unix, feature = "v8-host"))]
pub(super) use related_actor::{
    cancel_direct as cancel_direct_root, claim as claim_related_root,
    reserve_direct as reserve_direct_root, run as run_related_actor,
};

/// Complete rejected API replies without entering author code. Physical Close
/// and exact preparation replay remain available while the parent holds custody.
#[cfg(all(unix, feature = "v8-host"))]
fn reject_after_departure(command: PageCommand) -> Option<PageCommand> {
    match command {
        command @ (PageCommand::Close { .. }
        | PageCommand::PrepareScriptClose { .. }
        | PageCommand::ScriptCloseHistory { .. }
        | PageCommand::Trace { .. }
        | PageCommand::Capabilities { .. }
        | PageCommand::ChallengeAssessment { .. }) => return Some(command),
        PageCommand::PollContextEvents => {}
        PageCommand::ControlledNavigate { emitter, reply, .. } => {
            let _ = emitter.publish(crate::navigation_control::NavigationEvent::Stopped(
                crate::navigation_control::NavigationStop::Retired,
            ));
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::LoadHtml { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Goto { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::GotoResource { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Back { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Forward { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Refresh { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::WaitFor { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::WaitForUntil { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Evaluate { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ScopedWorker { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ResolveFocusedForm { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ResolveSelector { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ResolveTarget { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ResolveSemantic { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Extract { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Observe { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ObserveWith { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ObserveSemantic { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ObserveSemanticV6 { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ComputedStyle { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::AccessibilitySnapshot { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::AccessibilitySnapshotCompleted { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::AccessibilityText { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::BoxModel { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::PaintSnapshot { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::PaintSnapshotCompleted { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::PrintSnapshot { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::SetViewport { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::SetMedia { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Scroll { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ScrollIntoView { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ObserveDelta { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ObserveSemanticDelta { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::Execute { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ExecuteForced { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::ExecuteSensitive { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::SetFiles { reply, .. } => {
            let _ = reply.send(Err(RuntimeError::Closed));
        }
        PageCommand::WebdriverScript { reply, .. } => {
            let _ = reply.send(Err(crate::host::webdriver_script::ScriptError {
                code: "no such window".into(),
                message: "page departure completed".into(),
            }));
        }
        PageCommand::WebdriverFrames { reply, .. } => {
            let _ = reply.send(Err(crate::host::webdriver_script::ScriptError {
                code: "no such window".into(),
                message: "page departure completed".into(),
            }));
        }
        PageCommand::WebdriverActions { reply, .. } => {
            let _ = reply.send((
                Err(crate::host::webdriver_script::ScriptError {
                    code: "no such window".into(),
                    message: "page departure completed".into(),
                }),
                Default::default(),
                Vec::new(),
            ));
        }
    }
    None
}

#[cfg(all(test, not(feature = "v8-host")))]
mod no_v8_input_tests {
    use super::*;
    use crate::host::webdriver_script::ScriptContext;
    use crate::webdriver_actions::{Action, ActionCommand, ActionsRequest, Source, SourceKind};

    #[test]
    fn unsupported_actions_reply_without_receipts_and_keep_the_actor_root_live() {
        let owner = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let state = PageState::new(
            PageId(401),
            None,
            BrowserConfig {
                document_generation_base: 7,
                ..Default::default()
            },
            Default::default(),
            None,
        );
        let (sender, receiver) = mpsc::channel(4);
        let mut root = Root::new(state, receiver);
        let request = ActionsRequest {
            sources: vec![Source {
                id: "keyboard".into(),
                kind: SourceKind::Key,
                actions: vec![Action::Key {
                    down: true,
                    value: vec![u16::from(b'x')],
                }],
            }],
        };
        request.validate().unwrap();
        for command in [
            ActionCommand::Perform(request),
            ActionCommand::Release,
            ActionCommand::Cancel,
        ] {
            let (reply, mut response) = oneshot::channel();
            assert!(
                root.dispatch(
                    PageCommand::WebdriverActions {
                        command,
                        context: ScriptContext::default(),
                        cancellation: None,
                        reply,
                    },
                    &owner,
                )
                .is_none()
            );
            let (result, feedback, receipts) = response.try_recv().unwrap();
            let error = result.unwrap_err();
            assert_eq!(error.code, "unsupported operation");
            assert_eq!(error.message, "native input actions require V8");
            assert!(receipts.is_empty());
            assert!(feedback.updates.is_empty());
            assert_eq!(feedback.page_generation, 7);
            assert_eq!(root.state.document_generation, 7);
            assert!(
                !sender.is_closed(),
                "unsupported input must not retire the actor root"
            );
        }
        drop(root);
        assert!(
            sender.is_closed(),
            "ordinary root teardown still closes its receiver"
        );
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod retained_command_mailbox_tests {
    use super::*;
    fn idle_root() -> (Root, mpsc::Sender<PageCommand>) {
        let (sender, receiver) = mpsc::channel(4);
        let mut state = PageState::new(
            PageId(590),
            None,
            Default::default(),
            Default::default(),
            None,
            #[cfg(unix)]
            None,
        );
        state.network_quiet_since = None;
        (Root::new(state, receiver, sender.downgrade()), sender)
    }
    fn completion(
        generation: u64,
    ) -> (
        PageCommand,
        oneshot::Receiver<Result<PaintSnapshot, RuntimeError>>,
    ) {
        let (reply, result) = oneshot::channel();
        (
            PageCommand::PaintSnapshotCompleted {
                generation,
                stamp: None,
                result: Err(RuntimeError::PaintUnavailable("worker completed".into())),
                reply,
            },
            result,
        )
    }
    #[test]
    fn delayed_private_completion_wakes_idle_actor_without_a_user_command() {
        let (mut root, ordinary) = idle_root();
        let parked = Arc::new(AtomicBool::new(false));
        let notify = parked.clone();
        let scheduler = Some(
            tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .on_thread_park(move || {
                    notify.store(true, Ordering::Release);
                })
                .build()
                .unwrap(),
        );
        let sender = root._paint_completion_owner.clone();
        let (message, _result) = completion(root.state.document_generation);
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !parked.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            if !parked.load(Ordering::Acquire) {
                // Fail boundedly if the actor waited on only its ordinary queue.
                let (reply, _) = oneshot::channel();
                ordinary
                    .blocking_send(PageCommand::Capabilities { reply })
                    .unwrap();
            }
            sender.blocking_send(message).unwrap();
        });
        let message = root.next_command(&scheduler).unwrap();
        assert!(matches!(
            message,
            PageCommand::PaintSnapshotCompleted { .. }
        ));
        worker.join().unwrap();
    }
    #[test]
    fn private_completion_bypasses_wait_barrier_but_user_fifo_does_not() {
        let (mut root, ordinary) = idle_root();
        let (reply, waiting) = oneshot::channel();
        root.pending_wait = wait_fairness::Pending::from_command(
            &mut Some(PageCommand::WaitForUntil {
                predicate: WaitPredicate::Lifecycle(LifecycleState::NetworkIdle),
                timeout: None,
                reply,
            }),
            &root.state,
        );
        let (reply, _following) = oneshot::channel();
        ordinary
            .try_send(PageCommand::Capabilities { reply })
            .unwrap();
        let (message, mut completed) = completion(root.state.document_generation);
        root._paint_completion_owner.try_send(message).unwrap();
        let message = root.try_actor_command().unwrap();
        assert!(matches!(
            message,
            PageCommand::PaintSnapshotCompleted { .. }
        ));
        let context = Browser::in_process_with_config(Default::default()).new_context();
        root.dispatch(message, &context);
        assert!(matches!(
            completed.try_recv(),
            Ok(Err(RuntimeError::PaintUnavailable(_)))
        ));
        assert!(matches!(
            root.try_actor_command(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        drop(waiting);
        root.dispatch(PageCommand::PollContextEvents, &context);
        assert!(matches!(
            root.try_actor_command(),
            Ok(PageCommand::Capabilities { .. })
        ));
    }
    #[test]
    fn shutdown_completes_buffered_private_reply_before_root_drop() {
        let (mut root, _ordinary) = idle_root();
        let (message, mut completed) = completion(root.state.document_generation);
        root._paint_completion_owner.try_send(message).unwrap();
        root.shutdown();
        assert!(matches!(
            completed.try_recv(),
            Ok(Err(RuntimeError::Closed))
        ));
        assert!(root.paint_completions.is_closed());
    }
}
