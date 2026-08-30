use std::{collections::BTreeMap, time::Duration};

use oryn_common::agent::{Intent, IntentRequest};
use oryn_common::v3::{
    ActionResult, CapabilityDiagnostic, ExecutionDomain, Observation, ObservationDelta,
    PageDescriptor, PageId, PagePredicate, ProjectionProfile, Revision, SemanticAction,
    SemanticRef, SupportLevel,
};
use oryn_common::{
    protocol::{Element, ElementState, Rect},
    resolver::{ResolutionStrategy, ResolverContext, Target as ResolverTarget, resolve_target},
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
    geometry: BTreeMap<SemanticRef, Rect>,
    delta_cursor: oryn_common::v3::Revision,
    viewport: oryn_common::v4::ViewportState,
    media: oryn_common::v4::MediaState,
}

/// Parent-side OIL session for a complete browser context. Page-local OIL
/// state remains isolated per page while page selection and lifecycle are
/// coordinated outside sandboxed workers.
pub struct NativeContextOilSession {
    context: BrowserContext,
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
}

impl NativeContextOilSession {
    pub fn new(context: BrowserContext) -> Self {
        Self {
            context,
            sessions: BTreeMap::new(),
        }
    }

    pub async fn execute(&mut self, input: &str) -> Result<Vec<NativeOilOutput>, NativeOilError> {
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
                let result = session.execute_upload(line, &self.context).await?;
                self.context.record_intent(
                    Some(page_id),
                    "upload [TARGET] [FILE_LABELS]".into(),
                    before,
                    self.revision(page_id),
                    "completed".into(),
                );
                outputs.push(NativeOilOutput::Action { result });
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
        match page.perform(intent).await {
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
                Err(error.into())
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
            geometry: BTreeMap::new(),
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
                outputs.push(self.execute_command(command).await?);
            }
        }
        Ok(outputs)
    }

    pub(crate) async fn resolve_typed_target(
        &mut self,
        target: &oryn_common::agent::Target,
        action: SemanticAction,
        force: bool,
    ) -> Result<SemanticRef, NativeOilError> {
        if let oryn_common::agent::Target::Reference { reference } = target {
            return Ok(*reference);
        }
        self.resolve_for_action(agent_target_to_oil(target), action, force)
            .await
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
            .resolve_for_action(target, SemanticAction::Type, false)
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
        let result = self.page.set_files(reference, &handles, None).await?;
        self.last_observation = Some(self.page.observe().await?);
        Ok(result)
    }

    async fn execute_command(
        &mut self,
        command: Command,
    ) -> Result<NativeOilOutput, NativeOilError> {
        match command {
            Command::Goto(command) => {
                let navigation = self.page.goto(command.url).await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Back => {
                let navigation = self.page.back().await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Forward => {
                let navigation = self.page.forward().await?;
                self.last_observation = None;
                self.delta_cursor = navigation.revision;
                Ok(NativeOilOutput::Navigation { navigation })
            }
            Command::Refresh(_) => {
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
                let observation = self.page.observe().await?;
                self.delta_cursor = observation.revision;
                self.last_observation = Some(observation);
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
                    let observation = self.page.observe_profile(ProjectionProfile::Full).await?;
                    let value = observation
                        .nodes
                        .iter()
                        .find(|node| target_matches_node(&target, node))
                        .map(|node| node.name.clone())
                        .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))?;
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
                    ExtractWhat::Meta => {
                        return Ok(NativeOilOutput::Unsupported {
                            diagnostic: CapabilityDiagnostic {
                                capability: "oil.extract.meta".into(),
                                support: SupportLevel::Unsupported,
                                alternatives: vec![ExecutionDomain::Chromium],
                                handoff_lossy: false,
                                detail: Some(
                                    "metadata extraction is outside the controlled corpus".into(),
                                ),
                            },
                        });
                    }
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
                    self.last_observation = Some(self.page.observe().await?);
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
                let observation = self.page.observe_profile(ProjectionProfile::Full).await?;
                self.last_observation = Some(observation.clone());
                let target = observation
                    .nodes
                    .iter()
                    .find(|node| node.actions.contains(&SemanticAction::Submit))
                    .or_else(|| {
                        observation
                            .nodes
                            .iter()
                            .find(|node| node.name.to_ascii_lowercase().contains("submit"))
                    })
                    .ok_or_else(|| {
                        NativeOilError::TargetNotFound(Target {
                            atomic: TargetAtomic::Role("form".into()),
                            relation: None,
                        })
                    })?;
                let result = self
                    .page
                    .execute(NativeAction {
                        target: target.semantic_ref,
                        action: if target.actions.contains(&SemanticAction::Submit) {
                            SemanticAction::Submit
                        } else {
                            SemanticAction::Click
                        },
                        value: None,
                    })
                    .await?;
                self.last_observation = Some(self.page.observe().await?);
                Ok(NativeOilOutput::Action { result })
            }
            Command::Wait(command) => {
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
                        self.assert_target_visibility(&target, true).await?;
                        return Ok(NativeOilOutput::Text {
                            value: "visible".into(),
                        });
                    }
                    WaitCondition::Hidden(target) => {
                        self.assert_target_visibility(&target, false).await?;
                        return Ok(NativeOilOutput::Text {
                            value: "hidden".into(),
                        });
                    }
                    WaitCondition::Until(_) | WaitCondition::Items { .. } => {
                        return Ok(NativeOilOutput::Unsupported {
                            diagnostic: CapabilityDiagnostic {
                                capability: "oil.wait.dynamic_predicate".into(),
                                support: SupportLevel::Unsupported,
                                alternatives: vec![ExecutionDomain::Chromium],
                                handoff_lossy: true,
                                detail: Some(
                                    "arbitrary JS and counted-item waits are not implemented"
                                        .into(),
                                ),
                            },
                        });
                    }
                };
                self.page.wait_for(predicate).await?;
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
                let observation = self.page.observe().await?;
                self.last_observation = Some(observation.clone());
                let preferred = ["close", "cancel", "dismiss"];
                let target = observation
                    .nodes
                    .iter()
                    .find(|node| {
                        node.actions.contains(&SemanticAction::Click)
                            && preferred
                                .iter()
                                .any(|name| node.name.to_ascii_lowercase().contains(name))
                    })
                    .or_else(|| {
                        observation.nodes.iter().find(|node| {
                            node.actions.contains(&SemanticAction::Click)
                                && matches!(node.name.trim(), "" | "×" | "x" | "X")
                        })
                    })
                    .ok_or(NativeOilError::TargetNotFound(Target {
                        atomic: TargetAtomic::Text(command.target),
                        relation: None,
                    }))?;
                let result = self
                    .page
                    .execute(NativeAction {
                        target: target.semantic_ref,
                        action: SemanticAction::Click,
                        value: None,
                    })
                    .await?;
                self.last_observation = Some(self.page.observe().await?);
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
        if let Some(observation) = &self.last_observation {
            return Ok(observation.clone());
        }
        let observation = self.page.observe().await?;
        self.last_observation = Some(observation.clone());
        Ok(observation)
    }

    async fn assert_target_visibility(
        &mut self,
        target: &Target,
        expected_visible: bool,
    ) -> Result<(), NativeOilError> {
        if let TargetAtomic::Selector { value, .. } = &target.atomic {
            let predicate = if expected_visible {
                WaitPredicate::SelectorVisible(value.clone())
            } else {
                WaitPredicate::SelectorHidden(value.clone())
            };
            for attempt in 0..500 {
                match self.page.wait_for(predicate.clone()).await {
                    Ok(()) => {
                        self.last_observation = Some(self.page.observe().await?);
                        return Ok(());
                    }
                    Err(RuntimeError::WaitUnsatisfied(_)) => {}
                    Err(error) => return Err(error.into()),
                }
                let _ = self.page.evaluate("__oryn_drain(true);''").await;
                if attempt < 499 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            return Err(NativeOilError::TargetNotFound(target.clone()));
        }
        // Text/role waits must tolerate genuinely asynchronous resource and
        // Fetch completions. Poll the semantic projection while advancing one
        // scheduler checkpoint; a fixed two-pass drain races loopback and HTTP/2.
        for attempt in 0..500 {
            let observation = self.page.observe_profile(ProjectionProfile::Full).await?;
            let found = observation.nodes.iter().find(|node| match &target.atomic {
                TargetAtomic::Id(alias) => node.alias as usize == *alias,
                TargetAtomic::Text(text) => node
                    .name
                    .to_ascii_lowercase()
                    .contains(&text.to_ascii_lowercase()),
                TargetAtomic::Role(role) => node.role.eq_ignore_ascii_case(role),
                TargetAtomic::Selector { value, .. } => {
                    node.selector.as_deref() == Some(value.as_str())
                }
            });
            let visible = found.is_some_and(|node| {
                node.states.contains(&oryn_common::v3::NodeState::Visible)
                    && !node.states.contains(&oryn_common::v3::NodeState::Hidden)
            });
            self.last_observation = Some(observation);
            if visible == expected_visible {
                return Ok(());
            }
            let _ = self.page.evaluate("__oryn_drain(true);''").await;
            if attempt < 499 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
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
            Err(RuntimeError::StaleSemanticRef)
                if !matches!(&target.atomic, TargetAtomic::Id(_)) =>
            {
                let recovered = self.resolve_for_action(target, action, force).await?;
                if force {
                    self.page.execute_forced(execute(recovered, value)).await?
                } else {
                    self.page.execute(execute(recovered, value)).await?
                }
            }
            result => result?,
        };
        self.last_observation = Some(self.page.observe().await?);
        Ok(NativeOilOutput::Action { result })
    }

    async fn resolve_for_action(
        &mut self,
        target: Target,
        action: SemanticAction,
        force: bool,
    ) -> Result<SemanticRef, NativeOilError> {
        // Compact observations intentionally omit non-actionable landmarks. A
        // relational target (for example `"+" near "Wireless Mouse"`) needs
        // those landmarks, so resolve it against a fresh full projection.
        if !matches!(&target.atomic, TargetAtomic::Selector { .. }) {
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
        }
        Ok(if force {
            self.resolve_any_target(&target).await?
        } else if let TargetAtomic::Selector { value, .. } = &target.atomic {
            self.page.resolve_selector(value, action).await?
        } else {
            self.resolve_target(&target, action)?
        })
    }

    fn cache_v4_observation(&mut self, observation: &oryn_common::v4::Observation) {
        self.geometry = observation
            .nodes
            .iter()
            .filter_map(|node| {
                let rect = node.geometry.as_ref()?.box_model.as_ref()?.border_box;
                Some((
                    node.semantic.semantic_ref,
                    Rect {
                        x: rect.x as f32,
                        y: rect.y as f32,
                        width: rect.width as f32,
                        height: rect.height as f32,
                    },
                ))
            })
            .collect();
        self.last_observation = Some(observation.to_v3_compat());
    }

    async fn resolve_any_target(&mut self, target: &Target) -> Result<SemanticRef, NativeOilError> {
        let observation = self
            .page
            .observe_with(oryn_common::v4::ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await?;
        let found = observation
            .nodes
            .iter()
            .filter(|node| match &target.atomic {
                TargetAtomic::Id(alias) => node.semantic.alias as usize == *alias,
                TargetAtomic::Text(text) => node
                    .semantic
                    .name
                    .to_ascii_lowercase()
                    .contains(&text.to_ascii_lowercase()),
                TargetAtomic::Role(role) => node.semantic.role.eq_ignore_ascii_case(role),
                TargetAtomic::Selector { value, .. } => {
                    node.semantic.selector.as_deref() == Some(value.as_str())
                }
            });
        let found = found.min_by_key(|node| match &target.atomic {
            TargetAtomic::Text(text) => {
                let exact = node.semantic.name.eq_ignore_ascii_case(text);
                (!exact, node.semantic.document_order)
            }
            _ => (false, node.semantic.document_order),
        });
        let reference = found
            .map(|node| node.semantic.semantic_ref)
            .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))?;
        self.cache_v4_observation(&observation);
        Ok(reference)
    }

    fn resolve_target(
        &self,
        target: &Target,
        action: SemanticAction,
    ) -> Result<SemanticRef, NativeOilError> {
        let observation = self
            .last_observation
            .as_ref()
            .ok_or(NativeOilError::NoObservation)?;
        if target.relation.is_none()
            && let TargetAtomic::Text(text) = &target.atomic
            && let Some(exact) = observation
                .nodes
                .iter()
                .filter(|node| {
                    node.actions.contains(&action)
                        && node.name.trim().eq_ignore_ascii_case(text.trim())
                })
                .min_by_key(|node| {
                    let native_control = matches!(
                        node.role.as_str(),
                        "button"
                            | "link"
                            | "checkbox"
                            | "radio"
                            | "switch"
                            | "textbox"
                            | "searchbox"
                            | "combobox"
                            | "listbox"
                            | "option"
                    );
                    (!native_control, node.document_order)
                })
        {
            return Ok(exact.semantic_ref);
        }
        if self.geometry.is_empty() {
            return resolve_structural_target(observation, target, action);
        }
        let elements = observation
            .nodes
            .iter()
            .filter_map(|node| {
                // Geometry can be unavailable for an explicitly unsupported
                // subtree (for example table layout) while unrelated controls
                // on the same page remain actionable. Keep those nodes out of
                // the geometry-aware resolver instead of making one bounded
                // capability limitation poison every target on the page.
                let rect = self.geometry.get(&node.semantic_ref).cloned()?;
                let mut attributes = std::collections::HashMap::new();
                if let Some(id) = node
                    .selector
                    .as_deref()
                    .and_then(|value| value.strip_prefix('#'))
                {
                    attributes.insert("id".into(), id.into());
                }
                attributes.insert("role".into(), node.role.clone());
                Some(Element {
                    id: node.alias,
                    element_type: node.role.clone(),
                    role: Some(node.role.clone()),
                    text: Some(
                        node.description
                            .clone()
                            .unwrap_or_else(|| node.name.clone()),
                    ),
                    // Semantic names already apply the DOM accessible-name rules
                    // (label[for], wrapping label, aria-label, placeholder, and
                    // element text). Expose that result to the shared resolver as
                    // both visible text and label so PreferInput can select a
                    // labelled control instead of the label element itself.
                    label: (!node.actions.is_empty()).then(|| node.name.clone()),
                    value: node.value.clone(),
                    placeholder: None,
                    selector: node.selector.clone().unwrap_or_default(),
                    xpath: None,
                    rect,
                    attributes,
                    state: ElementState {
                        visible: true,
                        disabled: node.states.contains(&oryn_common::v3::NodeState::Disabled),
                        checked: node.states.contains(&oryn_common::v3::NodeState::Checked),
                        selected: node.states.contains(&oryn_common::v3::NodeState::Selected),
                        required: node.states.contains(&oryn_common::v3::NodeState::Required),
                        readonly: node.states.contains(&oryn_common::v3::NodeState::Readonly),
                        expanded: node.states.contains(&oryn_common::v3::NodeState::Expanded),
                        value: node.value.clone(),
                        ..Default::default()
                    },
                    children: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        let context = ResolverContext::from_elements(observation.page.url.clone(), elements);
        let strategy = match action {
            SemanticAction::Type | SemanticAction::Clear | SemanticAction::Select => {
                ResolutionStrategy::PreferInput
            }
            SemanticAction::Check | SemanticAction::Uncheck => ResolutionStrategy::PreferCheckable,
            _ => ResolutionStrategy::PreferClickable,
        };
        let resolver_target = normalize_symbolic_control_target(target);
        let resolved = resolve_target(&resolver_target.to_resolver_target(), &context, strategy)
            .map_err(|error| NativeOilError::TargetResolution {
                target: target.clone(),
                detail: error.to_string(),
            })?;
        let alias = match resolved {
            ResolverTarget::Id(alias) => alias,
            ResolverTarget::Selector(selector) => observation
                .nodes
                .iter()
                .find(|node| {
                    node.selector.as_deref() == Some(selector.as_str())
                        && node.actions.contains(&action)
                })
                .map(|node| node.alias as usize)
                .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))?,
            _ => return Err(NativeOilError::TargetNotFound(target.clone())),
        };
        if let Some(reference) = observation
            .nodes
            .iter()
            .find(|node| node.alias as usize == alias && node.actions.contains(&action))
            .map(|node| node.semantic_ref)
        {
            return Ok(reference);
        }

        // The shared geometric resolver may prefer a non-actionable text node
        // whose accessible text duplicates a control label (common in help
        // copy such as `Click "Show Button"`). Preserve its spatial ranking
        // when it selects an actionable node, but fall back to the semantic
        // action filter instead of reporting the landmark as the target.
        resolve_structural_target(observation, target, action).map_err(|_| {
            NativeOilError::TargetResolution {
                target: target.clone(),
                detail: format!("resolver selected non-actionable alias {alias}"),
            }
        })
    }
}

fn agent_target_to_oil(target: &oryn_common::agent::Target) -> Target {
    use oryn_common::agent::Target as AgentTarget;
    let (atomic, relation) = match target {
        AgentTarget::Reference { .. } => unreachable!("references bypass OIL resolution"),
        AgentTarget::Alias { alias } => (TargetAtomic::Id(*alias as usize), None),
        AgentTarget::Text { text } => (TargetAtomic::Text(text.clone()), None),
        AgentTarget::Role { role } => (TargetAtomic::Role(role.clone()), None),
        AgentTarget::Css { selector } => (
            TargetAtomic::Selector {
                kind: "css".into(),
                value: selector.clone(),
            },
            None,
        ),
        AgentTarget::XPath { expression } => (
            TargetAtomic::Selector {
                kind: "xpath".into(),
                value: expression.clone(),
            },
            None,
        ),
        AgentTarget::Near { target, anchor } => agent_relation(target, RelationKind::Near, anchor),
        AgentTarget::Inside { target, container } => {
            agent_relation(target, RelationKind::Inside, container)
        }
        AgentTarget::After { target, anchor } => {
            agent_relation(target, RelationKind::After, anchor)
        }
        AgentTarget::Before { target, anchor } => {
            agent_relation(target, RelationKind::Before, anchor)
        }
        AgentTarget::Contains { target, content } => {
            agent_relation(target, RelationKind::Contains, content)
        }
    };
    Target { atomic, relation }
}

fn agent_relation(
    target: &oryn_common::agent::Target,
    kind: RelationKind,
    related: &oryn_common::agent::Target,
) -> (TargetAtomic, Option<Box<oryn_core::ast::TargetRelation>>) {
    let target = agent_target_to_oil(target);
    (
        target.atomic,
        Some(Box::new(oryn_core::ast::TargetRelation {
            kind,
            target: agent_target_to_oil(related),
        })),
    )
}

fn resolve_structural_target(
    observation: &Observation,
    target: &Target,
    action: SemanticAction,
) -> Result<SemanticRef, NativeOilError> {
    let matches_atomic =
        |node: &oryn_common::v3::SemanticNodeView, atomic: &TargetAtomic| match atomic {
            TargetAtomic::Id(alias) => node.alias as usize == *alias,
            TargetAtomic::Text(text) => node
                .name
                .to_ascii_lowercase()
                .contains(&text.to_ascii_lowercase()),
            TargetAtomic::Role(role) => node.role.eq_ignore_ascii_case(role),
            TargetAtomic::Selector { value, .. } => {
                node.selector.as_deref() == Some(value.as_str())
            }
        };
    let related = target.relation.as_ref().and_then(|relation| {
        observation
            .nodes
            .iter()
            .find(|node| matches_atomic(node, &relation.target.atomic))
    });
    let contains = |container: SemanticRef, candidate: &oryn_common::v3::SemanticNodeView| {
        let by_ref = observation
            .nodes
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        let mut parent = candidate.parent;
        while let Some(reference) = parent {
            if reference == container {
                return true;
            }
            parent = by_ref.get(&reference).and_then(|node| node.parent);
        }
        false
    };
    observation
        .nodes
        .iter()
        .filter(|node| node.actions.contains(&action) && matches_atomic(node, &target.atomic))
        .find(|node| match (target.relation.as_deref(), related) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(relation), Some(anchor)) => match relation.kind {
                RelationKind::Before => node.document_order < anchor.document_order,
                RelationKind::After => node.document_order > anchor.document_order,
                RelationKind::Inside => contains(anchor.semantic_ref, node),
                RelationKind::Contains => contains(node.semantic_ref, anchor),
                RelationKind::Near => true,
            },
        })
        .map(|node| node.semantic_ref)
        .ok_or_else(|| NativeOilError::TargetNotFound(target.clone()))
}

fn normalize_symbolic_control_target(target: &Target) -> Target {
    let mut normalized = target.clone();
    if let TargetAtomic::Text(text) = &target.atomic {
        normalized.atomic = match text.trim() {
            "+" => TargetAtomic::Text("Increase quantity".into()),
            "-" | "−" => TargetAtomic::Text("Decrease quantity".into()),
            _ => normalized.atomic,
        };
    }
    normalized
}

fn target_matches_node(target: &Target, node: &oryn_common::v3::SemanticNodeView) -> bool {
    match &target.atomic {
        TargetAtomic::Id(alias) => node.alias as usize == *alias,
        TargetAtomic::Text(text) => node
            .name
            .to_ascii_lowercase()
            .contains(&text.to_ascii_lowercase()),
        TargetAtomic::Role(role) => node.role.eq_ignore_ascii_case(role),
        TargetAtomic::Selector { value, .. } => node.selector.as_deref() == Some(value.as_str()),
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
    async fn repeated_custom_element_renders_release_detached_wrapper_graphs() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/angular-shop",
            include_str!("../../../test-harness/scenarios/spa/angular-shop.html"),
        )
        .await
        .expect("load custom-element fixture");
        let mut session = NativeOilSession::new(page);
        for command in [
            "type css(#search) \"wireless\"",
            "type css(#search) \"\"",
            "click css(#sort-price)",
            "click css(#sort-rating)",
            "click css(#sort-name)",
        ] {
            session
                .execute(command)
                .await
                .unwrap_or_else(|error| panic!("{command} failed: {error}"));
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
}
