use std::{collections::BTreeMap, time::Duration};

use oryn_common::v3::{
    ActionResult, CapabilityDiagnostic, ExecutionDomain, Observation, ObservationDelta,
    PageDescriptor, PageId, PagePredicate, ProjectionProfile, SemanticAction, SemanticRef,
    SupportLevel,
};
use oryn_common::{
    protocol::{Element, ElementState, Rect},
    resolver::{ResolutionStrategy, ResolverContext, Target as ResolverTarget, resolve_target},
};
use oryn_core::ast::{Command, ExtractWhat, RelationKind, Target, TargetAtomic, WaitCondition};
use serde::{Deserialize, Serialize};

use crate::runtime::{
    BrowserContext, ContentKind, LifecycleState, NativeAction, NavigationResult, PageHandle,
    RuntimeError, ScrollRequest, WaitPredicate,
};

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
            if line == "pages" || line.starts_with("page ") {
                outputs.push(self.execute_page_command(line).await?);
                continue;
            }
            let page = self.context.active_page().ok_or_else(|| {
                NativeOilError::PageCommand("browser context has no active page".into())
            })?;
            let page_id = page.page_id();
            let session = self
                .sessions
                .entry(page_id)
                .or_insert_with(|| NativeOilSession::new(page));
            outputs.extend(session.execute(line).await?);
        }
        Ok(outputs)
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
            Command::Observe(command) if command.diff => {
                let from = self.delta_cursor;
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
                self.run_action(command.target, SemanticAction::Click, None)
                    .await
            }
            Command::Type(command) => {
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
            self.page
                .wait_for(if expected_visible {
                    WaitPredicate::SelectorVisible(value.clone())
                } else {
                    WaitPredicate::SelectorHidden(value.clone())
                })
                .await?;
            self.last_observation = Some(self.page.observe().await?);
            return Ok(());
        }
        for attempt in 0..2 {
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
            if attempt == 0 {
                // Advance one deterministic timer checkpoint only when the
                // predicate is not already satisfied (for debounced search,
                // slow-loading fixtures, and delayed recovery controls).
                let _ = self.page.evaluate("__oryn_drain(true);''").await;
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
        let semantic_ref = if let TargetAtomic::Selector { value, .. } = &target.atomic {
            self.page.resolve_selector(value, action).await?
        } else {
            self.resolve_target(&target, action)?
        };
        let result = self
            .page
            .execute(NativeAction {
                target: semantic_ref,
                action,
                value,
            })
            .await?;
        self.last_observation = Some(self.page.observe().await?);
        Ok(NativeOilOutput::Action { result })
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
        let found = observation.nodes.iter().find(|node| match &target.atomic {
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
        if self.geometry.is_empty() {
            return resolve_structural_target(observation, target, action);
        }
        let elements = observation
            .nodes
            .iter()
            .map(|node| {
                let mut attributes = std::collections::HashMap::new();
                if let Some(id) = node
                    .selector
                    .as_deref()
                    .and_then(|value| value.strip_prefix('#'))
                {
                    attributes.insert("id".into(), id.into());
                }
                attributes.insert("role".into(), node.role.clone());
                Ok(Element {
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
                    rect: self
                        .geometry
                        .get(&node.semantic_ref)
                        .cloned()
                        .ok_or_else(|| NativeOilError::TargetResolution {
                            target: target.clone(),
                            detail: format!(
                                "geometry is unavailable for semantic alias {}",
                                node.alias
                            ),
                        })?,
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
            .collect::<Result<Vec<_>, NativeOilError>>()?;
        let context = ResolverContext::from_elements(observation.page.url.clone(), elements);
        let strategy = match action {
            SemanticAction::Type | SemanticAction::Clear | SemanticAction::Select => {
                ResolutionStrategy::PreferInput
            }
            SemanticAction::Check | SemanticAction::Uncheck => ResolutionStrategy::PreferCheckable,
            _ => ResolutionStrategy::PreferClickable,
        };
        let resolved =
            resolve_target(&target.to_resolver_target(), &context, strategy).map_err(|error| {
                NativeOilError::TargetResolution {
                    target: target.clone(),
                    detail: error.to_string(),
                }
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
        observation
            .nodes
            .iter()
            .find(|node| node.alias as usize == alias && node.actions.contains(&action))
            .map(|node| node.semantic_ref)
            .ok_or_else(|| NativeOilError::TargetResolution {
                target: target.clone(),
                detail: format!("resolver selected non-actionable alias {alias}"),
            })
    }
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
}
