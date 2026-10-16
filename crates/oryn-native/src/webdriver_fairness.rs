//! One serialized WebDriver command, resumed at shared-actor turn boundaries.
//! V8 handles remain in the executable; only native operation tokens are retained.
use super::*;
use crate::host::webdriver_script::{
    AutomationRealmSnapshot, ScriptContext, ScriptError, ScriptId, ScriptPoll, ScriptRequest,
    ScriptValue,
};
use crate::webdriver_action_execution::{ActionExecution, CancelExecution, ReleaseExecution};
use crate::webdriver_actions::{Action, ActionCommand, InputState, OsKeyReceipt};

type ActionReply = oneshot::Sender<(
    Result<(), ScriptError>,
    oryn_js::drag_image::FeedbackBatch,
    Vec<OsKeyReceipt>,
)>;
enum Reply {
    Script(oneshot::Sender<Result<ScriptValue, ScriptError>>),
    Actions(ActionReply),
    #[cfg(unix)]
    Worker(
        oneshot::Sender<Result<crate::worker::WorkerResponse, RuntimeError>>,
        bool,
    ),
}
struct Script {
    request: Option<ScriptRequest>,
    context: ScriptContext,
    token: Option<ScriptId>,
    generation: Option<u64>,
}
enum Work {
    Script(Box<Script>),
    Actions(Box<Actions>),
}
enum Completion {
    Finished(
        Result<ScriptValue, ScriptError>,
        oryn_js::drag_image::FeedbackBatch,
        Vec<OsKeyReceipt>,
    ),
    Failed(RuntimeError, oryn_js::drag_image::FeedbackBatch),
}
/// Native cleanup custody can leave a retiring V8 root, but its close
/// acknowledgement must remain retained until poll reports finished + joined.
pub(super) struct PaintCleanup {
    worker: Option<std::thread::JoinHandle<Result<PaintSnapshot, RuntimeError>>>,
    completion: Option<(Reply, Completion)>,
    diagnostic: Option<String>,
}
impl PaintCleanup {
    pub(super) fn poll(&mut self) -> bool {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return false;
        }
        if let Some(worker) = self.worker.take() {
            self.diagnostic = match worker.join() {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(
                    format!("native paint cleanup worker failed: {error}")
                        .chars()
                        .take(2048)
                        .collect(),
                ),
                Err(_) => Some("native paint cleanup worker panicked".into()),
            };
        }
        if let Some((reply, completion)) = self.completion.take() {
            reply.complete(completion);
        }
        true
    }
    pub(super) fn poll_with_trace(&mut self, state: &mut PageState) -> bool {
        let settled = self.poll();
        if let Some(detail) = self.diagnostic.take() {
            state.trace.push(
                Revision(state.revision),
                None,
                TraceEventKind::ScriptDiagnostic {
                    capability: "paint.cleanup".into(),
                    detail,
                },
            );
        }
        settled
    }
}
impl Drop for PaintCleanup {
    fn drop(&mut self) {
        // Backstop for abnormal ownership loss only. Shared actors retain this
        // object and poll it before removing the root or acknowledging close.
        if let Some(worker) = self.worker.take() {
            self.diagnostic = match worker.join() {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(
                    format!("native paint cleanup worker failed: {error}")
                        .chars()
                        .take(2048)
                        .collect(),
                ),
                Err(_) => Some("native paint cleanup worker panicked".into()),
            };
        }
        if let Some(detail) = self.diagnostic.take() {
            eprintln!("{detail}");
        }
        if let Some((reply, completion)) = self.completion.take() {
            reply.complete(completion);
        }
    }
}
impl Reply {
    fn complete(self, completion: Completion) {
        let (result, feedback, receipts) = match completion {
            Completion::Finished(result, feedback, receipts) => (result, feedback, receipts),
            Completion::Failed(error, feedback) => {
                #[cfg(unix)]
                if let Self::Worker(reply, _) = self {
                    let _ = reply.send(Err(error));
                    return;
                }
                let error = if matches!(error, RuntimeError::Closed | RuntimeError::PageTaskYield) {
                    closed_error()
                } else {
                    ScriptError::retired()
                };
                (Err(error), feedback, Vec::new())
            }
        };
        match self {
            Self::Script(reply) => {
                let _ = reply.send(result);
            }
            Self::Actions(reply) => {
                let _ = reply.send((result.map(|_| ()), feedback, receipts));
            }
            #[cfg(unix)]
            Self::Worker(reply, actions) => {
                let response = if actions {
                    crate::worker::WorkerResponse::WebdriverActions {
                        result: result.map(|_| ()),
                        feedback,
                        receipts,
                    }
                } else {
                    crate::worker::WorkerResponse::WebdriverScript { result }
                };
                let _ = reply.send(Ok(response));
            }
        }
    }
}

pub(super) struct Pending {
    cleanup: Option<PaintCleanup>,
    completion: Option<Completion>,
    reply: Reply,
    work: Work,
    cancellation: Option<Arc<oryn_js::prompts::PromptController>>,
    scope: Option<AutomationRealmSnapshot>,
    root_generation: u64,
}
impl Pending {
    pub(super) fn from_command(
        command: &mut Option<PageCommand>,
        state: &PageState,
    ) -> Option<Self> {
        let (reply, work, cancellation, scope) = match command.take()? {
            PageCommand::WebdriverScript {
                request,
                context,
                cancellation,
                reply,
            } => (
                Reply::Script(reply),
                Work::script(request, context),
                cancellation,
                None,
            ),
            PageCommand::WebdriverActions {
                command,
                context,
                cancellation,
                reply,
            } => (
                Reply::Actions(reply),
                Work::Actions(Box::new(Actions::new(command, context))),
                cancellation,
                None,
            ),
            #[cfg(unix)]
            PageCommand::ScopedWorker {
                scope,
                request,
                reply,
            } => match request {
                crate::worker::WorkerRequest::WebdriverScript {
                    request,
                    mut context,
                } => {
                    context.selected = Some(scope.clone());
                    (
                        Reply::Worker(reply, false),
                        Work::script(request, context),
                        None,
                        Some(scope),
                    )
                }
                crate::worker::WorkerRequest::WebdriverActions {
                    command,
                    mut context,
                } => {
                    context.selected = Some(scope.clone());
                    (
                        Reply::Worker(reply, true),
                        Work::Actions(Box::new(Actions::new(command, context))),
                        None,
                        Some(scope),
                    )
                }
                request => {
                    *command = Some(PageCommand::ScopedWorker {
                        scope,
                        request,
                        reply,
                    });
                    return None;
                }
            },
            incoming => {
                *command = Some(incoming);
                return None;
            }
        };
        Some(Self {
            cleanup: None,
            completion: None,
            reply,
            work,
            cancellation,
            scope,
            root_generation: state.document_generation,
        })
    }
    pub(super) fn with_cancellation(
        mut self,
        controller: Arc<oryn_js::prompts::PromptController>,
    ) -> Self {
        if self.cancellation.is_none() {
            self.cancellation = Some(controller);
        }
        self
    }
    pub(super) fn next_delay(&self) -> Duration {
        Duration::from_millis(1)
    }
    fn closed(&self) -> bool {
        match &self.reply {
            Reply::Script(reply) => reply.is_closed(),
            Reply::Actions(reply) => reply.is_closed(),
            #[cfg(unix)]
            Reply::Worker(reply, _) => reply.is_closed(),
        }
    }
    pub(super) fn poll(mut self, state: &mut PageState) -> Option<Self> {
        if self.completion.is_some() {
            return self.finish_completion(state);
        }
        let interrupted = self.closed()
            || self
                .cancellation
                .as_ref()
                .is_some_and(|token| token.inspect().is_err());
        if interrupted {
            self.cleanup = self.work.take_cleanup();
            self.work
                .abort(state, self.root_generation == state.document_generation);
            let feedback = if matches!(&self.work, Work::Actions(_))
                && state
                    .executable
                    .as_ref()
                    .is_some_and(|page| page.has_agent_host())
            {
                state.take_drag_feedback()
            } else {
                Default::default()
            };
            self.completion = Some(Completion::Finished(
                Err(closed_error()),
                feedback,
                Vec::new(),
            ));
            return self.finish_completion(state);
        }
        if self.root_generation != state.document_generation {
            return self.fail(state, RuntimeError::StaleSemanticRef);
        }
        let scope = self.scope.clone();
        let mut step = |page: &mut PageState| {
            let result = self.work.poll(page, self.cancellation.as_ref());
            match result {
                Ok(None) => Ok(None),
                result => {
                    let result = result.map(|value| value.expect("completed command owns a value"));
                    if result.is_err() {
                        self.cleanup = self.work.take_cleanup();
                    }
                    self.work.finish(page, result.is_err(), false);
                    let (feedback, receipts) = if matches!(&self.work, Work::Actions(_)) {
                        (
                            page.take_drag_feedback(),
                            page.webdriver_input.take_os_receipts(),
                        )
                    } else {
                        (Default::default(), Vec::new())
                    };
                    Ok(Some((result, feedback, receipts)))
                }
            }
        };
        #[cfg(unix)]
        let result: Result<_, RuntimeError> = if let Some(scope) = scope {
            state.with_scoped_frame(scope, false, None, false, false, step)
        } else {
            step(state)
        };
        #[cfg(not(unix))]
        let result: Result<_, RuntimeError> = {
            let _ = scope;
            step(state)
        };
        match result {
            Ok(None) => Some(self),
            Ok(Some((result, mut feedback, receipts))) => {
                feedback.page_generation = state.document_generation;
                self.completion = Some(Completion::Finished(result, feedback, receipts));
                self.finish_completion(state)
            }
            Err(error) => self.fail(state, error),
        }
    }
    fn finish_completion(mut self, state: &mut PageState) -> Option<Self> {
        if self
            .cleanup
            .as_mut()
            .is_some_and(|cleanup| !cleanup.poll_with_trace(state))
        {
            return Some(self);
        }
        self.cleanup = None;
        self.reply
            .complete(self.completion.take().expect("completed command reply"));
        None
    }
    fn fail(mut self, state: &mut PageState, error: RuntimeError) -> Option<Self> {
        if self.cleanup.is_none() {
            self.cleanup = self.work.take_cleanup();
        }
        self.work
            .abort(state, self.root_generation == state.document_generation);
        let feedback = if matches!(&self.work, Work::Actions(_))
            && state
                .executable
                .as_ref()
                .is_some_and(|page| page.has_agent_host())
        {
            state.take_drag_feedback()
        } else {
            Default::default()
        };
        self.completion = Some(Completion::Failed(error, feedback));
        self.finish_completion(state)
    }
    /// Transfer native cleanup custody without entering a retired frame. The
    /// recipient must retain this cleanup before acknowledging terminal close.
    pub(super) fn reject(
        mut self,
        state: &mut PageState,
        error: RuntimeError,
    ) -> Option<PaintCleanup> {
        if self.cleanup.is_none() {
            self.cleanup = self.work.take_cleanup();
        }
        self.work
            .abort(state, self.root_generation == state.document_generation);
        let feedback = if matches!(&self.work, Work::Actions(_))
            && state
                .executable
                .as_ref()
                .is_some_and(|page| page.has_agent_host())
        {
            state.take_drag_feedback()
        } else {
            Default::default()
        };
        let completion = Completion::Failed(error, feedback);
        if let Some(mut cleanup) = self.cleanup.take() {
            cleanup.completion = Some((self.reply, completion));
            if cleanup.poll_with_trace(state) {
                None
            } else {
                Some(cleanup)
            }
        } else {
            self.reply.complete(completion);
            None
        }
    }
}

fn closed_error() -> ScriptError {
    ScriptError {
        code: "no such window".into(),
        message: "page closed during WebDriver command".into(),
    }
}
fn script_runtime(error: RuntimeError) -> ScriptError {
    ScriptError::javascript(error.to_string())
}
impl Work {
    fn script(request: ScriptRequest, context: ScriptContext) -> Self {
        Self::Script(Box::new(Script {
            request: Some(request),
            context,
            token: None,
            generation: None,
        }))
    }
    fn poll(
        &mut self,
        state: &mut PageState,
        cancellation: Option<&Arc<oryn_js::prompts::PromptController>>,
    ) -> Result<Option<ScriptValue>, ScriptError> {
        match self {
            Self::Actions(actions) => actions
                .poll(state, cancellation)
                .map(|done| done.then_some(ScriptValue::Null)),
            Self::Script(script) => {
                let Script {
                    request,
                    context,
                    token,
                    generation,
                } = script.as_mut();
                if let Some(request) = request.take() {
                    *generation = Some(state.document_generation);
                    let executable = state.executable.as_mut().ok_or_else(ScriptError::retired)?;
                    let bindings = if let Some(handle) = context.top_window_handle.take() {
                        let root = executable
                            .automation_realms()
                            .into_iter()
                            .find(|entry| entry.parent_realm.is_none() && entry.live)
                            .ok_or_else(ScriptError::retired)?;
                        vec![crate::host::webdriver_script::WindowHandleBinding {
                            realm: root.realm,
                            document_generation: root.document_generation,
                            stream_epoch: root.stream_epoch,
                            handle,
                            top_level: true,
                        }]
                    } else {
                        std::mem::take(&mut context.handles)
                    };
                    executable.set_webdriver_window_handles(bindings)?;
                    *token = Some(if let Some(selected) = context.selected.as_ref() {
                        executable.begin_webdriver_script_in_context(selected, request)?
                    } else {
                        executable.begin_webdriver_script(request)?
                    });
                }
                if *generation != Some(state.document_generation) {
                    return Err(ScriptError::retired());
                }
                let executable = state.executable.as_mut().ok_or_else(ScriptError::retired)?;
                match executable.poll_webdriver_script(token.expect("started script token"))? {
                    ScriptPoll::Complete(value) => {
                        *token = None;
                        state.poll_page_tasks().map_err(script_runtime)?;
                        if *generation != Some(state.document_generation) {
                            return Err(ScriptError::retired());
                        }
                        state.sync_executable_semantics().map_err(script_runtime)?;
                        state.refresh_layout().map_err(script_runtime)?;
                        Ok(Some(value))
                    }
                    ScriptPoll::Pending => {
                        state.poll_page_tasks().map_err(script_runtime)?;
                        Ok(None)
                    }
                }
            }
        }
    }
    fn finish(&mut self, state: &mut PageState, failed: bool, interrupted: bool) {
        match self {
            Self::Script(script) => {
                let Script {
                    token, generation, ..
                } = script.as_mut();
                if *generation == Some(state.document_generation)
                    && let Some(token) = token.take()
                    && let Some(page) = state.executable.as_mut()
                {
                    page.cancel_webdriver_script(token);
                }
            }
            Self::Actions(actions) => actions.finish(state, failed, interrupted),
        }
    }
    fn take_cleanup(&mut self) -> Option<PaintCleanup> {
        match self {
            Self::Actions(actions) => actions.paint.as_mut().and_then(Paint::take_cleanup),
            Self::Script(_) => None,
        }
    }
    fn abort(&mut self, state: &mut PageState, same_root: bool) {
        match self {
            Self::Script(script) => {
                let token = &mut script.token;
                if same_root
                    && let Some(token) = token.take()
                    && let Some(page) = state.executable.as_mut()
                    && page.has_agent_host()
                {
                    page.cancel_webdriver_script(token);
                }
            }
            Self::Actions(actions) => {
                actions.paint = None;
                if let Some(input) = actions.input.as_mut() {
                    input.retire();
                    if same_root
                        && let Some(page) = state.executable.as_ref()
                        && page.has_agent_host()
                    {
                        page.cancel_webdriver_keyboard_defaults();
                        page.reset_webdriver_pointer_sources();
                    }
                }
            }
        }
    }
}

struct Actions {
    command: Option<ActionCommand>,
    context: ScriptContext,
    input: Option<InputState>,
    generation: Option<u64>,
    selected: Option<AutomationRealmSnapshot>,
    explicit_cancel: bool,
    prepare_paint: bool,
    paint_stamp: Option<(u64, u64, u64)>,
    paint: Option<Paint>,
    operation: Option<ActionExecution>,
    release: Option<ReleaseExecution>,
    cancel: Option<CancelExecution>,
}
impl Actions {
    fn new(command: ActionCommand, context: ScriptContext) -> Self {
        Self {
            explicit_cancel: matches!(&command, ActionCommand::Cancel),
            command: Some(command),
            context,
            input: None,
            generation: None,
            selected: None,
            prepare_paint: false,
            paint_stamp: None,
            paint: None,
            operation: None,
            release: None,
            cancel: None,
        }
    }
    fn poll(
        &mut self,
        state: &mut PageState,
        cancellation: Option<&Arc<oryn_js::prompts::PromptController>>,
    ) -> Result<bool, ScriptError> {
        if self.input.is_none() {
            if let Some(ActionCommand::Perform(request)) = &self.command {
                request.validate()?;
            }
            self.input = Some(std::mem::take(&mut state.webdriver_input));
            self.generation = Some(state.document_generation);
            let executable = state.executable.as_mut().ok_or_else(ScriptError::retired)?;
            let selected = if let Some(selected) = self.context.selected.take() {
                selected
            } else {
                executable
                    .automation_realms()
                    .into_iter()
                    .find(|entry| entry.parent_realm.is_none() && entry.live)
                    .ok_or_else(ScriptError::retired)?
            };
            self.selected = Some(selected.clone());
            executable.validate_action_context(&selected)?;
            self.prepare_paint = matches!(&self.command, Some(ActionCommand::Perform(request)) if request.sources.iter().any(|source| source.actions.iter().any(|action| matches!(action, Action::PointerButton {..} | Action::PointerMove {..} | Action::OsPointerMove {..}))));
            if self.prepare_paint {
                self.paint = Some(Paint::new(state)?);
            }
        }
        if self.generation != Some(state.document_generation) {
            return Err(ScriptError::retired());
        }
        let selected = self.selected.as_ref().ok_or_else(ScriptError::retired)?;
        state
            .executable
            .as_ref()
            .ok_or_else(ScriptError::retired)?
            .validate_action_context(selected)?;
        if self.prepare_paint
            && self.paint.is_none()
            && state
                .executable
                .as_ref()
                .and_then(|page| page.root_paint_inventory_stamp())
                != self.paint_stamp
        {
            self.paint = Some(Paint::new(state)?);
        }
        if let Some(paint) = self.paint.as_mut() {
            if !paint.poll(state, selected, cancellation)? {
                return Ok(false);
            }
            self.paint_stamp = paint.stamp;
            self.paint = None;
        }
        let input = self.input.as_mut().expect("actions own input state");
        if let Some(command) = self.command.take() {
            match command {
                ActionCommand::Perform(request) => {
                    self.operation = Some(ActionExecution::new(request, selected.clone(), input)?)
                }
                ActionCommand::Release => {
                    self.release = Some(ReleaseExecution::new(selected.clone()))
                }
                ActionCommand::Cancel => self.cancel = Some(CancelExecution::new(selected.clone())),
            }
        }
        let executable = state.executable.as_mut().ok_or_else(ScriptError::retired)?;
        let done = if let Some(operation) = self.operation.as_mut() {
            operation.poll(executable, input, Instant::now())?
        } else if let Some(release) = self.release.as_mut() {
            release.poll(executable, input, Instant::now())?
        } else if let Some(cancel) = self.cancel.as_mut() {
            cancel.poll(executable, input, Instant::now())?
        } else {
            true
        };
        state
            .executable
            .as_ref()
            .ok_or_else(ScriptError::retired)?
            .publish_drag_feedback_checkpoint()?;
        state.poll_page_tasks().map_err(script_runtime)?;
        state
            .executable
            .as_ref()
            .ok_or_else(ScriptError::retired)?
            .validate_action_context(selected)?;
        if done {
            state.sync_executable_semantics().map_err(script_runtime)?;
            state.refresh_layout().map_err(script_runtime)?;
        }
        Ok(done)
    }
    fn finish(&mut self, state: &mut PageState, failed: bool, cancelled: bool) {
        self.paint = None;
        let interrupted = failed
            && (cancelled
                || self.explicit_cancel
                || self.generation != Some(state.document_generation)
                || state.executable.as_ref().is_none_or(|page| {
                    self.selected
                        .as_ref()
                        .is_some_and(|selected| page.validate_action_context(selected).is_err())
                }));
        if let Some(mut input) = self.input.take() {
            if interrupted {
                if let Some(page) = state.executable.as_ref() {
                    page.cancel_webdriver_keyboard_defaults();
                    page.reset_webdriver_pointer_sources();
                }
                input.retire();
            }
            state.webdriver_input = input;
        }
    }
}

/// A complete, principal-preserving plan tree. Planning may enqueue native CSS
/// requests but never waits for a child or drains an image stream on the actor.
struct PaintTree {
    plan: PaintSnapshotPlan,
    images: Vec<(
        crate::executable::SelectedPaintImageLoad,
        Vec<PaintImageTarget>,
    )>,
    children: Vec<(u64, PaintTree)>,
}
impl PaintTree {
    fn prepare(
        state: &mut PageState,
        frames: &mut Vec<AutomationRealmSnapshot>,
    ) -> Result<Self, RuntimeError> {
        let mut plan = state.paint_snapshot_plan_for_media_without_frames(false, true)?;
        let mut images = Vec::new();
        if state.frame_scope.is_some() {
            let mut by_url = BTreeMap::new();
            for (index, image) in plan.snapshot.images.iter().enumerate() {
                by_url.insert(image.url.clone(), index);
                for alias in &image.url_aliases {
                    by_url.insert(alias.clone(), index);
                }
            }
            for (url, targets) in std::mem::take(&mut plan.pending) {
                if let Some(image) = by_url.get(url.as_str()) {
                    for target in targets {
                        target.assign(&mut plan.snapshot, *image);
                    }
                } else {
                    let load = state
                        .executable
                        .as_mut()
                        .ok_or(RuntimeError::EvaluationUnavailable)?
                        .prepare_selected_paint_image(
                            url.as_str(),
                            plan.config
                                .max_resource_cache_entry_bytes
                                .min(plan.config.max_response_bytes),
                        )
                        .map_err(map_host_error)?;
                    images.push((load, targets));
                }
            }
        }
        let tree = Self {
            plan,
            images,
            children: Vec::new(),
        };
        #[cfg(unix)]
        let mut tree = tree;
        #[cfg(unix)]
        {
            let inventory = state
                .executable
                .as_ref()
                .ok_or(RuntimeError::EvaluationUnavailable)?
                .automation_realms();
            let current = state
                .frame_scope
                .as_ref()
                .map(|scope| scope.realm)
                .or_else(|| {
                    inventory
                        .iter()
                        .find(|entry| entry.live && entry.parent_realm.is_none())
                        .map(|entry| entry.realm)
                });
            let Some(current) = current else {
                return Ok(tree);
            };
            let mut ancestor = Some(current);
            let mut seen = BTreeSet::new();
            while let Some(realm) = ancestor {
                if !seen.insert(realm) || seen.len() > 32 {
                    return Err(RuntimeError::PaintUnavailable(
                        "frame paint depth exceeds 32 or cyclic realm ancestry".into(),
                    ));
                }
                ancestor = inventory
                    .iter()
                    .find(|entry| entry.realm == realm)
                    .and_then(|entry| entry.parent_realm);
            }
            let children = inventory
                .into_iter()
                .filter(|entry| entry.live && entry.parent_realm == Some(current))
                .collect::<Vec<_>>();
            if children.len() > 256 {
                return Err(RuntimeError::PaintUnavailable(
                    "frame paint exceeds 256 direct children".into(),
                ));
            }
            for scope in children {
                let Some(owner) = scope.frame.map(pack_node_id) else {
                    continue;
                };
                let Some(node) = tree
                    .plan
                    .snapshot
                    .nodes
                    .iter()
                    .chain(
                        tree.plan
                            .snapshot
                            .svg_foreign_objects
                            .iter()
                            .flat_map(|object| object.nodes.iter()),
                    )
                    .find(|node| node.node_id == owner)
                else {
                    continue;
                };
                let Some(model) = node
                    .projection
                    .as_ref()
                    .map(|projection| &projection.box_model)
                    .or(node.geometry.box_model.as_ref())
                else {
                    continue;
                };
                if model.content_box.width <= 0.
                    || model.content_box.height <= 0.
                    || node.geometry.availability
                        != oryn_common::v4::GeometryAvailability::Available
                {
                    continue;
                }
                if !node.geometry.in_viewport
                    && !node.projection.as_ref().is_some_and(|projection| {
                        projection.paint_intersects_viewport(tree.plan.snapshot.viewport)
                    })
                {
                    continue;
                }
                let viewport = ViewportState {
                    width: model.content_box.width,
                    height: model.content_box.height,
                    device_pixel_ratio: tree.plan.snapshot.viewport.device_pixel_ratio,
                };
                if frames.len() >= 256 {
                    return Err(RuntimeError::PaintUnavailable(
                        "composed frame paint resource budget exceeded".into(),
                    ));
                }
                frames.push(scope.clone());
                let child = state.with_scoped_frame(
                    scope.clone(),
                    false,
                    Some(viewport),
                    false,
                    true,
                    |frame| Self::prepare(frame, frames),
                )?;
                if !state.executable.as_ref().is_some_and(|page| {
                    page.automation_realms()
                        .iter()
                        .any(|entry| entry.live && entry.same_document_as(&scope))
                }) {
                    return Err(RuntimeError::PaintUnavailable(
                        "frame lifetime changed during paint capture".into(),
                    ));
                }
                tree.children.push((owner, child));
            }
        }
        #[cfg(not(unix))]
        let _ = frames;
        Ok(tree)
    }
    fn complete(
        mut self,
        cancelled: &std::sync::atomic::AtomicBool,
        deadline: Instant,
    ) -> Result<PaintSnapshot, RuntimeError> {
        let mut count = 0usize;
        let mut nodes = self.plan.snapshot.nodes.len().saturating_add(
            self.plan
                .snapshot
                .svg_foreign_objects
                .iter()
                .map(|object| object.nodes.len())
                .sum::<usize>(),
        );
        let mut bytes = self
            .plan
            .snapshot
            .images
            .iter()
            .map(|image| image.bytes.len())
            .sum::<usize>();
        for (owner, child) in self.children {
            if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err(RuntimeError::Closed);
            }
            let child = child.complete(cancelled, deadline)?;
            let mut pending = vec![&child];
            while let Some(snapshot) = pending.pop() {
                count = count.saturating_add(1);
                nodes = nodes.saturating_add(snapshot.nodes.len()).saturating_add(
                    snapshot
                        .svg_foreign_objects
                        .iter()
                        .map(|object| object.nodes.len())
                        .sum::<usize>(),
                );
                bytes = bytes
                    .saturating_add(
                        snapshot
                            .images
                            .iter()
                            .map(|image| image.bytes.len())
                            .sum::<usize>(),
                    )
                    .saturating_add(
                        snapshot
                            .fonts
                            .values()
                            .map(|font| font.bytes.len())
                            .sum::<usize>(),
                    );
                if count > 256
                    || nodes > self.plan.config.max_layout_nodes
                    || bytes
                        > self
                            .plan
                            .config
                            .max_response_bytes
                            .saturating_add(self.plan.config.max_page_font_bytes)
                {
                    return Err(RuntimeError::PaintUnavailable(
                        "composed frame paint resource budget exceeded".into(),
                    ));
                }
                pending.extend(snapshot.frames.iter().map(|frame| frame.snapshot.as_ref()));
            }
            self.plan
                .snapshot
                .frames
                .push(oryn_common::v4::PaintFrameSnapshot {
                    owner_node: owner,
                    snapshot: Box::new(child),
                });
        }
        let mut image_bytes = self
            .plan
            .snapshot
            .images
            .iter()
            .map(|image| image.bytes.len())
            .sum::<usize>();
        for (load, targets) in self.images {
            let limit = self.plan.config.max_resource_cache_entry_bytes.min(
                self.plan
                    .config
                    .max_response_bytes
                    .saturating_sub(image_bytes),
            );
            let image = load
                .with_limit(limit)
                .run(cancelled, deadline)
                .map_err(map_host_error)?;
            bytes = bytes.saturating_add(image.bytes.len());
            if bytes
                > self
                    .plan
                    .config
                    .max_response_bytes
                    .saturating_add(self.plan.config.max_page_font_bytes)
            {
                return Err(RuntimeError::PaintUnavailable(
                    "composed frame paint resource budget exceeded".into(),
                ));
            }
            image_bytes = image_bytes.saturating_add(image.bytes.len());
            if image_bytes > self.plan.config.max_response_bytes {
                return Err(RuntimeError::PaintUnavailable(
                    "frame paint resource budget exceeded".into(),
                ));
            }
            let index = self.plan.snapshot.images.len();
            self.plan.snapshot.images.push(image);
            for target in targets {
                target.assign(&mut self.plan.snapshot, index);
            }
        }
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(RuntimeError::Closed);
        }
        complete_paint_snapshot_cancellable(self.plan, cancelled)
    }
}

/// CSS image custody and the native renderer are retained separately. Neither
/// pending images nor the paint worker may occupy the shared actor's thread.
struct Paint {
    frames: Vec<AutomationRealmSnapshot>,
    generation: u64,
    identity: Option<crate::executable::CaptureDocumentIdentity>,
    deadline: Instant,
    css: Option<std::rc::Rc<crate::executable::NativeCssCapture>>,
    worker: Option<std::thread::JoinHandle<Result<PaintSnapshot, RuntimeError>>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    stamp: Option<(u64, u64, u64)>,
}
impl Paint {
    fn new(state: &PageState) -> Result<Self, ScriptError> {
        Ok(Self {
            frames: Vec::new(),
            generation: state.document_generation,
            identity: state
                .executable
                .as_ref()
                .and_then(|page| page.capture_document_identity()),
            deadline: Instant::now() + state.config.request_timeout,
            css: Some(
                crate::executable::NativeCssCapture::new()
                    .map_err(|error| ScriptError::javascript(error.to_string()))?,
            ),
            worker: None,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stamp: None,
        })
    }
    fn poll(
        &mut self,
        state: &mut PageState,
        selected: &AutomationRealmSnapshot,
        cancellation: Option<&Arc<oryn_js::prompts::PromptController>>,
    ) -> Result<bool, ScriptError> {
        if self.generation != state.document_generation
            || self.identity
                != state
                    .executable
                    .as_ref()
                    .and_then(|page| page.capture_document_identity())
        {
            return Err(ScriptError::retired());
        }
        if Instant::now() >= self.deadline
            || cancellation.is_some_and(|token| token.inspect().is_err())
        {
            return Err(ScriptError {
                code: "timeout".into(),
                message: "native action paint preparation cancelled or timed out".into(),
            });
        }
        if self.worker.is_none() {
            let owner = self
                .css
                .as_ref()
                .expect("paint preparation owns CSS capture");
            let _capture_scope = state
                .executable
                .as_ref()
                .map(|page| page.activate_css_capture(owner.clone()));
            if owner.pending() {
                state.poll_page_tasks().map_err(script_runtime)?;
                return Ok(false);
            }
            let page = state.executable.as_mut().ok_or_else(ScriptError::retired)?;
            let previous = page.replace_native_paint_cancellation(Some(
                crate::executable::NativePaintCancellation {
                    cancellation: cancellation.cloned(),
                    deadline: self.deadline,
                },
            ));
            self.frames.clear();
            let plan = PaintTree::prepare(state, &mut self.frames);
            if self.generation == state.document_generation
                && self.identity
                    == state
                        .executable
                        .as_ref()
                        .and_then(|page| page.capture_document_identity())
                && let Some(page) = state.executable.as_mut()
            {
                page.replace_native_paint_cancellation(previous);
            }
            let plan = plan.map_err(script_runtime)?;
            if self.generation != state.document_generation
                || self.identity
                    != state
                        .executable
                        .as_ref()
                        .and_then(|page| page.capture_document_identity())
            {
                return Err(ScriptError::retired());
            }
            if owner.pending() {
                state.poll_page_tasks().map_err(script_runtime)?;
                return Ok(false);
            }
            owner
                .release()
                .map_err(|error| ScriptError::javascript(error.to_string()))?;
            self.css = None;
            self.stamp = state
                .executable
                .as_ref()
                .and_then(|page| page.root_paint_inventory_stamp());
            let cancelled = self.cancelled.clone();
            let deadline = self.deadline;
            self.worker = Some(std::thread::spawn(move || {
                plan.complete(&cancelled, deadline)
            }));
            return Ok(false);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Ok(false);
        }
        let snapshot = self
            .worker
            .take()
            .expect("paint worker retained")
            .join()
            .map_err(|_| ScriptError::javascript("native action paint worker panicked"))?
            .map_err(script_runtime)?;
        let scoped = state.frame_scope.is_some();
        let page = state.executable.as_mut().ok_or_else(ScriptError::retired)?;
        page.validate_action_context(selected)?;
        let inventory = page.automation_realms();
        if self.frames.iter().any(|scope| {
            !inventory
                .iter()
                .any(|entry| entry.live && entry.same_document_as(scope))
        }) {
            return Err(ScriptError::retired());
        }
        if page.root_paint_inventory_stamp() != self.stamp {
            return Err(ScriptError::retired());
        }
        let parent_realm = page
            .current_automation_scope()
            .map(|scope| scope.realm)
            .or_else(|| {
                inventory
                    .iter()
                    .find(|entry| entry.live && entry.parent_realm.is_none())
                    .map(|entry| entry.realm)
            })
            .ok_or_else(ScriptError::retired)?;
        Self::install_frames(page, &snapshot, &self.frames, parent_realm)?;
        if scoped {
            page.install_selected_paint_capture_inventory(&snapshot)
                .map_err(|error| ScriptError::javascript(error.to_string()))?;
        } else if let Some(stamp) = self.stamp {
            page.install_root_paint_capture_inventory(stamp, &snapshot)
                .map_err(|error| ScriptError::javascript(error.to_string()))?;
        }
        Ok(true)
    }
    fn take_cleanup(&mut self) -> Option<PaintCleanup> {
        self.cancelled.store(true, Ordering::Release);
        self.worker.take().map(|worker| PaintCleanup {
            worker: Some(worker),
            completion: None,
            diagnostic: None,
        })
    }
    fn install_frames(
        page: &mut crate::executable::RustExecutableDocument,
        snapshot: &PaintSnapshot,
        frames: &[AutomationRealmSnapshot],
        parent: oryn_js::realm::RealmId,
    ) -> Result<(), ScriptError> {
        for child in &snapshot.frames {
            let scope = frames
                .iter()
                .find(|scope| {
                    scope.parent_realm == Some(parent)
                        && scope.frame.map(pack_node_id) == Some(child.owner_node)
                })
                .ok_or_else(ScriptError::retired)?;
            let previous = page.current_automation_scope();
            page.set_automation_scope(Some(scope.clone()))
                .map_err(|error| ScriptError::javascript(error.to_string()))?;
            let result = (|| {
                Self::install_frames(page, &child.snapshot, frames, scope.realm)?;
                page.install_selected_paint_capture_inventory(&child.snapshot)
                    .map_err(|error| ScriptError::javascript(error.to_string()))
            })();
            let restoration = page
                .set_automation_scope(previous)
                .map_err(|error| ScriptError::javascript(error.to_string()));
            result?;
            restoration?;
        }
        Ok(())
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(owner) = self.css.take() {
            let _ = owner.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(id: u64) -> PageState {
        let mut state = PageState::new(
            PageId(id),
            None,
            Default::default(),
            Default::default(),
            None,
            #[cfg(unix)]
            None,
        );
        state.bind_direct_fixture_terminal_owner();
        state
            .load_document(
                "https://webdriver-fairness.test/".into(),
                "<body><input autofocus></body>",
                None,
                None,
                HistoryUpdate::Push,
                &BTreeMap::new(),
                Vec::new(),
            )
            .unwrap();
        state
    }
    fn pending_script() -> ScriptRequest {
        ScriptRequest {
            body: "globalThis.finishCommand=arguments[0]"
                .encode_utf16()
                .collect(),
            args: vec![],
            asynchronous: true,
            timeout_ms: None,
        }
    }
    #[test]
    fn root_async_script_survives_child_document_replacement() {
        let mut page = state(849);
        let (reply, mut response) = oneshot::channel();
        let request = ScriptRequest {
            body: r#"
                const done = arguments[0];
                const frame = document.createElement('iframe');
                document.body.append(frame);
                (async () => {
                    for (let i = 0; i < 8; i++) {
                        frame.width = 300;
                        await new Promise(resolve => {
                            frame.onload = resolve;
                            frame.srcdoc = '<style>@layer { target {color:red} } @media(min-width:500px) { @layer { target {color:green} } }</style><target></target>';
                        });
                        const target = frame.contentDocument.querySelector('target');
                        getComputedStyle(target).color;
                        frame.width = 500;
                        getComputedStyle(target).color;
                    }
                    done('child loaded');
                })().catch(error => done(String(error)));
            "#.encode_utf16().collect(),
            args: vec![],
            asynchronous: true,
            timeout_ms: Some(5000),
        };
        let mut pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverScript {
                request,
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &page,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while let Some(command) = pending.take() {
            assert!(Instant::now() < deadline, "child replacement must settle");
            pending = command.poll(&mut page);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            response.try_recv().unwrap().unwrap(),
            ScriptValue::String("child loaded".encode_utf16().collect())
        );
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_script_yields_root_turn_and_finishes_after_sibling_progress() {
        let browser = Browser::in_process_with_config(Default::default());
        let context = browser.new_context();
        let waiting = context.new_page();
        waiting
            .load_html("https://webdriver-fairness.test/", "<body>waiting</body>")
            .await
            .unwrap();
        waiting
            .evaluate("globalThis.sibling=window.open('about:blank')")
            .await
            .unwrap();
        let sibling = context
            .wait_for_page(
                PagePredicate::Opener {
                    page_id: waiting.page_id(),
                },
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let source = waiting.clone();
        let command =
            tokio::spawn(async move { source.execute_webdriver_script(pending_script()).await });
        // Real related pages share one actor/host. Observing the installed
        // callback proves the command is pending before the sibling completes it.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if sibling
                    .evaluate("String(typeof opener.finishCommand==='function')")
                    .await
                    .unwrap()
                    == "true"
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("sibling commands must run while the source script is pending");
        assert!(!command.is_finished(), "callback remains pending");
        assert_eq!(sibling.evaluate(
            "globalThis.siblingAdvanced=true;opener.finishCommand('after sibling');'advanced'"
        ).await.unwrap(), "advanced");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), command)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            ScriptValue::String("after sibling".encode_utf16().collect())
        );
        assert_eq!(
            sibling.evaluate("String(siblingAdvanced)").await.unwrap(),
            "true"
        );
        sibling.close().await.unwrap();
        waiting.close().await.unwrap();
    }
    #[test]
    fn script_deadline_remains_host_owned_across_turns() {
        let mut state = state(842);
        let (reply, mut response) = oneshot::channel();
        let mut request = pending_script();
        request.timeout_ms = Some(0);
        let pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverScript {
                request,
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &state,
        )
        .unwrap();
        assert!(pending.poll(&mut state).is_none());
        assert_eq!(
            response.try_recv().unwrap().unwrap_err().code,
            "script timeout"
        );
    }
    #[test]
    fn abandoned_script_releases_token_without_starting_another_command() {
        let mut state = state(843);
        let (reply, response) = oneshot::channel();
        let pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverScript {
                request: pending_script(),
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &state,
        )
        .unwrap();
        let pending = pending.poll(&mut state).expect("callback remains pending");
        let Work::Script(script) = &pending.work else {
            panic!("script owns token")
        };
        let token = script.token.expect("script token");
        drop(response);
        assert!(pending.poll(&mut state).is_none());
        assert!(
            state
                .executable
                .as_mut()
                .unwrap()
                .poll_webdriver_script(token)
                .is_err()
        );
    }
    #[test]
    fn document_replacement_rejects_pending_script_without_entering_new_document() {
        let mut state = state(844);
        let (reply, mut response) = oneshot::channel();
        let pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverScript {
                request: pending_script(),
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &state,
        )
        .unwrap()
        .poll(&mut state)
        .unwrap();
        state
            .load_document(
                "https://webdriver-fairness.test/replaced".into(),
                "<body>replacement</body>",
                None,
                None,
                HistoryUpdate::Push,
                &BTreeMap::new(),
                Vec::new(),
            )
            .unwrap();
        assert!(pending.poll(&mut state).is_none());
        assert!(response.try_recv().unwrap().is_err());
        assert_eq!(state.evaluate("typeof finishCommand").unwrap(), "undefined");
    }
    #[test]
    fn pending_action_pause_yields_and_abandonment_retires_held_sources() {
        let mut waiting = state(845);
        let request = crate::webdriver_actions::ActionsRequest::parse(&ScriptValue::from_json_text(r#"{"actions":[{"id":"held","type":"key","actions":[{"type":"keyDown","value":"\ue008"},{"type":"pause","duration":60000}]}]}"#).unwrap()).unwrap();
        let (reply, mut response) = oneshot::channel();
        let pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverActions {
                command: ActionCommand::Perform(request),
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &waiting,
        )
        .unwrap()
        .poll(&mut waiting)
        .unwrap();
        let pending = pending.poll(&mut waiting).unwrap();
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let mut sibling = state(846);
        assert_eq!(
            sibling.evaluate("'action sibling'").unwrap(),
            "action sibling"
        );
        drop(response);
        assert!(pending.poll(&mut waiting).is_none());
        assert!(!waiting.webdriver_input.modifiers().shift);
        assert!(waiting.webdriver_input.source_kind("held").is_err());
    }
    fn gated_paint(
        state: &PageState,
    ) -> (
        Paint,
        std::sync::mpsc::Sender<()>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let settled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_settled = settled.clone();
        let mut paint = Paint::new(state).unwrap();
        paint.worker = Some(std::thread::spawn(move || {
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            worker_settled.store(true, Ordering::Release);
            Err(RuntimeError::PaintUnavailable("worker failure".into()))
        }));
        (paint, sender, settled)
    }
    fn await_cleanup(cleanup: &mut PaintCleanup) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cleanup.poll() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn pending_paint_worker_yields_and_transfers_unsettled_cleanup() {
        let mut state = state(847);
        let selected = state
            .executable
            .as_ref()
            .unwrap()
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_none() && realm.live)
            .unwrap();
        let (mut paint, sender, settled) = gated_paint(&state);
        let cancelled = paint.cancelled.clone();
        assert!(!paint.poll(&mut state, &selected, None).unwrap());
        assert_eq!(
            state.evaluate("'worker yielded'").unwrap(),
            "worker yielded"
        );
        let mut cleanup = paint.take_cleanup().unwrap();
        drop(paint);
        assert!(cancelled.load(Ordering::Acquire));
        assert!(!settled.load(Ordering::Acquire));
        assert!(
            !cleanup.poll(),
            "cancellation cannot claim a gated worker settled"
        );
        sender.send(()).unwrap();
        await_cleanup(&mut cleanup);
        assert!(settled.load(Ordering::Acquire));
        assert!(
            cleanup.worker.is_none(),
            "completion includes joining the worker"
        );
    }
    #[test]
    fn pending_paint_worker_keeps_original_deadline_and_cleanup_custody() {
        let mut state = state(848);
        let selected = state
            .executable
            .as_ref()
            .unwrap()
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_none() && realm.live)
            .unwrap();
        let (mut paint, sender, settled) = gated_paint(&state);
        paint.deadline = Instant::now();
        assert_eq!(
            paint.poll(&mut state, &selected, None).unwrap_err().code,
            "timeout"
        );
        let mut cleanup = paint.take_cleanup().unwrap();
        assert!(!cleanup.poll());
        assert!(!settled.load(Ordering::Acquire));
        sender.send(()).unwrap();
        await_cleanup(&mut cleanup);
    }
    #[test]
    fn panicking_cleanup_retains_primary_rejection_until_settlement() {
        let mut state = state(854);
        let (release, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || -> Result<PaintSnapshot, RuntimeError> {
            wait.recv().unwrap();
            panic!("injected cleanup panic");
        });
        let (reply, mut response) = oneshot::channel();
        let mut cleanup = PaintCleanup {
            worker: Some(worker),
            diagnostic: None,
            completion: Some((
                Reply::Actions(reply),
                Completion::Failed(RuntimeError::Closed, Default::default()),
            )),
        };
        assert!(!cleanup.poll_with_trace(&mut state));
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        await_cleanup(&mut cleanup);
        assert_eq!(
            cleanup.diagnostic.as_deref(),
            Some("native paint cleanup worker panicked")
        );
        assert_eq!(
            response.try_recv().unwrap().0.unwrap_err().code,
            "no such window"
        );
        assert!(cleanup.poll_with_trace(&mut state));
        assert!(cleanup.diagnostic.is_none());
        assert!(state.trace.events().iter().any(|event| matches!(
            &event.kind, TraceEventKind::ScriptDiagnostic { capability, detail }
                if capability == "paint.cleanup" && detail == "native paint cleanup worker panicked"
        )));
    }
    #[test]
    fn terminal_rejection_defers_command_reply_until_worker_cleanup_settles() {
        let mut state = state(852);
        let (paint, sender, settled) = gated_paint(&state);
        let (reply, mut response) = oneshot::channel();
        let mut pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverActions {
                command: ActionCommand::Cancel,
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &state,
        )
        .unwrap();
        let Work::Actions(actions) = &mut pending.work else {
            unreachable!()
        };
        actions.paint = Some(paint);
        let mut cleanup = pending
            .reject(&mut state, RuntimeError::Closed)
            .expect("unsettled worker transferred to retiring root");
        assert!(!cleanup.poll());
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(!settled.load(Ordering::Acquire));
        sender.send(()).unwrap();
        await_cleanup(&mut cleanup);
        assert_eq!(
            response.try_recv().unwrap().0.unwrap_err().code,
            "no such window"
        );
    }
    #[test]
    fn normal_failure_retains_command_barrier_until_worker_is_joined() {
        let mut state = state(853);
        let (paint, sender, settled) = gated_paint(&state);
        let (reply, mut response) = oneshot::channel();
        let mut pending = Pending::from_command(
            &mut Some(PageCommand::WebdriverActions {
                command: ActionCommand::Cancel,
                context: Default::default(),
                cancellation: None,
                reply,
            }),
            &state,
        )
        .unwrap();
        let Work::Actions(actions) = &mut pending.work else {
            unreachable!()
        };
        actions.paint = Some(paint);
        let mut pending = pending
            .fail(&mut state, RuntimeError::Closed)
            .expect("cleanup retains serialized command");
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(!settled.load(Ordering::Acquire));
        sender.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match pending.poll(&mut state) {
                Some(next) => pending = next,
                None => break,
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(response.try_recv().unwrap().0.is_err());
        assert!(settled.load(Ordering::Acquire));
    }
    #[cfg(unix)]
    fn framed_state(id: u64) -> PageState {
        struct Buffered;
        impl crate::network::NetworkBroker for Buffered {
            fn send(
                &self,
                request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                Ok(crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![],
                    body: Vec::new(),
                })
            }
        }
        let mut state = PageState::new(
            PageId(id),
            None,
            Default::default(),
            Default::default(),
            Some(SharedNetworkBroker::new(Buffered)),
            None,
        );
        state.bind_direct_fixture_terminal_owner();
        state.load_document("https://webdriver-fairness.test/frames".into(), r#"<iframe style='width:120px;height:80px' srcdoc='<p style="color:blue">Child paint</p>'></iframe><iframe style='display:none' srcdoc='<p>Hidden</p>'></iframe>"#, None, None, HistoryUpdate::Push, &BTreeMap::new(), Vec::new()).unwrap();
        state.poll_page_tasks().unwrap();
        state
    }
    #[cfg(unix)]
    #[test]
    fn frame_plan_tree_preserves_composed_geometry_and_restores_parent_scope() {
        let mut state = framed_state(849);
        let owner = crate::executable::NativeCssCapture::new().unwrap();
        let _scope = state
            .executable
            .as_ref()
            .unwrap()
            .activate_css_capture(owner.clone());
        let mut frames = Vec::new();
        let tree = PaintTree::prepare(&mut state, &mut frames).unwrap();
        assert!(!owner.pending());
        assert_eq!(
            frames.len(),
            1,
            "hidden frame remains excluded by paint geometry"
        );
        assert!(
            state
                .executable
                .as_ref()
                .unwrap()
                .current_automation_scope()
                .is_none()
        );
        let snapshot = tree
            .complete(
                &std::sync::atomic::AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let expected =
            complete_paint_snapshot(state.paint_snapshot_plan_for_media(false).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&snapshot.nodes).unwrap(),
            serde_json::to_value(&expected.nodes).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.frames).unwrap(),
            serde_json::to_value(&expected.frames).unwrap()
        );
        let parent = state
            .executable
            .as_ref()
            .unwrap()
            .automation_realms()
            .into_iter()
            .find(|scope| scope.parent_realm.is_none())
            .unwrap()
            .realm;
        Paint::install_frames(
            state.executable.as_mut().unwrap(),
            &snapshot,
            &frames,
            parent,
        )
        .unwrap();
        assert!(
            state
                .executable
                .as_ref()
                .unwrap()
                .current_automation_scope()
                .is_none()
        );
    }
    #[cfg(unix)]
    #[test]
    fn scoped_pending_script_restores_scope_and_rejects_detached_frame() {
        let mut state = framed_state(850);
        let selected = state
            .executable
            .as_ref()
            .unwrap()
            .automation_realms()
            .into_iter()
            .find(|scope| scope.parent_realm.is_some())
            .unwrap();
        let (reply, mut response) = oneshot::channel();
        let pending = Pending::from_command(
            &mut Some(PageCommand::ScopedWorker {
                scope: selected,
                request: crate::worker::WorkerRequest::WebdriverScript {
                    request: pending_script(),
                    context: Default::default(),
                },
                reply,
            }),
            &state,
        )
        .unwrap()
        .poll(&mut state)
        .unwrap();
        assert!(
            state
                .executable
                .as_ref()
                .unwrap()
                .current_automation_scope()
                .is_none()
        );
        assert_eq!(state.evaluate("typeof finishCommand").unwrap(), "undefined");
        state
            .evaluate("document.querySelector('iframe').remove()")
            .unwrap();
        assert!(pending.poll(&mut state).is_none());
        assert!(response.try_recv().unwrap().is_err());
        assert!(
            state
                .executable
                .as_ref()
                .unwrap()
                .current_automation_scope()
                .is_none()
        );
    }
    #[test]
    fn ordinary_action_error_preserves_held_inputs_until_release() {
        fn run(state: &mut PageState, command: ActionCommand) -> Result<(), ScriptError> {
            let (reply, mut response) = oneshot::channel();
            let mut pending = Some(
                Pending::from_command(
                    &mut Some(PageCommand::WebdriverActions {
                        command,
                        context: Default::default(),
                        cancellation: None,
                        reply,
                    }),
                    state,
                )
                .unwrap(),
            );
            for _ in 0..64 {
                let Some(current) = pending.take() else {
                    break;
                };
                pending = current.poll(state);
            }
            assert!(
                pending.is_none(),
                "zero-duration input must finish within bounded checkpoints"
            );
            response.try_recv().unwrap().0
        }
        let mut state = state(851);
        let parse = |json: &str| {
            ActionCommand::Perform(
                crate::webdriver_actions::ActionsRequest::parse(
                    &ScriptValue::from_json_text(json).unwrap(),
                )
                .unwrap(),
            )
        };
        run(&mut state, parse(r#"{"actions":[{"id":"held","type":"key","actions":[{"type":"keyDown","value":"\ue008"}]}]}"#)).unwrap();
        assert!(state.webdriver_input.modifiers().shift);
        assert!(run(&mut state, parse(r#"{"actions":[{"id":"held","type":"none","actions":[{"type":"pause","duration":0}]}]}"#)).is_err());
        assert!(
            state.webdriver_input.modifiers().shift,
            "ordinary admission error preserves dispatched held input"
        );
        run(&mut state, ActionCommand::Release).unwrap();
        assert!(!state.webdriver_input.modifiers().shift);
        assert!(state.webdriver_input.source_kind("held").is_err());
    }
}
