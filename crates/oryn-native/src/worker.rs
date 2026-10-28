use std::{
    collections::BTreeMap,
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
    oil::{NativeOilOutput, NativeOilSession},
    runtime::{NativeAction, RuntimeError},
};

pub const WORKER_PROTOCOL_VERSION: u32 = 22;
use crate::worker_route::WorkerRoute;
pub const MAX_IPC_FRAME_BYTES: usize = 24 * 1024 * 1024;
static WORKER_BINARY_SHA256: OnceLock<String> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    #[serde(default)]
    pub native_scrollbar_width: f64,
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
            native_scrollbar_width: config.native_scrollbar_width,
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
            native_scrollbar_width: if self.native_scrollbar_width.is_finite()
                && (0.0..=128.0).contains(&self.native_scrollbar_width)
            {
                self.native_scrollbar_width
            } else {
                0.0
            },
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
    ScriptCloseHistory,
    PrepareScriptClose {
        token: u64,
        expected: oryn_js::worker_wire::Creator,
    },
    ControlledNavigate {
        id: crate::navigation_control::NavigationId,
        request: crate::navigation_control::NavigationRequest,
    },
    Initialize {
        #[serde(default)]
        related_actor: Option<WorkerRoute>,
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
        #[serde(default)]
        service_authority: Option<oryn_js::navigation::ServiceNavigationAdmission>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        url: String,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
    },
    Back,
    Forward,
    Refresh,
    LoadHtml {
        policy_token: Option<String>,
        url: String,
        html: String,
    },
    WaitFor {
        predicate: WaitPredicate,
    },
    WaitForUntil {
        predicate: WaitPredicate,
        timeout_ms: Option<u64>,
    },
    Evaluate {
        source: String,
    },
    Scoped {
        scope: crate::host::webdriver_script::AutomationRealmSnapshot,
        request: Box<WorkerRequest>,
    },
    WebdriverFrames {
        #[serde(rename = "operation")]
        command: crate::host::webdriver_script::FrameCommand,
    },
    WebdriverScript {
        request: crate::host::webdriver_script::ScriptRequest,
        context: crate::host::webdriver_script::ScriptContext,
    },
    PollScript { ticket: u64 },
    WebdriverActions {
        #[serde(rename = "operation")]
        command: crate::webdriver_actions::ActionCommand,
        context: crate::host::webdriver_script::ScriptContext,
    },
    Capabilities,
    ChallengeAssessment,
    ResolveFocusedForm,
    ResolveSelector {
        selector: String,
        action: oryn_common::v3::SemanticAction,
    },
    ResolveTarget {
        target: oryn_common::agent::Target,
        action: Option<oryn_common::v3::SemanticAction>,
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
    ObserveStyles {
        options: oryn_common::v4::ObservationOptions,
        properties: Vec<String>,
    },
    ObserveSemanticV6 {
        options: oryn_common::v6::SemanticObservationOptions,
    },
    ComputedStyle {
        target: SemanticRef,
    },
    AccessibilitySnapshot,
    AccessibilityText {
        target: SemanticRef,
        #[serde(rename = "operation")]
        command: oryn_js::accessibility::TextCommand,
    },
    BoxModel {
        target: SemanticRef,
    },
    PaintSnapshot,
    PrintSnapshot {
        options: oryn_common::v4::PrintOptions,
    },
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
    ReleaseEnvironment,
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
    ScriptCloseHistory {
        proof: crate::context_service::ScriptCloseHistoryProof,
    },
    ScriptClosePreparation {
        outcome: crate::context_service::ScriptClosePreparation,
    },
    WebdriverFrames {
        result: Result<
            crate::host::webdriver_script::FrameResponse,
            crate::host::webdriver_script::ScriptError,
        >,
    },
    WebdriverScript {
        result: Result<
            crate::host::webdriver_script::ScriptValue,
            crate::host::webdriver_script::ScriptError,
        >,
    },
    ScriptPending { ticket: u64 },
    WebdriverActions {
        result: Result<(), crate::host::webdriver_script::ScriptError>,
        feedback: oryn_js::drag_image::FeedbackBatch,
        receipts: Vec<crate::webdriver_actions::OsKeyReceipt>,
    },
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
    StyleObservation {
        snapshot: crate::runtime::NativeStyleObservation,
    },
    SemanticObservationV6 {
        observation: oryn_common::v6::SemanticObservation,
    },
    ComputedStyle {
        style: oryn_common::v4::ComputedStyle,
    },
    AccessibilitySnapshot {
        snapshot: Box<crate::accessibility_platform::NativeSnapshot>,
    },
    AccessibilityText {
        snapshot: oryn_js::accessibility::TextSnapshot,
    },
    BoxModel {
        box_model: oryn_common::v4::BoxModel,
    },
    PaintSnapshot {
        snapshot: Box<oryn_common::v4::PaintSnapshot>,
    },
    PrintSnapshot {
        snapshot: Box<oryn_common::v4::PrintSnapshot>,
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
    EnvironmentReleased,
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
    wire_limits: WorkerLimits,
    parent_policy_authority: bool,
    page: PageHandle,
    oil: NativeOilSession,
    pending_script: Option<PendingScript>,
}

// One outstanding script per environment. The actor owns execution and its
// native deadline; this future only retains the response, without occupying
// the process's sole command reader. Tickets never repeat across environments.
struct PendingScript {
    ticket: u64,
    future: std::pin::Pin<Box<dyn std::future::Future<Output = WorkerResponse>>>,
}
static NEXT_SCRIPT_TICKET: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

impl PageSession {
    fn poll_script(&mut self, ticket: u64) -> WorkerResponse {
        let Some(pending) = self.pending_script.as_mut() else {
            return error(RuntimeError::ProtocolViolation("no pending script".into()));
        };
        if pending.ticket != ticket {
            return error(RuntimeError::ProtocolViolation("foreign script ticket".into()));
        }
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match pending.future.as_mut().poll(&mut context) {
            std::task::Poll::Pending => WorkerResponse::ScriptPending { ticket },
            std::task::Poll::Ready(response) => {
                self.pending_script.take();
                response
            }
        }
    }

    fn start_script(&mut self, request: WorkerRequest) -> WorkerResponse {
        if self.pending_script.is_some() {
            return error(RuntimeError::ProtocolViolation("script already pending".into()));
        }
        let Ok(ticket) = NEXT_SCRIPT_TICKET.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |value| value.checked_add(1),
        ) else {
            return error(RuntimeError::ProtocolViolation("script ticket exhausted".into()));
        };
        let page = self.page.clone();
        self.pending_script = Some(PendingScript {
            ticket,
            future: Box::pin(async move {
                match request {
                    WorkerRequest::WebdriverScript { request, context } =>
                        WorkerResponse::WebdriverScript {
                            result: page.execute_webdriver_script_in_context(request, context).await,
                        },
                    WorkerRequest::Scoped { scope, request } =>
                        match page.scoped_worker_request(scope, *request).await {
                            Ok(response) => response,
                            Err(value) => error(value),
                        },
                    _ => unreachable!("only async scripts are admitted"),
                }
            }),
        });
        self.poll_script(ticket)
    }
}

fn asynchronous_script(request: &WorkerRequest) -> bool {
    match request {
        WorkerRequest::WebdriverScript { request, .. } => request.asynchronous,
        WorkerRequest::Scoped { request, .. } => asynchronous_script(request),
        _ => false,
    }
}

impl PageSession {
    #[cfg(test)]
    fn new(
        config: BrowserConfig,
        context: ContextOptions,
        broker: Option<SharedNetworkBroker>,
        #[cfg(unix)] context_client: Option<SharedContextClient>,
        page_id: PageId,
        opener_page_id: Option<PageId>,
    ) -> Self {
        Self::new_with_related(
            config,
            context,
            broker,
            #[cfg(unix)]
            context_client,
            page_id,
            opener_page_id,
            None,
        )
        .expect("native page session admission")
    }
    fn new_with_related(
        config: BrowserConfig,
        context: ContextOptions,
        broker: Option<SharedNetworkBroker>,
        #[cfg(unix)] context_client: Option<SharedContextClient>,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        related_actor: Option<WorkerRoute>,
    ) -> Result<Self, crate::runtime::RuntimeError> {
        let wire_limits = WorkerLimits::from(&config);
        #[cfg(unix)]
        let parent_policy_authority = broker.is_some() && context_client.is_some();
        #[cfg(not(unix))]
        let parent_policy_authority = false;
        #[cfg(all(unix, feature = "v8-host"))]
        if let Some(actor) = related_actor.as_ref() {
            let route = context_client.as_ref().and_then(|client| client.current_route())
                .ok_or_else(|| crate::runtime::RuntimeError::ProtocolViolation("related root route unavailable".into()))?;
            if let Some(page) = crate::runtime::claim_owned_related_page(
                actor, &route, &config, &context, opener_page_id,
            )? {
                return Ok(Self { wire_limits, oil: NativeOilSession::new(page.clone()), page,
                    parent_policy_authority, pending_script: None });
            }
        }
        let browser = Browser::in_process_with_config(config);
        #[cfg(all(unix, not(feature = "v8-host")))]
        let _ = &context_client;
        let context = match broker {
            #[cfg(all(unix, feature = "v8-host"))]
            Some(broker) if context_client.is_some() => browser
                .new_context_with_broker_and_services(
                    context,
                    broker,
                    context_client.expect("checked context client"),
                ),
            Some(broker) => browser.new_context_with_broker(context, broker),
            None => browser.new_context_with_options(context),
        };
        #[cfg(all(unix, feature = "v8-host"))]
        let page = match related_actor {
            Some(actor) => {
                context.new_related_page_with_identity(page_id, opener_page_id, actor)?
            }
            None => context.new_page_with_identity(page_id, opener_page_id),
        };
        #[cfg(not(all(unix, feature = "v8-host")))]
        let page = {
            if related_actor.is_some() {
                return Err(crate::runtime::RuntimeError::ProtocolViolation(
                    "related actor requires native V8 host".into(),
                ));
            }
            context.new_page_with_identity(page_id, opener_page_id)
        };
        let oil = NativeOilSession::new(page.clone());
        Ok(Self {
            wire_limits,
            page,
            oil,
            parent_policy_authority,
            pending_script: None,
        })
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
    let mut sessions = BTreeMap::<u64, Option<PageSession>>::new();
    let mut active_session_id: Option<u64> = None;
    let mut active_routes = BTreeMap::<u64, WorkerRoute>::new();
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
            let request_started = std::time::Instant::now();
            let wire_limits = match &envelope.command {
                WorkerRequest::Initialize { limits, .. } => {
                    WorkerLimits::from(&limits.browser_config())
                }
                _ => envelope
                    .route
                    .as_ref()
                    .and_then(|route| sessions.get(&route.environment))
                    .and_then(Option::as_ref)
                    .map(|session| session.wire_limits.clone())
                    .unwrap_or_else(|| WorkerLimits::from(&BrowserConfig::default())),
            };
            // Diagnostic identities do not authorize requests; normal validation follows.
            let progress_scope = envelope.route.as_ref().map(|route| {
                crate::ipc_progress::request(route, envelope.message_id, &envelope.command)
            });
            let progress = progress_scope
                .as_ref()
                .and_then(crate::ipc_progress::RequestScope::identity);
            crate::ipc_progress::record(
                progress,
                crate::ipc_progress::Phase::RequestReceived,
                None,
                None,
            );
            let mut timing = crate::native_timing::Scope::start(
                "native_worker_dispatch",
                serde_json::to_value(progress).unwrap_or(serde_json::Value::Null),
            );
            let response = if envelope.protocol_version != WORKER_PROTOCOL_VERSION {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "unsupported worker protocol version".into(),
                    )),
                }.into()
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
                        _ => active_routes.get(&route.environment) == Some(route),
                    }
            }) {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "missing or stale worker route authority".into(),
                    )),
                }.into()
            } else if active_session_id.is_some_and(|session_id| session_id != envelope.session_id)
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "worker session identifier changed".into(),
                    )),
                }.into()
            } else if envelope.message_id <= last_message_id {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "duplicate or out-of-order worker message".into(),
                    )),
                }.into()
            } else if matches!(&envelope.command, WorkerRequest::Initialize { .. })
                && envelope
                    .route
                    .as_ref()
                    .is_some_and(|route| active_routes.contains_key(&route.environment))
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "worker session is already initialized".into(),
                    )),
                }.into()
            } else if matches!(&envelope.command, WorkerRequest::Initialize { .. })
                && sessions.len() >= 256
            {
                WorkerResponse::LimitExceeded {
                    resource: "process environments".into(),
                }.into()
            } else if sessions.len() > 1
                && matches!(
                    &envelope.command,
                    WorkerRequest::Reset { .. } | WorkerRequest::Exit
                )
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "process control requires every sibling environment to be released".into(),
                    )),
                }.into()
            } else {
                last_message_id = envelope.message_id;
                let environment = envelope
                    .route
                    .as_ref()
                    .expect("validated route")
                    .environment;
                dispatch_owned(
                    sessions.entry(environment).or_default(),
                    broker.clone().map(|broker| {
                        broker.for_route(envelope.route.clone().expect("validated route"))
                    }),
                    #[cfg(unix)]
                    context_client.as_ref().map(|client| {
                        client.for_route(envelope.route.clone().expect("validated route"))
                    }),
                    envelope.command,
                    progress,
                )
                .await
            };
            let mut custodied=response;
            let response=&custodied.response;
            timing.finish(!matches!(
                response,
                WorkerResponse::Error { .. } | WorkerResponse::LimitExceeded { .. }
            ));
            crate::ipc_progress::result(
                progress,
                crate::ipc_progress::Phase::RequestReturned,
                None,
                None,
                Some(!matches!(
                    response,
                    WorkerResponse::Error { .. } | WorkerResponse::LimitExceeded { .. }
                )),
            );
            if matches!(response, WorkerResponse::Ready { .. }) {
                active_session_id = Some(envelope.session_id);
                let route = envelope
                    .route
                    .clone()
                    .expect("ready response has validated route");
                active_routes.insert(route.environment, route);
            }
            if matches!(response, WorkerResponse::EnvironmentReleased)
                && let Some(route) = envelope.route.as_ref()
            {
                sessions.remove(&route.environment);
                active_routes.remove(&route.environment);
            }
            let reset = matches!(response, WorkerResponse::ResetReady { .. });
            let exiting = matches!(response, WorkerResponse::Exiting);
            let response = CustodiedWorkerEnvelope { envelope:WorkerEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                route: envelope.route,
                session_id: envelope.session_id,
                message_id: envelope.message_id,
                correlation_id: envelope.message_id,
                response:std::mem::replace(&mut custodied.response,WorkerResponse::Exiting),
            },receipts:custodied.receipts.take() };
            let deadline = request_started + Duration::from_millis(wire_limits.command_wall_ms);
            if matches!(
                &response.envelope.response,
                WorkerResponse::SemanticObservation { .. }
                    | WorkerResponse::SemanticObservationV6 { .. }
                    | WorkerResponse::SemanticDelta { .. }
            ) {
                let mut budget = WireBudgetWriter {
                    writer: &mut writer,
                    remaining: wire_limits.ipc_frame_bytes.saturating_sub(1),
                    deadline,
                };
                write_envelope_deep_safe_checked(&mut budget, &response.envelope, &mut || {
                    semantic_wire_checkpoint(deadline)
                })?;
            } else {
                serde_json::to_writer(&mut writer, &response)?;
            }
            writer.write_all(b"\n")?;
            writer.flush()?;
            crate::ipc_progress::record(
                progress,
                crate::ipc_progress::Phase::ResponseWritten,
                None,
                None,
            );
            if reset {
                active_session_id = None;
                active_routes.clear();
                sessions.clear();
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
            Ok(WorkerRequest::Exit) if active_session_id.is_none() => WorkerResponse::Exiting,
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
        let response = OwnedWorkerResponse(Some(response));
        write_legacy(&mut writer, response.0.as_ref().unwrap())?;
        if exiting {
            break;
        }
    }
    let mut cleanup_failure = None;
    for (environment, session) in sessions {
        if let Some(session) = session
            && let Err(error) = session.page.close().await
        {
            cleanup_failure.get_or_insert_with(|| error.to_string());
        }
        #[cfg(unix)]
        if let Some(client) = context_client
            .as_ref()
            .zip(active_routes.get(&environment))
            .map(|(client, route)| client.for_route(route.clone()))
            && let Err(error) = client.drain_resource_teardown(Duration::from_secs(2))
        {
            cleanup_failure.get_or_insert_with(|| error.to_string());
        }
        #[cfg(not(unix))]
        let _ = environment;
    }
    match cleanup_failure {
        Some(error) => Err(std::io::Error::other(error)),
        None => Ok(()),
    }
}

fn write_legacy(writer: &mut impl Write, response: &WorkerResponse) -> std::io::Result<()> {
    write_response_deep_safe(&mut *writer, response)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

async fn dispatch_owned(
    session: &mut Option<PageSession>,
    broker: Option<SharedNetworkBroker>,
    #[cfg(unix)] context_client: Option<SharedContextClient>,
    request: WorkerRequest,
    progress: Option<crate::ipc_progress::Identity>,
) -> CustodiedWorkerResponse {
    let request=match request {
        WorkerRequest::Scoped{scope,request}=>match *request {
            WorkerRequest::WebdriverActions{command,context}=>{
                let Some(session)=session.as_mut()else{return WorkerResponse::Error{error:Box::new(RuntimeError::Closed)}.into()};
                return match session.page.scoped_owned_actions(scope,command,context).await{
                    Ok(response)=>response,Err(error)=>WorkerResponse::Error{error:Box::new(error)}.into(),
                };
            }
            request=>WorkerRequest::Scoped{scope,request:Box::new(request)},
        },request=>request,
    };
    if let WorkerRequest::WebdriverActions{command,context}=request {
        let Some(session)=session.as_mut() else {
            return WorkerResponse::Error{error:Box::new(RuntimeError::ProtocolViolation("worker has not been initialized".into()))}.into();
        };
        let (result,receipts)=match session.page.execute_native_actions_with_owned_receipts(command,context).await{
            Ok(receipts)=>(Ok(()),receipts),Err(error)=>(Err(error),Vec::new()),
        };
        return CustodiedWorkerResponse::actions(result,session.page.take_drag_feedback_batch(),receipts);
    }
    dispatch(session,broker,#[cfg(unix)]context_client,request,progress).await.into()
}
async fn dispatch(
    session: &mut Option<PageSession>,
    broker: Option<SharedNetworkBroker>,
    #[cfg(unix)] context_client: Option<SharedContextClient>,
    request: WorkerRequest,
    progress: Option<crate::ipc_progress::Identity>,
) -> WorkerResponse {
    if let WorkerRequest::Initialize {
        related_actor,
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
        *session = Some(
            match PageSession::new_with_related(
                config,
                *context,
                broker,
                #[cfg(unix)]
                context_client,
                page_id,
                opener_page_id,
                related_actor,
            ) {
                Ok(session) => session,
                Err(value) => return error(value),
            },
        );
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
        if let Some(active) = session.as_ref()
            && let Err(value) = active.page.close().await
        {
            return error(value);
        }
        #[cfg(unix)]
        if let Some(client) = &context_client
            && let Err(value) = client.drain_resource_teardown(Duration::from_secs(2))
        {
            return error(value);
        }
        session.take();
        return WorkerResponse::Exiting;
    }
    if matches!(request, WorkerRequest::ReleaseEnvironment) {
        let Some(active) = session.as_ref() else {
            return error(crate::runtime::RuntimeError::Closed);
        };
        return match active.page.close().await {
            Ok(()) => {
                #[cfg(unix)]
                if let Some(client) = &context_client
                    && let Err(value) = client.drain_resource_teardown(Duration::from_secs(2))
                {
                    return error(value);
                }
                session.take();
                WorkerResponse::EnvironmentReleased
            }
            Err(value) => error(value),
        };
    }
    if let WorkerRequest::Reset { nonce } = request {
        let Some(active) = session.as_ref() else {
            return WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                    "worker has not been initialized".into(),
                )),
            };
        };
        if let Err(value) = active.page.close().await {
            return error(value);
        }
        #[cfg(unix)]
        if let Some(client) = &context_client
            && let Err(value) = client.drain_resource_teardown(Duration::from_secs(2))
        {
            return error(value);
        }
        session.take();
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
    if asynchronous_script(&request) {
        return session.start_script(request);
    }
    match request {
        WorkerRequest::PollScript { ticket } => session.poll_script(ticket),
        WorkerRequest::ControlledNavigate { id, request } => {
            match session.page.run_remote_navigation(id, request).await {
                Ok(()) => WorkerResponse::Satisfied,
                Err(error) => WorkerResponse::Error {
                    error: Box::new(error),
                },
            }
        }
        WorkerRequest::Goto { url } => navigation(session.page.goto(url).await),
        WorkerRequest::GotoResource {
            service_authority,
            form_authority,
            url,
            post,
            referrer_policy,
        } => {
            if let Some(authority) = service_authority {
                if post.is_some() || form_authority.is_some() || referrer_policy.is_some() {
                    return WorkerResponse::Error{error:Box::new(RuntimeError::ProtocolViolation("service navigation cannot carry form authority, body, or referrer override".into()))};
                }
                navigation(session.page.goto_service_resource(url, authority).await)
            } else {
                navigation(
                    session
                        .page
                        .goto_resource(url, post, referrer_policy, form_authority)
                        .await,
                )
            }
        }
        WorkerRequest::Back => navigation(session.page.back().await),
        WorkerRequest::Forward => navigation(session.page.forward().await),
        WorkerRequest::Refresh => navigation(session.page.refresh().await),
        WorkerRequest::LoadHtml {
            policy_token,
            url,
            html,
        } => match if session.parent_policy_authority {
            session
                .page
                .load_html_with_policy(url, html, policy_token)
                .await
        } else {
            // A standalone dispatcher owns its native BrowserContext. A child
            // with a parent endpoint must instead consume the forwarded grant.
            session.page.load_html(url, html).await
        } {
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
        WorkerRequest::WaitForUntil {
            predicate,
            timeout_ms,
        } => match session
            .page
            .wait_for_timeout(predicate, timeout_ms.map(Duration::from_millis))
            .await
        {
            Ok(()) => WorkerResponse::Satisfied,
            Err(value) => error(value),
        },
        WorkerRequest::Scoped { scope, request } => {
            match session.page.scoped_worker_request(scope, *request).await {
                Ok(response) => response,
                Err(value) => error(value),
            }
        }
        WorkerRequest::WebdriverFrames { command } => WorkerResponse::WebdriverFrames {
            result: session.page.webdriver_frames(command).await,
        },
        WorkerRequest::WebdriverActions { command, context } => {
            let outcome = session
                .page
                .execute_native_actions_with_receipts(command, context)
                .await;
            let (result, receipts) = match outcome {
                Ok(receipts) => (Ok(()), receipts),
                Err(error) => (Err(error), Vec::new()),
            };
            WorkerResponse::WebdriverActions {
                result,
                receipts,
                feedback: session.page.take_drag_feedback_batch(),
            }
        }
        WorkerRequest::WebdriverScript { request, context } => WorkerResponse::WebdriverScript {
            result: session
                .page
                .execute_webdriver_script_in_context(request, context)
                .await,
        },
        WorkerRequest::Evaluate { source } => match session.page.evaluate(source).await {
            Ok(value) => WorkerResponse::Value { value },
            Err(value) => error(value),
        },
        WorkerRequest::ScriptCloseHistory => match session.page.script_close_history().await {
            Ok(proof) => WorkerResponse::ScriptCloseHistory { proof },
            Err(value) => error(value),
        },
        WorkerRequest::PrepareScriptClose { token, expected } => {
            match session.page.prepare_script_close(token, expected).await {
                Ok(outcome) => WorkerResponse::ScriptClosePreparation { outcome },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Capabilities => match session.page.capabilities().await {
            Ok(capabilities) => WorkerResponse::Capabilities { capabilities },
            Err(value) => error(value),
        },
        WorkerRequest::ChallengeAssessment => match session.page.challenge_assessment().await {
            Ok(assessment) => WorkerResponse::ChallengeAssessment { assessment },
            Err(value) => error(value),
        },
        WorkerRequest::ResolveFocusedForm => match session.page.resolve_focused_form().await {
            Ok(semantic_ref) => WorkerResponse::SemanticRef { semantic_ref },
            Err(value) => error(value),
        },
        WorkerRequest::ResolveSelector { selector, action } => {
            match session.page.resolve_selector(selector, action).await {
                Ok(semantic_ref) => WorkerResponse::SemanticRef { semantic_ref },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ResolveTarget { target, action } => {
            match session.page.resolve_target(target, action).await {
                Ok(result) => WorkerResponse::Resolution { result },
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
        WorkerRequest::ObserveStyles { options, properties } => match session.page.observe_styles(options, properties).await {
            Ok(snapshot) => WorkerResponse::StyleObservation { snapshot },
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
        WorkerRequest::AccessibilitySnapshot => match session.page.accessibility_snapshot().await {
            Ok(snapshot) => WorkerResponse::AccessibilitySnapshot {
                snapshot: Box::new(snapshot),
            },
            Err(value) => error(value),
        },
        WorkerRequest::AccessibilityText { target, command } => {
            match session.page.accessibility_text(target, command).await {
                Ok(snapshot) => WorkerResponse::AccessibilityText { snapshot },
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
        WorkerRequest::PrintSnapshot { options } => {
            match session.page.print_snapshot(options).await {
                Ok(snapshot) => WorkerResponse::PrintSnapshot {
                    snapshot: Box::new(snapshot),
                },
                Err(value) => error(value),
            }
        }
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
                    Err(value) => {
                        return error(RuntimeError::BatchExecutionFailed {
                            failed_index: results.len(),
                            completed: results,
                            cause: Box::new(value),
                        });
                    }
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
            Err(value) => error(value.into_runtime().unwrap_or_else(|failure| {
                RuntimeError::ProtocolViolation(format!(
                    "OIL receipt serialization failed: {failure}"
                ))
            })),
        },
        WorkerRequest::Trace => match session.page.trace().await {
            Ok(events) => WorkerResponse::Trace { events },
            Err(value) => error(value),
        },
        WorkerRequest::Close => match session.page.close_with_observation(progress).await {
            Ok(()) => WorkerResponse::Closed,
            Err(value) => error(value),
        },
        WorkerRequest::Initialize { .. }
        | WorkerRequest::Ping
        | WorkerRequest::Reset { .. }
        | WorkerRequest::ReleaseEnvironment
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
    #[cfg(all(unix, feature = "v8-host"))]
    #[tokio::test(flavor = "current_thread")]
    async fn pending_script_transport_preserves_tickets_deadlines_and_close() {
        let mut session = PageSession::new(
            BrowserConfig::default(), ContextOptions::default(), None, None,
            PageId(1), None,
        );
        session.page.goto("about:blank").await.unwrap();
        let script = |timeout_ms| WorkerRequest::WebdriverScript {
            request: crate::host::webdriver_script::ScriptRequest {
                body: "void 0;".encode_utf16().collect(), args: vec![],
                asynchronous: true, timeout_ms,
            },
            context: Default::default(),
        };
        let WorkerResponse::ScriptPending { ticket } = session.start_script(script(Some(30))) else {
            panic!("async command must release the dispatcher while pending");
        };
        assert!(matches!(session.start_script(script(None)), WorkerResponse::Error { .. }));
        assert!(matches!(session.poll_script(ticket + 1), WorkerResponse::Error { .. }));
        assert_eq!(session.pending_script.as_ref().unwrap().ticket, ticket);
        let completion = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match session.poll_script(ticket) {
                    WorkerResponse::ScriptPending { ticket: observed } => {
                        assert_eq!(observed, ticket);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    response => break response,
                }
            }
        }).await.unwrap();
        assert!(matches!(completion, WorkerResponse::WebdriverScript { result: Err(ref error) }
            if error.code == "script timeout"));
        assert!(session.pending_script.is_none());
        assert!(matches!(session.poll_script(ticket), WorkerResponse::Error { .. }));
        let WorkerResponse::ScriptPending { ticket: next } = session.start_script(script(None)) else {
            panic!("null timeout script should remain pending");
        };
        assert_ne!(next, ticket);
        tokio::time::timeout(Duration::from_secs(2), session.page.close()).await.unwrap().unwrap();
        let completion = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match session.poll_script(next) {
                    WorkerResponse::ScriptPending { .. } =>
                        tokio::time::sleep(Duration::from_millis(10)).await,
                    response => break response,
                }
            }
        }).await.unwrap();
        assert!(matches!(completion, WorkerResponse::WebdriverScript { result: Err(_) }));
        assert!(session.pending_script.is_none());
    }

    #[test]
    fn native_scrollbar_width_round_trips_and_rejects_nonfinite_geometry() {
        let config = BrowserConfig {
            native_scrollbar_width: 17.5,
            ..Default::default()
        };
        let encoded = serde_json::to_vec(&WorkerLimits::from(&config)).unwrap();
        let mut limits: WorkerLimits = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(limits.browser_config().native_scrollbar_width, 17.5);
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 129.0] {
            limits.native_scrollbar_width = invalid;
            assert_eq!(limits.browser_config().native_scrollbar_width, 0.0);
        }
    }
    use std::io::{BufReader, Cursor};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn process_dispatches_two_environment_grants_and_releases_only_one() {
        let first = WorkerRoute::issue(31, 1, 0).unwrap();
        let second = WorkerRoute::issue(31, 2, 0).unwrap();
        let initialize = |page_id| WorkerRequest::Initialize {
            related_actor: None,
            nonce: format!("environment-{page_id}"),
            limits: WorkerLimits::default(),
            context: Box::new(ContextOptions::default()),
            page_id: PageId(page_id),
            opener_page_id: None,
            document_generation_base: 0,
        };
        let requests = vec![
            (first.clone(), initialize(1)),
            (second.clone(), initialize(2)),
            (
                first.clone(),
                WorkerRequest::LoadHtml {
                    policy_token: None,
                    url: "https://first.test/".into(),
                    html: "<p>alpha</p><button>alpha</button>".into(),
                },
            ),
            (
                second.clone(),
                WorkerRequest::LoadHtml {
                    policy_token: None,
                    url: "https://second.test/".into(),
                    html: "<p>beta</p><button>beta</button>".into(),
                },
            ),
            (
                first.clone(),
                WorkerRequest::Observe {
                    profile: Some(ProjectionProfile::Full),
                },
            ),
            (
                second.clone(),
                WorkerRequest::Observe {
                    profile: Some(ProjectionProfile::Full),
                },
            ),
            (
                first.clone(),
                WorkerRequest::Evaluate {
                    source: "document.querySelector('p').textContent".into(),
                },
            ),
            (
                second.clone(),
                WorkerRequest::Evaluate {
                    source: "document.querySelector('p').textContent".into(),
                },
            ),
            (
                first.clone(),
                WorkerRequest::Reset {
                    nonce: "forbidden-with-sibling".into(),
                },
            ),
            (first.clone(), WorkerRequest::ReleaseEnvironment),
            (first, WorkerRequest::Ping),
            (
                second.clone(),
                WorkerRequest::Observe {
                    profile: Some(ProjectionProfile::Full),
                },
            ),
            (
                second.clone(),
                WorkerRequest::Evaluate {
                    source: "document.querySelector('p').textContent".into(),
                },
            ),
            (second, WorkerRequest::Exit),
        ];
        let mut input = Vec::new();
        for (index, (route, command)) in requests.into_iter().enumerate() {
            serde_json::to_writer(
                &mut input,
                &HostEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    route: Some(route),
                    session_id: 31,
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
        assert_eq!(replies.len(), 14);
        assert!(matches!(replies[0].response, WorkerResponse::Ready { .. }));
        assert!(matches!(replies[1].response, WorkerResponse::Ready { .. }));
        assert!(matches!(replies[2].response, WorkerResponse::Loaded { .. }));
        assert!(matches!(replies[3].response, WorkerResponse::Loaded { .. }));
        // Observation checks the document owned by each route even when this
        // in-memory dispatcher was compiled without a JavaScript runtime.
        for (index, page, url, expected, other) in [
            (4, 1, "https://first.test/", "alpha", "beta"),
            (5, 2, "https://second.test/", "beta", "alpha"),
            (11, 2, "https://second.test/", "beta", "alpha"),
        ] {
            let WorkerResponse::Observation { observation } = &replies[index].response else {
                panic!(
                    "expected document observation: {:?}",
                    replies[index].response
                );
            };
            assert_eq!(observation.page.page_id, PageId(page));
            assert_eq!(observation.page.url, url);
            assert!(
                observation
                    .nodes
                    .iter()
                    .any(|node| node.role == "button" && node.name == expected)
            );
            assert!(observation.nodes.iter().all(|node| node.name != other));
        }
        #[cfg(feature = "v8-host")]
        {
            assert!(
                matches!(&replies[6].response, WorkerResponse::Value { value } if value == "alpha")
            );
            assert!(
                matches!(&replies[7].response, WorkerResponse::Value { value } if value == "beta")
            );
            assert!(
                matches!(&replies[12].response, WorkerResponse::Value { value } if value == "beta")
            );
        }
        #[cfg(not(feature = "v8-host"))]
        for index in [6, 7, 12] {
            assert!(matches!(
                &replies[index].response,
                WorkerResponse::Error { error }
                    if matches!(error.as_ref(), crate::runtime::RuntimeError::EvaluationUnavailable)
            ));
        }
        assert!(matches!(replies[8].response, WorkerResponse::Error { .. }));
        assert!(matches!(
            replies[9].response,
            WorkerResponse::EnvironmentReleased
        ));
        assert!(matches!(replies[10].response, WorkerResponse::Error { .. }));
        assert!(matches!(replies[13].response, WorkerResponse::Exiting));
        for (index, reply) in replies.iter().enumerate() {
            assert_eq!(reply.correlation_id, index as u64 + 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serves_initialized_page_requests_over_v8_json_lines() {
        let route = WorkerRoute::issue(7, 7, 0).unwrap();
        let requests = vec![
            WorkerRequest::Initialize {
                related_actor: None,
                nonce: "test-nonce".into(),
                limits: WorkerLimits::default(),
                context: Box::new(ContextOptions::default()),
                page_id: PageId(7),
                opener_page_id: None,
                document_generation_base: 0,
            },
            WorkerRequest::LoadHtml {
                policy_token: None,
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

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn reset_waits_for_script_ownership_and_preserves_failed_cleanup_for_retry() {
        use crate::context_service::{
            ContextScriptLoad, ContextServiceRequest, ContextServiceResponse, ContextTransport,
            IpcContextClient, serve_context_transport,
        };
        struct CleanupService;
        impl ContextTransport for CleanupService {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                match request {
                    ContextServiceRequest::CancelScriptLoad { .. } => {
                        Ok(ContextServiceResponse::ScriptLoadCancelled)
                    }
                    _ => Err(crate::RuntimeError::ContextService(
                        "unexpected fixture request".into(),
                    )),
                }
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let server = std::thread::spawn(move || {
            serve_context_transport(parent, std::sync::Arc::new(CleanupService))
        });
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        let owner = ContextScriptLoad::from_reserved(client.clone(), PageId(1), 71);
        let resource_job = client.track_resource_job().unwrap();
        let mut session = Some(PageSession::new(
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            None,
            PageId(1),
            None,
        ));
        let response = dispatch(
            &mut session,
            None,
            Some(client.clone()),
            WorkerRequest::Reset {
                nonce: "first".into(),
            },
            None,
        )
        .await;
        assert!(matches!(response, WorkerResponse::Error { .. }));
        assert!(
            session.is_some(),
            "failed teardown must retain retry ownership"
        );
        assert!(
            client.track_resource_job().is_err(),
            "failed drain keeps the old route sealed"
        );
        drop(resource_job);
        assert!(client.drain_script_cleanup(Duration::ZERO).is_err());
        drop(owner);
        client.drain_script_cleanup(Duration::from_secs(5)).unwrap();
        let response = dispatch(
            &mut session,
            None,
            Some(client.clone()),
            WorkerRequest::Reset {
                nonce: "retry".into(),
            },
            None,
        )
        .await;
        assert!(matches!(response, WorkerResponse::ResetReady { nonce, .. } if nonce == "retry"));
        assert!(session.is_none());
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
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
                    related_actor: None,
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
                    related_actor: None,
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
                    related_actor: None,
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
                    related_actor: None,
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
                    related_actor: None,
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

#[cfg(test)]
mod semantic_identity_contract_tests {
    use super::*;
    #[test]
    fn pending_worker14_identity_resolution_preserves_optional_action_and_native_reference() {
        let reference = oryn_common::v3::SemanticRef {
            page_id: oryn_common::v3::PageId(91),
            document_generation: 7,
            node: 4294967301,
        };
        for action in [None, Some(oryn_common::v3::SemanticAction::Click)] {
            let request = WorkerRequest::ResolveTarget {
                target: oryn_common::agent::Target::Inside {
                    target: Box::new(oryn_common::agent::Target::Reference { reference }),
                    container: Box::new(oryn_common::agent::Target::Css {
                        selector: "section[data-owner]".into(),
                    }),
                },
                action,
            };
            let bytes = serde_json::to_vec(&request).unwrap();
            let WorkerRequest::ResolveTarget {
                target,
                action: decoded,
            } = serde_json::from_slice(&bytes).unwrap()
            else {
                panic!("identity request")
            };
            assert_eq!(decoded, action);
            assert!(
                matches!(target,oryn_common::agent::Target::Inside{target,..} if matches!(*target,oryn_common::agent::Target::Reference{reference:actual} if actual==reference))
            );
        }
    }
}

#[cfg(test)]
mod nested_command_wire_tests {
    use super::*;
    #[test]
    fn nested_operations_preserve_outer_command_tags_and_round_trip() {
        let requests = [
            (
                "webdriver_frames",
                WorkerRequest::WebdriverFrames {
                    command: crate::host::webdriver_script::FrameCommand::Inventory,
                },
            ),
            (
                "webdriver_actions",
                WorkerRequest::WebdriverActions {
                    command: crate::webdriver_actions::ActionCommand::Release,
                    context: Default::default(),
                },
            ),
            (
                "accessibility_text",
                WorkerRequest::AccessibilityText {
                    target: SemanticRef {
                        page_id: PageId(9),
                        document_generation: 3,
                        node: 7,
                    },
                    command: oryn_js::accessibility::TextCommand::Read,
                },
            ),
        ];
        for (tag, request) in requests {
            let value = serde_json::to_value(&request).unwrap();
            assert_eq!(value["command"], tag);
            assert!(value.get("operation").is_some());
            assert_eq!(
                serde_json::from_value::<WorkerRequest>(value.clone()).unwrap(),
                request
            );
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove("operation");
            assert!(serde_json::from_value::<WorkerRequest>(missing).is_err());
            let mut malformed = value;
            malformed["operation"] = serde_json::Value::Null;
            assert!(serde_json::from_value::<WorkerRequest>(malformed).is_err());
        }
        assert_eq!(
            serde_json::to_value(WorkerRequest::Ping).unwrap(),
            serde_json::json!({"command":"ping"})
        );
        assert_eq!(
            serde_json::to_value(WorkerRequest::Goto {
                url: "https://example.test/".into()
            })
            .unwrap(),
            serde_json::json!({"command":"goto","url":"https://example.test/"})
        );
    }
}
struct OwnedWorkerResponse(Option<WorkerResponse>);
include!("owned_worker_wire.rs");
impl serde::Serialize for OwnedWorkerResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().unwrap().serialize(serializer)
    }
}
impl Drop for OwnedWorkerResponse {
    fn drop(&mut self) {
        if let Some(response) = self.0.take() {
            dispose_worker_response(response);
        }
    }
}
struct OwnedWorkerEnvelope(WorkerEnvelope);
impl serde::Serialize for OwnedWorkerEnvelope {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}
impl Drop for OwnedWorkerEnvelope {
    fn drop(&mut self) {
        let response = std::mem::replace(&mut self.0.response, WorkerResponse::Exiting);
        dispose_worker_response(response);
    }
}
fn dispose_worker_response(response: WorkerResponse) {
    match response {
        WorkerResponse::SemanticObservationV6 { observation } => {
            crate::semantic::dispose_semantic_observation(observation)
        }
        WorkerResponse::SemanticObservation { mut observation } => {
            for mut node in observation.accessibility.drain(..) {
                crate::semantic::dispose_owned_properties(&mut node.properties);
            }
            for mut node in observation.structures.drain(..) {
                crate::semantic::dispose_owned_properties(&mut node.properties);
            }
        }
        WorkerResponse::SemanticDelta { delta } => {
            drop(crate::semantic::OwnedSemanticChanges::new(delta.changes))
        }
        _ => {}
    }
}
#[cfg(test)]
mod response_depth_controls {
    use super::*;
    struct FailWriter;
    impl std::io::Write for FailWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("expected writer failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn response() -> WorkerResponse {
        WorkerResponse::SemanticDelta {
            delta: oryn_common::v6::SemanticDelta {
                contract_version: 6,
                page_id: oryn_common::v3::PageId(31),
                from_revision: oryn_common::v3::Revision(1),
                to_revision: oryn_common::v3::Revision(2),
                semantic_revision: 2,
                changes: crate::semantic::deep_changes_for_test(),
                diagnostics: Vec::new(),
            },
        }
    }
    #[test]
    fn failed_framed_writer_drops_deep_response_on_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let owner = OwnedWorkerEnvelope(WorkerEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    route: None,
                    session_id: 1,
                    message_id: 2,
                    correlation_id: 2,
                    response: response(),
                });
                assert!(serde_json::to_writer(FailWriter, &owner).is_err());
                drop(owner);
                let owner = OwnedWorkerResponse(Some(response()));
                assert!(write_legacy(&mut FailWriter, owner.0.as_ref().unwrap()).is_err());
                drop(owner);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
fn write_response_deep_safe(
    writer: &mut impl std::io::Write,
    response: &WorkerResponse,
) -> std::io::Result<()> {
    write_response_deep_safe_checked(writer, response, &mut || Ok(()))
}
fn write_response_deep_safe_checked(
    writer: &mut impl Write,
    response: &WorkerResponse,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    match response {
        WorkerResponse::SemanticObservation { observation } => {
            writer.write_all(b"{\"status\":\"semantic_observation\",\"observation\":")?;
            crate::semantic::write_observation_v5_wire(writer, observation, check)?;
            writer.write_all(b"}")
        }
        WorkerResponse::SemanticObservationV6 { observation } => {
            writer.write_all(b"{\"status\":\"semantic_observation_v6\",\"observation\":")?;
            crate::semantic::write_observation_v6_wire(writer, observation, check)?;
            writer.write_all(b"}")
        }
        WorkerResponse::SemanticDelta { delta } => {
            writer.write_all(b"{\"status\":\"semantic_delta\",\"delta\":")?;
            crate::semantic::write_delta_wire(writer, delta, check)?;
            writer.write_all(b"}")
        }
        response => {
            serde_json::to_writer(writer, response)?;
            Ok(())
        }
    }
}
#[cfg(test)]
fn write_envelope_deep_safe(
    writer: &mut impl std::io::Write,
    envelope: &WorkerEnvelope,
) -> std::io::Result<()> {
    write_envelope_deep_safe_checked(writer, envelope, &mut || Ok(()))
}
fn write_envelope_deep_safe_checked(
    writer: &mut impl Write,
    envelope: &WorkerEnvelope,
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    writer.write_all(b"{\"protocol_version\":")?;
    serde_json::to_writer(&mut *writer, &envelope.protocol_version)?;
    writer.write_all(b",\"route\":")?;
    serde_json::to_writer(&mut *writer, &envelope.route)?;
    writer.write_all(b",\"session_id\":")?;
    serde_json::to_writer(&mut *writer, &envelope.session_id)?;
    writer.write_all(b",\"message_id\":")?;
    serde_json::to_writer(&mut *writer, &envelope.message_id)?;
    writer.write_all(b",\"correlation_id\":")?;
    serde_json::to_writer(&mut *writer, &envelope.correlation_id)?;
    writer.write_all(b",\"response\":")?;
    write_response_deep_safe_checked(writer, &envelope.response, check)?;
    writer.write_all(b"}")
}
#[cfg(test)]
mod successful_semantic_wire_controls {
    use super::*;
    fn response(deep: bool) -> OwnedWorkerResponse {
        let changes = if deep {
            crate::semantic::deep_changes_for_test()
        } else {
            oryn_common::v6::SemanticGraphChanges::default()
        };
        OwnedWorkerResponse(Some(WorkerResponse::SemanticDelta {
            delta: oryn_common::v6::SemanticDelta {
                contract_version: 6,
                page_id: oryn_common::v3::PageId(31),
                from_revision: oryn_common::v3::Revision(1),
                to_revision: oryn_common::v3::Revision(2),
                semantic_revision: 2,
                changes,
                diagnostics: Vec::new(),
            },
        }))
    }
    #[test]
    fn shallow_response_exact_wire() {
        let response = response(false);
        let mut bytes = Vec::new();
        write_response_deep_safe(&mut bytes, response.0.as_ref().unwrap()).unwrap();
        assert_eq!(
            bytes,
            serde_json::to_vec(response.0.as_ref().unwrap()).unwrap()
        );
        let envelope = OwnedWorkerEnvelope(WorkerEnvelope {
            protocol_version: WORKER_PROTOCOL_VERSION,
            route: None,
            session_id: 1,
            message_id: 2,
            correlation_id: 2,
            response: response.0.as_ref().unwrap().clone(),
        });
        let mut bytes = Vec::new();
        write_envelope_deep_safe(&mut bytes, &envelope.0).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&envelope.0).unwrap());
    }
    #[test]
    fn successful_deep_response_and_real_writer_limit() {
        struct Limited {
            remaining: usize,
        }
        impl Write for Limited {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > self.remaining {
                    return Err(std::io::Error::other("output bound"));
                }
                self.remaining -= bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let response = response(true);
                let mut bytes = Vec::new();
                write_response_deep_safe(&mut bytes, response.0.as_ref().unwrap()).unwrap();
                assert!(bytes.len() > 32768);
                assert!(
                    write_response_deep_safe(
                        &mut Limited { remaining: 1024 },
                        response.0.as_ref().unwrap()
                    )
                    .is_err()
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
struct WireBudgetWriter<'a, W: Write> {
    writer: &'a mut W,
    remaining: usize,
    deadline: std::time::Instant,
}
impl<W: Write> Write for WireBudgetWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if std::time::Instant::now() >= self.deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "semantic response deadline",
            ));
        }
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other("semantic response IPC limit"));
        }
        let count = self.writer.write(bytes)?;
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
fn semantic_wire_checkpoint(deadline: std::time::Instant) -> std::io::Result<()> {
    if std::time::Instant::now() >= deadline {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "semantic response deadline",
        ))
    } else {
        Ok(())
    }
}
