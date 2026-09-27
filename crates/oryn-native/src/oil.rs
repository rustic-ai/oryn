use std::{collections::BTreeMap, time::Duration};

use oryn_common::agent::{Intent, IntentRequest};
use oryn_common::v3::{
    ActionResult, CapabilityDiagnostic, ExecutionDomain, Observation, ObservationDelta,
    PageDescriptor, PageId, PagePredicate, ProjectionProfile, Revision, SemanticAction,
    SemanticRef, SupportLevel,
};
use oryn_core::ast::{Command, ExtractWhat, RelationKind, Target, TargetAtomic, WaitCondition};
use serde::{Deserialize, Serialize};

use crate::IntentResult;
use crate::runtime::{
    BrowserContext, ContentKind, LifecycleState, NativeAction, NavigationResult, PageHandle,
    RuntimeError, ScrollRequest, WaitPredicate,
};

pub const TYPED_INTENT_PREFIX: &str = "@oryn-intent ";

pub struct NativeOilSession {
    page: PageHandle,
    last_observation: Option<Observation>,
    delta_cursor: oryn_common::v3::Revision,
    viewport: oryn_common::v4::ViewportState,
    media: oryn_common::v4::MediaState,
}

/// Parent-side OIL session for a complete browser context. Page-local OIL
/// state remains isolated per page while page selection and lifecycle are
/// coordinated outside sandboxed workers.
pub struct NativeContextOilSession {
    context: BrowserContext,
    workflow_audit: Option<crate::workflow_instrumentation::WorkflowAudit>,
    sessions: BTreeMap<PageId, NativeOilSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeOilOutput {
    Observation {
        observation: Observation,
    },
    GeometryObservation {
        observation: oryn_common::v4::Observation,
    },
    Delta {
        delta: ObservationDelta,
    },
    Action {
        result: ActionResult,
    },
    Navigation {
        navigation: NavigationResult,
    },
    Text {
        value: String,
    },
    Structured {
        value: serde_json::Value,
    },
    Box {
        box_model: oryn_common::v4::BoxModel,
    },
    Viewport {
        viewport: oryn_common::v4::ViewportState,
    },
    Media {
        media: oryn_common::v4::MediaState,
    },
    Scroll {
        scroll: oryn_common::v4::ScrollState,
    },
    Trace {
        events: Vec<crate::trace::TraceEvent>,
    },
    Pages {
        pages: Vec<PageDescriptor>,
    },
    Page {
        page: PageDescriptor,
    },
    Unsupported {
        diagnostic: CapabilityDiagnostic,
    },
    Intent {
        result: IntentResult,
    },
    Exit,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeOilError {
    #[error("OIL parse error: {0}")]
    Parse(String),
    #[error("run 'observe' before using a semantic target")]
    NoObservation,
    #[error("semantic target did not resolve: {0:?}")]
    TargetNotFound(Target),
    #[error("semantic target did not resolve: {target:?}: {detail}")]
    TargetResolution { target: Target, detail: String },
    #[error("invalid page command: {0}")]
    PageCommand(String),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("OIL script failed at command {failed_index} after completed commands: {cause}")]
    ScriptExecutionFailed {
        completed: Vec<NativeOilOutput>,
        failed_index: usize,
        #[source]
        cause: Box<NativeOilError>,
    },
}

impl NativeOilError {
    fn with_completed(self, completed: Vec<NativeOilOutput>, failed_index: usize) -> Self {
        if completed.is_empty() {
            self
        } else {
            Self::ScriptExecutionFailed {
                completed,
                failed_index,
                cause: Box::new(self),
            }
        }
    }

    pub fn completed_outputs(&self) -> &[NativeOilOutput] {
        match self {
            Self::ScriptExecutionFailed { completed, .. } => completed,
            _ => &[],
        }
    }

    /// Preserve typed native causes and committed output receipts across IPC/tool adapters.
    pub fn into_runtime(self) -> Result<RuntimeError, serde_json::Error> {
        Ok(match self {
            Self::Runtime(error) => error,
            Self::ScriptExecutionFailed {
                completed,
                failed_index,
                cause,
            } => RuntimeError::OilExecutionFailed {
                completed: completed
                    .into_iter()
                    .map(|output| serde_json::to_string(&output))
                    .collect::<Result<_, _>>()?,
                failed_index,
                cause: Box::new(cause.into_runtime()?),
            },
            error => RuntimeError::Script(error.to_string()),
        })
    }
}

impl NativeContextOilSession {
    pub fn new(context: BrowserContext) -> Self {
        Self {
            context,
            sessions: BTreeMap::new(),
            workflow_audit: None,
        }
    }

    pub fn set_workflow_audit(&mut self, audit: crate::workflow_instrumentation::WorkflowAudit) {
        self.workflow_audit = Some(audit);
    }

    pub async fn execute(&mut self, input: &str) -> Result<Vec<NativeOilOutput>, NativeOilError> {
        let guard = self
            .workflow_audit
            .as_ref()
            .map(|audit| audit.begin(self.context.id(), input));
        let result = self.execute_inner(input).await;
        if let Some(guard) = guard {
            guard.finish(result.is_ok());
        }
        result
    }

    async fn execute_inner(&mut self, input: &str) -> Result<Vec<NativeOilOutput>, NativeOilError> {
        let mut outputs = Vec::new();
        for (failed_index, line) in input
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .enumerate()
        {
            match self.execute_line(line).await {
                Ok(result) => outputs.extend(result),
                Err(error) => return Err(error.with_completed(outputs, failed_index)),
            }
        }
        Ok(outputs)
    }

    async fn execute_line(&mut self, input: &str) -> Result<Vec<NativeOilOutput>, NativeOilError> {
        let mut outputs = Vec::new();
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(payload) = line.strip_prefix(TYPED_INTENT_PREFIX) {
                let request: IntentRequest = serde_json::from_str(payload)
                    .map_err(|error| NativeOilError::Parse(error.to_string()))?;
                let intent = request.validate().map_err(NativeOilError::Parse)?;
                outputs.push(NativeOilOutput::Intent {
                    result: self.perform(intent).await?,
                });
                continue;
            }
            if line.starts_with("upload ") {
                let page = self.context.active_page().ok_or_else(|| {
                    NativeOilError::PageCommand("browser context has no active page".into())
                })?;
                let page_id = page.page_id();
                let before = self.revision(page_id);
                let session = self
                    .sessions
                    .entry(page_id)
                    .or_insert_with(|| NativeOilSession::new(page));
                let outcome = session.execute_upload(line, &self.context).await;
                self.context.record_intent(
                    Some(page_id),
                    "upload [TARGET] [FILE_LABELS]".into(),
                    before,
                    self.revision(page_id),
                    match &outcome {
                        Ok(_) => "completed".into(),
                        Err(error) => format!("failed:{error}"),
                    },
                );
                outputs.push(NativeOilOutput::Action { result: outcome? });
                continue;
            }
            if line == "pages" || line.starts_with("page ") {
                let page_id = self.context.active_page().map(|page| page.page_id());
                let before = page_id.and_then(|id| self.revision(id));
                match self.execute_page_command(line).await {
                    Ok(output) => {
                        let after = self
                            .context
                            .active_page()
                            .and_then(|page| self.revision(page.page_id()));
                        self.context.record_intent(
                            page_id,
                            scrub_oil_input(line),
                            before,
                            after,
                            "completed".into(),
                        );
                        outputs.push(output);
                    }
                    Err(error) => {
                        self.context.record_intent(
                            page_id,
                            scrub_oil_input(line),
                            before,
                            before,
                            format!("failed:{error}"),
                        );
                        return Err(error);
                    }
                }
                continue;
            }
            let page = self.context.active_page().ok_or_else(|| {
                NativeOilError::PageCommand("browser context has no active page".into())
            })?;
            let page_id = page.page_id();
            let before = self.revision(page_id);
            let session = self
                .sessions
                .entry(page_id)
                .or_insert_with(|| NativeOilSession::new(page));
            match session.execute(line).await {
                Ok(result) => {
                    let after = self.revision(page_id);
                    self.context.record_intent(
                        Some(page_id),
                        scrub_oil_input(line),
                        before,
                        after,
                        "completed".into(),
                    );
                    outputs.extend(result);
                }
                Err(error) => {
                    self.context.record_intent(
                        Some(page_id),
                        scrub_oil_input(line),
                        before,
                        self.revision(page_id),
                        format!("failed:{error}"),
                    );
                    return Err(error);
                }
            }
        }
        Ok(outputs)
    }

    /// Execute the stable typed agent contract against the active page.
    /// The page implementation delegates to the same semantic resolver and
    /// native action engine as OIL.
    pub async fn perform(&mut self, intent: Intent) -> Result<IntentResult, NativeOilError> {
        let page = self.context.active_page().ok_or_else(|| {
            NativeOilError::PageCommand("browser context has no active page".into())
        })?;
        let page_id = page.page_id();
        let before = self.revision(page_id);
        let audit_label = intent_audit_label(&intent);
        let session = self
            .sessions
            .entry(page_id)
            .or_insert_with(|| NativeOilSession::new(page));
        match session.perform_typed(intent).await {
            Ok(result) => {
                let after = self.revision(page_id);
                self.context.record_intent(
                    Some(page_id),
                    audit_label,
                    before,
                    after,
                    "completed".into(),
                );
                Ok(result)
            }
            Err(error) => {
                self.context.record_intent(
                    Some(page_id),
                    audit_label,
                    before,
                    self.revision(page_id),
                    format!("failed:{error}"),
                );
                Err(error)
            }
        }
    }

    fn revision(&self, page_id: PageId) -> Option<oryn_common::v3::Revision> {
        self.context
            .pages()
            .into_iter()
            .find(|page| page.page.page_id == page_id)
            .map(|page| page.revision)
    }

    async fn execute_page_command(
        &mut self,
        input: &str,
    ) -> Result<NativeOilOutput, NativeOilError> {
        let tokens = page_tokens(input)?;
        match tokens.as_slice() {
            [command] if command == "pages" => Ok(NativeOilOutput::Pages {
                pages: self.context.pages(),
            }),
            [page, command] if page == "page" && command == "list" => Ok(NativeOilOutput::Pages {
                pages: self.context.pages(),
            }),
            [page, command] if page == "page" && command == "current" => {
                let active = self.context.active_page().ok_or_else(|| {
                    NativeOilError::PageCommand("browser context has no active page".into())
                })?;
                Ok(NativeOilOutput::Page {
                    page: self.descriptor(active.page_id())?,
                })
            }
            [page, command, id] if page == "page" && command == "activate" => {
                let page_id = parse_page_id(id)?;
                self.context.activate_page(page_id)?;
                Ok(NativeOilOutput::Page {
                    page: self.descriptor(page_id)?,
                })
            }
            [page, command] if page == "page" && command == "close" => {
                let active = self.context.active_page().ok_or_else(|| {
                    NativeOilError::PageCommand("browser context has no active page".into())
                })?;
                let page_id = active.page_id();
                self.context.close_page(page_id).await?;
                self.sessions.remove(&page_id);
                Ok(NativeOilOutput::Page {
                    page: self.descriptor(page_id)?,
                })
            }
            [page, command, id] if page == "page" && command == "close" => {
                let page_id = parse_page_id(id)?;
                self.context.close_page(page_id).await?;
                self.sessions.remove(&page_id);
                Ok(NativeOilOutput::Page {
                    page: self.descriptor(page_id)?,
                })
            }
            [page, command, field, value] if page == "page" && command == "wait" => {
                let predicate = match field.as_str() {
                    "origin" => PagePredicate::Origin {
                        origin: value.clone(),
                    },
                    "url" => PagePredicate::UrlContains {
                        value: value.clone(),
                    },
                    "title" => PagePredicate::TitleContains {
                        value: value.clone(),
                    },
                    "opener" => PagePredicate::Opener {
                        page_id: parse_page_id(value)?,
                    },
                    _ => {
                        return Err(NativeOilError::PageCommand(format!(
                            "unknown page predicate {field:?}"
                        )));
                    }
                };
                let handle = self
                    .context
                    .wait_for_page(predicate, Duration::from_secs(30))
                    .await?;
                Ok(NativeOilOutput::Page {
                    page: self.descriptor(handle.page_id())?,
                })
            }
            _ => Err(NativeOilError::PageCommand(format!(
                "unsupported syntax: {input}"
            ))),
        }
    }

    fn descriptor(&self, page_id: PageId) -> Result<PageDescriptor, NativeOilError> {
        self.context
            .pages()
            .into_iter()
            .find(|page| page.page.page_id == page_id)
            .ok_or_else(|| NativeOilError::PageCommand(format!("unknown page {}", page_id.0)))
    }
}

fn intent_audit_label(intent: &Intent) -> String {
    let name = match intent {
        Intent::Navigate { .. } => "navigate",
        Intent::Back => "back",
        Intent::Forward => "forward",
        Intent::Refresh => "refresh",
        Intent::Observe { .. } => "observe",
        Intent::Click { .. } => "click",
        Intent::Type { .. } => "type",
        Intent::Clear { .. } => "clear",
        Intent::Check { .. } => "check",
        Intent::Select { .. } => "select",
        Intent::Submit { .. } => "submit",
        Intent::Focus { .. } => "focus",
        Intent::Hover { .. } => "hover",
        Intent::ClipboardRead => "clipboard_read",
        Intent::ClipboardWrite { .. } => "clipboard_write",
        Intent::Upload { .. } => "upload",
        Intent::Extract { .. } => "extract",
        Intent::WaitRevision { .. } => "wait_revision",
    };
    format!("intent:{name}")
}

fn scrub_oil_input(input: &str) -> String {
    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or_default();
    if matches!(command, "type" | "fill") {
        let target = parts.next().unwrap_or_default();
        return format!("{command} {target} [REDACTED]");
    }
    if matches!(command, "evaluate" | "eval" | "execute") {
        return format!("{command} [REDACTED_SCRIPT]");
    }
    input.to_string()
}

fn parse_page_id(value: &str) -> Result<PageId, NativeOilError> {
    value
        .parse::<u64>()
        .map(PageId)
        .map_err(|_| NativeOilError::PageCommand(format!("invalid page id {value:?}")))
}

fn page_tokens(input: &str) -> Result<Vec<String>, NativeOilError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(expected) = quote {
            if character == expected {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quote.is_some() {
        return Err(NativeOilError::PageCommand(
            "unterminated quote in page command".into(),
        ));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

impl NativeOilSession {
    pub fn new(page: PageHandle) -> Self {
        Self {
            page,
            last_observation: None,
            delta_cursor: oryn_common::v3::Revision(0),
            viewport: oryn_common::v4::ViewportState::default(),
            media: oryn_common::v4::MediaState::default(),
        }
    }

    pub async fn execute(&mut self, input: &str) -> Result<Vec<NativeOilOutput>, NativeOilError> {
        let normalized = oryn_core::normalize(input);
        let script = oryn_core::parse(&normalized)
            .map_err(|error| NativeOilError::Parse(error.to_string()))?;
        let mut outputs = Vec::new();
        for line in script.lines {
            if let Some(command) = line.command {
                match self.execute_command(command).await {
                    Ok(output) => outputs.push(output),
                    Err(error) => {
                        let failed_index = outputs.len();
                        return Err(error.with_completed(outputs, failed_index));
                    }
                }
            }
        }
        Ok(outputs)
    }

    async fn perform_typed(&mut self, mut intent: Intent) -> Result<IntentResult, NativeOilError> {
        let target = match &mut intent {
            Intent::Click { target, .. }
            | Intent::Type { target, .. }
            | Intent::Clear { target }
            | Intent::Check { target, .. }
            | Intent::Select { target, .. }
            | Intent::Submit { target }
            | Intent::Focus { target }
            | Intent::Hover { target }
            | Intent::Upload { target, .. } => Some(target),
            Intent::Navigate { .. }
            | Intent::Back
            | Intent::Forward
            | Intent::Refresh
            | Intent::Observe { .. }
            | Intent::ClipboardRead
            | Intent::ClipboardWrite { .. }
            | Intent::Extract { .. }
            | Intent::WaitRevision { .. } => None,
        };
        if let Some(target) = target {
            *target = bind_observed_aliases(target.clone(), self.last_observation.as_ref())?;
        }
        if matches!(
            &intent,
            Intent::Navigate { .. } | Intent::Back | Intent::Forward | Intent::Refresh
        ) {
            self.prepare_root_navigation();
        }
        let outcome = self.page.perform(intent).await;
        let mut result = self.accept_native_outcome(outcome)?;
        match &mut result {
            IntentResult::Observation { observation } => {
                self.delta_cursor = observation.revision;
                self.last_observation = Some(observation.clone());
            }
            IntentResult::GeometryObservation { observation } => {
                self.delta_cursor = observation.revision;
                self.cache_v4_observation(observation);
            }
            IntentResult::SemanticObservation { observation } => {
                self.delta_cursor = observation.observation.revision;
                self.last_observation = Some(observation.observation.clone());
            }
            IntentResult::SemanticObservationV6 { observation } => {
                self.delta_cursor = observation.revision;
                self.last_observation = Some(Observation {
                    contract_version: oryn_common::v3::CONTRACT_VERSION,
                    page: observation.page.clone(),
                    revision: observation.revision,
                    profile: observation.profile,
                    source: observation.source,
                    nodes: observation
                        .nodes
                        .iter()
                        .map(|node| node.semantic.clone())
                        .collect(),
                    capabilities: observation.capabilities.clone(),
                    diagnostics: observation.diagnostics.clone(),
                    execution_domain: observation.execution_domain,
                });
            }
            IntentResult::Delta { delta } => self.merge_observation_delta(delta),
            IntentResult::SemanticDeltaV6 { delta } => {
                self.merge_observation_delta(&ObservationDelta {
                    contract_version: oryn_common::v3::CONTRACT_VERSION,
                    page_id: delta.page_id,
                    from_revision: delta.from_revision,
                    to_revision: delta.to_revision,
                    upserted: delta
                        .changes
                        .upserted_nodes
                        .iter()
                        .map(|node| node.semantic.clone())
                        .collect(),
                    removed: delta.changes.removed_nodes.clone(),
                })
            }
            IntentResult::Action { result } => {
                *result = self.refresh_after_action(result.clone()).await;
            }
            IntentResult::Navigation { navigation } => {
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
            }
            IntentResult::Structured { .. }
            | IntentResult::Text { .. }
            | IntentResult::Wait { .. } => {}
        }
        Ok(result)
    }

    fn prepare_root_navigation(&mut self) {
        self.page = self.page.with_frame_scope(None);
        self.last_observation = None;
        self.delta_cursor = Revision(0);
    }

    fn merge_observation_delta(&mut self, delta: &ObservationDelta) {
        self.delta_cursor = delta.to_revision;
        let Some(observation) = self.last_observation.as_mut() else {
            return;
        };
        if observation.page.page_id != delta.page_id
            || observation.revision < delta.from_revision
            || observation.revision > delta.to_revision
            || delta.to_revision < delta.from_revision
            || delta.upserted.iter().any(|node| {
                node.semantic_ref.page_id != delta.page_id
                    || node.semantic_ref.document_generation != observation.page.document_generation
            })
            || delta.removed.iter().any(|reference| {
                reference.page_id != delta.page_id
                    || reference.document_generation != observation.page.document_generation
            })
        {
            self.last_observation = None;
            return;
        }
        observation.nodes.retain(|node| {
            !delta.removed.contains(&node.semantic_ref)
                && !delta
                    .upserted
                    .iter()
                    .any(|new| new.semantic_ref == node.semantic_ref)
        });
        observation.nodes.extend(delta.upserted.iter().cloned());
        observation.nodes.sort_by_key(|node| node.document_order);
        let mut aliases = std::collections::BTreeSet::new();
        let mut references = std::collections::BTreeSet::new();
        if observation
            .nodes
            .iter()
            .any(|node| !aliases.insert(node.alias) || !references.insert(node.semantic_ref))
        {
            self.last_observation = None;
            return;
        }
        observation.revision = delta.to_revision;
    }

    pub(crate) async fn resolve_typed_target(
        &mut self,
        target: &oryn_common::agent::Target,
        action: SemanticAction,
        force: bool,
    ) -> Result<SemanticRef, NativeOilError> {
        let bound = bind_observed_aliases(target.clone(), self.last_observation.as_ref())?;
        self.page
            .resolve_target(bound, if force { None } else { Some(action) })
            .await?
            .selected
            .ok_or_else(|| RuntimeError::StaleSemanticRef.into())
    }

    async fn execute_upload(
        &mut self,
        input: &str,
        context: &BrowserContext,
    ) -> Result<ActionResult, NativeOilError> {
        let tokens = input.split_whitespace().collect::<Vec<_>>();
        let file_index = tokens
            .iter()
            .position(|token| token.starts_with("$file:"))
            .ok_or_else(|| {
                NativeOilError::Parse("upload requires at least one $file:LABEL".into())
            })?;
        if file_index <= 1 {
            return Err(NativeOilError::Parse(
                "upload requires a target before $file:LABEL".into(),
            ));
        }
        let target_text = tokens[1..file_index].join(" ");
        let synthetic = format!("type {target_text} \"__oryn_upload__\"");
        let parsed = oryn_core::parse(&oryn_core::normalize(&synthetic))
            .map_err(|error| NativeOilError::Parse(error.to_string()))?;
        let target = parsed
            .lines
            .into_iter()
            .find_map(|line| match line.command {
                Some(Command::Type(command)) => Some(command.target),
                _ => None,
            })
            .ok_or_else(|| NativeOilError::Parse("upload target is invalid".into()))?;
        let reference = self
            .resolve_for_action(target, SemanticAction::Click, false)
            .await?;
        let handles = tokens[file_index..]
            .iter()
            .map(|token| {
                token
                    .strip_prefix("$file:")
                    .filter(|label| !label.is_empty())
                    .ok_or_else(|| {
                        NativeOilError::Parse("upload file operands must use $file:LABEL".into())
                    })
                    .and_then(|label| {
                        context
                            .upload_handle(label)
                            .map_err(NativeOilError::Runtime)
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outcome = self.page.set_files(reference, &handles, None).await;
        let result = self.accept_native_outcome(outcome)?;
        Ok(self.refresh_after_action(result).await)
    }

    fn invalidate_failed_action(&mut self, error: &RuntimeError) {
        if matches!(
            error,
            RuntimeError::ActionExecutionFailed { .. }
                | RuntimeError::BatchExecutionFailed { .. }
                | RuntimeError::OilExecutionFailed { .. }
        ) {
            self.last_observation = None;
            self.delta_cursor = Revision(0);
        }
    }

    fn accept_native_outcome<T>(
        &mut self,
        outcome: Result<T, RuntimeError>,
    ) -> Result<T, NativeOilError> {
        outcome.map_err(|error| {
            self.invalidate_failed_action(&error);
            NativeOilError::Runtime(error)
        })
    }

    async fn execute_command(
        &mut self,
        command: Command,
    ) -> Result<NativeOilOutput, NativeOilError> {
        let result = self.execute_command_inner(command).await;
        if let Err(NativeOilError::Runtime(error)) = &result {
            self.invalidate_failed_action(error);
        }
        result
    }

    async fn execute_command_inner(
        &mut self,
        command: Command,
    ) -> Result<NativeOilOutput, NativeOilError> {
        match command {
            Command::Frames => {
                let response = self
                    .page
                    .webdriver_frames(crate::host::webdriver_script::FrameCommand::Inventory)
                    .await
                    .map_err(|error| NativeOilError::PageCommand(error.to_string()))?;
                let crate::host::webdriver_script::FrameResponse::Inventory(frames) = response
                else {
                    return Err(NativeOilError::PageCommand(
                        "invalid native frame inventory".into(),
                    ));
                };
                Ok(NativeOilOutput::Structured {
                    value: serde_json::to_value(frames)
                        .map_err(|error| NativeOilError::PageCommand(error.to_string()))?,
                })
            }
            Command::Frame(command) => {
                use crate::host::webdriver_script::{
                    FrameCommand, FrameResponse, WebDriverFrameTarget,
                };
                let current = self.page.frame_scope().cloned();
                let command = match command.target {
                    oryn_core::ast::FrameTarget::Main => FrameCommand::Select {
                        current,
                        target: WebDriverFrameTarget::Top,
                    },
                    oryn_core::ast::FrameTarget::Parent => FrameCommand::Select {
                        current,
                        target: WebDriverFrameTarget::Parent,
                    },
                    oryn_core::ast::FrameTarget::Target(target) => {
                        let reference = self.resolve_any_target(&target).await?;
                        FrameCommand::SelectSemantic { current, reference }
                    }
                };
                let response = self.page.native_frame_command(command).await?;
                let FrameResponse::Selected(selected) = response else {
                    return Err(NativeOilError::PageCommand(
                        "invalid native frame selection".into(),
                    ));
                };
                self.page = self
                    .page
                    .with_frame_scope(if selected.parent_realm.is_none() {
                        None
                    } else {
                        Some(selected.clone())
                    });
                self.last_observation = None;
                self.delta_cursor = Revision(0);
                Ok(NativeOilOutput::Structured {
                    value: serde_json::to_value(selected)
                        .map_err(|error| NativeOilError::PageCommand(error.to_string()))?,
                })
            }

            Command::Goto(command) => {
                self.prepare_root_navigation();
                let navigation = self.page.goto(command.url).await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Back => {
                self.prepare_root_navigation();
                let navigation = self.page.back().await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Forward => {
                self.prepare_root_navigation();
                let navigation = self.page.forward().await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Refresh(_) => {
                self.prepare_root_navigation();
                let navigation = self.page.refresh().await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Observe(command) if command.near.is_some() => {
                let query = command.near.as_deref().unwrap_or_default().to_lowercase();
                let mut observation = self.page.observe_profile(ProjectionProfile::Full).await?;
                let matching = observation
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| {
                        node.name.to_lowercase().contains(&query)
                            || node.role.to_lowercase().contains(&query)
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                observation.nodes = observation
                    .nodes
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| matching.iter().any(|anchor| index.abs_diff(*anchor) <= 2))
                    .map(|(_, node)| node)
                    .collect();
                observation.profile = ProjectionProfile::Scoped;
                self.delta_cursor = observation.revision;
                self.last_observation = Some(observation.clone());
                Ok(NativeOilOutput::Observation { observation })
            }
            Command::Observe(command) if command.diff || command.since.is_some() => {
                let from = command.since.map(Revision).unwrap_or(self.delta_cursor);
                let delta = self.page.observe_delta(from).await?;
                self.merge_observation_delta(&delta);
                Ok(NativeOilOutput::Delta { delta })
            }
            Command::Observe(command) => {
                let profile = if command.full {
                    ProjectionProfile::Full
                } else {
                    ProjectionProfile::Compact
                };
                if command.positions || command.viewport || command.hidden {
                    let observation = self
                        .page
                        .observe_with(oryn_common::v4::ObservationOptions {
                            profile,
                            include_geometry: command.positions || command.viewport,
                            viewport_only: command.viewport,
                            include_hidden: command.hidden,
                        })
                        .await?;
                    self.delta_cursor = observation.revision;
                    self.cache_v4_observation(&observation);
                    return Ok(NativeOilOutput::GeometryObservation { observation });
                }
                let observation = self.page.observe_profile(profile).await?;
                self.delta_cursor = observation.revision;
                self.last_observation = Some(observation.clone());
                Ok(NativeOilOutput::Observation { observation })
            }
            Command::Title => {
                let observation = self.observe_if_needed().await?;
                Ok(NativeOilOutput::Text {
                    value: observation.page.title,
                })
            }
            Command::Url => {
                let observation = self.observe_if_needed().await?;
                Ok(NativeOilOutput::Text {
                    value: observation.page.url,
                })
            }
            Command::Html(command) => {
                let value = self
                    .page
                    .extract(ContentKind::Html, command.selector)
                    .await?;
                Ok(NativeOilOutput::Structured { value })
            }
            Command::Text(command) => {
                if let Some(target) = command.target {
                    let reference = self.resolve_any_target(&target).await?;
                    let observation = self.page.observe_profile(ProjectionProfile::Full).await?;
                    let value = observation
                        .nodes
                        .iter()
                        .find(|node| node.semantic_ref == reference)
                        .map(|node| node.name.clone())
                        .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))?;
                    self.delta_cursor = observation.revision;
                    self.last_observation = Some(observation);
                    Ok(NativeOilOutput::Text { value })
                } else {
                    let value = self
                        .page
                        .extract(ContentKind::Text, command.selector)
                        .await?;
                    Ok(NativeOilOutput::Structured { value })
                }
            }
            Command::Extract(command) => {
                let kind = match command.what {
                    ExtractWhat::Links => ContentKind::Links,
                    ExtractWhat::Images => ContentKind::Images,
                    ExtractWhat::Tables => ContentKind::Tables,
                    ExtractWhat::Text => ContentKind::Text,
                    ExtractWhat::Css(selector) => {
                        let value = self.page.extract(ContentKind::Text, Some(selector)).await?;
                        return Ok(NativeOilOutput::Structured { value });
                    }
                    ExtractWhat::Meta => ContentKind::Meta,
                };
                let value = self.page.extract(kind, command.selector).await?;
                Ok(NativeOilOutput::Structured { value })
            }
            Command::Box(command) => {
                let target = self.resolve_any_target(&command.target).await?;
                let box_model = self.page.box_model(target).await?;
                Ok(NativeOilOutput::Box { box_model })
            }
            Command::Click(command) => {
                self.run_action_with_force(
                    command.target,
                    SemanticAction::Click,
                    None,
                    command.force,
                )
                .await
            }
            Command::Type(command) => {
                if let Some(label) = command.secret {
                    let target = self
                        .resolve_for_action(command.target, SemanticAction::Type, false)
                        .await?;
                    let result = self.page.fill_secret_label(target, &label, None).await?;
                    let result = self.refresh_after_action(result).await;
                    return Ok(NativeOilOutput::Action { result });
                }
                self.run_action(command.target, SemanticAction::Type, Some(command.text))
                    .await
            }
            Command::Clear(command) => {
                self.run_action(command.target, SemanticAction::Clear, None)
                    .await
            }
            Command::Check(command) => {
                self.run_action(command.target, SemanticAction::Check, None)
                    .await
            }
            Command::Uncheck(command) => {
                self.run_action(command.target, SemanticAction::Uncheck, None)
                    .await
            }
            Command::Select(command) => {
                self.run_action(command.target, SemanticAction::Select, Some(command.value))
                    .await
            }
            Command::Hover(command) => {
                self.run_action(command.target, SemanticAction::Hover, None)
                    .await
            }
            Command::Focus(command) => {
                self.run_action(command.target, SemanticAction::Focus, None)
                    .await
            }
            Command::Submit(command) if command.target.is_some() => {
                self.run_action(
                    command.target.expect("checked target"),
                    SemanticAction::Submit,
                    None,
                )
                .await
            }
            Command::Submit(_) => {
                let target = self.page.resolve_focused_form().await?;
                let result = self
                    .page
                    .execute(NativeAction {
                        target,
                        action: SemanticAction::Submit,
                        value: None,
                    })
                    .await?;
                Ok(NativeOilOutput::Action {
                    result: self.refresh_after_action(result).await,
                })
            }
            Command::Wait(command) => {
                let timeout = command
                    .timeout
                    .as_deref()
                    .map(parse_oil_wait_timeout)
                    .transpose()?;
                let predicate = match command.condition {
                    WaitCondition::Load | WaitCondition::Ready => {
                        WaitPredicate::Lifecycle(LifecycleState::Load)
                    }
                    WaitCondition::Idle => WaitPredicate::Lifecycle(LifecycleState::NetworkIdle),
                    WaitCondition::Navigation => WaitPredicate::Lifecycle(LifecycleState::Commit),
                    WaitCondition::Url(value) => WaitPredicate::UrlContains(value),
                    WaitCondition::Exists(selector) => WaitPredicate::SelectorExists(selector),
                    WaitCondition::Gone(selector) => WaitPredicate::SelectorGone(selector),
                    WaitCondition::Visible(target) => {
                        self.assert_target_visibility(&target, true, timeout)
                            .await?;
                        return Ok(NativeOilOutput::Text {
                            value: "visible".into(),
                        });
                    }
                    WaitCondition::Hidden(target) => {
                        self.assert_target_visibility(&target, false, timeout)
                            .await?;
                        return Ok(NativeOilOutput::Text {
                            value: "hidden".into(),
                        });
                    }
                    WaitCondition::Items { selector, count } => {
                        if !count.is_finite()
                            || count < 0.0
                            || count.fract() != 0.0
                            || count > 9_007_199_254_740_991.0
                        {
                            return Err(NativeOilError::Parse(
                                "item wait count must be a nonnegative exact integer".into(),
                            ));
                        }
                        WaitPredicate::SelectorCount {
                            selector,
                            minimum: count as u64,
                        }
                    }
                    WaitCondition::Until(expression) => WaitPredicate::JavaScript(expression),
                };
                self.wait_for_page_predicate(predicate, timeout).await?;
                Ok(NativeOilOutput::Text {
                    value: "satisfied".into(),
                })
            }
            Command::Scroll(command) => {
                let scroll = if let Some(target) = command.target {
                    let target = self.resolve_any_target(&target).await?;
                    self.page.scroll_into_view(target).await?
                } else {
                    let amount = command
                        .amount
                        .unwrap_or(if command.page { 1.0 } else { 400.0 });
                    let amount = if command.page {
                        amount * self.viewport.height
                    } else {
                        amount
                    };
                    let (x, y) = match command.direction.as_deref().unwrap_or("down") {
                        "up" => (0.0, -amount),
                        "left" => (-amount, 0.0),
                        "right" => (amount, 0.0),
                        _ => (0.0, amount),
                    };
                    self.page
                        .scroll(ScrollRequest {
                            x,
                            y,
                            relative: true,
                        })
                        .await?
                };
                Ok(NativeOilOutput::Scroll { scroll })
            }
            Command::Viewport(command) => {
                let viewport = self
                    .page
                    .set_viewport(oryn_common::v4::ViewportState {
                        width: command.width,
                        height: command.height,
                        device_pixel_ratio: 1.0,
                    })
                    .await?;
                self.viewport = viewport;
                Ok(NativeOilOutput::Viewport { viewport })
            }
            Command::Media(command) => {
                let current = self.media;
                let media = match (command.feature.as_deref(), command.value.as_deref()) {
                    (Some("color-scheme"), Some("dark")) => oryn_common::v4::MediaState {
                        color_scheme: oryn_common::v4::PreferredColorScheme::Dark,
                        ..current
                    },
                    (Some("color-scheme"), Some("light")) => oryn_common::v4::MediaState {
                        color_scheme: oryn_common::v4::PreferredColorScheme::Light,
                        ..current
                    },
                    (Some("reduced-motion"), Some("reduce")) => oryn_common::v4::MediaState {
                        reduced_motion: oryn_common::v4::ReducedMotion::Reduce,
                        ..current
                    },
                    (Some("reduced-motion"), Some("no-preference")) => {
                        oryn_common::v4::MediaState {
                            reduced_motion: oryn_common::v4::ReducedMotion::NoPreference,
                            ..current
                        }
                    }
                    (None, None) => oryn_common::v4::MediaState::default(),
                    _ => {
                        return Ok(NativeOilOutput::Unsupported {
                            diagnostic: CapabilityDiagnostic {
                                capability: "oil.media.value".into(),
                                support: SupportLevel::Unsupported,
                                alternatives: vec![ExecutionDomain::Chromium],
                                handoff_lossy: false,
                                detail: Some(
                                    "supported values are light, dark, reduce, and no-preference"
                                        .into(),
                                ),
                            },
                        });
                    }
                };
                let media = self.page.set_media(media).await?;
                self.media = media;
                Ok(NativeOilOutput::Media { media })
            }
            Command::Dismiss(command) => {
                let (target, container) = self.resolve_dismiss_target(&command.target).await?;
                // Recheck the original containment after asynchronous style reads;
                // a replacement container or reparented button is never a retry.
                let scoped = oryn_common::agent::Target::Inside {
                    target: Box::new(oryn_common::agent::Target::Reference { reference: target }),
                    container: Box::new(oryn_common::agent::Target::Reference {
                        reference: container,
                    }),
                };
                if self
                    .page
                    .resolve_semantic(scoped, SemanticAction::Click)
                    .await?
                    .selected
                    != Some(target)
                {
                    return Err(RuntimeError::StaleSemanticRef.into());
                }
                let result = self
                    .page
                    .execute(NativeAction {
                        target,
                        action: SemanticAction::Click,
                        value: None,
                    })
                    .await?;
                let result = self.refresh_after_action(result).await;
                Ok(NativeOilOutput::Action { result })
            }
            Command::Trace(_) => Ok(NativeOilOutput::Trace {
                events: self.page.trace().await?,
            }),
            Command::Exit => Ok(NativeOilOutput::Exit),
            other => Ok(NativeOilOutput::Unsupported {
                diagnostic: CapabilityDiagnostic {
                    capability: command_name(&other),
                    support: SupportLevel::Unsupported,
                    alternatives: vec![
                        ExecutionDomain::Chromium,
                        ExecutionDomain::Webkit,
                        ExecutionDomain::UserBrowser,
                    ],
                    handoff_lossy: true,
                    detail: Some("not implemented by the current native proof slice".into()),
                },
            }),
        }
    }

    async fn observe_if_needed(&mut self) -> Result<Observation, NativeOilError> {
        // Metadata is live even when a previous observation established aliases.
        let observation = self.page.observe().await?;
        Ok(observation)
    }

    async fn wait_for_page_predicate(
        &mut self,
        predicate: WaitPredicate,
        timeout: Option<Duration>,
    ) -> Result<(), NativeOilError> {
        match timeout {
            Some(timeout) => self.page.wait_for_timeout(predicate, Some(timeout)).await?,
            None => self.page.wait_for(predicate).await?,
        }
        Ok(())
    }

    async fn assert_target_visibility(
        &mut self,
        target: &Target,
        expected_visible: bool,
        timeout: Option<Duration>,
    ) -> Result<(), NativeOilError> {
        // Text/role waits must tolerate genuinely asynchronous resource and
        // Fetch completions. Poll the semantic projection while advancing one
        // scheduler checkpoint; a fixed two-pass drain races loopback and HTTP/2.
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout.unwrap_or(Duration::from_secs(30)))
            .ok_or_else(|| NativeOilError::Parse("wait timeout is out of range".into()))?;
        validate_target_aliases(target, self.last_observation.as_ref())?;
        let bound =
            bind_observed_aliases(oil_target_to_agent(target), self.last_observation.as_ref())?;
        loop {
            let reference = match self.page.resolve_target(bound.clone(), None).await {
                Ok(result) => result.selected,
                Err(RuntimeError::StaleSemanticRef) if !target_has_alias(target) => None,
                Err(error) => return Err(error.into()),
            };
            let observation = self.page.observe_profile(ProjectionProfile::Full).await?;
            let found = observation
                .nodes
                .iter()
                .find(|node| Some(node.semantic_ref) == reference);
            let visible = found.is_some_and(|node| {
                node.states.contains(&oryn_common::v3::NodeState::Visible)
                    && !node.states.contains(&oryn_common::v3::NodeState::Hidden)
            });
            self.last_observation = Some(observation);
            if visible == expected_visible {
                return Ok(());
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(10))).await;
        }
        Err(NativeOilError::TargetNotFound(target.clone()))
    }

    async fn run_action(
        &mut self,
        target: Target,
        action: SemanticAction,
        value: Option<String>,
    ) -> Result<NativeOilOutput, NativeOilError> {
        self.run_action_with_force(target, action, value, false)
            .await
    }

    async fn run_action_with_force(
        &mut self,
        target: Target,
        action: SemanticAction,
        value: Option<String>,
        force: bool,
    ) -> Result<NativeOilOutput, NativeOilError> {
        let semantic_ref = self
            .resolve_for_action(target.clone(), action, force)
            .await?;
        let execute = |semantic_ref, value: Option<String>| NativeAction {
            target: semantic_ref,
            action,
            value,
        };
        let first = if force {
            self.page
                .execute_forced(execute(semantic_ref, value.clone()))
                .await
        } else {
            self.page
                .execute(execute(semantic_ref, value.clone()))
                .await
        };
        // A page-authored handler may replace a list between semantic
        // resolution and dispatch. Text, role, and selector targets are
        // declarative and can be resolved once more against the new revision;
        // observation aliases intentionally cannot, because carrying an alias
        // across replacement would falsely preserve semantic identity.
        let result = match first {
            Err(RuntimeError::StaleSemanticRef) if !target_has_alias(&target) => {
                let recovered = self.resolve_for_action(target, action, force).await?;
                if force {
                    self.page.execute_forced(execute(recovered, value)).await?
                } else {
                    self.page.execute(execute(recovered, value)).await?
                }
            }
            result => result?,
        };
        let result = self.refresh_after_action(result).await;
        Ok(NativeOilOutput::Action { result })
    }

    /// An action receipt is authoritative even when its follow-up observation
    /// fails (for example, when the action closes the page). Never invite an
    /// action replay by replacing that receipt with an observation error.
    async fn refresh_after_action(&mut self, mut result: ActionResult) -> ActionResult {
        let cursor = self.delta_cursor;
        let document = self.last_observation.as_ref().map(|observation| {
            (
                observation.page.page_id,
                observation.page.document_generation,
            )
        });
        self.last_observation = None;
        self.delta_cursor = Revision(0);
        let selection = if result
            .effects
            .iter()
            .any(|effect| matches!(effect, oryn_common::v3::Effect::Navigation { .. }))
            && let Some(current) = self.page.frame_scope().cloned()
        {
            use crate::host::webdriver_script::{FrameCommand, FrameResponse};
            // The unscoped command waits for the existing navigation barrier.
            // The actor resolves only this action's exact installed successor;
            // the URL effect is a trigger, never selection authority.
            match self
                .page
                .with_frame_scope(None)
                .native_frame_command(FrameCommand::ResolveActionSelection {
                    current,
                    action_id: result.action_id,
                })
                .await
            {
                Ok(FrameResponse::Selected(destination)) => {
                    self.page = self.page.with_frame_scope(Some(destination));
                    Ok(())
                }
                Ok(_) => Err(RuntimeError::ProtocolViolation(
                    "invalid action frame selection response".into(),
                )),
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        };
        let observation = match selection {
            Ok(()) => self.page.observe().await,
            Err(error) => Err(error),
        };
        match observation {
            Ok(observation) => {
                // Refresh aliases without consuming changes the caller has not
                // observed. A document replacement starts a new delta baseline.
                self.delta_cursor = if document.is_none_or(|document| document == (observation.page.page_id, observation.page.document_generation)) { cursor } else { observation.revision };
                self.last_observation = Some(observation);
            }
            Err(error) => result.diagnostics.push(format!("Action completed; follow-up observation unavailable: {error}. Cached aliases were cleared.")),
        }
        result
    }

    async fn resolve_for_action(
        &mut self,
        target: Target,
        action: SemanticAction,
        force: bool,
    ) -> Result<SemanticRef, NativeOilError> {
        validate_target_aliases(&target, self.last_observation.as_ref())?;
        Ok(if force {
            self.resolve_any_target(&target).await?
        } else {
            self.page
                .resolve_semantic(
                    bind_observed_aliases(
                        oil_target_to_agent(&target),
                        self.last_observation.as_ref(),
                    )?,
                    action,
                )
                .await?
                .selected
                .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))?
        })
    }

    async fn resolve_dismiss_target(
        &mut self,
        name: &str,
    ) -> Result<(SemanticRef, SemanticRef), NativeOilError> {
        let requested = Target {
            atomic: TargetAtomic::Text(name.into()),
            relation: None,
        };
        let generic = matches!(
            name.trim().to_ascii_lowercase().as_str(),
            "popup" | "modal" | "dialog"
        );
        let named = if generic {
            None
        } else {
            Some(
                self.page
                    .resolve_target(oryn_common::agent::Target::Text { text: name.into() }, None)
                    .await?
                    .selected
                    .ok_or_else(|| NativeOilError::TargetNotFound(requested.clone()))?,
            )
        };
        let observation = self
            .page
            .observe_with(oryn_common::v4::ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: false,
            })
            .await?;
        self.cache_v4_observation(&observation);
        let nodes = observation
            .nodes
            .iter()
            .map(|node| (node.semantic.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        if named.is_some_and(|reference| !nodes.contains_key(&reference)) {
            return Err(RuntimeError::StaleSemanticRef.into());
        }
        let candidates = observation
            .nodes
            .iter()
            .filter(|node| {
                dismiss_control_visible(node) && dismiss_name_rank(&node.semantic.name).is_some()
            })
            .collect::<Vec<_>>();
        let mut containers = BTreeMap::new();
        let mut inspected = std::collections::BTreeSet::new();
        if let Some(reference) = named {
            containers.insert(reference, (0i64, 0u32));
        } else {
            // Only inspect ancestors of actual dismissal controls. Geometry and
            // computed CSS come from the native layout revision, not class-name
            // guesses or authored getComputedStyle/window helpers.
            for candidate in &candidates {
                let mut parent = candidate.semantic.parent;
                let mut seen = std::collections::BTreeSet::new();
                while let Some(reference) = parent {
                    if !seen.insert(reference) {
                        return Err(RuntimeError::StaleSemanticRef.into());
                    }
                    let Some(node) = nodes.get(&reference) else {
                        break;
                    };
                    parent = node.semantic.parent;
                    if !inspected.insert(reference)
                        || matches!(node.semantic.role.as_str(), "document" | "RootWebArea")
                    {
                        continue;
                    }
                    let Some(bounds) = node
                        .geometry
                        .as_ref()
                        .and_then(|geometry| geometry.box_model.as_ref())
                        .map(|model| model.border_box)
                    else {
                        continue;
                    };
                    let style = self.page.computed_style(reference).await?;
                    if style.style_revision != observation.style_revision {
                        return Err(RuntimeError::StaleSemanticRef.into());
                    }
                    let position = style
                        .properties
                        .get("position")
                        .map(String::as_str)
                        .unwrap_or("static");
                    let z = style
                        .properties
                        .get("z-index")
                        .and_then(|value| value.parse::<i64>().ok())
                        .unwrap_or(0);
                    let viewport_area = observation.viewport.width * observation.viewport.height;
                    let width = (bounds.x + bounds.width).min(observation.viewport.width)
                        - bounds.x.max(0.0);
                    let height = (bounds.y + bounds.height).min(observation.viewport.height)
                        - bounds.y.max(0.0);
                    let coverage = if viewport_area > 0.0 {
                        width.max(0.0) * height.max(0.0) / viewport_area
                    } else {
                        0.0
                    };
                    let semantic = matches!(node.semantic.role.as_str(), "dialog" | "alertdialog");
                    if semantic
                        || (matches!(position, "fixed" | "absolute")
                            && (z >= 100 || coverage >= 0.3))
                    {
                        containers.insert(reference, (z, node.semantic.document_order));
                    }
                }
            }
        }
        let mut selected = None;
        for candidate in candidates {
            let rank = dismiss_name_rank(&candidate.semantic.name).unwrap();
            for (&container, &(z, order)) in &containers {
                if !dismiss_inside(&nodes, candidate.semantic.semantic_ref, container) {
                    continue;
                }
                if rank == 0
                    && !dismiss_icon_at_corner(&nodes, candidate.semantic.semantic_ref, container)
                {
                    continue;
                }
                let score = (
                    z,
                    order,
                    rank,
                    std::cmp::Reverse(candidate.semantic.document_order),
                );
                if selected
                    .as_ref()
                    .is_none_or(|(previous, _, _)| score > *previous)
                {
                    selected = Some((score, candidate.semantic.semantic_ref, container));
                }
            }
        }
        selected
            .map(|(_, target, container)| (target, container))
            .ok_or(NativeOilError::TargetNotFound(requested))
    }

    fn cache_v4_observation(&mut self, observation: &oryn_common::v4::Observation) {
        self.delta_cursor = observation.revision;
        self.last_observation = Some(observation.to_v3_compat());
    }

    async fn resolve_any_target(&mut self, target: &Target) -> Result<SemanticRef, NativeOilError> {
        validate_target_aliases(target, self.last_observation.as_ref())?;
        let bound =
            bind_observed_aliases(oil_target_to_agent(target), self.last_observation.as_ref())?;
        let reference = self
            .page
            .resolve_target(bound, None)
            .await?
            .selected
            .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))?;
        let observation = self
            .page
            .observe_with(oryn_common::v4::ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await?;
        if !observation
            .nodes
            .iter()
            .any(|node| node.semantic.semantic_ref == reference)
        {
            return Err(RuntimeError::StaleSemanticRef.into());
        }
        self.cache_v4_observation(&observation);
        Ok(reference)
    }
}

fn dismiss_name_rank(name: &str) -> Option<u8> {
    let name = name.trim().to_ascii_lowercase();
    if matches!(name.as_str(), "close" | "dismiss") {
        Some(3)
    } else if ["close", "dismiss", "cancel"].iter().any(|word| {
        name.split(|character: char| !character.is_alphanumeric())
            .any(|token| token == *word)
    }) {
        Some(2)
    } else if matches!(name.as_str(), "×" | "x") {
        Some(1)
    } else if name.is_empty() {
        Some(0)
    } else {
        None
    }
}
fn dismiss_control_visible(node: &oryn_common::v4::SemanticNodeView) -> bool {
    node.semantic.actions.contains(&SemanticAction::Click)
        && node
            .semantic
            .states
            .contains(&oryn_common::v3::NodeState::Visible)
        && !node
            .semantic
            .states
            .contains(&oryn_common::v3::NodeState::Hidden)
        && !node
            .semantic
            .states
            .contains(&oryn_common::v3::NodeState::Disabled)
        && node
            .geometry
            .as_ref()
            .is_some_and(|geometry| geometry.interactable)
}
fn dismiss_inside(
    nodes: &BTreeMap<SemanticRef, &oryn_common::v4::SemanticNodeView>,
    child: SemanticRef,
    container: SemanticRef,
) -> bool {
    let mut parent = nodes.get(&child).and_then(|node| node.semantic.parent);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(reference) = parent {
        if reference == container {
            return true;
        }
        if !seen.insert(reference) {
            return false;
        }
        parent = nodes.get(&reference).and_then(|node| node.semantic.parent);
    }
    false
}
fn dismiss_icon_at_corner(
    nodes: &BTreeMap<SemanticRef, &oryn_common::v4::SemanticNodeView>,
    child: SemanticRef,
    container: SemanticRef,
) -> bool {
    let Some(node) = nodes.get(&child) else {
        return false;
    };
    let Some(bounds) = node
        .geometry
        .as_ref()
        .and_then(|geometry| geometry.box_model.as_ref())
        .map(|model| model.border_box)
    else {
        return false;
    };
    if bounds.width <= 0.0 || bounds.height <= 0.0 || bounds.width > 64.0 || bounds.height > 64.0 {
        return false;
    }
    let mut parent = node.semantic.parent;
    let mut seen = std::collections::BTreeSet::new();
    while let Some(reference) = parent {
        if !seen.insert(reference) {
            return false;
        }
        let Some(node) = nodes.get(&reference) else {
            return false;
        };
        if let Some(area) = node
            .geometry
            .as_ref()
            .and_then(|geometry| geometry.box_model.as_ref())
            .map(|model| model.border_box)
        {
            let top = bounds.y - area.y;
            let right = area.x + area.width - bounds.x - bounds.width;
            if area.width > bounds.width * 2.0
                && area.height > bounds.height * 2.0
                && (0.0..=64.0).contains(&top)
                && (0.0..=64.0).contains(&right)
            {
                return true;
            }
        }
        if reference == container {
            break;
        }
        parent = node.semantic.parent;
    }
    false
}

fn parse_oil_wait_timeout(value: &str) -> Result<Duration, NativeOilError> {
    let invalid =
        || NativeOilError::Parse("wait timeout must be an integer duration in ms, s, or m".into());
    let (number, multiplier) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1u64)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1_000)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60_000)
    } else {
        return Err(invalid());
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let milliseconds = number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(multiplier))
        .ok_or_else(invalid)?;
    Ok(Duration::from_millis(milliseconds))
}

fn observed_alias(
    observation: Option<&Observation>,
    alias: usize,
) -> Result<SemanticRef, NativeOilError> {
    let observation = observation.ok_or(NativeOilError::NoObservation)?;
    observation
        .nodes
        .iter()
        .find(|node| node.alias as usize == alias)
        .map(|node| node.semantic_ref)
        .ok_or_else(|| RuntimeError::StaleSemanticRef.into())
}
fn validate_target_aliases(
    target: &Target,
    observation: Option<&Observation>,
) -> Result<(), NativeOilError> {
    if let TargetAtomic::Id(alias) = &target.atomic {
        observed_alias(observation, *alias)?;
    }
    if let Some(relation) = &target.relation {
        validate_target_aliases(&relation.target, observation)?;
    }
    Ok(())
}
fn target_has_alias(target: &Target) -> bool {
    matches!(&target.atomic, TargetAtomic::Id(_))
        || target
            .relation
            .as_ref()
            .is_some_and(|relation| target_has_alias(&relation.target))
}
fn bind_observed_aliases(
    target: oryn_common::agent::Target,
    observation: Option<&Observation>,
) -> Result<oryn_common::agent::Target, NativeOilError> {
    use oryn_common::agent::Target as T;
    let bind = |target: Box<T>| bind_observed_aliases(*target, observation).map(Box::new);
    Ok(match target {
        T::Alias { alias } => T::Reference {
            reference: observed_alias(observation, alias as usize)?,
        },
        T::Near { target, anchor } => T::Near {
            target: bind(target)?,
            anchor: bind(anchor)?,
        },
        T::Inside { target, container } => T::Inside {
            target: bind(target)?,
            container: bind(container)?,
        },
        T::After { target, anchor } => T::After {
            target: bind(target)?,
            anchor: bind(anchor)?,
        },
        T::Before { target, anchor } => T::Before {
            target: bind(target)?,
            anchor: bind(anchor)?,
        },
        T::Contains { target, content } => T::Contains {
            target: bind(target)?,
            content: bind(content)?,
        },
        target => target,
    })
}

fn oil_target_to_agent(target: &Target) -> oryn_common::agent::Target {
    use oryn_common::agent::Target as AgentTarget;
    let atomic = match &target.atomic {
        TargetAtomic::Id(alias) => AgentTarget::Alias {
            alias: *alias as u32,
        },
        TargetAtomic::Text(text) => AgentTarget::Text { text: text.clone() },
        TargetAtomic::Role(role) => AgentTarget::Role { role: role.clone() },
        TargetAtomic::Selector { kind, value } if kind.eq_ignore_ascii_case("xpath") => {
            AgentTarget::XPath {
                expression: value.clone(),
            }
        }
        TargetAtomic::Selector { value, .. } => AgentTarget::Css {
            selector: value.clone(),
        },
    };
    let Some(relation) = &target.relation else {
        return atomic;
    };
    let related = Box::new(oil_target_to_agent(&relation.target));
    let atomic = Box::new(atomic);
    match relation.kind {
        RelationKind::Near => AgentTarget::Near {
            target: atomic,
            anchor: related,
        },
        RelationKind::Inside => AgentTarget::Inside {
            target: atomic,
            container: related,
        },
        RelationKind::After => AgentTarget::After {
            target: atomic,
            anchor: related,
        },
        RelationKind::Before => AgentTarget::Before {
            target: atomic,
            anchor: related,
        },
        RelationKind::Contains => AgentTarget::Contains {
            target: atomic,
            content: related,
        },
    }
}

fn command_name(command: &Command) -> String {
    format!(
        "oil.{}",
        format!("{command:?}")
            .split(['(', ' '])
            .next()
            .unwrap_or("unknown")
            .to_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Browser;

    #[tokio::test(flavor = "current_thread")]
    async fn uncertain_action_outcomes_clear_aliases_and_preserve_error_identity() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://outcomes.test/", "<button>Save</button>")
            .await
            .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        let observation = page.observe().await.unwrap();
        let alias = observation
            .nodes
            .iter()
            .find(|node| node.name == "Save")
            .unwrap()
            .alias as usize;
        session.last_observation = Some(observation.clone());
        session.delta_cursor = observation.revision;
        assert!(
            session
                .accept_native_outcome::<()>(Err(RuntimeError::Disabled))
                .is_err()
        );
        assert!(observed_alias(session.last_observation.as_ref(), alias).is_ok());
        let error = RuntimeError::ActionExecutionFailed {
            action_id: oryn_common::v3::ActionId(9),
            cause: Box::new(RuntimeError::Semantic("publication interrupted".into())),
        };
        let result = session.accept_native_outcome::<()>(Err(error.clone()));
        assert!(matches!(result,Err(NativeOilError::Runtime(actual)) if actual==error));
        assert!(session.last_observation.is_none());
        assert_eq!(session.delta_cursor, Revision(0));
        assert!(matches!(
            observed_alias(session.last_observation.as_ref(), alias),
            Err(NativeOilError::NoObservation)
        ));
        page.close().await.unwrap();
    }

    #[test]
    fn oil_wait_duration_units_and_overflow_are_explicit() {
        for (input, expected) in [("0ms", 0), ("12ms", 12), ("2s", 2_000), ("3m", 180_000)] {
            assert_eq!(
                parse_oil_wait_timeout(input).unwrap(),
                Duration::from_millis(expected)
            );
        }
        for input in [
            "",
            "ms",
            "-1s",
            "1.5s",
            "1h",
            "18446744073709551615m",
            "18446744073709551616ms",
        ] {
            assert!(parse_oil_wait_timeout(input).is_err(), "{input}");
        }
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn oil_wait_honors_zero_deadline_without_calling_author_drain() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://wait.test/", "<button id=ready>Ready</button>")
            .await
            .unwrap();
        page.evaluate("globalThis.drainCalls=0;globalThis.__oryn_drain=()=>{drainCalls++;throw Error('author helper called');};true").await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session
            .execute("wait exists \"#ready\" --timeout 0ms")
            .await
            .unwrap();
        for command in [
            "wait exists \"#missing\" --timeout 0ms",
            "wait visible \"Missing\" --timeout 0ms",
        ] {
            let result = tokio::time::timeout(Duration::from_secs(2), session.execute(command))
                .await
                .expect("OIL must not retry an expired native wait");
            assert!(result.is_err());
        }
        assert_eq!(page.evaluate("String(drainCalls)").await.unwrap(), "0");
        page.close().await.unwrap();
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn oil_item_waits_use_live_scope_and_native_deadlines() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://wait.test/",
            "<main><span class=item>one</span><span class=item>two</span></main>",
        )
        .await
        .unwrap();
        page.evaluate(
            "globalThis.detached=document.createElement('div');detached.className='detached';true",
        )
        .await
        .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        for command in [
            "wait items \".item\" 1 --timeout 0ms",
            "wait items \".item\" 2 --timeout 0ms",
            "wait items \".missing\" 0 --timeout 0ms",
            "wait gone \".detached\" --timeout 0ms",
        ] {
            session.execute(command).await.unwrap();
        }
        for command in [
            "wait items \".item\" 3 --timeout 0ms",
            "wait items \".detached\" 1 --timeout 0ms",
            "wait items \"[\" 0 --timeout 0ms",
            "wait gone \"[\" --timeout 0ms",
            "wait items \".item\" 1.5 --timeout 0ms",
        ] {
            assert!(session.execute(command).await.is_err(), "{command}");
        }
        page.evaluate("setTimeout(()=>{const item=document.createElement('span');item.className='item';document.querySelector('main').append(item);},20);true").await.unwrap();
        session
            .execute("wait items \".item\" 3 --timeout 2s")
            .await
            .unwrap();
        page.close().await.unwrap();
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn oil_expression_wait_uses_native_truthiness_errors_and_one_initial_attempt() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://wait.test/", "<body>Wait</body>")
            .await
            .unwrap();
        page.evaluate("globalThis.checks=0;globalThis.Boolean=()=>{throw Error('author Boolean called');};true").await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session
            .execute("wait until \"({ready:true})\" --timeout 0ms")
            .await
            .unwrap();
        assert!(
            session
                .execute("wait until \"(++checks,false)\" --timeout 0ms")
                .await
                .is_err()
        );
        assert_eq!(page.evaluate("String(checks)").await.unwrap(), "1");
        assert!(
            session
                .execute("wait until \"(()=>{throw Error('predicate failure')})()\" --timeout 1s")
                .await
                .is_err()
        );
        assert!(
            session
                .execute("wait until \"(\" --timeout 1s")
                .await
                .is_err()
        );
        page.evaluate("globalThis.ready=false;setTimeout(()=>ready=true,20);true")
            .await
            .unwrap();
        session
            .execute("wait until \"ready\" --timeout 2s")
            .await
            .unwrap();
        page.close().await.unwrap();
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn html_extraction_preserves_customized_builtin_creation_identity() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://custom.test/", "<main></main>")
            .await
            .unwrap();
        page.evaluate("globalThis.p=document.createElement('p',{is:'future-p'});p.className='foo';document.querySelector('main').append(p);true").await.unwrap();
        assert_eq!(
            page.extract(ContentKind::Html, Some("p".into()))
                .await
                .unwrap(),
            serde_json::json!("<p is=\"future-p\" class=\"foo\"></p>")
        );
        page.evaluate("p.setAttribute('is','authored');true")
            .await
            .unwrap();
        assert_eq!(
            page.extract(ContentKind::Html, Some("p".into()))
                .await
                .unwrap(),
            serde_json::json!("<p class=\"foo\" is=\"authored\"></p>")
        );
        page.evaluate("p.removeAttribute('is');true").await.unwrap();
        assert_eq!(
            page.extract(ContentKind::Html, Some("p".into()))
                .await
                .unwrap(),
            serde_json::json!("<p is=\"future-p\" class=\"foo\"></p>")
        );
        page.close().await.unwrap();
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn targetless_submit_uses_focused_controls_native_form_owner() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://forms.test/",r#"<form id=first><input id=inside><div tabindex=0 id=generic>Generic</div></form><form id=second></form><input id=external form=second><button type=button>Submit unrelated</button>"#).await.unwrap();
        page.evaluate("globalThis.submitted='';for(const form of document.querySelectorAll('form'))form.addEventListener('submit',event=>{event.preventDefault();submitted+=form.id+';';});true").await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        assert!(session.execute("submit").await.is_err());
        page.evaluate("document.querySelector('#external').focus();true")
            .await
            .unwrap();
        session.execute("submit").await.unwrap();
        assert_eq!(page.evaluate("submitted").await.unwrap(), "second;");
        page.evaluate("document.querySelector('#inside').focus();true")
            .await
            .unwrap();
        session.execute("submit").await.unwrap();
        assert_eq!(page.evaluate("submitted").await.unwrap(), "second;first;");
        page.evaluate("document.querySelector('#generic').focus();true")
            .await
            .unwrap();
        assert!(session.execute("submit").await.is_err());
        assert_eq!(page.evaluate("submitted").await.unwrap(), "second;first;");
        page.close().await.unwrap();
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_frame_batch_preserves_prefix_and_stops_after_failure() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://batch.test/",
            r#"<iframe id=child srcdoc="<input aria-label='Value'>"></iframe>"#,
        )
        .await
        .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session.execute(r##"frame css("#child")"##).await.unwrap();
        let target = session
            .page
            .resolve_selector("input", SemanticAction::Type)
            .await
            .unwrap();
        let mut stale = target;
        stale.document_generation += 1;
        let action = |target, value: &str| NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some(value.into()),
        };
        let error = session
            .page
            .execute_batch(vec![
                action(target, "first"),
                action(stale, "failed"),
                action(target, "suffix"),
            ])
            .await
            .unwrap_err();
        assert!(
            matches!(&error,RuntimeError::BatchExecutionFailed{completed,failed_index:1,..} if completed.len()==1)
        );
        assert_eq!(
            session
                .page
                .evaluate("document.querySelector('input').value")
                .await
                .unwrap(),
            "first"
        );
        page.close().await.unwrap();
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn native_identity_resolution_preserves_relations_selectors_and_action_checks() {
        use oryn_common::agent::Target as T;
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/",r#"<section data-scope="a,b"><span class=message>Plain text</span><button disabled>Disabled</button></section><section><span class=message>Other text</span></section>"#).await.unwrap();
        let mut oil = NativeOilSession::new(page.clone());
        let target = T::Inside {
            target: Box::new(T::Css {
                selector: "span.message".into(),
            }),
            container: Box::new(T::Css {
                selector: r#"section[data-scope="a,b"]"#.into(),
            }),
        };
        let reference = oil
            .resolve_typed_target(&target, SemanticAction::Click, true)
            .await
            .unwrap();
        assert!(
            page.execute(NativeAction {
                target: reference,
                action: SemanticAction::Click,
                value: None
            })
            .await
            .is_err()
        );
        let xpath = T::XPath {
            expression: "//section[@data-scope='a,b']/span".into(),
        };
        assert_eq!(
            page.resolve_target(xpath, None).await.unwrap().selected,
            Some(reference)
        );
        let nested = T::Inside {
            target: Box::new(T::Reference { reference }),
            container: Box::new(T::Css {
                selector: "section[data-scope]".into(),
            }),
        };
        assert_eq!(
            oil.resolve_typed_target(&nested, SemanticAction::Click, true)
                .await
                .unwrap(),
            reference
        );
        let extracted = page
            .extract(
                ContentKind::Text,
                Some(r#"section[data-scope="a,b"]"#.into()),
            )
            .await
            .unwrap();
        assert!(extracted.as_str().unwrap().contains("Plain text"));
        assert!(!extracted.as_str().unwrap().contains("Other text"));
        assert!(
            page.extract(ContentKind::Text, Some("[".into()))
                .await
                .is_err()
        );
        let overlap = page
            .extract(
                ContentKind::Text,
                Some("section[data-scope], section[data-scope] span".into()),
            )
            .await
            .unwrap();
        assert_eq!(overlap.as_str().unwrap().matches("Plain text").count(), 1);

        assert!(
            page.resolve_target(
                T::Css {
                    selector: "[".into()
                },
                None
            )
            .await
            .is_err()
        );
        assert!(
            page.resolve_target(
                T::XPath {
                    expression: "count(//span)".into()
                },
                None
            )
            .await
            .is_err()
        );
        let disabled = T::Text {
            text: "Disabled".into(),
        };
        let disabled_ref = page
            .resolve_target(disabled.clone(), None)
            .await
            .unwrap()
            .selected
            .unwrap();
        assert!(
            page.resolve_target(disabled, Some(SemanticAction::Click))
                .await
                .is_err()
        );
        assert!(
            page.execute(NativeAction {
                target: disabled_ref,
                action: SemanticAction::Click,
                value: None
            })
            .await
            .is_err()
        );
        page.close().await.unwrap();
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn oil_alias_does_not_rebind_after_document_replacement_or_metadata_read() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/old",
            "<title>Old</title><button>Old button</button>",
        )
        .await
        .unwrap();
        let mut oil = NativeOilSession::new(page.clone());
        let original = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        let old = original
            .nodes
            .iter()
            .find(|node| node.name == "Old button")
            .unwrap()
            .clone();
        oil.last_observation = Some(original);
        let target = Target {
            atomic: TargetAtomic::Id(old.alias as usize),
            relation: None,
        };
        page.load_html(
            "https://example.test/new",
            "<title>New</title><button>New button</button>",
        )
        .await
        .unwrap();
        assert_eq!(oil.observe_if_needed().await.unwrap().page.title, "New");
        assert_eq!(
            observed_alias(oil.last_observation.as_ref(), old.alias as usize).unwrap(),
            old.semantic_ref
        );
        assert!(
            oil.resolve_for_action(target.clone(), SemanticAction::Click, false)
                .await
                .is_err()
        );
        assert!(oil.resolve_any_target(&target).await.is_err());
        use oryn_common::agent::Target as T;
        let nested = T::Inside {
            target: Box::new(T::Role {
                role: "button".into(),
            }),
            container: Box::new(T::Alias { alias: old.alias }),
        };
        let T::Inside { container, .. } =
            bind_observed_aliases(nested, oil.last_observation.as_ref()).unwrap()
        else {
            panic!("inside")
        };
        assert!(matches!(*container,T::Reference{reference} if reference==old.semantic_ref));
        let declarative = Target {
            atomic: TargetAtomic::Text("New button".into()),
            relation: None,
        };
        assert!(
            oil.resolve_for_action(declarative, SemanticAction::Click, false)
                .await
                .is_ok()
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn observation_delta_cache_preserves_order_and_rejects_incoherent_alias_identity() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/delta",
            "<button>First</button><button>Second</button>",
        )
        .await
        .unwrap();
        let original = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        let first = original
            .nodes
            .iter()
            .find(|node| node.name == "First")
            .unwrap()
            .clone();
        let second = original
            .nodes
            .iter()
            .find(|node| node.name == "Second")
            .unwrap()
            .clone();
        let mut changed = first.clone();
        changed.name = "Changed".into();
        let delta = ObservationDelta {
            contract_version: oryn_common::v3::CONTRACT_VERSION,
            page_id: original.page.page_id,
            from_revision: original.revision,
            to_revision: Revision(original.revision.0 + 1),
            upserted: vec![changed],
            removed: Vec::new(),
        };
        let mut oil = NativeOilSession::new(page.clone());
        oil.last_observation = Some(original.clone());
        oil.merge_observation_delta(&delta);
        let cached = oil.last_observation.as_ref().unwrap();
        assert_eq!(cached.revision, delta.to_revision);
        assert!(
            cached
                .nodes
                .windows(2)
                .all(|pair| pair[0].document_order <= pair[1].document_order)
        );
        assert_eq!(
            observed_alias(Some(cached), first.alias as usize).unwrap(),
            first.semantic_ref
        );
        assert!(
            cached
                .nodes
                .iter()
                .any(|node| node.semantic_ref == first.semantic_ref && node.name == "Changed")
        );
        for case in 0..5 {
            let mut invalid = delta.clone();
            match case {
                0 => invalid.page_id = PageId(original.page.page_id.0 + 1),
                1 => invalid.from_revision = Revision(original.revision.0 + 1),
                2 => invalid.upserted[0].semantic_ref.document_generation += 1,
                3 => {
                    let mut collision = second.clone();
                    collision.alias = first.alias;
                    invalid.upserted.push(collision);
                }
                4 => invalid.upserted[0].semantic_ref.page_id = PageId(original.page.page_id.0 + 1),
                _ => unreachable!(),
            }
            oil.last_observation = Some(original.clone());
            oil.merge_observation_delta(&invalid);
            assert!(
                oil.last_observation.is_none(),
                "case {case} kept ambiguous or foreign aliases"
            );
        }
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_action_receipt_survives_failed_followup_and_clears_aliases() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/action", "<button>Save</button>")
            .await
            .unwrap();
        let observation = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        let target = observation
            .nodes
            .iter()
            .find(|node| node.name == "Save")
            .unwrap();
        let alias = target.alias as usize;
        let receipt = page
            .execute(NativeAction {
                target: target.semantic_ref,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .unwrap();
        let expected = serde_json::to_value(&receipt).unwrap();
        let mut oil = NativeOilSession::new(page.clone());
        oil.last_observation = Some(observation);
        oil.delta_cursor = receipt.revision_after;
        // The action has completed; only the subsequent observation now fails.
        page.close().await.unwrap();
        let receipt = oil.refresh_after_action(receipt).await;
        assert!(
            receipt
                .diagnostics
                .iter()
                .any(|detail| detail
                    .contains("Action completed; follow-up observation unavailable:"))
        );
        let mut actual = serde_json::to_value(&receipt).unwrap();
        let mut expected = expected;
        actual.as_object_mut().unwrap().remove("diagnostics");
        expected.as_object_mut().unwrap().remove("diagnostics");
        assert_eq!(
            actual, expected,
            "observation failure must preserve the exact action and effects"
        );
        assert!(matches!(
            observed_alias(oil.last_observation.as_ref(), alias),
            Err(NativeOilError::NoObservation)
        ));
        assert_eq!(oil.delta_cursor, Revision(0));
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn typed_context_intents_share_selected_frame_and_observed_aliases() {
        use oryn_common::agent::{Input, ObserveOptions, Target as AgentTarget};
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        page.load_html("https://example.test/root",r#"<input aria-label='Parent'><iframe id=child srcdoc="<input aria-label='Child'>"></iframe>"#).await.unwrap();
        let mut oil = NativeContextOilSession::new(context);
        oil.execute(r##"frame css("#child")"##).await.unwrap();
        for version in [3, 4, 5, 6] {
            let options = ObserveOptions {
                include_geometry: version == 4,
                include_structures: version == 5,
                semantic_contract_version: (version == 6).then_some(6),
                ..Default::default()
            };
            let result = oil.perform(Intent::Observe { options }).await.unwrap();
            assert!(matches!(
                (version, result),
                (3, IntentResult::Observation { .. })
                    | (4, IntentResult::GeometryObservation { .. })
                    | (5, IntentResult::SemanticObservation { .. })
                    | (6, IntentResult::SemanticObservationV6 { .. })
            ));
            let observation = oil.sessions[&id].last_observation.as_ref().unwrap();
            assert!(!observation.nodes.iter().any(|node| node.name == "Parent"));
            let alias = observation
                .nodes
                .iter()
                .find(|node| node.name == "Child")
                .unwrap()
                .alias;
            let value = format!("value-{version}");
            let result = oil
                .perform(Intent::Type {
                    target: AgentTarget::Alias { alias },
                    input: Input::Text {
                        value: value.clone(),
                    },
                })
                .await
                .unwrap();
            assert!(matches!(result, IntentResult::Action { .. }));
            assert!(
                oil.sessions[&id]
                    .last_observation
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|node| node.name == "Child"
                        && node.value.as_deref() == Some(value.as_str()))
            );
            assert!(
                page.observe()
                    .await
                    .unwrap()
                    .nodes
                    .iter()
                    .filter(|node| node.name == "Parent")
                    .all(|node| node.value.as_deref().unwrap_or_default().is_empty())
            );
        }
        assert!(
            oil.perform(Intent::Navigate {
                url: "http://[".into()
            })
            .await
            .is_err()
        );
        let session = &oil.sessions[&id];
        assert!(session.page.frame_scope().is_none());
        assert!(session.last_observation.is_none());
        assert_eq!(session.delta_cursor, Revision(0));
        page.close().await.unwrap();
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_frame_oil_actions_use_live_child_dom_and_keep_refs_isolated() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/root",r#"<input aria-label='Parent input'><iframe id=child srcdoc="<input aria-label='Child input'><button onclick='this.textContent=String(document.querySelector(&quot;input&quot;).value)'>Save child</button>"></iframe>"#).await.unwrap();
        let parent = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        let parent_ref = parent
            .nodes
            .iter()
            .find(|node| node.name == "Parent input")
            .unwrap()
            .semantic_ref;
        let mut oil = NativeOilSession::new(page.clone());
        oil.execute(r##"frame css("#child")"##).await.unwrap();
        let output = oil.execute("observe").await.unwrap();
        let NativeOilOutput::Observation { observation } = &output[0] else {
            panic!("observation")
        };
        assert!(
            observation
                .nodes
                .iter()
                .any(|node| node.name == "Child input")
        );
        assert!(
            !observation
                .nodes
                .iter()
                .any(|node| node.name == "Parent input")
        );
        assert!(
            oil.page
                .execute(NativeAction {
                    target: parent_ref,
                    action: SemanticAction::Type,
                    value: Some("wrong".into())
                })
                .await
                .is_err()
        );
        oil.execute("type \"Child input\" \"saved\"\nclick \"Save child\"")
            .await
            .unwrap();
        let selector = oryn_common::agent::Target::Css {
            selector: "input".into(),
        };
        let selected = oil
            .page
            .resolve_target(selector, None)
            .await
            .unwrap()
            .selected
            .unwrap();
        assert_ne!(selected, parent_ref);
        let child = oil
            .page
            .observe_profile(ProjectionProfile::Full)
            .await
            .unwrap();
        assert!(child.nodes.iter().any(|node| node.name == "saved"));
        oil.execute("frame parent").await.unwrap();
        assert!(
            oil.page
                .observe_profile(ProjectionProfile::Full)
                .await
                .unwrap()
                .nodes
                .iter()
                .any(|node| node.name == "Parent input")
        );
        page.close().await.unwrap();
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn opaque_sandbox_frame_is_automation_accessible_but_detachment_invalidates_scope() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/root",r#"<iframe id=child sandbox='allow-scripts' srcdoc="<input aria-label='Isolated input'>"></iframe>"#).await.unwrap();
        let mut oil = NativeOilSession::new(page.clone());
        oil.execute(r##"frame css("#child")"##).await.unwrap();
        oil.execute("type \"Isolated input\" \"native\"")
            .await
            .unwrap();
        assert!(
            page.evaluate("document.getElementById('child').contentDocument===null")
                .await
                .unwrap()
                .contains("true")
        );
        page.evaluate("document.getElementById('child').remove()")
            .await
            .unwrap();
        assert!(oil.execute("observe").await.is_err());
        oil.execute("frame main").await.unwrap();
        assert!(oil.execute("observe").await.is_ok());
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn action_alias_refresh_preserves_unobserved_delta_until_public_read() {
        let page = Browser::native_test_with_config(crate::runtime::BrowserConfig::default())
            .new_context()
            .new_page();
        page.load_html(
            "https://example.test/action-delta",
            "<input aria-label=Email><button>Save</button>",
        )
        .await
        .unwrap();
        let mut oil = NativeOilSession::new(page.clone());
        oil.execute("observe").await.unwrap();
        let before = oil.last_observation.clone().unwrap();
        let input = before
            .nodes
            .iter()
            .find(|node| node.actions.contains(&SemanticAction::Type))
            .unwrap();
        let alias = input.alias;
        oil.execute(&format!("type {alias} first\ntype {alias} second"))
            .await
            .unwrap();
        let refreshed = oil.last_observation.clone().unwrap();
        assert_eq!(oil.delta_cursor, before.revision);
        assert!(refreshed.revision > before.revision);
        assert!(
            refreshed
                .nodes
                .iter()
                .any(|node| node.alias == alias && node.value.as_deref() == Some("second"))
        );
        let outputs = oil.execute("observe --diff").await.unwrap();
        let NativeOilOutput::Delta { delta } = &outputs[0] else {
            panic!("delta")
        };
        assert_eq!(delta.from_revision, before.revision);
        assert_eq!(delta.to_revision, refreshed.revision);
        assert_eq!(delta.upserted.len(), 1);
        assert_eq!(delta.upserted[0].value.as_deref(), Some("second"));
        assert_eq!(
            oil.last_observation.as_ref().unwrap().revision,
            delta.to_revision
        );
        assert_eq!(oil.delta_cursor, delta.to_revision);
        let outputs = oil.execute("observe --diff").await.unwrap();
        let NativeOilOutput::Delta { delta } = &outputs[0] else {
            panic!("delta")
        };
        assert_eq!(delta.from_revision, delta.to_revision);
        assert!(delta.upserted.is_empty() && delta.removed.is_empty());
        oil.execute(&format!("type {alias} third")).await.unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('input').value")
                .await
                .unwrap(),
            "third"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn existing_oil_syntax_drives_native_observe_and_actions() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/form",
            "<input aria-label=Email><button>Save</button>",
        )
        .await
        .expect("load HTML");
        let mut session = NativeOilSession::new(page);
        let observation = session.execute("observe").await.expect("observe");
        let NativeOilOutput::Observation { observation } = &observation[0] else {
            panic!("expected observation")
        };
        let input_alias = observation
            .nodes
            .iter()
            .find(|node| node.actions.contains(&SemanticAction::Type))
            .expect("input")
            .alias;

        let outputs = session
            .execute(&format!("type {input_alias} \"agent@example.test\""))
            .await
            .expect("execute type");
        assert!(matches!(outputs[0], NativeOilOutput::Action { .. }));

        let outputs = session
            .execute("observe --diff")
            .await
            .expect("observe delta");
        let NativeOilOutput::Delta { delta } = &outputs[0] else {
            panic!("expected delta, got {:?}", outputs[0])
        };
        assert_eq!(delta.upserted.len(), 1);
        assert_eq!(
            delta.upserted[0].value.as_deref(),
            Some("agent@example.test")
        );

        let outputs = session
            .execute("observe --full")
            .await
            .expect("observe full");
        let NativeOilOutput::Observation { observation } = &outputs[0] else {
            panic!("expected full observation")
        };
        assert_eq!(observation.profile, ProjectionProfile::Full);

        let outputs = session
            .execute("observe --near \"Email\"")
            .await
            .expect("scoped observation");
        let NativeOilOutput::Observation { observation } = &outputs[0] else {
            panic!("expected scoped observation")
        };
        assert_eq!(observation.profile, ProjectionProfile::Scoped);
        assert!(!observation.nodes.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn observe_since_uses_the_explicit_revision_base() {
        let page = Browser::new().new_context().new_page();
        let base = page
            .load_html("https://example.test/form", "<input aria-label=Email>")
            .await
            .expect("load HTML");
        let mut session = NativeOilSession::new(page);
        let observation = session.execute("observe").await.expect("observe");
        let NativeOilOutput::Observation { observation } = &observation[0] else {
            panic!("expected observation")
        };
        let input_alias = observation.nodes[0].alias;
        session
            .execute(&format!("type {input_alias} \"agent@example.test\""))
            .await
            .expect("type value");

        let outputs = session
            .execute(&format!("observe --since {}", base.0))
            .await
            .expect("observe explicit delta");
        let NativeOilOutput::Delta { delta } = &outputs[0] else {
            panic!("expected delta")
        };
        assert_eq!(delta.from_revision, base);
        assert_eq!(delta.upserted.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_html_extraction_uses_template_void_raw_text_and_namespace_rules() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/serialization",r##"<main id="content"><template><span>A &amp; B</span><br></template><script type="text/plain">a < b && c > d</script><img alt='a"b'><svg><use xlink:href="#item"></use></svg></main>"##).await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        let outputs = session
            .execute("html --selector \"#content\"")
            .await
            .unwrap();
        let expected = r##"<main id="content"><template><span>A &amp; B</span><br></template><script type="text/plain">a < b && c > d</script><img alt="a&quot;b"><svg><use xlink:href="#item"></use></svg></main>"##;
        assert!(
            matches!(&outputs[0],NativeOilOutput::Structured{value} if value==&serde_json::json!(expected))
        );
        assert_eq!(
            page.extract(ContentKind::Html, Some("#missing".into()))
                .await
                .unwrap(),
            serde_json::json!("")
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_metadata_extraction_preserves_duplicates_and_selector_scope() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/meta",r#"<!doctype html><head>
            <meta name="description" content="first"><meta id="chosen" name="description" content="second">
            <meta name="" property="og:title" content="Native title"><meta charset="utf-8">
            <template><meta name="inert" content="excluded"></template></head><body>Content</body>"#).await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        let outputs=session.execute("extract meta\nextract meta --selector \"#chosen\"\nextract meta --selector \"#missing\"").await.unwrap();
        let expected = serde_json::json!([
            {"name":"description","content":"first"},{"name":"description","content":"second"},
            {"name":"og:title","content":"Native title"},{"name":null,"content":""}]);
        assert!(matches!(&outputs[0],NativeOilOutput::Structured{value} if value==&expected));
        assert!(
            matches!(&outputs[1],NativeOilOutput::Structured{value} if value==&serde_json::json!([{"name":"description","content":"second"}]))
        );
        assert!(
            matches!(&outputs[2],NativeOilOutput::Structured{value} if value==&serde_json::json!([]))
        );
        assert!(
            page.extract(ContentKind::Meta, Some("[".into()))
                .await
                .is_err()
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_oil_extracts_text_links_images_and_tables() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/content",
            "<article>Native content</article><a href=/next>Next</a><img src=hero.png alt=Hero><table><tr><th>Name</th></tr><tr><td>Oryn</td></tr></table>",
        )
        .await
        .expect("load content");
        let mut session = NativeOilSession::new(page);
        let outputs = session
            .execute(
                "extract text --selector \"article\"\nextract links\nextract images\nextract tables",
            )
            .await
            .expect("extract content");
        assert!(matches!(
            &outputs[0],
            NativeOilOutput::Structured { value } if value == "Native content"
        ));
        assert!(
            matches!(
                &outputs[1],
                NativeOilOutput::Structured { value } if value[0]["href"] == "/next"
            ),
            "{outputs:#?}"
        );
        assert!(matches!(
            &outputs[2],
            NativeOilOutput::Structured { value } if value[0]["alt"] == "Hero"
        ));
        assert!(matches!(
            &outputs[3],
            NativeOilOutput::Structured { value } if value[0][1][0] == "Oryn"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reuses_semantic_resolver_for_relational_and_role_targets() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/actions",
            "<button id=first>First</button><button>Last</button>",
        )
        .await
        .expect("load HTML");
        let mut session = NativeOilSession::new(page);
        session.execute("observe").await.expect("observe");

        let outputs = session
            .execute("hover \"First\" before \"Last\"")
            .await
            .expect("resolve relation");
        let NativeOilOutput::Action { result } = &outputs[0] else {
            panic!("expected hover action")
        };
        assert!(matches!(
            result.effects.first(),
            Some(oryn_common::v3::Effect::Event { event_type, .. }) if event_type == "mouseover"
        ));

        let outputs = session
            .execute("focus css(\"#first\")")
            .await
            .expect("resolve selector");
        assert!(matches!(outputs[0], NativeOilOutput::Action { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolves_dom_containment_from_full_native_semantics() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/containment",
            "<main aria-label=Panel><button>Inner</button></main><button>Outer</button>",
        )
        .await
        .expect("load HTML");
        let mut session = NativeOilSession::new(page);
        session
            .execute("observe --full")
            .await
            .expect("observe full");
        let outputs = session
            .execute("hover \"Inner\" inside \"Panel\"")
            .await
            .expect("resolve containment");
        assert!(matches!(outputs[0], NativeOilOutput::Action { .. }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn unicode_oil_target_and_value_reach_native_input_unchanged() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://forms.example.test/", "<label>名前<input></label>")
            .await
            .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session
            .execute("observe\ntype \"名前\" \"Zoë — 日本語 👩🏽‍💻 # text\"")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('input').value")
                .await
                .unwrap(),
            "Zoë — 日本語 👩🏽‍💻 # text"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn oil_drives_scripted_native_document_without_scanner_or_cdp() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/svelte",
            include_str!("../../../test-harness/scenarios/spa/svelte-tasks.html"),
        )
        .await
        .expect("load executable fixture");
        let mut session = NativeOilSession::new(page);
        session.execute("observe").await.expect("observe");
        session
            .execute("click \"Add item\"")
            .await
            .expect("click via OIL");
        let outputs = session.execute("observe").await.expect("observe result");
        let NativeOilOutput::Observation { observation } = &outputs[0] else {
            panic!("expected observation")
        };
        assert!(observation.nodes.iter().any(|node| node.name == "Items: 1"));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn cart_workflow_resolves_visible_button_text_and_input_id() {
        let page = Browser::new().new_context().new_page();
        let fixture = include_str!("../../../test-harness/scenarios/intent-tests/flow-cart.html")
            .replace(
                "<link rel=\"stylesheet\" href=\"/shared/styles.css\">",
                &format!(
                    "<style>{}</style>",
                    include_str!("../../../test-harness/public/shared/styles.css")
                ),
            );
        page.load_html("https://example.test/cart", &fixture)
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("getComputedStyle(document.querySelector('.container')).maxWidth")
                .await
                .unwrap(),
            "800px"
        );
        page.evaluate("globalThis.nativeClickTrace=[];for(const type of ['pointerdown','mousedown','pointerup','mouseup','click'])document.addEventListener(type,e=>nativeClickTrace.push([type,e.target.localName,e.target.textContent,e.target.getAttribute('onclick'),e.target.closest('[data-product-id]')?.getAttribute('data-product-id'),e.button,e.isTrusted].join(':')),true)").await.unwrap();
        page.evaluate("globalThis.nativeClickErrors=[];addEventListener('error',e=>nativeClickErrors.push([e.message,e.filename,e.lineno,e.colno]));document.addEventListener('click',e=>nativeClickTrace.push(['bubble',e.target.localName,e.target.closest('[data-product-id]')?.getAttribute('data-product-id'),e.clientX,e.clientY,e.detail,e.defaultPrevented,e.isTrusted].join(':')))").await.unwrap();
        // Observe the actual native candidate ranking; do not dispatch a CSS
        // replacement click or change the original multi-command workflow.
        let near_geometry = page.evaluate(r#"JSON.stringify([...document.querySelectorAll('[data-product-id] strong,[data-product-id] button[aria-label=\"Increase quantity\"]')].slice(0,8).map(n=>{const r=n.getBoundingClientRect();return {text:n.textContent.slice(0,40),product:n.closest('[data-product-id]').getAttribute('data-product-id'),x:r.x,y:r.y,width:r.width,height:r.height}}))"#).await;
        let plus_resolution = page
            .resolve_target(
                oryn_common::agent::Target::Near {
                    target: Box::new(oryn_common::agent::Target::Text { text: "+".into() }),
                    anchor: Box::new(oryn_common::agent::Target::Text {
                        text: "Wireless Mouse".into(),
                    }),
                },
                Some(SemanticAction::Click),
            )
            .await;
        let intended_resolution = page
            .resolve_target(
                oryn_common::agent::Target::Css {
                    selector: "[data-product-id='1'] button[aria-label='Increase quantity']".into(),
                },
                Some(SemanticAction::Click),
            )
            .await;
        let mut session = NativeOilSession::new(page.clone());
        let outputs=session.execute("observe\nclick \"+\" near \"Wireless Mouse\"\ntype \"coupon-code\" \"SAVE10\"\nclick \"Apply\"").await.unwrap();
        let action_receipts = outputs
            .iter()
            .filter_map(|output| match output {
                NativeOilOutput::Action { result } => Some(result),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            page.evaluate("document.getElementById('qty-1').textContent")
                .await
                .unwrap(),
            "2", "native plus resolution: {plus_resolution:?}; intended product resolution: {intended_resolution:?}; action receipts: {action_receipts:?}; native page trace: {:?}; fixture state: {:?}; near geometry: {near_geometry:?}",
            page.trace().await,
            page.evaluate("JSON.stringify({events:nativeClickTrace,errors:nativeClickErrors,updateQuantity:typeof updateQuantity,applyCoupon:typeof applyCoupon,readyState:document.readyState,qty1:document.getElementById('qty-1').textContent,qty2:document.getElementById('qty-2').textContent,coupon:document.getElementById('coupon-code').value,message:document.getElementById('coupon-message').textContent,subtotal:document.getElementById('subtotal').textContent,total:document.getElementById('total').textContent})").await
        );
        assert_eq!(
            page.evaluate("document.getElementById('coupon-code').value")
                .await
                .unwrap(),
            "SAVE10"
        );
        assert!(matches!(
            session.execute("click \"nonexistent-control\"").await,
            Err(NativeOilError::Runtime(RuntimeError::Semantic(_)))
        ));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn pointer_target_recovers_after_ancestor_class_timer_removal() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/recovery", r#"
            <style>.disabled { pointer-events: none; opacity: .5 }</style>
            <div id="container"><button id="target" onclick="this.textContent='Recovered'">Try</button></div>
            <button onclick="document.getElementById('container').classList.add('disabled'); setTimeout(()=>document.getElementById('container').classList.remove('disabled'),100)">Disable temporarily</button>
        "#).await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        let result = tokio::time::timeout(Duration::from_secs(5), session.execute("click \"Disable temporarily\"\nwait visible css(#container:not(.disabled) #target)\nclick \"Try\"")).await;
        assert!(matches!(result, Ok(Ok(_))), "{result:?}; DOM: {}", page.evaluate("JSON.stringify({html:document.body.innerHTML,style:getComputedStyle(document.getElementById('target')).pointerEvents,rect:document.getElementById('target').getBoundingClientRect()})").await.unwrap());
        assert_eq!(
            page.evaluate("document.getElementById('target').textContent")
                .await
                .unwrap(),
            "Recovered"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn numeric_calendar_inputs_are_editable_and_report_constraints_through_oil() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/booking", "<label for=n>Seats</label><input id=n type=number min=1 max=9 step=2><label for=d>Date</label><input id=d type=date min=2026-10-01><label for=t>Time</label><input id=t type=time min=21:00 max=06:00>").await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session
            .execute("type \"Seats\" \"4\"\ntype \"Date\" \"2026-09-30\"\ntype \"Time\" \"12:00\"")
            .await
            .unwrap();
        let observation = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        for (name, value) in [("Seats", "4"), ("Date", "2026-09-30"), ("Time", "12:00")] {
            let control = observation
                .nodes
                .iter()
                .find(|n| n.name == name && n.validation.is_some())
                .unwrap();
            assert_eq!(control.value.as_deref(), Some(value));
            let validation = control.validation.as_ref().unwrap();
            assert!(validation.will_validate && !validation.valid, "{control:?}");
        }
        session
            .execute("type \"Seats\" \"3\"\ntype \"Date\" \"2026-10-01\"\ntype \"Time\" \"23:00\"")
            .await
            .unwrap();
        let observation = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        assert!(
            observation
                .nodes
                .iter()
                .filter_map(|n| n.validation.as_ref())
                .all(|v| v.valid)
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn dismiss_honors_named_container_and_never_clicks_global_cancel() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/scoped-dismiss",r#"<button onclick="globalThis.outside=(globalThis.outside||0)+1">Cancel unrelated</button><section role="dialog" aria-label="Other"><button onclick="globalThis.other=(globalThis.other||0)+1">Close</button></section><section role="dialog" aria-label="Wanted"><button onclick="globalThis.wanted=(globalThis.wanted||0)+1">Close</button></section>"#).await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        assert!(session.execute("dismiss \"Missing\"").await.is_err());
        assert_eq!(
            page.evaluate(
                "[globalThis.outside||0,globalThis.other||0,globalThis.wanted||0].join(',')"
            )
            .await
            .unwrap(),
            "0,0,0"
        );
        session.execute("dismiss \"Wanted\"").await.unwrap();
        assert_eq!(
            page.evaluate(
                "[globalThis.outside||0,globalThis.other||0,globalThis.wanted||0].join(',')"
            )
            .await
            .unwrap(),
            "0,0,1"
        );
    }
    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn dismiss_generic_popup_requires_native_overlay_and_handles_small_geometry() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/overlay-dismiss",r#"<button onclick="globalThis.outside=(globalThis.outside||0)+1">Close unrelated</button><div id="overlay" style="display:none;position:fixed;left:200px;top:100px;width:200px;height:100px;z-index:3000;background:white"><button onclick="globalThis.inside=(globalThis.inside||0)+1;document.getElementById('overlay').style.display='none'">Dismiss</button></div>"#).await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        assert!(session.execute("dismiss \"popup\"").await.is_err());
        assert_eq!(page.evaluate("globalThis.outside||0").await.unwrap(), "0");
        page.evaluate("document.getElementById('overlay').style.display='block'")
            .await
            .unwrap();
        session.execute("dismiss \"popup\"").await.unwrap();
        assert_eq!(
            page.evaluate("[globalThis.outside||0,globalThis.inside||0].join(',')")
                .await
                .unwrap(),
            "0,1"
        );
    }
    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn dismiss_unnamed_icon_is_scoped_to_overlay_corner() {
        let page = Browser::new().new_context().new_page();
        page.load_html("https://example.test/icon-dismiss",r#"<button style="width:20px;height:20px" onclick="globalThis.dismissCounts.outside++"></button><div role="dialog" aria-label="Icon panel" style="position:fixed;left:200px;top:100px;width:300px;height:180px;z-index:3000;background:white"><button style="position:absolute;left:10px;bottom:10px;width:24px;height:24px" onclick="globalThis.dismissCounts.primary++"></button><button style="position:absolute;right:8px;top:8px;width:24px;height:24px" onclick="globalThis.dismissCounts.closed++"><svg width="16" height="16"><path d="M0 0L16 16M16 0L0 16"/></svg></button></div>"#).await.unwrap();
        // Window.closed is a native read-only accessor, not a writable test
        // counter. Keep the business counters in a fixture-owned object.
        assert_eq!(page.evaluate("(()=>{const d=Object.getOwnPropertyDescriptor(window,'closed');return typeof d.get==='function'&&d.set===undefined&&Reflect.set(window,'closed',1)===false&&window.closed===false})()").await.unwrap(),"true");
        page.evaluate("globalThis.dismissCounts={outside:0,primary:0,closed:0};globalThis.nativeClickTargets=[];document.addEventListener('click',e=>nativeClickTargets.push(e.isTrusted&&e.button===0&&e.target.closest('button')===document.querySelector('[role=dialog] button:last-child')),true);globalThis.nativeClickTrace=[];for(const type of ['pointerdown','mousedown','pointerup','mouseup','click'])document.addEventListener(type,e=>nativeClickTrace.push([type,e.target.localName,e.target.closest('button')?.getAttribute('onclick'),e.button,e.isTrusted].join(':')),true)").await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session.execute("dismiss \"popup\"").await.unwrap();
        assert_eq!(
            page.evaluate(
                "[dismissCounts.outside,dismissCounts.primary,dismissCounts.closed].join(',')"
            )
            .await
            .unwrap(),
            "0,0,1",
            "native input trace: {}",
            page.evaluate("JSON.stringify(nativeClickTrace)")
                .await
                .unwrap()
        );
        assert_eq!(
            page.evaluate("JSON.stringify(nativeClickTargets)")
                .await
                .unwrap(),
            "[true]",
            "one trusted primary click must reach the original unnamed corner button"
        );
        assert_eq!(
            page.evaluate("String(window.closed)").await.unwrap(),
            "false"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn dismiss_visible_modal_after_native_form_submission() {
        let page = Browser::new().new_context().new_page();
        let html = include_str!("../../../test-harness/scenarios/forms/login.html").replace(
            "<link rel=\"stylesheet\" href=\"/shared/styles.css\">",
            &format!(
                "<style>{}</style>",
                include_str!("../../../test-harness/public/shared/styles.css")
            ),
        );
        page.load_html("https://example.test/login", html)
            .await
            .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session.execute("observe\ntype \"Email Address\" \"test@example.com\"\ntype \"Password\" \"password123\"\ncheck \"Remember me\"\nclick \"Sign In\"").await.unwrap();
        assert_eq!(
            page.evaluate("document.getElementById('success-modal').style.display")
                .await
                .unwrap(),
            "flex"
        );
        let full = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        let close = full.nodes.iter().find(|node| node.name == "Close").unwrap();
        assert!(
            close.states.contains(&oryn_common::v3::NodeState::Visible),
            "{close:?}"
        );
        session
            .execute("wait visible css(.modal-content)\ndismiss \"modal\"")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("document.getElementById('success-modal').style.display")
                .await
                .unwrap(),
            "none"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn repeated_custom_element_renders_release_detached_wrapper_graphs() {
        let page = Browser::new().new_context().new_page();
        let load = page
            .load_html(
                "https://example.test/angular-shop",
                include_str!("../../../test-harness/scenarios/spa/angular-shop.html"),
            )
            .await;
        if let Err(error) = load {
            // Observe only after the original load has failed. The loader may
            // already have stopped parsing; this is not its pre-timeout state.
            let diagnostic = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                page.evaluate(r#"JSON.stringify({
                    ready: document.readyState,
                    closed: window.closed,
                    definitions: ['shop-header','product-card','search-bar','cart-modal','shop-container','shop-footer'].map(name => [name, typeof customElements.get(name)]),
                    container: !!document.querySelector('shop-container'),
                    gridChildren: document.getElementById('product-grid')?.children.length ?? null,
                    searchShadow: !!document.querySelector('search-bar')?.shadowRoot,
                    footerShadow: !!document.querySelector('shop-footer')?.shadowRoot
                })"#),
            ).await;
            panic!(
                "load custom-element fixture: {error}; post-failure state (1s bound): {diagnostic:?}"
            );
        }
        // Document CSS selectors do not cross the component's shadow root.
        // OIL's semantic targets reach the composed controls by their ID hint
        // or accessible name, preserving the document selector boundary.
        assert_eq!(
            page.evaluate("String(document.querySelector('#search')===null)")
                .await
                .unwrap(),
            "true"
        );
        let mut session = NativeOilSession::new(page.clone());
        for (command, count, sort) in [
            ("type \"search\" \"wireless\"", 2, "rating"),
            ("type \"search\" \"\"", 8, "rating"),
            ("click \"Price\"", 8, "price"),
            ("click \"Rating\"", 8, "rating"),
            ("click \"Name\"", 8, "name"),
        ] {
            session
                .execute(command)
                .await
                .unwrap_or_else(|error| panic!("{command} failed: {error}"));
            assert_eq!(page.evaluate("document.getElementById('product-grid').children.length+','+document.querySelector('search-bar').shadowRoot.querySelector('button.active').dataset.sort").await.unwrap(), format!("{count},{sort}"), "{command}");
        }
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_table_geometry_does_not_poison_unrelated_target_resolution() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/table-and-nav",
            "<nav><button>Dashboard</button></nav><table><tr><td><button>Row action</button></td></tr></table>",
        )
        .await
        .expect("load mixed geometry document");
        let mut session = NativeOilSession::new(page);

        let outputs = session
            .execute("click \"Dashboard\"")
            .await
            .expect("resolve control outside unsupported table subtree");

        assert!(matches!(outputs[0], NativeOilOutput::Action { .. }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn duplicate_help_text_falls_back_to_the_actionable_control() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/duplicate-label",
            "<p>Click Delayed Action Button when it appears.</p><button>Delayed Action Button</button>",
        )
        .await
        .expect("load duplicate label document");
        let mut session = NativeOilSession::new(page);

        let outputs = session
            .execute("click \"Delayed Action Button\"")
            .await
            .expect("resolve actionable duplicate");

        assert!(matches!(outputs[0], NativeOilOutput::Action { .. }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn exact_numeric_link_beats_aggregate_pagination_text() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/pages",
            r#"<ul onclick="if(event.target===this)document.querySelector('#status').textContent='container'">First123Last<li><a onclick="document.querySelector('#status').textContent='page-3'">3</a></li></ul><output id=status>waiting</output>"#,
        )
        .await
        .expect("load pagination document");
        let mut session = NativeOilSession::new(page);

        session
            .execute("click \"3\"")
            .await
            .expect("click exact page");
        let output = session
            .execute("text --selector \"#status\"")
            .await
            .expect("extract selected page");

        assert!(matches!(
            &output[0],
            NativeOilOutput::Structured { value } if value == "page-3"
        ));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn wait_visible_uses_stylesheet_layout_and_advances_timer_tasks() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/delayed-visibility",
            r#"<style>
                .delayed { visibility: hidden; }
                .delayed.show { visibility: visible; }
            </style>
            <button id="show" onclick="setTimeout(() => document.getElementById('delayed').classList.add('show'), 100)">Show</button>
            <button id="delayed" class="delayed">Delayed Action Button</button>"#,
        )
        .await
        .expect("load delayed visibility document");
        let mut session = NativeOilSession::new(page);

        let outputs = session
            .execute(
                "click css(#show)\nwait visible css(#delayed.show)\nclick \"Delayed Action Button\"",
            )
            .await
            .expect("wait for computed visibility and click delayed control");

        assert!(matches!(outputs[2], NativeOilOutput::Action { .. }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn secret_label_oil_uses_parent_store_and_never_types_the_label() {
        let mut options = crate::runtime::ContextOptions::default();
        options.action_policy.rules.insert(
            crate::security::ConsequentialAction::CredentialFill,
            crate::security::PolicyRule::Allow,
        );
        let context = crate::runtime::Browser::in_process_with_config(
            crate::runtime::BrowserConfig::default(),
        )
        .new_context_with_options(options);
        context.import_secret(
            "https://secret.example",
            "account-password",
            b"correct horse".to_vec(),
        );
        let page = context.new_page();
        page.load_html(
            "https://secret.example/form",
            "<label>Password <input type=password></label>",
        )
        .await
        .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session
            .execute("type \"Password\" $secret:account-password")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("document.querySelector('input').value")
                .await
                .unwrap(),
            "correct horse"
        );
        let observation = page.observe().await.unwrap();
        assert_eq!(observation.nodes[0].value.as_deref(), Some("[REDACTED]"));
        assert!(!format!("{observation:?}").contains("correct horse"));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn wait_visible_distinguishes_unsupported_geometry_from_hidden_content() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/visible-table",
            "<table class=results><tr><td>Visible result</td></tr></table>",
        )
        .await
        .expect("load visible table");
        let mut session = NativeOilSession::new(page);

        let outputs = session
            .execute("wait visible css(.results)")
            .await
            .expect("visible table does not require table box geometry");

        assert!(matches!(outputs[0], NativeOilOutput::Text { ref value } if value == "visible"));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn force_click_explicitly_bypasses_hidden_geometry() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/forced",
            r#"<button style="display:none" onclick="document.querySelector('#status').textContent='clicked'">Hidden action</button><output id=status>waiting</output>"#,
        )
        .await
        .expect("load hidden action document");
        let mut session = NativeOilSession::new(page);

        session
            .execute("click \"Hidden action\" --force")
            .await
            .expect("force click hidden semantic target");
        let output = session
            .execute("text --selector \"#status\"")
            .await
            .expect("extract forced action result");

        assert!(matches!(
            &output[0],
            NativeOilOutput::Structured { value } if value == "clicked"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_oil_is_truthfully_reported() {
        let page = Browser::new().new_context().new_page();
        let mut session = NativeOilSession::new(page);
        let outputs = session
            .execute("screenshot")
            .await
            .expect("parse screenshot");
        let NativeOilOutput::Unsupported { diagnostic } = &outputs[0] else {
            panic!("expected capability diagnostic")
        };
        assert_eq!(diagnostic.support, SupportLevel::Unsupported);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_trace_records_intents_without_sensitive_values() {
        let context =
            Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html(
            "https://example.test/login",
            "<input aria-label=Password type=password>",
        )
        .await
        .expect("load form");
        let mut session = NativeContextOilSession::new(context.clone());
        session.execute("observe").await.expect("observe");
        session
            .execute("type \"Password\" \"never-store-this\"")
            .await
            .expect("type password");
        let trace = context.trace_archive(Vec::new()).await.expect("trace");
        let serialized = serde_json::to_string(&trace).expect("serialize trace");
        assert!(!serialized.contains("never-store-this"));
        assert!(
            trace
                .intents
                .iter()
                .any(|intent| intent.input.contains("[REDACTED]"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_session_executes_the_versioned_typed_intent_envelope() {
        let context =
            Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html(
            "https://intent.example/",
            "<button id=continue>Continue</button>",
        )
        .await
        .unwrap();
        let request = oryn_common::agent::IntentRequest::new(Intent::Observe {
            options: oryn_common::agent::ObserveOptions::default(),
        });
        let input = format!(
            "{TYPED_INTENT_PREFIX}{}",
            serde_json::to_string(&request).unwrap()
        );
        let mut session = NativeContextOilSession::new(context.clone());

        let outputs = session.execute(&input).await.unwrap();

        assert!(matches!(
            &outputs[0],
            NativeOilOutput::Intent {
                result: IntentResult::Observation { observation }
            } if observation.nodes.iter().any(|node| node.name == "Continue")
        ));
        let trace = context.trace_archive(Vec::new()).await.unwrap();
        assert!(
            trace
                .intents
                .iter()
                .any(|record| record.input == "intent:observe")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_session_rejects_an_unknown_typed_intent_contract() {
        let context =
            Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        context.new_page();
        let input = format!(
            "{TYPED_INTENT_PREFIX}{}",
            serde_json::json!({
                "contract_version": oryn_common::agent::AGENT_INTENT_CONTRACT_VERSION + 1,
                "intent": {"intent": "back"}
            })
        );
        let mut session = NativeContextOilSession::new(context);

        let error = session.execute(&input).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported agent intent contract version")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn multiline_failure_preserves_prefix_and_does_not_replay_actions() {
        let context =
            Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html("https://oil.example/", "<input aria-label='Name'>")
            .await
            .unwrap();
        let mut session = NativeOilSession::new(page);
        let error = session
            .execute("observe\ntype \"Name\" \"Alice\"\nclick \"Missing\"")
            .await
            .unwrap_err();
        assert!(
            matches!(&error, NativeOilError::ScriptExecutionFailed { failed_index: 2, completed, .. }
            if completed.len() == 2 && matches!(&completed[1], NativeOilOutput::Action { .. }))
        );
        let native = error.into_runtime().unwrap();
        let encoded = serde_json::to_vec(&native).unwrap();
        let decoded: RuntimeError = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, native);
        let RuntimeError::OilExecutionFailed {
            completed,
            failed_index,
            ..
        } = decoded
        else {
            panic!("missing receipts")
        };
        assert_eq!(failed_index, 2);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&completed[1]).unwrap()["kind"],
            "action"
        );
        // Recovery observes the existing state; it never reruns the successful prefix.
        assert!(matches!(
            &session.execute("observe").await.unwrap()[0],
            NativeOilOutput::Observation { .. }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn context_prefix_covers_page_typed_and_upload_parse_failures() {
        for failed_line in ["page invalid", "@oryn-intent {}", "upload"] {
            let context =
                Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
            context.new_page();
            let mut session = NativeContextOilSession::new(context);
            let error = session
                .execute(&format!("# comment\npages\n\n{failed_line}"))
                .await
                .unwrap_err();
            assert!(
                matches!(error, NativeOilError::ScriptExecutionFailed { failed_index: 1, completed, .. }
                if matches!(completed.as_slice(), [NativeOilOutput::Pages { .. }]))
            );
        }
    }

    #[test]
    fn first_command_failure_preserves_native_classification() {
        let error = NativeOilError::Runtime(RuntimeError::Closed).with_completed(Vec::new(), 0);
        assert!(matches!(
            error,
            NativeOilError::Runtime(RuntimeError::Closed)
        ));
        let error = NativeOilError::Runtime(RuntimeError::Closed)
            .with_completed(
                vec![NativeOilOutput::Text {
                    value: "retained".into(),
                }],
                1,
            )
            .into_runtime()
            .unwrap();
        assert!(matches!(error.root_cause(), RuntimeError::Closed));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn target_read_refresh_keeps_alias_snapshot_and_delta_cursor_together() {
        let context =
            Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html("https://oil.example/", "<button>Original</button>")
            .await
            .unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session.execute("observe").await.unwrap();
        page.load_html("https://oil.example/next", "<button>Replacement</button>")
            .await
            .unwrap();
        session.execute("text \"Replacement\"").await.unwrap();
        let revision = session.last_observation.as_ref().unwrap().revision;
        assert_eq!(session.delta_cursor, revision);
        let outputs = session.execute("observe --diff").await.unwrap();
        assert!(
            matches!(&outputs[0], NativeOilOutput::Delta { delta } if delta.from_revision == revision)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn frame_transport_failure_keeps_native_terminal_cause() {
        let context =
            Browser::in_process_with_config(crate::BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.close().await.unwrap();
        let mut session = NativeOilSession::new(page);
        let error = session.execute("frame main").await.unwrap_err();
        assert!(matches!(
            error,
            NativeOilError::Runtime(RuntimeError::Closed)
        ));
    }
}

#[cfg(all(test, unix, feature = "v8-host"))]
mod oil_selected_navigation_tests {
    use super::*;
    use crate::host::webdriver_script::{AutomationRealmSnapshot, FrameCommand, FrameResponse};
    use crate::network::{
        NetworkBroker, NetworkError, NetworkEvent, NetworkOperation, NetworkRequest,
        NetworkResponse, SharedNetworkBroker,
    };
    use crate::security::{ConsequentialAction, PolicyRule};
    use oryn_common::v3::{ActionId, Effect};
    use std::sync::{Arc, Mutex};

    const PAYMENT: &str = "https://payment-oil.test";
    type Requests = Arc<Mutex<Vec<NetworkRequest>>>;
    struct Broker {
        status: u16,
        requests: Requests,
    }
    impl NetworkBroker for Broker {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!("native frame driver uses owned operations")
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let id = request.request_id;
            let child = request.url.ends_with("/child");
            let submitted = request.url.ends_with("/authorize");
            let status = if submitted { self.status } else { 200 };
            self.requests.lock().unwrap().push(request);
            let (sender, events) = tokio::sync::mpsc::channel(8);
            let mut headers = vec![("content-type".into(), "text/html".into())];
            if status == 302 {
                headers.push(("location".into(), "/approved".into()));
            }
            sender
                .try_send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: None,
                    request_id: id,
                    status,
                    headers,
                    content_length: None,
                    upload: None,
                })
                .unwrap();
            let body = if child {
                "<!doctype html><title>Payment</title><form method=post action=/authorize><input name=quote value=owned><button id=authorize>Authorize payment</button></form><button id=noop type=button>No navigation</button>"
            } else if status == 403 {
                "<!doctype html><title>Denied</title><p id=payment-denied>Denied</p>"
            } else {
                "<!doctype html><title>Approved</title><p id=payment-authorized>Authorized</p><button id=next>Next</button>"
            };
            if status != 302 {
                sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: id,
                        bytes: body.as_bytes().to_vec().into(),
                    })
                    .unwrap();
            }
            sender
                .try_send(NetworkEvent::Complete {
                    encoded_body_size: None,
                    request_id: id,
                })
                .unwrap();
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: id,
                events,
                demand: None,
            })
        }
    }
    async fn fixture(status: u16) -> (PageHandle, NativeOilSession, Requests) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut options = crate::ContextOptions::default();
        options
            .action_policy
            .rules
            .insert(ConsequentialAction::FormSubmission, PolicyRule::Allow);
        let context = crate::Browser::in_process_with_config(Default::default())
            .new_context_with_broker(
                options,
                SharedNetworkBroker::new(Broker {
                    status,
                    requests: requests.clone(),
                }),
            );
        let page = context.new_page();
        page.load_html("https://merchant-oil.test/root", format!("<!doctype html><title>Merchant</title><iframe id=payment src='{PAYMENT}/child'></iframe>")).await.unwrap();
        let mut session = NativeOilSession::new(page.clone());
        session
            .execute("frame css(#payment)\nobserve")
            .await
            .unwrap();
        requests.lock().unwrap().clear();
        (page, session, requests)
    }
    fn assert_post_count(requests: &Requests, count: usize) {
        let requests = requests.lock().unwrap();
        let posts = requests
            .iter()
            .filter(|request| request.method == "POST")
            .collect::<Vec<_>>();
        assert_eq!(
            posts.len(),
            count,
            "selection refresh never replays input or submission"
        );
        for post in posts {
            assert_eq!(post.body, b"quote=owned");
        }
    }
    async fn current_at(page: &PageHandle, url: &str) -> AutomationRealmSnapshot {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let FrameResponse::Inventory(frames) = page
                .native_frame_command(FrameCommand::Inventory)
                .await
                .unwrap()
            else {
                panic!("inventory response")
            };
            if let Some(frame) = frames
                .into_iter()
                .find(|frame| frame.parent_realm.is_some() && frame.url == url && frame.live)
            {
                return frame;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "frame did not reach {url}"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    async fn successful_navigation(status: u16) {
        let (page, mut session, requests) = fixture(status).await;
        let source = session.page.frame_scope().unwrap().clone();
        let stale_reader = session.page.clone();
        let old_target = session
            .page
            .resolve_selector("#authorize", SemanticAction::Click)
            .await
            .unwrap();
        let output = session.execute("click css(#authorize)").await.unwrap();
        let NativeOilOutput::Action { result } = &output[0] else {
            panic!("action receipt")
        };
        assert_eq!(result.action_id, ActionId(1));
        assert!(result.effects.iter().any(|effect| matches!(effect, Effect::Navigation { url } if url == &format!("{PAYMENT}/authorize"))));
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|detail| detail.contains("follow-up observation unavailable"))
        );
        let destination = session.page.frame_scope().unwrap();
        assert!(destination.same_navigable_as(&source));
        assert!(!destination.same_document_as(&source));
        assert_eq!(
            destination.url,
            format!(
                "{PAYMENT}/{}",
                if status == 302 {
                    "approved"
                } else {
                    "authorize"
                }
            )
        );
        let selector = if status == 403 {
            "#payment-denied"
        } else {
            "#payment-authorized"
        };
        let output = session
            .execute(&format!("wait visible css({selector}) --timeout 1s"))
            .await
            .unwrap();
        assert!(matches!(&output[0], NativeOilOutput::Text { value } if value == "visible"));
        assert!(stale_reader.observe().await.is_err());
        assert!(
            session
                .page
                .execute(NativeAction {
                    target: old_target,
                    action: SemanticAction::Click,
                    value: None
                })
                .await
                .is_err()
        );
        assert_post_count(&requests, 1);
        session.execute("frame main").await.unwrap();
        assert!(session.page.frame_scope().is_none());
        assert_eq!(
            session.page.evaluate("document.title").await.unwrap(),
            "Merchant"
        );
        page.close().await.unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_selected_navigation_post_refreshes_session_and_preserves_stale_clones() {
        successful_navigation(200).await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_selected_navigation_redirect_refreshes_committed_destination_without_replay() {
        successful_navigation(302).await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_selected_navigation_http_error_document_remains_selected() {
        successful_navigation(403).await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_action_ids_survive_selected_navigation_and_root_interleave() {
        let (page, mut session, requests) = fixture(200).await;
        page.evaluate(
            "document.body.insertAdjacentHTML('afterbegin','<input id=root-action>');'ready'",
        )
        .await
        .unwrap();
        let mut root = NativeOilSession::new(page.clone());
        let first = root.execute("focus css(#root-action)").await.unwrap();
        let second = session.execute("click css(#authorize)").await.unwrap();
        session.page.evaluate("document.body.insertAdjacentHTML('beforeend','<input id=destination-action>');'ready'").await.unwrap();
        let destination = session
            .execute("focus css(#destination-action)")
            .await
            .unwrap();
        let last = root.execute("focus css(#root-action)").await.unwrap();
        let ids = [&first, &second, &destination, &last].map(|outputs| {
            let NativeOilOutput::Action { result } = &outputs[0] else {
                panic!("action receipt")
            };
            assert_eq!(result.page_id, page.page_id());
            result.action_id.0
        });
        assert_eq!(ids[0], 1);
        assert_eq!(ids[1], 2);
        assert_eq!(ids, [1, 2, 3, 4]);
        assert_post_count(&requests, 1);
        page.close().await.unwrap();
    }
    async fn stale_successor(replace_connection: bool) {
        let (page, mut session, requests) = fixture(200).await;
        let source = session.page.frame_scope().unwrap().clone();
        let target = session
            .page
            .resolve_selector("#authorize", SemanticAction::Click)
            .await
            .unwrap();
        let result = session
            .page
            .execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .unwrap();
        let installed = current_at(&page, &format!("{PAYMENT}/authorize")).await;
        assert!(!installed.same_document_as(&source));
        assert!(
            page.native_frame_command(FrameCommand::ResolveActionSelection {
                current: source.clone(),
                action_id: ActionId(result.action_id.0 + 1)
            })
            .await
            .is_err()
        );
        let script = if replace_connection {
            format!(
                "const old=document.getElementById('payment');const fresh=document.createElement('iframe');fresh.id='payment';fresh.src='{PAYMENT}/later';old.replaceWith(fresh);true"
            )
        } else {
            format!("document.getElementById('payment').src='{PAYMENT}/later';true")
        };
        page.evaluate(script).await.unwrap();
        let later = current_at(&page, &format!("{PAYMENT}/later")).await;
        assert!(!later.same_document_as(&installed));
        assert_eq!(later.same_navigable_as(&source), !replace_connection);
        let refreshed = session.refresh_after_action(result).await;
        assert_eq!(refreshed.action_id, ActionId(1));
        assert!(
            refreshed
                .diagnostics
                .iter()
                .any(|detail| detail.contains("follow-up observation unavailable"))
        );
        assert!(
            session
                .page
                .frame_scope()
                .unwrap()
                .same_document_as(&source)
        );
        assert!(session.last_observation.is_none());
        assert!(
            session
                .execute("wait visible css(#payment-authorized) --timeout 1ms")
                .await
                .is_err()
        );
        assert_post_count(&requests, 1);
        page.close().await.unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_selected_navigation_rejects_later_unrelated_document_in_same_connection() {
        stale_successor(false).await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_selected_navigation_rejects_replaced_embedding_connection() {
        stale_successor(true).await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn oil_selected_navigation_does_not_refresh_unowned_document_replacement() {
        let (page, mut session, requests) = fixture(200).await;
        let source = session.page.frame_scope().unwrap().clone();
        let target = session
            .page
            .resolve_selector("#noop", SemanticAction::Click)
            .await
            .unwrap();
        let result = session
            .page
            .execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .unwrap();
        assert!(
            !result
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::Navigation { .. }))
        );
        page.evaluate(format!(
            "document.getElementById('payment').src='{PAYMENT}/later';true"
        ))
        .await
        .unwrap();
        let later = current_at(&page, &format!("{PAYMENT}/later")).await;
        assert!(later.same_navigable_as(&source));
        assert!(!later.same_document_as(&source));
        let refreshed = session.refresh_after_action(result).await;
        assert_eq!(refreshed.action_id, ActionId(1));
        assert!(
            refreshed
                .diagnostics
                .iter()
                .any(|detail| detail.contains("follow-up observation unavailable"))
        );
        assert!(
            session
                .page
                .frame_scope()
                .unwrap()
                .same_document_as(&source)
        );
        assert!(session.last_observation.is_none());
        assert_post_count(&requests, 0);
        page.close().await.unwrap();
    }
}
