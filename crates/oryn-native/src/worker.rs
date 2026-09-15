use std::{
    io::{BufRead, Write},
    sync::OnceLock,
    time::Duration,
};

use oryn_common::v3::{
    Observation, ObservationDelta, PageId, PageInfo, ProjectionProfile, Revision, SemanticRef,
};
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use crate::context_service::SharedContextClient;
use crate::{
    Browser, BrowserConfig, ContentKind, ContextOptions, NavigationResult, PageHandle,
    WaitPredicate,
    network::SharedNetworkBroker,
    oil::{NativeOilError, NativeOilOutput, NativeOilSession},
    runtime::NativeAction,
};

pub const WORKER_PROTOCOL_VERSION: u32 = 10;
use crate::worker_route::WorkerRoute;
pub const MAX_IPC_FRAME_BYTES: usize = 24 * 1024 * 1024;
static WORKER_BINARY_SHA256: OnceLock<String> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerLimits {
    pub runtime_profile: crate::runtime::RuntimeProfile,
    pub v8_heap_bytes: usize,
    pub worker_rss_bytes: usize,
    pub ipc_frame_bytes: usize,
    pub command_wall_ms: u64,
    pub navigation_wall_ms: u64,
    pub queue_capacity: usize,
    pub max_layout_nodes: usize,
    pub max_font_bytes: usize,
    pub max_page_font_bytes: usize,
    pub max_viewport_css_px: u32,
    pub max_resource_cache_entries: usize,
    pub max_resource_cache_bytes: usize,
    pub max_resource_cache_entry_bytes: usize,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self::from(&BrowserConfig::default())
    }
}

impl From<&BrowserConfig> for WorkerLimits {
    fn from(config: &BrowserConfig) -> Self {
        Self {
            runtime_profile: config.runtime_profile,
            v8_heap_bytes: config.max_v8_heap_bytes,
            worker_rss_bytes: config.max_worker_rss_bytes,
            ipc_frame_bytes: config.max_ipc_frame_bytes,
            command_wall_ms: config.command_timeout.as_millis() as u64,
            navigation_wall_ms: config.navigation_timeout.as_millis() as u64,
            queue_capacity: config.worker_queue_capacity,
            max_layout_nodes: config.max_layout_nodes,
            max_font_bytes: config.max_font_bytes,
            max_page_font_bytes: config.max_page_font_bytes,
            max_viewport_css_px: config.max_viewport_css_px,
            max_resource_cache_entries: config.max_resource_cache_entries,
            max_resource_cache_bytes: config.max_resource_cache_bytes,
            max_resource_cache_entry_bytes: config.max_resource_cache_entry_bytes,
        }
    }
}

impl WorkerLimits {
    pub fn browser_config(&self) -> BrowserConfig {
        BrowserConfig {
            runtime_profile: self.runtime_profile,
            worker_lifecycle: crate::runtime::WorkerLifecycle::IsolatedOnDemand,
            worker_queue_capacity: self.queue_capacity.min(32),
            request_timeout: Duration::from_millis(self.navigation_wall_ms.min(30_000)),
            max_response_bytes: 16 * 1024 * 1024,
            max_redirects: 10,
            max_v8_heap_bytes: self.v8_heap_bytes.min(256 * 1024 * 1024),
            max_worker_rss_bytes: self.worker_rss_bytes.min(512 * 1024 * 1024),
            max_ipc_frame_bytes: self.ipc_frame_bytes.min(MAX_IPC_FRAME_BYTES),
            command_timeout: Duration::from_millis(self.command_wall_ms.min(10_000)),
            navigation_timeout: Duration::from_millis(self.navigation_wall_ms.min(30_000)),
            document_generation_base: 0,
            storage_limits: crate::storage::StorageLimits::default(),
            max_layout_nodes: self.max_layout_nodes.min(100_000),
            max_font_bytes: self.max_font_bytes.min(8 * 1024 * 1024),
            max_page_font_bytes: self.max_page_font_bytes.min(32 * 1024 * 1024),
            max_viewport_css_px: self.max_viewport_css_px.min(16_384),
            max_revision_snapshots: 128,
            max_revision_bytes: 16 * 1024 * 1024,
            max_resource_cache_entries: self.max_resource_cache_entries.min(256),
            max_resource_cache_bytes: self.max_resource_cache_bytes.min(32 * 1024 * 1024),
            max_resource_cache_entry_bytes: self
                .max_resource_cache_entry_bytes
                .min(8 * 1024 * 1024),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostEnvelope {
    pub protocol_version: u32,
    pub route: Option<WorkerRoute>,
    pub session_id: u64,
    pub message_id: u64,
    pub command: WorkerRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEnvelope {
    pub protocol_version: u32,
    pub route: Option<WorkerRoute>,
    pub session_id: u64,
    pub message_id: u64,
    pub correlation_id: u64,
    pub response: WorkerResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkerRequest {
    Initialize {
        nonce: String,
        limits: WorkerLimits,
        context: Box<ContextOptions>,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        document_generation_base: u64,
    },
    Ping,
    Goto {
        url: String,
    },
    GotoResource {
        url: String,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
    },
    Back,
    Forward,
    Refresh,
    LoadHtml {
        url: String,
        html: String,
    },
    WaitFor {
        predicate: WaitPredicate,
    },
    Evaluate {
        source: String,
    },
    Capabilities,
    ChallengeAssessment,
    ResolveSelector {
        selector: String,
        action: oryn_common::v3::SemanticAction,
    },
    ResolveSemantic {
        target: oryn_common::agent::Target,
        action: oryn_common::v3::SemanticAction,
    },
    Extract {
        kind: ContentKind,
        selector: Option<String>,
    },
    Observe {
        profile: Option<ProjectionProfile>,
    },
    ObserveWith {
        options: oryn_common::v4::ObservationOptions,
    },
    ObserveSemantic {
        profile: ProjectionProfile,
    },
    ObserveSemanticV6 {
        options: oryn_common::v6::SemanticObservationOptions,
    },
    ComputedStyle {
        target: SemanticRef,
    },
    BoxModel {
        target: SemanticRef,
    },
    PaintSnapshot,
    SetViewport {
        viewport: oryn_common::v4::ViewportState,
    },
    SetMedia {
        media: oryn_common::v4::MediaState,
    },
    Scroll {
        request: crate::runtime::ScrollRequest,
    },
    ScrollIntoView {
        target: SemanticRef,
    },
    ObserveDelta {
        from: Revision,
    },
    ObserveSemanticDelta {
        from: Revision,
    },
    Execute {
        action: NativeActionRequest,
    },
    ExecuteBatch {
        actions: Vec<NativeActionRequest>,
    },
    SetFiles {
        target: SemanticRef,
        files: Vec<crate::upload::UploadFile>,
    },
    Oil {
        input: String,
    },
    Trace,
    Reset {
        nonce: String,
    },
    Close,
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeActionRequest {
    pub target: SemanticRef,
    pub action: oryn_common::v3::SemanticAction,
    pub value: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkerResponse {
    Ready {
        nonce: String,
        protocol_version: u32,
        runtime: Box<crate::RuntimeBuildInfo>,
        binary_sha256: String,
        policy_digest: String,
        sandbox_state: String,
        secret_environment_present: bool,
        limits: WorkerLimits,
        code_identity: WorkerCodeIdentity,
    },
    ResetReady {
        nonce: String,
        code_identity: WorkerCodeIdentity,
    },
    Pong {
        protocol_version: u32,
    },
    Loaded {
        revision: Revision,
        page: PageInfo,
    },
    Navigated {
        navigation: NavigationResult,
    },
    Observation {
        observation: Observation,
    },
    ObservationV4 {
        observation: oryn_common::v4::Observation,
    },
    SemanticObservation {
        observation: oryn_common::v5::SemanticObservation,
    },
    SemanticObservationV6 {
        observation: oryn_common::v6::SemanticObservation,
    },
    ComputedStyle {
        style: oryn_common::v4::ComputedStyle,
    },
    BoxModel {
        box_model: oryn_common::v4::BoxModel,
    },
    PaintSnapshot {
        snapshot: Box<oryn_common::v4::PaintSnapshot>,
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
    Delta {
        delta: ObservationDelta,
    },
    SemanticDelta {
        delta: oryn_common::v6::SemanticDelta,
    },
    Action {
        result: oryn_common::v3::ActionResult,
    },
    Actions {
        results: Vec<oryn_common::v3::ActionResult>,
    },
    Oil {
        outputs: Vec<NativeOilOutput>,
    },
    Trace {
        events: Vec<crate::trace::TraceEvent>,
    },
    Capabilities {
        capabilities: Vec<oryn_common::v3::CapabilityDiagnostic>,
    },
    ChallengeAssessment {
        assessment: crate::challenge::ChallengeAssessment,
    },
    SemanticRef {
        semantic_ref: SemanticRef,
    },
    Resolution {
        result: oryn_common::v6::ResolutionResult,
    },
    Structured {
        value: serde_json::Value,
    },
    Value {
        value: String,
    },
    Satisfied,
    Closed,
    Exiting,
    Error {
        error: Box<crate::runtime::RuntimeError>,
    },
    LimitExceeded {
        resource: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerCodeIdentity {
    pub process_id: u32,
    pub binary_sha256: String,
    pub code_directory_hash: Option<String>,
}

struct PageSession {
    page: PageHandle,
    oil: NativeOilSession,
}

impl PageSession {
    fn new(
        config: BrowserConfig,
        context: ContextOptions,
        broker: Option<SharedNetworkBroker>,
        #[cfg(unix)] context_client: Option<SharedContextClient>,
        page_id: PageId,
        opener_page_id: Option<PageId>,
    ) -> Self {
        let browser = Browser::in_process_with_config(config);
        #[cfg(all(unix, not(feature = "v8-host")))]
        let _ = &context_client;
        let page = match broker {
            #[cfg(all(unix, feature = "v8-host"))]
            Some(broker) if context_client.is_some() => browser
                .new_context_with_broker_and_services(
                    context,
                    broker,
                    context_client.expect("checked context client"),
                )
                .new_page_with_identity(page_id, opener_page_id),
            Some(broker) => browser
                .new_context_with_broker(context, broker)
                .new_page_with_identity(page_id, opener_page_id),
            None => browser
                .new_context_with_options(context)
                .new_page_with_identity(page_id, opener_page_id),
        };
        let oil = NativeOilSession::new(page.clone());
        Self { page, oil }
    }
}

pub async fn serve<R: BufRead, W: Write>(reader: R, writer: W) -> std::io::Result<()> {
    serve_with_services(reader, writer, None, None).await
}

pub async fn serve_with_broker<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    broker: Option<SharedNetworkBroker>,
) -> std::io::Result<()> {
    serve_with_services(reader, writer, broker, None).await
}

pub async fn serve_with_services<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    broker: Option<SharedNetworkBroker>,
    #[cfg(unix)] context_client: Option<SharedContextClient>,
) -> std::io::Result<()> {
    let mut session: Option<PageSession> = None;
    let mut active_session_id: Option<u64> = None;
    let mut active_route: Option<WorkerRoute> = None;
    let mut last_message_id = 0_u64;
    loop {
        let mut line = Vec::new();
        let mut limited =
            std::io::Read::take(&mut reader, MAX_IPC_FRAME_BYTES.saturating_add(1) as u64);
        let read = limited.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > MAX_IPC_FRAME_BYTES {
            write_legacy(
                &mut writer,
                &WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::WorkerLimitExceeded {
                        resource: "IPC frame".into(),
                    }),
                },
            )?;
            break;
        }

        if let Ok(envelope) = serde_json::from_slice::<HostEnvelope>(&line) {
            let response = if envelope.protocol_version != WORKER_PROTOCOL_VERSION {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "unsupported worker protocol version".into(),
                    )),
                }
            } else if !envelope.route.as_ref().is_some_and(|route| {
                route.process_session == envelope.session_id
                    && match &envelope.command {
                        WorkerRequest::Initialize {
                            page_id,
                            document_generation_base,
                            ..
                        } => {
                            route.environment == page_id.0
                                && route.generation == *document_generation_base
                        }
                        _ => active_route.as_ref() == Some(route),
                    }
            }) {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "missing or stale worker route authority".into(),
                    )),
                }
            } else if active_session_id.is_some_and(|session_id| session_id != envelope.session_id)
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "worker session identifier changed".into(),
                    )),
                }
            } else if envelope.message_id <= last_message_id {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "duplicate or out-of-order worker message".into(),
                    )),
                }
            } else if active_session_id.is_some()
                && matches!(&envelope.command, WorkerRequest::Initialize { .. })
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "worker session is already initialized".into(),
                    )),
                }
            } else {
                last_message_id = envelope.message_id;
                dispatch(
                    &mut session,
                    broker.clone().map(|broker| {
                        broker.for_route(envelope.route.clone().expect("validated route"))
                    }),
                    #[cfg(unix)]
                    context_client.as_ref().map(|client| {
                        client.for_route(envelope.route.clone().expect("validated route"))
                    }),
                    envelope.command,
                )
                .await
            };
            if matches!(response, WorkerResponse::Ready { .. }) {
                active_session_id = Some(envelope.session_id);
                active_route = envelope.route.clone();
            }
            let reset = matches!(response, WorkerResponse::ResetReady { .. });
            let exiting = matches!(response, WorkerResponse::Exiting);
            let response = WorkerEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                route: envelope.route,
                session_id: envelope.session_id,
                message_id: envelope.message_id,
                correlation_id: envelope.message_id,
                response,
            };
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            if reset {
                active_session_id = None;
                active_route = None;
                last_message_id = 0;
            }
            if exiting {
                break;
            }
            continue;
        }

        // Protocol-v1 compatibility is intentionally limited to diagnostics.
        let response = match serde_json::from_slice::<WorkerRequest>(&line) {
            Ok(WorkerRequest::Ping) => WorkerResponse::Pong {
                protocol_version: WORKER_PROTOCOL_VERSION,
            },
            Ok(WorkerRequest::Exit) => WorkerResponse::Exiting,
            Ok(_) => WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(format!(
                    "page commands require worker protocol v{} initialization",
                    WORKER_PROTOCOL_VERSION
                ))),
            },
            Err(error) => WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(format!(
                    "invalid worker request: {error}"
                ))),
            },
        };
        let exiting = matches!(response, WorkerResponse::Exiting);
        write_legacy(&mut writer, &response)?;
        if exiting {
            break;
        }
    }
    if let Some(session) = session {
        let _ = session.page.close().await;
    }
    Ok(())
}

fn write_legacy(writer: &mut impl Write, response: &WorkerResponse) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, response)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

async fn dispatch(
    session: &mut Option<PageSession>,
    broker: Option<SharedNetworkBroker>,
    #[cfg(unix)] context_client: Option<SharedContextClient>,
    request: WorkerRequest,
) -> WorkerResponse {
    if let WorkerRequest::Initialize {
        nonce,
        limits,
        context,
        page_id,
        opener_page_id,
        document_generation_base,
    } = request
    {
        if !limits.runtime_profile.available_in_build() {
            return WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::SandboxUnavailable(format!(
                    "runtime profile {} is not present in this worker build",
                    limits.runtime_profile.capability_name()
                ))),
            };
        }
        let policy_digest = policy_digest(&context);
        let effective = WorkerLimits::from(&limits.browser_config());
        let mut config = effective.browser_config();
        config.document_generation_base = document_generation_base;
        *session = Some(PageSession::new(
            config,
            *context,
            broker,
            #[cfg(unix)]
            context_client,
            page_id,
            opener_page_id,
        ));
        let runtime = crate::runtime_build_info();
        return WorkerResponse::Ready {
            nonce,
            protocol_version: WORKER_PROTOCOL_VERSION,
            runtime: Box::new(runtime),
            binary_sha256: current_binary_sha256().unwrap_or_default(),
            policy_digest,
            sandbox_state: if cfg!(target_os = "macos") {
                "app_sandbox".into()
            } else if cfg!(target_os = "linux") {
                "linux_namespaces_landlock_seccomp".into()
            } else {
                "process_isolated_unsandboxed".into()
            },
            secret_environment_present: secret_environment_present(),
            limits: effective,
            code_identity: WorkerCodeIdentity {
                process_id: std::process::id(),
                binary_sha256: current_binary_sha256().unwrap_or_default(),
                code_directory_hash: current_code_directory_hash(),
            },
        };
    }
    if matches!(request, WorkerRequest::Ping) {
        return WorkerResponse::Pong {
            protocol_version: WORKER_PROTOCOL_VERSION,
        };
    }
    if matches!(request, WorkerRequest::Exit) {
        return WorkerResponse::Exiting;
    }
    if let WorkerRequest::Reset { nonce } = request {
        let Some(active) = session.take() else {
            return WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                    "worker has not been initialized".into(),
                )),
            };
        };
        if let Err(value) = active.page.close().await {
            return error(value);
        }
        return WorkerResponse::ResetReady {
            nonce,
            code_identity: WorkerCodeIdentity {
                process_id: std::process::id(),
                binary_sha256: current_binary_sha256().unwrap_or_default(),
                code_directory_hash: current_code_directory_hash(),
            },
        };
    }
    let Some(session) = session.as_mut() else {
        return WorkerResponse::Error {
            error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                "worker has not been initialized".into(),
            )),
        };
    };
    match request {
        WorkerRequest::Goto { url } => navigation(session.page.goto(url).await),
        WorkerRequest::GotoResource {
            url,
            post,
            referrer_policy,
        } => navigation(session.page.goto_resource(url, post, referrer_policy).await),
        WorkerRequest::Back => navigation(session.page.back().await),
        WorkerRequest::Forward => navigation(session.page.forward().await),
        WorkerRequest::Refresh => navigation(session.page.refresh().await),
        WorkerRequest::LoadHtml { url, html } => match session.page.load_html(url, html).await {
            Ok(revision) => match session.page.observe().await {
                Ok(observation) => WorkerResponse::Loaded {
                    revision,
                    page: observation.page,
                },
                Err(value) => error(value),
            },
            Err(value) => error(value),
        },
        WorkerRequest::WaitFor { predicate } => match session.page.wait_for(predicate).await {
            Ok(()) => WorkerResponse::Satisfied,
            Err(value) => error(value),
        },
        WorkerRequest::Evaluate { source } => match session.page.evaluate(source).await {
            Ok(value) => WorkerResponse::Value { value },
            Err(value) => error(value),
        },
        WorkerRequest::Capabilities => match session.page.capabilities().await {
            Ok(capabilities) => WorkerResponse::Capabilities { capabilities },
            Err(value) => error(value),
        },
        WorkerRequest::ChallengeAssessment => match session.page.challenge_assessment().await {
            Ok(assessment) => WorkerResponse::ChallengeAssessment { assessment },
            Err(value) => error(value),
        },
        WorkerRequest::ResolveSelector { selector, action } => {
            match session.page.resolve_selector(selector, action).await {
                Ok(semantic_ref) => WorkerResponse::SemanticRef { semantic_ref },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ResolveSemantic { target, action } => {
            match session.page.resolve_semantic(target, action).await {
                Ok(result) => WorkerResponse::Resolution { result },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Extract { kind, selector } => {
            match session.page.extract(kind, selector).await {
                Ok(value) => WorkerResponse::Structured { value },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Observe { profile } => {
            match session
                .page
                .observe_profile(profile.unwrap_or(ProjectionProfile::Compact))
                .await
            {
                Ok(observation) => WorkerResponse::Observation { observation },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ObserveWith { options } => match session.page.observe_with(options).await {
            Ok(observation) => WorkerResponse::ObservationV4 { observation },
            Err(value) => error(value),
        },
        WorkerRequest::ObserveSemantic { profile } => {
            match session.page.observe_semantics(profile).await {
                Ok(observation) => WorkerResponse::SemanticObservation { observation },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ObserveSemanticV6 { options } => {
            match session.page.observe_semantics_v6(options).await {
                Ok(observation) => WorkerResponse::SemanticObservationV6 { observation },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ComputedStyle { target } => {
            match session.page.computed_style(target).await {
                Ok(style) => WorkerResponse::ComputedStyle { style },
                Err(value) => error(value),
            }
        }
        WorkerRequest::BoxModel { target } => match session.page.box_model(target).await {
            Ok(box_model) => WorkerResponse::BoxModel { box_model },
            Err(value) => error(value),
        },
        WorkerRequest::PaintSnapshot => match session.page.paint_snapshot().await {
            Ok(snapshot) => WorkerResponse::PaintSnapshot {
                snapshot: Box::new(snapshot),
            },
            Err(value) => error(value),
        },
        WorkerRequest::SetViewport { viewport } => {
            match session.page.set_viewport(viewport).await {
                Ok(viewport) => WorkerResponse::Viewport { viewport },
                Err(value) => error(value),
            }
        }
        WorkerRequest::SetMedia { media } => match session.page.set_media(media).await {
            Ok(media) => WorkerResponse::Media { media },
            Err(value) => error(value),
        },
        WorkerRequest::Scroll { request } => match session.page.scroll(request).await {
            Ok(scroll) => WorkerResponse::Scroll { scroll },
            Err(value) => error(value),
        },
        WorkerRequest::ScrollIntoView { target } => {
            match session.page.scroll_into_view(target).await {
                Ok(scroll) => WorkerResponse::Scroll { scroll },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ObserveDelta { from } => match session.page.observe_delta(from).await {
            Ok(delta) => WorkerResponse::Delta { delta },
            Err(value) => error(value),
        },
        WorkerRequest::ObserveSemanticDelta { from } => {
            match session.page.observe_semantic_delta(from).await {
                Ok(delta) => WorkerResponse::SemanticDelta { delta },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Execute { action } => {
            let native_action = NativeAction {
                target: action.target,
                action: action.action,
                value: action.value,
            };
            let result = if action.sensitive {
                session.page.execute_sensitive(native_action).await
            } else if action.force {
                session.page.execute_forced(native_action).await
            } else {
                session.page.execute(native_action).await
            };
            match result {
                Ok(result) => WorkerResponse::Action { result },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ExecuteBatch { actions } => {
            if actions.len() > 10_000 {
                return WorkerResponse::LimitExceeded {
                    resource: "action batch".into(),
                };
            }
            let mut results = Vec::with_capacity(actions.len());
            for action in actions {
                let native_action = NativeAction {
                    target: action.target,
                    action: action.action,
                    value: action.value,
                };
                let result = if action.sensitive {
                    session.page.execute_sensitive(native_action).await
                } else if action.force {
                    session.page.execute_forced(native_action).await
                } else {
                    session.page.execute(native_action).await
                };
                match result {
                    Ok(result) => results.push(result),
                    Err(value) => return error(value),
                }
            }
            WorkerResponse::Actions { results }
        }
        WorkerRequest::SetFiles { target, files } => {
            match session.page.set_files_materialized(target, files).await {
                Ok(result) => WorkerResponse::Action { result },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Oil { input } => match session.oil.execute(&input).await {
            Ok(outputs) => WorkerResponse::Oil { outputs },
            Err(NativeOilError::Runtime(value)) => error(value),
            Err(value) => error(crate::runtime::RuntimeError::Script(value.to_string())),
        },
        WorkerRequest::Trace => match session.page.trace().await {
            Ok(events) => WorkerResponse::Trace { events },
            Err(value) => error(value),
        },
        WorkerRequest::Close => match session.page.close().await {
            Ok(()) => WorkerResponse::Closed,
            Err(value) => error(value),
        },
        WorkerRequest::Initialize { .. }
        | WorkerRequest::Ping
        | WorkerRequest::Reset { .. }
        | WorkerRequest::Exit => {
            unreachable!("handled before session dispatch")
        }
    }
}

fn navigation(result: Result<NavigationResult, crate::runtime::RuntimeError>) -> WorkerResponse {
    match result {
        Ok(navigation) => WorkerResponse::Navigated { navigation },
        Err(value) => error(value),
    }
}

fn error(value: crate::runtime::RuntimeError) -> WorkerResponse {
    match value {
        crate::runtime::RuntimeError::WorkerLimitExceeded { resource } => {
            WorkerResponse::LimitExceeded { resource }
        }
        value => WorkerResponse::Error {
            error: Box::new(value),
        },
    }
}

pub fn current_binary_sha256() -> std::io::Result<String> {
    if let Some(value) = WORKER_BINARY_SHA256.get() {
        return Ok(value.clone());
    }
    precompute_binary_sha256()?;
    Ok(WORKER_BINARY_SHA256
        .get()
        .expect("worker hash initialized")
        .clone())
}

pub fn precompute_binary_sha256() -> std::io::Result<()> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(std::env::current_exe()?)?;
    let _ = WORKER_BINARY_SHA256.set(format!("{:x}", Sha256::digest(bytes)));
    Ok(())
}

pub fn initialize_binary_sha256(value: String) -> std::io::Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid trusted worker SHA-256",
        ));
    }
    let _ = WORKER_BINARY_SHA256.set(value.to_ascii_lowercase());
    Ok(())
}

#[cfg(target_os = "macos")]
fn current_code_directory_hash() -> Option<String> {
    const CS_OPS_CDHASH: u32 = 5;
    unsafe extern "C" {
        fn csops(
            pid: libc::pid_t,
            ops: u32,
            useraddr: *mut libc::c_void,
            usersize: libc::size_t,
        ) -> libc::c_int;
    }
    let mut hash = [0_u8; 20];
    // SAFETY: csops writes exactly the supplied fixed-size CDHash buffer for
    // the current process.
    let status = unsafe {
        csops(
            std::process::id().cast_signed(),
            CS_OPS_CDHASH,
            hash.as_mut_ptr().cast(),
            hash.len(),
        )
    };
    (status == 0).then(|| hash.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(not(target_os = "macos"))]
fn current_code_directory_hash() -> Option<String> {
    None
}

pub fn policy_digest(context: &ContextOptions) -> String {
    use sha2::{Digest, Sha256};
    let encoded = serde_json::to_vec(&context.network_policy)
        .expect("network policy serialization is infallible");
    format!("{:x}", Sha256::digest(encoded))
}

fn secret_environment_present() -> bool {
    [
        "AZURE_OPENAI_API_KEY",
        "OPENAI_API_KEY",
        "OLLAMA_API_KEY",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "HOME",
        "DYLD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn serves_initialized_page_requests_over_v8_json_lines() {
        let route = WorkerRoute::issue(7, 7, 0).unwrap();
        let requests = vec![
            WorkerRequest::Initialize {
                nonce: "test-nonce".into(),
                limits: WorkerLimits::default(),
                context: Box::new(ContextOptions::default()),
                page_id: PageId(7),
                opener_page_id: None,
                document_generation_base: 0,
            },
            WorkerRequest::LoadHtml {
                url: "https://example.test".into(),
                html: "<button>Save</button>".into(),
            },
            WorkerRequest::Observe { profile: None },
            WorkerRequest::ObserveSemantic {
                profile: ProjectionProfile::Full,
            },
            WorkerRequest::ObserveSemanticV6 {
                options: oryn_common::v6::SemanticObservationOptions::default(),
            },
            WorkerRequest::ObserveSemanticDelta { from: Revision(0) },
            WorkerRequest::Oil {
                input: "observe\nclick \"Save\"".into(),
            },
            WorkerRequest::Trace,
            WorkerRequest::Exit,
        ];
        let mut input = Vec::new();
        for (index, command) in requests.into_iter().enumerate() {
            serde_json::to_writer(
                &mut input,
                &HostEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    session_id: 7,
                    route: Some(route.clone()),
                    message_id: index as u64 + 1,
                    command,
                },
            )
            .expect("serialize request");
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .expect("serve requests");
        let lines: Vec<WorkerEnvelope> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).expect("valid response"))
            .collect();

        assert_eq!(lines.len(), 9);
        assert!(matches!(lines[0].response, WorkerResponse::Ready { .. }));
        assert!(matches!(lines[1].response, WorkerResponse::Loaded { .. }));
        assert!(matches!(
            lines[2].response,
            WorkerResponse::Observation { .. }
        ));
        assert!(matches!(
            lines[3].response,
            WorkerResponse::SemanticObservation { .. }
        ));
        assert!(matches!(
            lines[4].response,
            WorkerResponse::SemanticObservationV6 { .. }
        ));
        assert!(matches!(
            lines[5].response,
            WorkerResponse::SemanticDelta { .. }
        ));
        assert!(matches!(lines[6].response, WorkerResponse::Oil { .. }));
        assert!(matches!(lines[7].response, WorkerResponse::Trace { .. }));
        assert!(matches!(lines[8].response, WorkerResponse::Exiting));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn legacy_protocol_only_allows_ping_and_exit() {
        let input = br#"{"command":"ping"}
{"command":"observe","profile":null}
{"command":"exit"}
"#;
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .expect("serve legacy requests");
        let values = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values[0]["status"], "pong");
        assert_eq!(values[1]["status"], "error");
        assert_eq!(values[2]["status"], "exiting");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reset_destroys_the_session_and_accepts_a_fresh_nonce_and_session() {
        let first = WorkerRoute::issue(41, 1, 0).unwrap();
        let second = WorkerRoute::issue(42, 2, 1_u64 << 32).unwrap();
        let requests = [
            HostEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                session_id: 41,
                message_id: 1,
                route: Some(first.clone()),
                command: WorkerRequest::Initialize {
                    nonce: "first".into(),
                    limits: WorkerLimits::default(),
                    context: Box::new(ContextOptions::default()),
                    page_id: PageId(1),
                    opener_page_id: None,
                    document_generation_base: 0,
                },
            },
            HostEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                session_id: 41,
                message_id: 2,
                route: Some(first),
                command: WorkerRequest::Reset {
                    nonce: "reset".into(),
                },
            },
            HostEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                session_id: 42,
                message_id: 1,
                route: Some(second.clone()),
                command: WorkerRequest::Initialize {
                    nonce: "second".into(),
                    limits: WorkerLimits::default(),
                    context: Box::new(ContextOptions::default()),
                    page_id: PageId(2),
                    opener_page_id: None,
                    document_generation_base: 1_u64 << 32,
                },
            },
            HostEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                session_id: 42,
                message_id: 2,
                route: Some(second),
                command: WorkerRequest::Exit,
            },
        ];
        let mut input = Vec::new();
        for request in requests {
            serde_json::to_writer(&mut input, &request).unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .unwrap();
        let responses = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<WorkerEnvelope>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(
            matches!(responses[0].response, WorkerResponse::Ready { ref nonce, .. } if nonce == "first")
        );
        assert!(
            matches!(responses[1].response, WorkerResponse::ResetReady { ref nonce, .. } if nonce == "reset")
        );
        assert!(
            matches!(responses[2].response, WorkerResponse::Ready { ref nonce, .. } if nonce == "second")
        );
        assert!(matches!(responses[3].response, WorkerResponse::Exiting));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_duplicate_unmatched_and_reinitialized_v8_messages() {
        let route = WorkerRoute::issue(9, 7, 0).unwrap();
        let commands = [
            (
                9,
                1,
                WorkerRequest::Initialize {
                    nonce: "nonce".into(),
                    limits: WorkerLimits::default(),
                    context: Box::new(ContextOptions::default()),
                    page_id: PageId(7),
                    opener_page_id: None,
                    document_generation_base: 0,
                },
            ),
            (9, 1, WorkerRequest::Ping),
            (10, 2, WorkerRequest::Ping),
            (
                9,
                3,
                WorkerRequest::Initialize {
                    nonce: "again".into(),
                    limits: WorkerLimits::default(),
                    context: Box::new(ContextOptions::default()),
                    page_id: PageId(7),
                    opener_page_id: None,
                    document_generation_base: 0,
                },
            ),
            (9, 4, WorkerRequest::Exit),
        ];
        let mut input = Vec::new();
        for (session_id, message_id, command) in commands {
            serde_json::to_writer(
                &mut input,
                &HostEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    session_id,
                    message_id,
                    command,
                    route: Some(route.clone()),
                },
            )
            .unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .unwrap();
        let responses = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<WorkerEnvelope>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            responses[0].response,
            WorkerResponse::Ready { .. }
        ));
        assert!(matches!(
            responses[1].response,
            WorkerResponse::Error { .. }
        ));
        assert!(matches!(
            responses[2].response,
            WorkerResponse::Error { .. }
        ));
        assert!(matches!(
            responses[3].response,
            WorkerResponse::Error { .. }
        ));
        assert!(matches!(responses[4].response, WorkerResponse::Exiting));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authenticated_worker_commands_reject_missing_and_forged_routes() {
        let route = WorkerRoute::issue(1, 1, 0).unwrap();
        let forged = WorkerRoute::issue(1, 1, 0).unwrap();
        let mut changed = route.clone();
        changed.generation = 1;
        let requests = [
            (
                Some(route.clone()),
                WorkerRequest::Initialize {
                    nonce: "route-test".into(),
                    limits: WorkerLimits::default(),
                    context: Box::new(ContextOptions::default()),
                    page_id: PageId(1),
                    opener_page_id: None,
                    document_generation_base: 0,
                },
            ),
            (None, WorkerRequest::Ping),
            (Some(forged), WorkerRequest::Ping),
            (Some(changed), WorkerRequest::Ping),
            (Some(route.clone()), WorkerRequest::Ping),
            (Some(route), WorkerRequest::Exit),
        ];
        let mut input = Vec::new();
        for (index, (route, command)) in requests.into_iter().enumerate() {
            serde_json::to_writer(
                &mut input,
                &HostEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    session_id: 1,
                    route,
                    message_id: index as u64 + 1,
                    command,
                },
            )
            .unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .unwrap();
        let replies = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<WorkerEnvelope>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 6);
        assert!(matches!(replies[0].response, WorkerResponse::Ready { .. }));
        for reply in &replies[1..4] {
            assert!(matches!(reply.response, WorkerResponse::Error { .. }));
        }
        assert!(matches!(replies[4].response, WorkerResponse::Pong { .. }));
        assert!(matches!(replies[5].response, WorkerResponse::Exiting));
    }
}
