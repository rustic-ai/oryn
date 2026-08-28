use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[cfg(feature = "v8-host")]
use std::collections::BTreeSet;

#[cfg(feature = "v8-host")]
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use oryn_common::v3::{
    ActionId, ActionResult, CONTRACT_VERSION, CapabilityDiagnostic, Effect, ExecutionDomain,
    Observation, ObservationDelta, ObservationSource, PageDescriptor, PageId, PageInfo,
    PageLifecycleEvent, PagePredicate, PageStatus, ProjectionProfile, Revision, SemanticAction,
    SemanticRef, SupportLevel,
};
use oryn_common::v4::{
    BoxModel, ComputedStyle, MediaState, ObservationOptions, PaintSnapshot, PreferredColorScheme,
    ReducedMotion, ScrollState, ViewportState,
};
#[cfg(feature = "v8-host")]
use oryn_common::v4::{PaintImageResource, PaintNodeSnapshot};
use serde::{Deserialize, Serialize};
#[cfg(feature = "v8-host")]
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, mpsc, oneshot};
use url::Url;

#[cfg(all(unix, feature = "v8-host"))]
use crate::context_service::SharedContextClient;
#[cfg(unix)]
use crate::context_service::{ContextEvent, ContextEventRecord};

use crate::html::{ParsedDocument, parse};
#[cfg(feature = "v8-host")]
use crate::network::NetworkBroker;
use crate::semantic::{
    empty_observation, is_hidden, project_document, project_document_full, project_element,
};
use crate::trace::{TraceEvent, TraceEventKind, TraceLog};
use crate::{
    download::{DownloadDescriptor, DownloadId, DownloadStore},
    execution::{
        FormValueState, HandoffBundle, HandoffLoss, HandoffOptions, OriginStorageState,
        SensitiveCookieHeader,
    },
    network::{NetworkRequest, PolicyBroker, RustlsTransport, SharedNetworkBroker},
    security::{
        ActionPolicy, ConfirmationToken, ConsequentialAction, NetworkPolicy, PolicyDecision,
        PolicyEngine, PolicyRequest, SecretError, SecretHandle, SecretStore, SecurityAuditRecord,
    },
    storage::{
        PersistentProfile, SharedContextStore, StorageArea, StorageLimits, StorageRequest,
        StorageResponse, StorageScope,
    },
};

static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_WORKER_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct Browser {
    config: BrowserConfig,
    worker_process: bool,
}

#[derive(Clone)]
pub struct BrowserContext {
    inner: Arc<BrowserContextInner>,
}

struct BrowserContextInner {
    id: u64,
    config: BrowserConfig,
    broker: Option<SharedNetworkBroker>,
    worker_process: bool,
    options: ContextOptions,
    group: ContextGroup,
    store: SharedContextStore,
    persistent_profile: Option<PersistentProfile>,
    policy: Arc<PolicyEngine>,
    secrets: Arc<SecretStore>,
    downloads: Arc<DownloadStore>,
    network_replay: crate::replay::NetworkReplayController,
    intent_trace: Mutex<Vec<crate::replay::IntentTraceRecord>>,
    domain_decisions: Mutex<Vec<crate::execution::DomainDecision>>,
    #[cfg(unix)]
    context_events: Mutex<Vec<ContextEventRecord>>,
    #[cfg(unix)]
    next_context_event_id: AtomicU64,
    #[cfg(unix)]
    parent_cookies: Option<std::sync::Arc<dyn reqwest::cookie::CookieStore>>,
    #[cfg(all(unix, feature = "v8-host"))]
    context_client: Option<crate::context_service::SharedContextClient>,
}

#[derive(Debug, Clone)]
pub struct PageHandle {
    id: u64,
    opener_page_id: Option<PageId>,
    context: Weak<BrowserContextInner>,
    sender: Option<mpsc::Sender<PageCommand>>,
    #[cfg(unix)]
    worker: Option<std::sync::Arc<WorkerPage>>,
}

#[cfg(unix)]
struct WorkerPage {
    page_id: PageId,
    opener_page_id: Option<PageId>,
    context: Weak<BrowserContextInner>,
    client: std::sync::Mutex<Option<crate::worker_client::WorkerClient>>,
    config: BrowserConfig,
    options: ContextOptions,
    parent_cookies: std::sync::Arc<dyn reqwest::cookie::CookieStore>,
    pending: AtomicUsize,
}

#[derive(Debug, Clone)]
struct PageRecord {
    handle: PageHandle,
    descriptor: PageDescriptor,
}

#[derive(Debug)]
struct ContextGroup {
    pages: Mutex<BTreeMap<PageId, PageRecord>>,
    active: Mutex<Option<PageId>>,
    events: Mutex<Vec<PageLifecycleEvent>>,
    notify: Notify,
}

impl ContextGroup {
    fn new() -> Self {
        Self {
            pages: Mutex::new(BTreeMap::new()),
            active: Mutex::new(None),
            events: Mutex::new(Vec::new()),
            notify: Notify::new(),
        }
    }

    fn register(
        &self,
        handle: PageHandle,
        opener_page_id: Option<PageId>,
        activate: bool,
        attention_required: bool,
        document_generation: u64,
    ) {
        let page_id = PageId(handle.id);
        if activate {
            *self.active.lock().expect("active page lock") = Some(page_id);
        }
        let descriptor = PageDescriptor {
            page: PageInfo {
                page_id,
                opener_page_id,
                url: "about:blank".into(),
                title: String::new(),
                document_generation,
            },
            active: activate,
            status: if attention_required {
                PageStatus::AttentionRequired
            } else {
                PageStatus::Open
            },
            revision: Revision(0),
        };
        let mut pages = self.pages.lock().expect("page group lock");
        if activate {
            for record in pages.values_mut() {
                record.descriptor.active = false;
            }
        }
        pages.insert(
            page_id,
            PageRecord {
                handle,
                descriptor: descriptor.clone(),
            },
        );
        drop(pages);
        self.push_event(PageLifecycleEvent::Opened { page: descriptor });
    }

    fn push_event(&self, event: PageLifecycleEvent) {
        self.events.lock().expect("page event lock").push(event);
        self.notify.notify_waiters();
    }

    fn descriptors(&self) -> Vec<PageDescriptor> {
        self.pages
            .lock()
            .expect("page group lock")
            .values()
            .map(|record| record.descriptor.clone())
            .collect()
    }

    fn handle(&self, page_id: PageId) -> Option<PageHandle> {
        self.pages
            .lock()
            .expect("page group lock")
            .get(&page_id)
            .filter(|record| {
                !matches!(
                    record.descriptor.status,
                    PageStatus::Closed | PageStatus::Crashed
                )
            })
            .map(|record| record.handle.clone())
    }

    fn activate(&self, page_id: PageId) -> Result<PageHandle, RuntimeError> {
        let mut pages = self.pages.lock().expect("page group lock");
        let handle = pages
            .get(&page_id)
            .filter(|record| {
                !matches!(
                    record.descriptor.status,
                    PageStatus::Closed | PageStatus::Crashed
                )
            })
            .map(|record| record.handle.clone())
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        for record in pages.values_mut() {
            record.descriptor.active = record.descriptor.page.page_id == page_id;
            if record.descriptor.active && record.descriptor.status == PageStatus::AttentionRequired
            {
                record.descriptor.status = PageStatus::Open;
            }
        }
        *self.active.lock().expect("active page lock") = Some(page_id);
        drop(pages);
        self.push_event(PageLifecycleEvent::Activated { page_id });
        Ok(handle)
    }

    fn update_observation(&self, observation: &Observation) {
        if let Some(record) = self
            .pages
            .lock()
            .expect("page group lock")
            .get_mut(&observation.page.page_id)
        {
            record.descriptor.page = observation.page.clone();
            record.descriptor.revision = observation.revision;
        }
    }

    fn update_navigation(&self, navigation: &NavigationResult) {
        if let Some(record) = self
            .pages
            .lock()
            .expect("page group lock")
            .get_mut(&navigation.page_id)
        {
            record.descriptor.page.url = navigation.final_url.clone();
            record.descriptor.page.title = navigation.title.clone();
            record.descriptor.page.document_generation = navigation.document_generation;
            record.descriptor.revision = navigation.revision;
        }
        self.push_event(PageLifecycleEvent::Navigated {
            page_id: navigation.page_id,
            url: navigation.final_url.clone(),
            document_generation: navigation.document_generation,
        });
    }

    fn update_loaded(&self, page: PageInfo, revision: Revision) {
        let page_id = page.page_id;
        if let Some(record) = self
            .pages
            .lock()
            .expect("page group lock")
            .get_mut(&page_id)
        {
            record.descriptor.page = page.clone();
            record.descriptor.revision = revision;
        }
        self.push_event(PageLifecycleEvent::Navigated {
            page_id,
            url: page.url,
            document_generation: page.document_generation,
        });
    }

    fn update_revision(&self, page_id: PageId, revision: Revision) {
        if let Some(record) = self
            .pages
            .lock()
            .expect("page group lock")
            .get_mut(&page_id)
        {
            record.descriptor.revision = revision;
        }
    }

    fn close(&self, page_id: PageId) {
        let mut pages = self.pages.lock().expect("page group lock");
        if let Some(record) = pages.get_mut(&page_id) {
            record.descriptor.status = PageStatus::Closed;
            record.descriptor.active = false;
        }
        let should_replace_active = self
            .active
            .lock()
            .expect("active page lock")
            .is_some_and(|active| active == page_id);
        if should_replace_active {
            let replacement = pages
                .values_mut()
                .find(|record| record.descriptor.status == PageStatus::Open)
                .map(|record| {
                    record.descriptor.active = true;
                    record.descriptor.page.page_id
                });
            *self.active.lock().expect("active page lock") = replacement;
        }
        drop(pages);
        self.push_event(PageLifecycleEvent::Closed { page_id });
    }

    fn crash(&self, page_id: PageId, detail: String) {
        if let Some(record) = self
            .pages
            .lock()
            .expect("page group lock")
            .get_mut(&page_id)
        {
            record.descriptor.status = PageStatus::Crashed;
            record.descriptor.active = false;
            record.descriptor.page.document_generation =
                record.descriptor.page.document_generation.saturating_add(1);
        }
        self.push_event(PageLifecycleEvent::Crashed { page_id, detail });
    }

    #[cfg(unix)]
    fn worker_started(&self, page_id: PageId, document_generation: u64) {
        let active_page = *self.active.lock().expect("active page lock");
        let mut pages = self.pages.lock().expect("page group lock");
        let Some(record) = pages.get_mut(&page_id) else {
            return;
        };
        let replaced = record.descriptor.status == PageStatus::Crashed;
        record.descriptor.status = PageStatus::Open;
        record.descriptor.active = active_page == Some(page_id);
        record.descriptor.page.url = "about:blank".into();
        record.descriptor.page.title.clear();
        record.descriptor.page.document_generation = document_generation;
        record.descriptor.revision = Revision(0);
        drop(pages);
        if replaced {
            self.push_event(PageLifecycleEvent::Navigated {
                page_id,
                url: "about:blank".into(),
                document_generation,
            });
        }
    }

    fn matching_page(&self, predicate: &PagePredicate) -> Option<PageHandle> {
        self.pages
            .lock()
            .expect("page group lock")
            .values()
            .find(|record| {
                !matches!(
                    record.descriptor.status,
                    PageStatus::Closed | PageStatus::Crashed
                ) && page_matches(&record.descriptor, predicate)
            })
            .map(|record| record.handle.clone())
    }
}

fn page_matches(page: &PageDescriptor, predicate: &PagePredicate) -> bool {
    match predicate {
        PagePredicate::Any => true,
        PagePredicate::Id { page_id } => page.page.page_id == *page_id,
        PagePredicate::Origin { origin } => Url::parse(&page.page.url)
            .ok()
            .map(|url| url.origin().ascii_serialization())
            .is_some_and(|value| value == *origin),
        PagePredicate::UrlContains { value } => page.page.url.contains(value),
        PagePredicate::TitleContains { value } => page.page.title.contains(value),
        PagePredicate::Opener { page_id } => page.page.opener_page_id == Some(*page_id),
    }
}

#[cfg(unix)]
struct PendingWorkerRequest<'a>(&'a AtomicUsize);

#[cfg(unix)]
impl Drop for PendingWorkerRequest<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(unix)]
impl std::fmt::Debug for WorkerPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("WorkerPage").finish_non_exhaustive()
    }
}

#[cfg(unix)]
impl WorkerPage {
    fn enter_queue(&self) -> Result<PendingWorkerRequest<'_>, RuntimeError> {
        let capacity = self.config.worker_queue_capacity.clamp(1, 32);
        self.pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending < capacity).then_some(pending + 1)
            })
            .map_err(|_| RuntimeError::WorkerLimitExceeded {
                resource: "command queue".into(),
            })?;
        Ok(PendingWorkerRequest(&self.pending))
    }

    fn request(
        &self,
        request: crate::worker::WorkerRequest,
    ) -> Result<crate::worker::WorkerResponse, RuntimeError> {
        let _pending = self.enter_queue()?;
        let mut client = self
            .client
            .lock()
            .map_err(|_| RuntimeError::WorkerCrashed("worker client lock poisoned".into()))?;
        if client.is_none() {
            let mut config = self.config.clone();
            config.document_generation_base =
                NEXT_WORKER_GENERATION.fetch_add(1, Ordering::Relaxed) << 32;
            let context_owner = self
                .context
                .upgrade()
                .map(|inner| BrowserContext { inner })
                .ok_or(RuntimeError::Closed)?;
            *client = Some(crate::worker_client::WorkerClient::spawn(
                &config,
                &self.options,
                self.parent_cookies.clone(),
                self.page_id,
                self.opener_page_id,
                context_owner,
            )?);
            if let Some(context) = self.context.upgrade() {
                context
                    .group
                    .worker_started(self.page_id, config.document_generation_base);
            }
        }
        let result = client
            .as_mut()
            .expect("worker initialized")
            .request(request);
        if matches!(
            result,
            Err(RuntimeError::WorkerCrashed(_)
                | RuntimeError::WorkerLimitExceeded { .. }
                | RuntimeError::ProtocolViolation(_))
        ) {
            *client = None;
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    pub worker_queue_capacity: usize,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
    pub max_redirects: usize,
    pub max_v8_heap_bytes: usize,
    pub max_worker_rss_bytes: usize,
    pub max_ipc_frame_bytes: usize,
    pub command_timeout: Duration,
    pub navigation_timeout: Duration,
    pub document_generation_base: u64,
    pub storage_limits: StorageLimits,
    pub max_layout_nodes: usize,
    pub max_font_bytes: usize,
    pub max_page_font_bytes: usize,
    pub max_viewport_css_px: u32,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            worker_queue_capacity: 32,
            request_timeout: Duration::from_secs(30),
            max_response_bytes: 16 * 1024 * 1024,
            max_redirects: 10,
            max_v8_heap_bytes: 256 * 1024 * 1024,
            max_worker_rss_bytes: 512 * 1024 * 1024,
            max_ipc_frame_bytes: 24 * 1024 * 1024,
            command_timeout: Duration::from_secs(10),
            navigation_timeout: Duration::from_secs(30),
            document_generation_base: 0,
            storage_limits: StorageLimits::default(),
            max_layout_nodes: 100_000,
            max_font_bytes: 8 * 1024 * 1024,
            max_page_font_bytes: 32 * 1024 * 1024,
            max_viewport_css_px: 16_384,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextOptions {
    /// Context-owned cookies and storage are discarded when the context closes.
    pub ephemeral: bool,
    pub network_policy: NetworkPolicy,
    #[serde(default)]
    pub action_policy: ActionPolicy,
    /// Whether page-authored scripts and `_blank` navigations may create popups.
    #[serde(default = "default_allow_popups")]
    pub allow_popups: bool,
    #[serde(default)]
    pub viewport: ViewportState,
    #[serde(default)]
    pub color_scheme: PreferredColorScheme,
    #[serde(default)]
    pub reduced_motion: ReducedMotion,
    /// Optional deterministic clock/random inputs used only for captured
    /// replay and evaluation. Live contexts leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub determinism: Option<crate::replay::DeterminismConfig>,
}

const fn default_allow_popups() -> bool {
    true
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            ephemeral: true,
            network_policy: NetworkPolicy::default(),
            action_policy: ActionPolicy::default(),
            allow_popups: true,
            viewport: ViewportState::default(),
            color_scheme: PreferredColorScheme::Light,
            reduced_motion: ReducedMotion::NoPreference,
            determinism: None,
        }
    }
}

impl ContextOptions {
    pub fn loopback_test() -> Self {
        let network_policy = NetworkPolicy {
            allow_loopback: true,
            ..NetworkPolicy::default()
        };
        Self {
            ephemeral: true,
            network_policy,
            action_policy: ActionPolicy::default(),
            allow_popups: true,
            viewport: ViewportState::default(),
            color_scheme: PreferredColorScheme::Light,
            reduced_motion: ReducedMotion::NoPreference,
            determinism: None,
        }
    }

    pub fn with_popups(mut self, allow_popups: bool) -> Self {
        self.allow_popups = allow_popups;
        self
    }

    pub fn with_viewport(mut self, viewport: ViewportState) -> Self {
        self.viewport = viewport;
        self
    }

    pub fn with_determinism(mut self, determinism: crate::replay::DeterminismConfig) -> Self {
        self.determinism = Some(determinism);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Commit,
    DomContentLoaded,
    Load,
    NetworkIdle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationResult {
    pub page_id: PageId,
    pub requested_url: String,
    pub final_url: String,
    #[serde(default)]
    pub title: String,
    pub status: u16,
    pub document_generation: u64,
    pub revision: Revision,
    pub lifecycle: Vec<LifecycleState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaitPredicate {
    Lifecycle(LifecycleState),
    UrlContains(String),
    SelectorExists(String),
    SelectorGone(String),
    SelectorVisible(String),
    SelectorHidden(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentKind {
    Html,
    Text,
    Links,
    Images,
    Tables,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScrollRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub relative: bool,
}

enum PageCommand {
    LoadHtml {
        url: String,
        html: String,
        reply: oneshot::Sender<Result<(Revision, PageInfo), RuntimeError>>,
    },
    Goto {
        url: String,
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    Back {
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    Forward {
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    Refresh {
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    WaitFor {
        predicate: WaitPredicate,
        reply: oneshot::Sender<Result<(), RuntimeError>>,
    },
    Evaluate {
        source: String,
        reply: oneshot::Sender<Result<String, RuntimeError>>,
    },
    Capabilities {
        reply: oneshot::Sender<Vec<CapabilityDiagnostic>>,
    },
    ResolveSelector {
        selector: String,
        action: SemanticAction,
        reply: oneshot::Sender<Result<SemanticRef, RuntimeError>>,
    },
    Extract {
        kind: ContentKind,
        selector: Option<String>,
        reply: oneshot::Sender<Result<serde_json::Value, RuntimeError>>,
    },
    Observe {
        profile: ProjectionProfile,
        reply: oneshot::Sender<Observation>,
    },
    ObserveWith {
        options: ObservationOptions,
        reply: oneshot::Sender<Result<oryn_common::v4::Observation, RuntimeError>>,
    },
    ComputedStyle {
        target: SemanticRef,
        reply: oneshot::Sender<Result<ComputedStyle, RuntimeError>>,
    },
    BoxModel {
        target: SemanticRef,
        reply: oneshot::Sender<Result<BoxModel, RuntimeError>>,
    },
    PaintSnapshot {
        reply: oneshot::Sender<Result<PaintSnapshot, RuntimeError>>,
    },
    SetViewport {
        viewport: ViewportState,
        reply: oneshot::Sender<Result<ViewportState, RuntimeError>>,
    },
    SetMedia {
        media: MediaState,
        reply: oneshot::Sender<Result<MediaState, RuntimeError>>,
    },
    Scroll {
        request: ScrollRequest,
        reply: oneshot::Sender<Result<ScrollState, RuntimeError>>,
    },
    ScrollIntoView {
        target: SemanticRef,
        reply: oneshot::Sender<Result<ScrollState, RuntimeError>>,
    },
    ObserveDelta {
        from: Revision,
        reply: oneshot::Sender<Result<ObservationDelta, RuntimeError>>,
    },
    Trace {
        reply: oneshot::Sender<Vec<TraceEvent>>,
    },
    Execute {
        action: NativeAction,
        reply: oneshot::Sender<Result<ActionResult, RuntimeError>>,
    },
    ExecuteForced {
        action: NativeAction,
        reply: oneshot::Sender<Result<ActionResult, RuntimeError>>,
    },
    Close,
}

impl Browser {
    pub fn new() -> Self {
        Self::with_config(BrowserConfig::default())
    }

    pub fn with_config(config: BrowserConfig) -> Self {
        Self {
            config,
            worker_process: uses_worker_process(),
        }
    }

    /// In-process construction is crate-private: only the page worker owns a
    /// production V8/DOM runtime.
    pub(crate) fn in_process_with_config(config: BrowserConfig) -> Self {
        Self {
            config,
            worker_process: false,
        }
    }

    /// Explicitly named compatibility probe. This is omitted from production
    /// builds unless the caller opts into the probe feature.
    #[cfg(feature = "in-process-probe")]
    pub fn in_process_probe() -> Self {
        Self::in_process_with_config(BrowserConfig::default())
    }

    pub fn new_context(&self) -> BrowserContext {
        self.new_context_with_options(ContextOptions::default())
    }

    pub fn new_context_with_options(&self, options: ContextOptions) -> BrowserContext {
        let store = SharedContextStore::memory(self.config.storage_limits);
        self.new_context_with_store(
            options,
            store,
            None,
            crate::replay::NetworkReplayController::live(),
        )
    }

    pub fn new_recording_context(
        &self,
        options: ContextOptions,
        capture_response_bodies: bool,
    ) -> BrowserContext {
        let store = SharedContextStore::memory(self.config.storage_limits);
        self.new_context_with_store(
            options,
            store,
            None,
            crate::replay::NetworkReplayController::recording(capture_response_bodies),
        )
    }

    pub fn new_replay_context(
        &self,
        options: ContextOptions,
        archive: crate::replay::NetworkArchive,
    ) -> Result<BrowserContext, RuntimeError> {
        let replay = crate::replay::NetworkReplayController::replay(archive)
            .map_err(|error| RuntimeError::Replay(error.to_string()))?;
        let store = SharedContextStore::memory(self.config.storage_limits);
        Ok(self.new_context_with_store(options, store, None, replay))
    }

    /// Restore a logical browser context from a sealed checkpoint. This is a
    /// reload/replay boundary: destination pages receive new identities and
    /// revisions; live V8 heap state is never claimed to survive.
    pub async fn restore_checkpoint(
        &self,
        mut options: ContextOptions,
        checkpoint: &crate::replay::ContextCheckpoint,
    ) -> Result<(BrowserContext, crate::replay::CheckpointRestore), RuntimeError> {
        checkpoint
            .verify()
            .map_err(|error| RuntimeError::Replay(error.to_string()))?;
        options.determinism = checkpoint.payload.determinism.clone();
        let context = self.new_replay_context(options, checkpoint.payload.network.clone())?;
        let mut page_ids = BTreeMap::new();
        let mut restored = Vec::new();
        let mut active_page_id = None;
        for source in &checkpoint.payload.pages {
            let destination_page_id = PageId(NEXT_PAGE_ID.fetch_add(1, Ordering::Relaxed));
            let opener_page_id = source
                .opener_page_id
                .and_then(|opener| page_ids.get(&opener).copied());
            let page = context.create_page(destination_page_id, opener_page_id, false, false);
            page_ids.insert(source.source_page_id, destination_page_id);

            for state in &source.storage {
                let scope = StorageScope {
                    origin: state.origin.clone(),
                    top_level_page_id: destination_page_id,
                };
                for (key, value) in &state.local {
                    context
                        .inner
                        .store
                        .execute(StorageRequest::Set {
                            area: StorageArea::Local,
                            scope: scope.clone(),
                            key: key.clone(),
                            value: value.clone(),
                        })
                        .map_err(|error| RuntimeError::Replay(error.to_string()))?;
                }
                for (key, value) in &state.session {
                    context
                        .inner
                        .store
                        .execute(StorageRequest::Set {
                            area: StorageArea::Session,
                            scope: scope.clone(),
                            key: key.clone(),
                            value: value.clone(),
                        })
                        .map_err(|error| RuntimeError::Replay(error.to_string()))?;
                }
            }

            let navigation = page.goto(source.url.clone()).await?;
            for field in &source.form_values {
                let action = match field.action {
                    SemanticAction::Check if !field.checked => SemanticAction::Uncheck,
                    action => action,
                };
                let target = page
                    .resolve_selector(field.selector.clone(), action)
                    .await?;
                if action == SemanticAction::Type {
                    page.execute(NativeAction {
                        target,
                        action: SemanticAction::Clear,
                        value: None,
                    })
                    .await?;
                }
                page.execute(NativeAction {
                    target,
                    action,
                    value: matches!(action, SemanticAction::Type | SemanticAction::Select)
                        .then(|| field.value.clone()),
                })
                .await?;
            }
            if source.active {
                context.activate_page(destination_page_id)?;
                active_page_id = Some(destination_page_id);
            }
            restored.push(crate::replay::RestoredPage {
                source_page_id: source.source_page_id,
                destination_page_id,
                destination_revision: navigation.revision.0,
            });
        }
        Ok((
            context,
            crate::replay::CheckpointRestore {
                checkpoint_sha256: checkpoint.sha256.clone(),
                pages: restored,
                active_page_id,
            },
        ))
    }

    pub async fn branch_from_checkpoint(
        &self,
        options: ContextOptions,
        checkpoint: &crate::replay::ContextCheckpoint,
        strategy: impl Into<String>,
    ) -> Result<(BrowserContext, crate::replay::ReplayBranch), RuntimeError> {
        let strategy = strategy.into();
        let (context, restore) = self.restore_checkpoint(options, checkpoint).await?;
        let branch_id = format!(
            "{}-{}",
            &checkpoint.sha256[..checkpoint.sha256.len().min(16)],
            restore
                .pages
                .first()
                .map(|page| page.destination_page_id.0)
                .unwrap_or_default()
        );
        Ok((
            context,
            crate::replay::ReplayBranch {
                branch_id,
                parent_checkpoint_sha256: checkpoint.sha256.clone(),
                strategy,
                restored_pages: restore.pages,
            },
        ))
    }

    /// Restore a checkpoint and replay every subsequently recorded OIL intent.
    /// The verifier never invents redacted values: those produce an explicit
    /// nondeterministic divergence instead of silently using placeholder data.
    pub async fn replay_task_trace(
        &self,
        options: ContextOptions,
        checkpoint: &crate::replay::ContextCheckpoint,
        trace: &crate::replay::TaskTraceArchive,
    ) -> Result<(BrowserContext, crate::replay::ReplayRun), RuntimeError> {
        use crate::replay::{ReplayDivergence, ReplayDivergenceKind, ReplayRun};

        if trace.schema_version != crate::replay::TRACE_ARCHIVE_VERSION {
            return Err(RuntimeError::Replay(format!(
                "unsupported task trace version {}",
                trace.schema_version
            )));
        }
        if trace.context_id != checkpoint.payload.context_id {
            return Err(RuntimeError::Replay(format!(
                "trace context {} does not match checkpoint context {}",
                trace.context_id, checkpoint.payload.context_id
            )));
        }
        if trace.determinism != checkpoint.payload.determinism {
            return Err(RuntimeError::Replay(
                "trace determinism inputs do not match the checkpoint".into(),
            ));
        }
        checkpoint
            .verify()
            .map_err(|error| RuntimeError::Replay(error.to_string()))?;
        // A checkpoint contains the archive known at capture time, while the
        // completed trace contains exchanges that occurred after it. Replay
        // restores state with the completed, policy-scrubbed archive.
        let mut replay_payload = checkpoint.payload.clone();
        replay_payload.network = trace.network.clone();
        let replay_checkpoint = crate::replay::ContextCheckpoint::seal(replay_payload)
            .map_err(|error| RuntimeError::Replay(error.to_string()))?;
        let (context, restore) = self.restore_checkpoint(options, &replay_checkpoint).await?;
        let page_map = restore
            .pages
            .iter()
            .map(|page| (page.source_page_id, page.destination_page_id))
            .collect::<BTreeMap<_, _>>();
        let replay_intents = trace
            .intents
            .iter()
            .filter(|intent| intent.sequence >= checkpoint.payload.intent_cursor)
            .collect::<Vec<_>>();
        let mut run = ReplayRun {
            checkpoint_sha256: checkpoint.sha256.clone(),
            trace_schema_version: trace.schema_version,
            attempted_intents: replay_intents.len() as u64,
            replayed_intents: 0,
            consumed_network_exchanges: 0,
            expected_network_exchanges: trace.network.exchanges.len(),
            deterministic: checkpoint.payload.nondeterministic_regions.is_empty()
                && trace.nondeterministic_regions.is_empty(),
            divergences: Vec::new(),
        };
        let mut session = crate::oil::NativeContextOilSession::new(context.clone());
        for intent in replay_intents {
            if intent.input.contains("[REDACTED]") {
                run.deterministic = false;
                run.divergences.push(ReplayDivergence {
                    sequence: intent.sequence,
                    kind: ReplayDivergenceKind::RedactedInput,
                    detail: "sensitive OIL input was redacted and cannot be replayed".into(),
                });
                continue;
            }
            if let Some(source_page_id) = intent.page_id {
                let Some(destination_page_id) = page_map.get(&source_page_id).copied() else {
                    run.divergences.push(ReplayDivergence {
                        sequence: intent.sequence,
                        kind: ReplayDivergenceKind::MissingPageMapping,
                        detail: format!("source page {} has no restored page", source_page_id.0),
                    });
                    continue;
                };
                context.activate_page(destination_page_id)?;
            }
            match session.execute(&intent.input).await {
                Ok(_) => {
                    run.replayed_intents += 1;
                    if intent.outcome.starts_with("failed:") {
                        run.divergences.push(ReplayDivergence {
                            sequence: intent.sequence,
                            kind: ReplayDivergenceKind::OutcomeMismatch,
                            detail: "recorded intent failed but replay completed".into(),
                        });
                    }
                }
                Err(error) => {
                    if !intent.outcome.starts_with("failed:") {
                        run.divergences.push(ReplayDivergence {
                            sequence: intent.sequence,
                            kind: ReplayDivergenceKind::ExecutionFailure,
                            detail: error.to_string(),
                        });
                    } else {
                        run.replayed_intents += 1;
                    }
                }
            }
        }
        run.consumed_network_exchanges = context.replayed_network_exchanges();
        if run.consumed_network_exchanges != run.expected_network_exchanges {
            run.divergences.push(ReplayDivergence {
                sequence: u64::MAX,
                kind: ReplayDivergenceKind::UnconsumedNetwork,
                detail: format!(
                    "consumed {} of {} archived exchanges",
                    run.consumed_network_exchanges, run.expected_network_exchanges
                ),
            });
        }
        Ok((context, run))
    }

    pub async fn replay_failure_artifact(
        &self,
        options: ContextOptions,
        artifact: &crate::replay::FailureArtifact,
    ) -> Result<(BrowserContext, crate::replay::ReplayRun), RuntimeError> {
        artifact
            .verify()
            .map_err(|error| RuntimeError::Replay(error.to_string()))?;
        self.replay_task_trace(
            options,
            &artifact.payload.checkpoint,
            &artifact.payload.trace,
        )
        .await
    }

    pub fn new_persistent_context(
        &self,
        mut options: ContextOptions,
        profile: PersistentProfile,
    ) -> BrowserContext {
        options.ephemeral = false;
        let store = SharedContextStore::persistent(&profile, self.config.storage_limits);
        self.new_context_with_store(
            options,
            store,
            Some(profile),
            crate::replay::NetworkReplayController::live(),
        )
    }

    fn new_context_with_store(
        &self,
        options: ContextOptions,
        store: SharedContextStore,
        persistent_profile: Option<PersistentProfile>,
        network_replay: crate::replay::NetworkReplayController,
    ) -> BrowserContext {
        let policy = Arc::new(PolicyEngine::new(options.action_policy.clone()));
        let secrets = Arc::new(SecretStore::default());
        let context_cookies: Arc<dyn reqwest::cookie::CookieStore> = persistent_profile
            .as_ref()
            .map(PersistentProfile::cookie_store)
            .unwrap_or_else(|| Arc::new(reqwest::cookie::Jar::default()));
        if self.worker_process {
            return BrowserContext {
                inner: Arc::new(BrowserContextInner {
                    id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
                    config: self.config.clone(),
                    broker: None,
                    worker_process: true,
                    options,
                    group: ContextGroup::new(),
                    store,
                    persistent_profile,
                    policy,
                    secrets,
                    downloads: Arc::new(DownloadStore::default()),
                    network_replay,
                    intent_trace: Mutex::new(Vec::new()),
                    domain_decisions: Mutex::new(Vec::new()),
                    #[cfg(unix)]
                    context_events: Mutex::new(Vec::new()),
                    #[cfg(unix)]
                    next_context_event_id: AtomicU64::new(1),
                    parent_cookies: Some(context_cookies),
                    #[cfg(all(unix, feature = "v8-host"))]
                    context_client: None,
                }),
            };
        }
        let timeout = self.config.request_timeout;
        let max_response_bytes = self.config.max_response_bytes;
        let network_policy = options.network_policy.clone();
        let transport_cookies = context_cookies.clone();
        let replay = network_replay.clone();
        let broker = std::thread::spawn(move || {
            RustlsTransport::with_cookie_store(timeout, max_response_bytes, transport_cookies)
                .ok()
                .map(|transport| {
                    SharedNetworkBroker::new(PolicyBroker::new(
                        network_policy,
                        replay.wrap(transport),
                    ))
                })
        })
        .join()
        .unwrap_or(None);
        BrowserContext {
            inner: Arc::new(BrowserContextInner {
                id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
                config: self.config.clone(),
                broker,
                worker_process: false,
                options,
                group: ContextGroup::new(),
                store,
                persistent_profile,
                policy,
                secrets,
                downloads: Arc::new(DownloadStore::default()),
                network_replay,
                intent_trace: Mutex::new(Vec::new()),
                domain_decisions: Mutex::new(Vec::new()),
                #[cfg(unix)]
                context_events: Mutex::new(Vec::new()),
                #[cfg(unix)]
                next_context_event_id: AtomicU64::new(1),
                #[cfg(unix)]
                parent_cookies: Some(context_cookies),
                #[cfg(all(unix, feature = "v8-host"))]
                context_client: None,
            }),
        }
    }

    /// Construct an in-process context around a broker supplied by the
    /// privileged parent. This is used only inside `oryn-page-worker`.
    pub fn new_context_with_broker(
        &self,
        options: ContextOptions,
        broker: SharedNetworkBroker,
    ) -> BrowserContext {
        let policy = Arc::new(PolicyEngine::new(options.action_policy.clone()));
        BrowserContext {
            inner: Arc::new(BrowserContextInner {
                id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
                config: self.config.clone(),
                broker: Some(broker),
                worker_process: false,
                options,
                group: ContextGroup::new(),
                store: SharedContextStore::memory(self.config.storage_limits),
                persistent_profile: None,
                policy,
                secrets: Arc::new(SecretStore::default()),
                downloads: Arc::new(DownloadStore::default()),
                network_replay: crate::replay::NetworkReplayController::live(),
                intent_trace: Mutex::new(Vec::new()),
                domain_decisions: Mutex::new(Vec::new()),
                #[cfg(unix)]
                context_events: Mutex::new(Vec::new()),
                #[cfg(unix)]
                next_context_event_id: AtomicU64::new(1),
                #[cfg(unix)]
                parent_cookies: None,
                #[cfg(all(unix, feature = "v8-host"))]
                context_client: None,
            }),
        }
    }

    #[cfg(all(unix, feature = "v8-host"))]
    pub fn new_context_with_broker_and_services(
        &self,
        options: ContextOptions,
        broker: SharedNetworkBroker,
        context_client: crate::context_service::SharedContextClient,
    ) -> BrowserContext {
        let policy = Arc::new(PolicyEngine::new(options.action_policy.clone()));
        BrowserContext {
            inner: Arc::new(BrowserContextInner {
                id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
                config: self.config.clone(),
                broker: Some(broker),
                worker_process: false,
                options,
                group: ContextGroup::new(),
                store: SharedContextStore::memory(self.config.storage_limits),
                persistent_profile: None,
                policy,
                secrets: Arc::new(SecretStore::default()),
                downloads: Arc::new(DownloadStore::default()),
                network_replay: crate::replay::NetworkReplayController::live(),
                intent_trace: Mutex::new(Vec::new()),
                domain_decisions: Mutex::new(Vec::new()),
                context_events: Mutex::new(Vec::new()),
                next_context_event_id: AtomicU64::new(1),
                parent_cookies: None,
                context_client: Some(context_client),
            }),
        }
    }
}

pub fn uses_worker_process() -> bool {
    cfg!(unix) && !cfg!(test) && !cfg!(feature = "in-process-probe")
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserContext {
    pub fn id(&self) -> u64 {
        self.inner.id
    }

    /// Create a one-use approval for one exact consequential operation.
    pub fn prepare_confirmation(&self, request: PolicyRequest) -> ConfirmationToken {
        self.inner.policy.prepare_confirmation(request)
    }

    /// Evaluate a host policy decision and append it to the context audit log.
    pub fn authorize(
        &self,
        request: PolicyRequest,
        confirmation: Option<ConfirmationToken>,
    ) -> PolicyDecision {
        self.inner.policy.authorize(request, confirmation)
    }

    pub fn security_audit(&self) -> Vec<SecurityAuditRecord> {
        self.inner.policy.audit_records()
    }

    pub fn network_archive(&self) -> crate::replay::NetworkArchive {
        self.inner.network_replay.archive()
    }

    pub fn replayed_network_exchanges(&self) -> usize {
        self.inner.network_replay.consumed()
    }

    pub async fn trace_archive(
        &self,
        mut nondeterministic_regions: Vec<crate::replay::NondeterministicRegion>,
    ) -> Result<crate::replay::TaskTraceArchive, RuntimeError> {
        let mut page_traces = BTreeMap::new();
        for descriptor in self.pages() {
            if matches!(descriptor.status, PageStatus::Closed | PageStatus::Crashed) {
                continue;
            }
            if let Some(page) = self.inner.group.handle(descriptor.page.page_id) {
                page_traces.insert(descriptor.page.page_id.0.to_string(), page.trace().await?);
            }
        }
        if page_traces
            .values()
            .flatten()
            .any(|event| matches!(event.kind, TraceEventKind::WebSocketFrame { .. }))
            && !nondeterministic_regions.contains(&crate::replay::NondeterministicRegion::WebSocket)
        {
            nondeterministic_regions.push(crate::replay::NondeterministicRegion::WebSocket);
        }
        Ok(crate::replay::TaskTraceArchive {
            schema_version: crate::replay::TRACE_ARCHIVE_VERSION,
            context_id: self.id(),
            determinism: self.inner.options.determinism.clone(),
            page_events: self.page_events(),
            page_traces,
            intents: self
                .inner
                .intent_trace
                .lock()
                .expect("intent trace lock")
                .clone(),
            domain_decisions: self
                .inner
                .domain_decisions
                .lock()
                .expect("domain decision trace lock")
                .clone(),
            network: self.network_archive(),
            nondeterministic_regions,
        })
    }

    pub async fn checkpoint(
        &self,
        name: impl Into<String>,
        task_metadata: BTreeMap<String, String>,
        nondeterministic_regions: Vec<crate::replay::NondeterministicRegion>,
    ) -> Result<crate::replay::ContextCheckpoint, RuntimeError> {
        let mut pages = Vec::new();
        for descriptor in self.pages() {
            if matches!(descriptor.status, PageStatus::Closed | PageStatus::Crashed) {
                continue;
            }
            let page = self
                .inner
                .group
                .handle(descriptor.page.page_id)
                .ok_or(RuntimeError::PageNotFound(descriptor.page.page_id))?;
            let origin = self.page_origin(descriptor.page.page_id)?;
            let scope = StorageScope {
                origin: origin.clone(),
                top_level_page_id: descriptor.page.page_id,
            };
            let read_entries = |area| -> Result<Vec<(String, String)>, RuntimeError> {
                match self
                    .inner
                    .store
                    .execute(StorageRequest::Entries {
                        area,
                        scope: scope.clone(),
                    })
                    .map_err(|error| RuntimeError::Replay(error.to_string()))?
                {
                    StorageResponse::Entries { entries } => Ok(entries),
                    _ => Err(RuntimeError::Replay(
                        "storage service returned an invalid checkpoint response".into(),
                    )),
                }
            };
            let observation = page.observe_profile(ProjectionProfile::Full).await?;
            let form_values = observation
                .nodes
                .into_iter()
                .filter_map(|node| {
                    let selector = node.selector?;
                    let value = node.value?;
                    if value == "[REDACTED]" {
                        return None;
                    }
                    let action = if node.actions.contains(&SemanticAction::Select) {
                        SemanticAction::Select
                    } else if node.actions.contains(&SemanticAction::Check) {
                        SemanticAction::Check
                    } else if node.actions.contains(&SemanticAction::Type) {
                        SemanticAction::Type
                    } else {
                        return None;
                    };
                    Some(FormValueState {
                        selector,
                        value,
                        action,
                        checked: node.states.contains(&oryn_common::v3::NodeState::Checked),
                    })
                })
                .collect();
            pages.push(crate::replay::CheckpointPage {
                source_page_id: descriptor.page.page_id,
                opener_page_id: descriptor.page.opener_page_id,
                url: descriptor.page.url,
                active: descriptor.active,
                storage: vec![OriginStorageState {
                    origin,
                    local: read_entries(StorageArea::Local)?,
                    session: read_entries(StorageArea::Session)?,
                }],
                form_values,
            });
        }
        crate::replay::ContextCheckpoint::seal(crate::replay::CheckpointPayload {
            schema_version: crate::replay::TRACE_ARCHIVE_VERSION,
            name: name.into(),
            context_id: self.id(),
            pages,
            task_metadata,
            determinism: self.inner.options.determinism.clone(),
            network: self.network_archive(),
            intent_cursor: self
                .inner
                .intent_trace
                .lock()
                .expect("intent trace lock")
                .len() as u64,
            nondeterministic_regions,
        })
        .map_err(|error| RuntimeError::Replay(error.to_string()))
    }

    pub(crate) fn network_replay_controller(&self) -> crate::replay::NetworkReplayController {
        self.inner.network_replay.clone()
    }

    pub(crate) fn record_intent(
        &self,
        page_id: Option<PageId>,
        input: String,
        revision_before: Option<Revision>,
        revision_after: Option<Revision>,
        outcome: String,
    ) {
        let mut trace = self.inner.intent_trace.lock().expect("intent trace lock");
        let sequence = trace.len() as u64;
        trace.push(crate::replay::IntentTraceRecord {
            sequence,
            page_id,
            input,
            revision_before,
            revision_after,
            outcome,
        });
    }

    pub fn record_domain_decision(&self, decision: crate::execution::DomainDecision) {
        self.inner
            .domain_decisions
            .lock()
            .expect("domain decision trace lock")
            .push(decision);
    }

    /// Import an origin-scoped secret into parent-owned memory.
    pub fn import_secret(
        &self,
        origin: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) -> SecretHandle {
        self.inner
            .secrets
            .insert(origin.into(), label.into(), value.into())
    }

    pub fn revoke_secret(&self, handle: SecretHandle) -> bool {
        self.inner.secrets.revoke(handle)
    }

    pub fn secret_labels(&self) -> Vec<String> {
        self.inner.secrets.labels()
    }

    pub fn quarantine_download(
        &self,
        source_url: impl Into<String>,
        suggested_filename: impl Into<String>,
        media_type: impl Into<String>,
        body: Vec<u8>,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<DownloadDescriptor, RuntimeError> {
        let source_url = source_url.into();
        let parsed =
            Url::parse(&source_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let source_origin = parsed.origin().ascii_serialization();
        self.require_policy(
            PolicyRequest {
                action: ConsequentialAction::Download,
                source_origin: source_origin.clone(),
                target_origin: None,
                detail: Some(source_url.clone()),
            },
            confirmation,
        )?;
        self.inner
            .downloads
            .quarantine(
                source_url,
                source_origin,
                suggested_filename.into(),
                media_type.into(),
                body,
                self.inner.policy.policy().max_download_bytes,
            )
            .map_err(|error| RuntimeError::Download(error.to_string()))
    }

    pub fn downloads(&self) -> Vec<DownloadDescriptor> {
        self.inner.downloads.list()
    }

    pub fn release_download(&self, id: DownloadId) -> Result<Vec<u8>, RuntimeError> {
        self.inner
            .downloads
            .release(id)
            .map_err(|error| RuntimeError::Download(error.to_string()))
    }

    pub fn discard_download(&self, id: DownloadId) -> bool {
        self.inner.downloads.discard(id)
    }

    /// Capture the explicitly transferable portion of one page before an
    /// execution-domain handoff. JavaScript heap state and parent secret
    /// handles never cross this boundary.
    pub async fn prepare_handoff(
        &self,
        page_id: PageId,
        target_domain: ExecutionDomain,
        options: HandoffOptions,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<HandoffBundle, RuntimeError> {
        if target_domain == ExecutionDomain::Native {
            return Err(RuntimeError::InvalidHandoff(
                "native-to-native movement is page navigation, not a domain handoff".into(),
            ));
        }
        let page = self
            .inner
            .group
            .handle(page_id)
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        let descriptor = self
            .pages()
            .into_iter()
            .find(|item| item.page.page_id == page_id)
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        let source_origin = self.page_origin(page_id)?;
        if target_domain == ExecutionDomain::UserBrowser || options.transfer_cookie_header {
            self.require_policy(
                PolicyRequest {
                    action: if target_domain == ExecutionDomain::UserBrowser {
                        ConsequentialAction::UserBrowserDelegation
                    } else {
                        ConsequentialAction::ProtectedRequest
                    },
                    source_origin: source_origin.clone(),
                    target_origin: None,
                    detail: Some(format!(
                        "handoff {:?}{}",
                        target_domain,
                        if options.transfer_cookie_header {
                            " with cookies"
                        } else {
                            ""
                        }
                    )),
                },
                confirmation,
            )?;
        }

        let mut losses = vec![
            HandoffLoss::JavascriptHeap,
            HandoffLoss::PendingPromises,
            HandoffLoss::OpenSockets,
            HandoffLoss::NativeHostObjects,
            HandoffLoss::IndexedDbNotTransferred,
            HandoffLoss::SecretHandlesRemainAtSource,
        ];
        if !options.transfer_cookie_header {
            losses.push(HandoffLoss::HttpOnlyCookiesNotTransferred);
        }
        if !options.allow_lossy {
            return Err(RuntimeError::LossyHandoffRequiresApproval {
                losses: losses.iter().map(|loss| format!("{loss:?}")).collect(),
            });
        }

        let scope = StorageScope {
            origin: source_origin.clone(),
            top_level_page_id: page_id,
        };
        let entries = |area| -> Result<Vec<(String, String)>, RuntimeError> {
            match self
                .inner
                .store
                .execute(StorageRequest::Entries {
                    area,
                    scope: scope.clone(),
                })
                .map_err(|error| RuntimeError::ContextService(error.to_string()))?
            {
                StorageResponse::Entries { entries } => Ok(entries),
                _ => Err(RuntimeError::ContextService(
                    "storage service returned the wrong response to handoff snapshot".into(),
                )),
            }
        };
        let storage = vec![OriginStorageState {
            origin: source_origin,
            local: if options.transfer_local_storage {
                entries(StorageArea::Local)?
            } else {
                Vec::new()
            },
            session: if options.transfer_session_storage {
                entries(StorageArea::Session)?
            } else {
                Vec::new()
            },
        }];
        let form_values = if options.transfer_form_values {
            page.observe_profile(ProjectionProfile::Full)
                .await?
                .nodes
                .into_iter()
                .filter_map(|node| {
                    let selector = node.selector?;
                    let value = node.value?;
                    let action = if node.actions.contains(&SemanticAction::Select) {
                        SemanticAction::Select
                    } else if node.actions.contains(&SemanticAction::Check) {
                        SemanticAction::Check
                    } else if node.actions.contains(&SemanticAction::Type) {
                        SemanticAction::Type
                    } else {
                        return None;
                    };
                    (value != "[REDACTED]").then_some(FormValueState {
                        selector,
                        value,
                        action,
                        checked: node.states.contains(&oryn_common::v3::NodeState::Checked),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        #[cfg(unix)]
        let cookie_header = options
            .transfer_cookie_header
            .then(|| Url::parse(&descriptor.page.url).ok())
            .flatten()
            .and_then(|url| self.cookie_header(&url))
            .map(SensitiveCookieHeader::new);
        #[cfg(not(unix))]
        let cookie_header = None;

        Ok(HandoffBundle {
            contract_version: crate::execution::EXECUTION_CONTRACT_VERSION,
            source_domain: ExecutionDomain::Native,
            target_domain,
            source_page_id: page_id,
            url: descriptor.page.url,
            document_generation: descriptor.page.document_generation,
            storage,
            form_values,
            cookie_header,
            task_metadata: options.task_metadata,
            checkpoint: options.checkpoint,
            losses,
        })
    }

    pub(crate) fn require_policy(
        &self,
        request: PolicyRequest,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<(), RuntimeError> {
        match self.authorize(request.clone(), confirmation) {
            PolicyDecision::Allowed => Ok(()),
            PolicyDecision::Denied => Err(RuntimeError::PolicyDenied {
                action: request.action,
            }),
            PolicyDecision::ConfirmationRequired => Err(RuntimeError::ConfirmationRequired {
                action: request.action,
            }),
        }
    }

    pub(crate) fn network_policy(&self) -> &NetworkPolicy {
        &self.inner.options.network_policy
    }

    #[cfg(unix)]
    pub(crate) fn shared_network_broker(&self) -> Option<SharedNetworkBroker> {
        self.inner.broker.clone()
    }

    pub(crate) fn popups_allowed(&self) -> bool {
        self.inner.options.allow_popups
    }

    pub(crate) fn request_timeout(&self) -> Duration {
        self.inner.config.request_timeout
    }

    #[cfg(unix)]
    pub(crate) fn cookie_header(&self, url: &Url) -> Option<String> {
        self.inner
            .parent_cookies
            .as_ref()
            .and_then(|cookies| reqwest::cookie::CookieStore::cookies(cookies.as_ref(), url))
            .and_then(|value| value.to_str().ok().map(str::to_string))
    }

    pub fn persistent_profile_path(&self) -> Option<&std::path::Path> {
        self.inner
            .persistent_profile
            .as_ref()
            .map(PersistentProfile::path)
    }

    pub(crate) fn context_store(&self) -> SharedContextStore {
        self.inner.store.clone()
    }

    #[cfg(unix)]
    pub(crate) fn push_context_event(&self, event: ContextEvent) {
        let event_id = self
            .inner
            .next_context_event_id
            .fetch_add(1, Ordering::Relaxed);
        self.inner
            .context_events
            .lock()
            .expect("context event lock")
            .push(ContextEventRecord { event_id, event });
    }

    #[cfg(unix)]
    pub(crate) fn poll_context_events(
        &self,
        page_id: PageId,
        after_event_id: u64,
        max_events: usize,
    ) -> Result<Vec<ContextEventRecord>, RuntimeError> {
        let page_origin = self.page_origin(page_id)?;
        Ok(self
            .inner
            .context_events
            .lock()
            .expect("context event lock")
            .iter()
            .filter(|record| record.event_id > after_event_id)
            .filter(|record| match &record.event {
                ContextEvent::Storage {
                    source_page_id,
                    change,
                } => {
                    *source_page_id != page_id
                        && change.scope.origin == page_origin
                        && (change.area == crate::storage::StorageArea::Local
                            || change.scope.top_level_page_id == page_id)
                }
                ContextEvent::Message { target_page_id, .. } => *target_page_id == page_id,
                ContextEvent::PageLifecycle { .. } => true,
            })
            .take(max_events)
            .cloned()
            .collect())
    }

    pub(crate) fn page_origin(&self, page_id: PageId) -> Result<String, RuntimeError> {
        let descriptor = self
            .pages()
            .into_iter()
            .find(|page| page.page.page_id == page_id)
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        if descriptor.page.url == "about:blank"
            && let Some(opener) = descriptor.page.opener_page_id
        {
            return self.page_origin(opener);
        }
        Url::parse(&descriptor.page.url)
            .map(|url| url.origin().ascii_serialization())
            .map_err(|error| {
                RuntimeError::ContextService(format!("page has no valid origin: {error}"))
            })
    }

    pub(crate) fn validate_page_url(
        &self,
        source_page_id: PageId,
        target: &str,
    ) -> Result<(), RuntimeError> {
        let url = self.resolve_page_url(source_page_id, target)?;
        if !matches!(url.scheme(), "http" | "https" | "about") {
            return Err(RuntimeError::ContextService(format!(
                "page navigation scheme {:?} is not permitted",
                url.scheme()
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_page_commit_url(
        &self,
        source_page_id: PageId,
        target: &str,
    ) -> Result<(), RuntimeError> {
        let url = self.resolve_page_url(source_page_id, target)?;
        if !matches!(url.scheme(), "http" | "https" | "about" | "fixture") {
            return Err(RuntimeError::ContextService(format!(
                "page commit scheme {:?} is not permitted",
                url.scheme()
            )));
        }
        Ok(())
    }

    pub(crate) fn resolve_page_url(
        &self,
        source_page_id: PageId,
        target: &str,
    ) -> Result<Url, RuntimeError> {
        let source = self
            .pages()
            .into_iter()
            .find(|page| page.page.page_id == source_page_id)
            .ok_or(RuntimeError::PageNotFound(source_page_id))?;
        Url::parse(target)
            .or_else(|_| Url::parse(&source.page.url)?.join(target))
            .map_err(|error| RuntimeError::ContextService(error.to_string()))
    }

    pub(crate) fn may_script_close(&self, source: PageId, target: PageId) -> bool {
        source == target
            || self
                .pages()
                .into_iter()
                .find(|page| page.page.page_id == target)
                .is_some_and(|page| page.page.opener_page_id == Some(source))
    }

    #[cfg(unix)]
    pub(crate) fn commit_page_url(&self, page_id: PageId, url: String) -> Result<(), RuntimeError> {
        let mut pages = self.inner.group.pages.lock().expect("page group lock");
        let page = pages
            .get_mut(&page_id)
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        page.descriptor.page.url = url;
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn queue_script_navigation(
        &self,
        page_id: PageId,
        url: String,
    ) -> Result<(), RuntimeError> {
        let page = self
            .inner
            .group
            .handle(page_id)
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = runtime.block_on(page.goto(url));
            }
        });
        Ok(())
    }

    pub fn new_page(&self) -> PageHandle {
        let id = PageId(NEXT_PAGE_ID.fetch_add(1, Ordering::Relaxed));
        self.create_page(id, None, true, false)
    }

    pub(crate) fn new_page_with_identity(
        &self,
        page_id: PageId,
        opener_page_id: Option<PageId>,
    ) -> PageHandle {
        self.create_page(page_id, opener_page_id, true, false)
    }

    pub(crate) fn new_popup(
        &self,
        opener_page_id: PageId,
        initial_url: Option<String>,
    ) -> PageHandle {
        let page_id = PageId(NEXT_PAGE_ID.fetch_add(1, Ordering::Relaxed));
        let page = self.create_page(page_id, Some(opener_page_id), false, true);
        #[cfg(unix)]
        if let Some(descriptor) = self
            .pages()
            .into_iter()
            .find(|descriptor| descriptor.page.page_id == page_id)
        {
            self.push_context_event(ContextEvent::PageLifecycle {
                event: PageLifecycleEvent::Opened { page: descriptor },
            });
        }
        if let Some(url) = initial_url {
            let page = page.clone();
            std::thread::spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    let _ = runtime.block_on(page.goto(url));
                }
            });
        }
        page
    }

    fn create_page(
        &self,
        page_id: PageId,
        opener_page_id: Option<PageId>,
        activate: bool,
        attention_required: bool,
    ) -> PageHandle {
        #[cfg(unix)]
        if self.inner.worker_process {
            let handle = PageHandle {
                id: page_id.0,
                opener_page_id,
                context: Arc::downgrade(&self.inner),
                sender: None,
                worker: Some(std::sync::Arc::new(WorkerPage {
                    page_id,
                    opener_page_id,
                    context: Arc::downgrade(&self.inner),
                    client: std::sync::Mutex::new(None),
                    config: self.inner.config.clone(),
                    options: self.inner.options.clone(),
                    parent_cookies: self
                        .inner
                        .parent_cookies
                        .as_ref()
                        .expect("worker context owns a cookie jar")
                        .clone(),
                    pending: AtomicUsize::new(0),
                })),
            };
            self.inner.group.register(
                handle.clone(),
                opener_page_id,
                activate,
                attention_required,
                self.inner.config.document_generation_base,
            );
            return handle;
        }
        let (sender, mut receiver) = mpsc::channel(self.inner.config.worker_queue_capacity);
        let config = self.inner.config.clone();
        let context_options = self.inner.options.clone();
        let broker = self.inner.broker.clone();
        #[cfg(all(unix, feature = "v8-host"))]
        let context_client = self.inner.context_client.clone().or_else(|| {
            Some(SharedContextClient::direct(
                BrowserContext {
                    inner: self.inner.clone(),
                },
                page_id,
            ))
        });
        std::thread::spawn(move || {
            let mut state = PageState::new(
                page_id,
                opener_page_id,
                config,
                context_options,
                broker,
                #[cfg(all(unix, feature = "v8-host"))]
                context_client,
            );
            while let Some(command) = receiver.blocking_recv() {
                match command {
                    PageCommand::LoadHtml { url, html, reply } => {
                        state.trace.push(
                            Revision(state.revision),
                            None,
                            TraceEventKind::NavigationStarted { url: url.clone() },
                        );
                        if let Err(error) = state.load_document(
                            url.clone(),
                            &html,
                            true,
                            &BTreeMap::new(),
                            #[cfg(feature = "v8-host")]
                            Vec::new(),
                        ) {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                        let observation = state.observation(ProjectionProfile::Compact);
                        let _ = reply.send(Ok((Revision(state.revision), observation.page)));
                    }
                    PageCommand::Goto { url, reply } => {
                        let _ = reply.send(state.navigate(url, HistoryUpdate::Push));
                    }
                    PageCommand::Back { reply } => {
                        let _ = reply.send(state.traverse_history(-1));
                    }
                    PageCommand::Forward { reply } => {
                        let _ = reply.send(state.traverse_history(1));
                    }
                    PageCommand::Refresh { reply } => {
                        let _ = reply.send(state.refresh());
                    }
                    PageCommand::WaitFor { predicate, reply } => {
                        let _ = reply.send(state.wait_for(&predicate));
                    }
                    PageCommand::Evaluate { source, reply } => {
                        let _ = reply.send(state.evaluate(&source));
                    }
                    PageCommand::Capabilities { reply } => {
                        let _ = reply.send(state.capabilities());
                    }
                    PageCommand::ResolveSelector {
                        selector,
                        action,
                        reply,
                    } => {
                        let _ = reply.send(state.resolve_selector(&selector, action));
                    }
                    PageCommand::Extract {
                        kind,
                        selector,
                        reply,
                    } => {
                        let _ = reply.send(state.extract(kind, selector.as_deref()));
                    }
                    PageCommand::Observe { profile, reply } => {
                        let _ = reply.send(state.observation(profile));
                    }
                    PageCommand::ObserveWith { options, reply } => {
                        let _ = reply.send(state.observation_v4(options));
                    }
                    PageCommand::ComputedStyle { target, reply } => {
                        let _ = reply.send(state.computed_style(target));
                    }
                    PageCommand::BoxModel { target, reply } => {
                        let _ = reply.send(state.box_model(target));
                    }
                    PageCommand::PaintSnapshot { reply } => {
                        let _ = reply.send(state.paint_snapshot());
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
                        let _ = reply.send(state.delta_from(from));
                    }
                    PageCommand::Trace { reply } => {
                        let _ = reply.send(state.trace.events().to_vec());
                    }
                    PageCommand::Execute { action, reply } => {
                        let _ = reply.send(state.execute(action));
                    }
                    PageCommand::ExecuteForced { action, reply } => {
                        let _ = reply.send(state.execute_forced(action));
                    }
                    PageCommand::Close => break,
                }
            }
        });
        let handle = PageHandle {
            id: page_id.0,
            opener_page_id,
            context: Arc::downgrade(&self.inner),
            sender: Some(sender),
            #[cfg(unix)]
            worker: None,
        };
        self.inner.group.register(
            handle.clone(),
            opener_page_id,
            activate,
            attention_required,
            self.inner.config.document_generation_base,
        );
        handle
    }

    pub fn pages(&self) -> Vec<PageDescriptor> {
        self.inner.group.descriptors()
    }

    pub fn page_events(&self) -> Vec<PageLifecycleEvent> {
        self.inner
            .group
            .events
            .lock()
            .expect("page event lock")
            .clone()
    }

    pub fn active_page(&self) -> Option<PageHandle> {
        let page_id = *self.inner.group.active.lock().expect("active page lock");
        page_id.and_then(|page_id| self.inner.group.handle(page_id))
    }

    pub fn activate_page(&self, page_id: PageId) -> Result<PageHandle, RuntimeError> {
        let page = self.inner.group.activate(page_id)?;
        #[cfg(unix)]
        self.push_context_event(ContextEvent::PageLifecycle {
            event: PageLifecycleEvent::Activated { page_id },
        });
        Ok(page)
    }

    pub async fn wait_for_page(
        &self,
        predicate: PagePredicate,
        wait: Duration,
    ) -> Result<PageHandle, RuntimeError> {
        let future = async {
            loop {
                let notified = self.inner.group.notify.notified();
                if let Some(page) = self.inner.group.matching_page(&predicate) {
                    return page;
                }
                notified.await;
            }
        };
        tokio::time::timeout(wait, future)
            .await
            .map_err(|_| RuntimeError::PageWaitTimeout)
    }

    pub async fn close_page(&self, page_id: PageId) -> Result<(), RuntimeError> {
        let page = self
            .inner
            .group
            .handle(page_id)
            .ok_or(RuntimeError::PageNotFound(page_id))?;
        page.close().await
    }
}

impl PageHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn page_id(&self) -> PageId {
        PageId(self.id)
    }

    pub fn opener_page_id(&self) -> Option<PageId> {
        self.opener_page_id
    }

    fn local_sender(&self) -> Result<&mpsc::Sender<PageCommand>, RuntimeError> {
        self.sender.as_ref().ok_or(RuntimeError::Closed)
    }

    #[cfg(unix)]
    fn worker_request(
        &self,
        request: crate::worker::WorkerRequest,
    ) -> Option<Result<crate::worker::WorkerResponse, RuntimeError>> {
        self.worker.as_ref().map(|worker| {
            let result = worker.request(request);
            if let Err(
                RuntimeError::WorkerCrashed(detail) | RuntimeError::ProtocolViolation(detail),
            ) = &result
            {
                if let Some(context) = self.context.upgrade() {
                    context.group.crash(self.page_id(), detail.clone());
                }
            } else if let Err(RuntimeError::WorkerLimitExceeded { resource }) = &result
                && let Some(context) = self.context.upgrade()
            {
                context
                    .group
                    .crash(self.page_id(), format!("worker exceeded {resource} limit"));
            }
            result
        })
    }

    pub async fn load_html(
        &self,
        url: impl Into<String>,
        html: impl Into<String>,
    ) -> Result<Revision, RuntimeError> {
        let url = url.into();
        let html = html.into();
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::LoadHtml {
            url: url.clone(),
            html: html.clone(),
        }) {
            return match response? {
                crate::worker::WorkerResponse::Loaded { revision, page } => {
                    if let Some(context) = self.context.upgrade() {
                        context.group.update_loaded(page, revision);
                    }
                    Ok(revision)
                }
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::LoadHtml {
                url: url.clone(),
                html,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        let (revision, page) = response.await.map_err(|_| RuntimeError::Closed)??;
        if let Some(context) = self.context.upgrade() {
            context.group.update_loaded(page, revision);
        }
        Ok(revision)
    }

    pub async fn goto(&self, url: impl Into<String>) -> Result<NavigationResult, RuntimeError> {
        self.goto_authorized(url, None).await
    }

    pub async fn goto_authorized(
        &self,
        url: impl Into<String>,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<NavigationResult, RuntimeError> {
        let url = url.into();
        if let Some(context) = self.context.upgrade() {
            let context = BrowserContext { inner: context };
            let source_origin = context
                .page_origin(self.page_id())
                .unwrap_or_else(|_| "null".into());
            let target_origin = Url::parse(&url)
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_else(|_| "null".into());
            if source_origin != target_origin && source_origin != "null" {
                context.require_policy(
                    PolicyRequest {
                        action: ConsequentialAction::CrossOriginNavigation,
                        source_origin,
                        target_origin: Some(target_origin),
                        detail: Some(url.clone()),
                    },
                    confirmation,
                )?;
            }
        }
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::Goto { url: url.clone() })
        {
            let navigation = worker_navigation(response?)?;
            self.record_navigation(&navigation);
            return Ok(navigation);
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Goto { url, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        let navigation = response.await.map_err(|_| RuntimeError::Closed)??;
        self.record_navigation(&navigation);
        Ok(navigation)
    }

    pub async fn back(&self) -> Result<NavigationResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Back) {
            let navigation = worker_navigation(response?)?;
            self.record_navigation(&navigation);
            return Ok(navigation);
        }
        self.navigation_command(|reply| PageCommand::Back { reply })
            .await
    }

    pub async fn forward(&self) -> Result<NavigationResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Forward) {
            let navigation = worker_navigation(response?)?;
            self.record_navigation(&navigation);
            return Ok(navigation);
        }
        self.navigation_command(|reply| PageCommand::Forward { reply })
            .await
    }

    pub async fn refresh(&self) -> Result<NavigationResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Refresh) {
            let navigation = worker_navigation(response?)?;
            self.record_navigation(&navigation);
            return Ok(navigation);
        }
        self.navigation_command(|reply| PageCommand::Refresh { reply })
            .await
    }

    async fn navigation_command(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<NavigationResult, RuntimeError>>) -> PageCommand,
    ) -> Result<NavigationResult, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(command(reply))
            .await
            .map_err(|_| RuntimeError::Closed)?;
        let navigation = response.await.map_err(|_| RuntimeError::Closed)??;
        self.record_navigation(&navigation);
        Ok(navigation)
    }

    fn record_navigation(&self, navigation: &NavigationResult) {
        if let Some(context) = self.context.upgrade() {
            context.group.update_navigation(navigation);
        }
    }

    pub async fn wait_for(&self, predicate: WaitPredicate) -> Result<(), RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::WaitFor {
            predicate: predicate.clone(),
        }) {
            return match response? {
                crate::worker::WorkerResponse::Satisfied => Ok(()),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::WaitFor { predicate, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn evaluate(&self, source: impl Into<String>) -> Result<String, RuntimeError> {
        let source = source.into();
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Evaluate {
            source: source.clone(),
        }) {
            return match response? {
                crate::worker::WorkerResponse::Value { value } => Ok(value),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Evaluate { source, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn capabilities(&self) -> Result<Vec<CapabilityDiagnostic>, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Capabilities) {
            return match response? {
                crate::worker::WorkerResponse::Capabilities { capabilities } => Ok(capabilities),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Capabilities { reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)
    }

    pub async fn resolve_selector(
        &self,
        selector: impl Into<String>,
        action: SemanticAction,
    ) -> Result<SemanticRef, RuntimeError> {
        let selector = selector.into();
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::ResolveSelector {
            selector: selector.clone(),
            action,
        }) {
            return match response? {
                crate::worker::WorkerResponse::SemanticRef { semantic_ref } => Ok(semantic_ref),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ResolveSelector {
                selector,
                action,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn extract(
        &self,
        kind: ContentKind,
        selector: Option<String>,
    ) -> Result<serde_json::Value, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Extract {
            kind,
            selector: selector.clone(),
        }) {
            return match response? {
                crate::worker::WorkerResponse::Structured { value } => Ok(value),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Extract {
                kind,
                selector,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn observe(&self) -> Result<Observation, RuntimeError> {
        self.observe_profile(ProjectionProfile::Compact).await
    }

    pub async fn observe_profile(
        &self,
        profile: ProjectionProfile,
    ) -> Result<Observation, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Observe {
            profile: Some(profile),
        }) {
            return match response? {
                crate::worker::WorkerResponse::Observation { observation } => {
                    if let Some(context) = self.context.upgrade() {
                        context.group.update_observation(&observation);
                    }
                    Ok(observation)
                }
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Observe { profile, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        let observation = response.await.map_err(|_| RuntimeError::Closed)?;
        if let Some(context) = self.context.upgrade() {
            context.group.update_observation(&observation);
        }
        Ok(observation)
    }

    /// Observe the page with contract-v4 viewport and optional geometry data.
    pub async fn observe_with(
        &self,
        options: ObservationOptions,
    ) -> Result<oryn_common::v4::Observation, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::ObserveWith { options })
        {
            return match response? {
                crate::worker::WorkerResponse::ObservationV4 { observation } => Ok(observation),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ObserveWith { options, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    /// Frozen contract-v3 observation for compatibility callers.
    pub async fn observe_v3(&self) -> Result<Observation, RuntimeError> {
        self.observe().await
    }

    pub async fn computed_style(&self, target: SemanticRef) -> Result<ComputedStyle, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::ComputedStyle { target })
        {
            return match response? {
                crate::worker::WorkerResponse::ComputedStyle { style } => Ok(style),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ComputedStyle { target, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn box_model(&self, target: SemanticRef) -> Result<BoxModel, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::BoxModel { target })
        {
            return match response? {
                crate::worker::WorkerResponse::BoxModel { box_model } => Ok(box_model),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::BoxModel { target, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    /// Export a bounded, read-only scene input for an optional renderer.
    /// The page worker retains DOM/V8 ownership; image bytes are acquired only
    /// through its typed parent network broker.
    pub async fn paint_snapshot(&self) -> Result<PaintSnapshot, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::PaintSnapshot) {
            return match response? {
                crate::worker::WorkerResponse::PaintSnapshot { snapshot } => Ok(*snapshot),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::PaintSnapshot { reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn set_viewport(
        &self,
        viewport: ViewportState,
    ) -> Result<ViewportState, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::SetViewport { viewport })
        {
            return match response? {
                crate::worker::WorkerResponse::Viewport { viewport } => Ok(viewport),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::SetViewport { viewport, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn set_media(&self, media: MediaState) -> Result<MediaState, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::SetMedia { media })
        {
            return match response? {
                crate::worker::WorkerResponse::Media { media } => Ok(media),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::SetMedia { media, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn scroll(&self, request: ScrollRequest) -> Result<ScrollState, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::Scroll { request })
        {
            return match response? {
                crate::worker::WorkerResponse::Scroll { scroll } => Ok(scroll),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Scroll { request, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn scroll_to(&self, x: f64, y: f64) -> Result<ScrollState, RuntimeError> {
        self.scroll(ScrollRequest {
            x,
            y,
            relative: false,
        })
        .await
    }

    pub async fn scroll_by(&self, x: f64, y: f64) -> Result<ScrollState, RuntimeError> {
        self.scroll(ScrollRequest {
            x,
            y,
            relative: true,
        })
        .await
    }

    pub async fn scroll_into_view(&self, target: SemanticRef) -> Result<ScrollState, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::ScrollIntoView { target })
        {
            return match response? {
                crate::worker::WorkerResponse::Scroll { scroll } => Ok(scroll),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ScrollIntoView { target, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn observe_delta(&self, from: Revision) -> Result<ObservationDelta, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) =
            self.worker_request(crate::worker::WorkerRequest::ObserveDelta { from })
        {
            return match response? {
                crate::worker::WorkerResponse::Delta { delta } => Ok(delta),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ObserveDelta { from, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    /// Contract-v4 delta with geometry attached to changed semantic nodes.
    pub async fn observe_delta_v4(
        &self,
        from: Revision,
    ) -> Result<oryn_common::v4::ObservationDelta, RuntimeError> {
        let mut delta = oryn_common::v4::ObservationDelta::from_v3(self.observe_delta(from).await?);
        let observation = self
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Compact,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await?;
        let current = observation
            .nodes
            .iter()
            .map(|node| (node.semantic.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        for node in &mut delta.upserted {
            node.geometry = current
                .get(&node.semantic.semantic_ref)
                .and_then(|current| current.geometry.clone());
        }
        delta.style_revision = observation.style_revision;
        delta.layout_revision = observation.layout_revision;
        delta.viewport = Some(observation.viewport);
        delta.scroll = Some(observation.scroll);
        Ok(delta)
    }

    pub async fn trace(&self) -> Result<Vec<TraceEvent>, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Trace) {
            return match response? {
                crate::worker::WorkerResponse::Trace { events } => Ok(events),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Trace { reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)
    }

    pub async fn execute(&self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        self.execute_authorized(action, None).await
    }

    pub async fn execute_authorized(
        &self,
        action: NativeAction,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<ActionResult, RuntimeError> {
        if action.action == SemanticAction::Submit
            && let Some(context) = self.context.upgrade()
        {
            let context = BrowserContext { inner: context };
            context.require_policy(
                PolicyRequest {
                    action: ConsequentialAction::FormSubmission,
                    source_origin: context
                        .page_origin(self.page_id())
                        .unwrap_or_else(|_| "null".into()),
                    target_origin: None,
                    detail: Some("semantic form submission".into()),
                },
                confirmation,
            )?;
        }
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Execute {
            action: crate::worker::NativeActionRequest {
                target: action.target,
                action: action.action,
                value: action.value.clone(),
                force: false,
            },
        }) {
            return match response? {
                crate::worker::WorkerResponse::Action { result } => {
                    if let Some(context) = self.context.upgrade() {
                        context
                            .group
                            .update_revision(self.page_id(), result.revision_after);
                        if result.effects.iter().any(|effect| {
                            matches!(effect, Effect::PageClosed { page_id } if *page_id == self.page_id())
                        }) {
                            context.group.close(self.page_id());
                            BrowserContext { inner: context.clone() }.push_context_event(ContextEvent::PageLifecycle {
                                event: PageLifecycleEvent::Closed {
                                    page_id: self.page_id(),
                                },
                            });
                        }
                    }
                    Ok(result)
                }
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::Execute { action, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        let result = response.await.map_err(|_| RuntimeError::Closed)??;
        if let Some(context) = self.context.upgrade() {
            context
                .group
                .update_revision(self.page_id(), result.revision_after);
            #[cfg(unix)]
            if result.effects.iter().any(|effect| {
                matches!(effect, Effect::PageClosed { page_id } if *page_id == self.page_id())
            }) {
                context.group.close(self.page_id());
                BrowserContext { inner: context.clone() }.push_context_event(ContextEvent::PageLifecycle {
                    event: PageLifecycleEvent::Closed {
                        page_id: self.page_id(),
                    },
                });
            }
        }
        Ok(result)
    }

    /// Fill a control from an opaque parent-owned secret handle. The handle is
    /// origin-scoped and the operation is independently policy-controlled.
    pub async fn fill_secret(
        &self,
        target: SemanticRef,
        handle: SecretHandle,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<ActionResult, RuntimeError> {
        let context = self.context.upgrade().ok_or(RuntimeError::Closed)?;
        let context = BrowserContext { inner: context };
        let origin = context.page_origin(self.page_id())?;
        let request = PolicyRequest {
            action: ConsequentialAction::CredentialFill,
            source_origin: origin.clone(),
            target_origin: None,
            detail: Some("opaque secret fill".into()),
        };
        context.require_policy(request, confirmation)?;
        let bytes = context
            .inner
            .secrets
            .materialize(handle, &origin)
            .map_err(|error| RuntimeError::Secret(error.to_string()))?;
        let value = String::from_utf8(bytes)
            .map_err(|_| RuntimeError::Secret(SecretError::InvalidUtf8.to_string()))?;
        self.execute_authorized(
            NativeAction {
                target,
                action: SemanticAction::Type,
                value: Some(value),
            },
            None,
        )
        .await
    }

    pub(crate) async fn execute_forced(
        &self,
        action: NativeAction,
    ) -> Result<ActionResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Execute {
            action: crate::worker::NativeActionRequest {
                target: action.target,
                action: action.action,
                value: action.value.clone(),
                force: true,
            },
        }) {
            return match response? {
                crate::worker::WorkerResponse::Action { result } => Ok(result),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ExecuteForced { action, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    /// Execute an action and return the geometry-aware contract-v4 result.
    pub async fn execute_v4(
        &self,
        action: NativeAction,
    ) -> Result<oryn_common::v4::ActionResult, RuntimeError> {
        let result = self.execute(action).await?;
        let revision_before = result.revision_before;
        let mut result = oryn_common::v4::ActionResult::from_v3(result);
        let observation = self
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Compact,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await?;
        if let Some(delta) = &mut result.delta {
            let current = observation
                .nodes
                .iter()
                .map(|node| (node.semantic.semantic_ref, node))
                .collect::<BTreeMap<_, _>>();
            for node in &mut delta.upserted {
                node.geometry = current
                    .get(&node.semantic.semantic_ref)
                    .and_then(|current| current.geometry.clone());
            }
            delta.style_revision = observation.style_revision;
            delta.layout_revision = observation.layout_revision;
            delta.viewport = Some(observation.viewport);
            delta.scroll = Some(observation.scroll);
        }
        if observation.style_revision.0 > revision_before.0 {
            result.effects.push(oryn_common::v4::Effect::StyleChanged {
                style_revision: observation.style_revision,
            });
        }
        if observation.layout_revision.0 > revision_before.0 {
            result.effects.push(oryn_common::v4::Effect::LayoutChanged {
                layout_revision: observation.layout_revision,
            });
        }
        Ok(result)
    }

    pub async fn close(&self) -> Result<(), RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self.worker_request(crate::worker::WorkerRequest::Close) {
            return match response? {
                crate::worker::WorkerResponse::Closed => {
                    if let Some(context) = self.context.upgrade() {
                        context.group.close(self.page_id());
                        BrowserContext { inner: context }.push_context_event(
                            ContextEvent::PageLifecycle {
                                event: PageLifecycleEvent::Closed {
                                    page_id: self.page_id(),
                                },
                            },
                        );
                    }
                    Ok(())
                }
                response => Err(unexpected_worker_response(response)),
            };
        }
        self.local_sender()?
            .send(PageCommand::Close)
            .await
            .map_err(|_| RuntimeError::Closed)?;
        if let Some(context) = self.context.upgrade() {
            context.group.close(self.page_id());
            #[cfg(unix)]
            BrowserContext { inner: context }.push_context_event(ContextEvent::PageLifecycle {
                event: PageLifecycleEvent::Closed {
                    page_id: self.page_id(),
                },
            });
        }
        Ok(())
    }
}

#[cfg(unix)]
fn worker_navigation(
    response: crate::worker::WorkerResponse,
) -> Result<NavigationResult, RuntimeError> {
    match response {
        crate::worker::WorkerResponse::Navigated { navigation } => Ok(navigation),
        response => Err(unexpected_worker_response(response)),
    }
}

#[cfg(unix)]
fn unexpected_worker_response(response: crate::worker::WorkerResponse) -> RuntimeError {
    RuntimeError::ProtocolViolation(format!("unexpected worker response: {response:?}"))
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeError {
    #[error("page runtime is closed")]
    Closed,
    #[error("page {0:?} is not open in this browser context")]
    PageNotFound(PageId),
    #[error("timed out waiting for a matching page")]
    PageWaitTimeout,
    #[error("semantic reference belongs to page {actual:?}, expected {expected:?}")]
    WrongPage { expected: PageId, actual: PageId },
    #[error("semantic reference belongs to document generation {actual}, expected {expected}")]
    WrongDocumentGeneration { expected: u64, actual: u64 },
    #[error("semantic reference is stale or unknown")]
    StaleSemanticRef,
    #[error("{action:?} is not supported by this element")]
    UnsupportedAction { action: SemanticAction },
    #[error("semantic target is disabled")]
    Disabled,
    #[error("revision {0:?} is not available for delta observation")]
    UnknownRevision(Revision),
    #[error("native script realm failed: {0}")]
    Script(String),
    #[error("browser context service failed: {0}")]
    ContextService(String),
    #[error("network navigation failed: {0}")]
    Network(String),
    #[error("navigation history has no entry in that direction")]
    HistoryBoundary,
    #[error("wait predicate is not currently satisfied: {0}")]
    WaitUnsatisfied(String),
    #[error("JavaScript evaluation requires the v8-host feature")]
    EvaluationUnavailable,
    #[error("sandboxed native worker is unavailable: {0}")]
    SandboxUnavailable(String),
    #[error("sandboxed native worker crashed: {0}")]
    WorkerCrashed(String),
    #[error("sandboxed native worker exceeded {resource} limit")]
    WorkerLimitExceeded { resource: String },
    #[error("sandboxed native worker protocol violation: {0}")]
    ProtocolViolation(String),
    #[error("native layout is unavailable: {0}")]
    LayoutUnavailable(String),
    #[error("geometry is unsupported for this target: {0}")]
    UnsupportedGeometry(String),
    #[error("native paint snapshot is unavailable: {0}")]
    PaintUnavailable(String),
    #[error("semantic target is not interactable: {0}")]
    NotInteractable(String),
    #[error("invalid viewport: {0}")]
    InvalidViewport(String),
    #[error("host policy denied consequential action {action:?}")]
    PolicyDenied { action: ConsequentialAction },
    #[error("host confirmation is required for consequential action {action:?}")]
    ConfirmationRequired { action: ConsequentialAction },
    #[error("secret handle failed: {0}")]
    Secret(String),
    #[error("download quarantine failed: {0}")]
    Download(String),
    #[error("execution-domain handoff is invalid: {0}")]
    InvalidHandoff(String),
    #[error("execution-domain handoff is lossy and requires explicit approval: {losses:?}")]
    LossyHandoffRequiresApproval { losses: Vec<String> },
    #[error("trace replay failed: {0}")]
    Replay(String),
}

#[cfg(feature = "v8-host")]
fn map_host_error(error: crate::host::HostError) -> RuntimeError {
    match error {
        crate::host::HostError::LimitExceeded { resource } => {
            RuntimeError::WorkerLimitExceeded { resource }
        }
        error => RuntimeError::Script(error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAction {
    pub target: SemanticRef,
    pub action: SemanticAction,
    pub value: Option<String>,
}

struct PageState {
    page_id: PageId,
    opener_page_id: Option<PageId>,
    config: BrowserConfig,
    url: String,
    document_generation: u64,
    revision: u64,
    style_revision: u64,
    layout_revision: u64,
    viewport: ViewportState,
    scroll_state: ScrollState,
    next_action_id: u64,
    document: Option<ParsedDocument>,
    #[cfg(feature = "v8-host")]
    executable: Option<crate::executable::ExecutableDocument>,
    snapshots: BTreeMap<u64, Vec<oryn_common::v3::SemanticNodeView>>,
    trace: TraceLog,
    history: Vec<String>,
    history_index: Option<usize>,
    broker: Option<SharedNetworkBroker>,
    #[cfg(all(unix, feature = "v8-host"))]
    context_client: Option<crate::context_service::SharedContextClient>,
    #[cfg(feature = "v8-host")]
    resource_cache: BTreeMap<String, String>,
    #[cfg(feature = "v8-host")]
    web_fonts: Vec<oryn_layout::WebFont>,
    #[cfg(feature = "v8-host")]
    layout: Option<oryn_layout::LayoutOutput>,
    #[cfg(feature = "v8-host")]
    color_scheme: PreferredColorScheme,
    #[cfg(feature = "v8-host")]
    reduced_motion: ReducedMotion,
    #[cfg(feature = "v8-host")]
    determinism: Option<crate::replay::DeterminismConfig>,
}

#[derive(Debug, Clone, Copy)]
enum HistoryUpdate {
    Push,
    Preserve,
    Reload,
}

impl PageState {
    fn new(
        page_id: PageId,
        opener_page_id: Option<PageId>,
        config: BrowserConfig,
        context_options: ContextOptions,
        broker: Option<SharedNetworkBroker>,
        #[cfg(all(unix, feature = "v8-host"))] context_client: Option<
            crate::context_service::SharedContextClient,
        >,
    ) -> Self {
        let document_generation = config.document_generation_base;
        Self {
            page_id,
            opener_page_id,
            config,
            url: String::new(),
            document_generation,
            revision: 0,
            style_revision: 0,
            layout_revision: 0,
            viewport: context_options.viewport,
            scroll_state: ScrollState {
                content_width: context_options.viewport.width,
                content_height: context_options.viewport.height,
                ..ScrollState::default()
            },
            next_action_id: 0,
            document: None,
            #[cfg(feature = "v8-host")]
            executable: None,
            snapshots: BTreeMap::new(),
            trace: TraceLog::new(page_id),
            history: Vec::new(),
            history_index: None,
            broker,
            #[cfg(all(unix, feature = "v8-host"))]
            context_client,
            #[cfg(feature = "v8-host")]
            resource_cache: BTreeMap::new(),
            #[cfg(feature = "v8-host")]
            web_fonts: Vec::new(),
            #[cfg(feature = "v8-host")]
            layout: None,
            #[cfg(feature = "v8-host")]
            color_scheme: context_options.color_scheme,
            #[cfg(feature = "v8-host")]
            reduced_motion: context_options.reduced_motion,
            #[cfg(feature = "v8-host")]
            determinism: context_options.determinism,
        }
    }

    fn load_document(
        &mut self,
        url: String,
        html: &str,
        push_history: bool,
        resources: &BTreeMap<String, String>,
        #[cfg(feature = "v8-host")] web_fonts: Vec<oryn_layout::WebFont>,
    ) -> Result<(), RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        let _ = resources;
        self.document_generation += 1;
        self.revision += 1;
        self.url = url;
        // A new top-level document starts at the viewport origin. Carrying a
        // prior document's scroll offset into layout can make otherwise
        // visible controls fail pointer-interactability until another layout
        // mutation happens to repair the stale viewport relationship.
        self.scroll_state = ScrollState {
            content_width: self.viewport.width,
            content_height: self.viewport.height,
            ..ScrollState::default()
        };
        #[cfg(feature = "v8-host")]
        {
            self.layout = None;
        }
        #[cfg(feature = "v8-host")]
        {
            self.web_fonts = web_fonts;
            // V8 enters isolates on creation and requires reverse-order drops;
            // retire the previous document realm before creating its successor.
            let storage = self
                .executable
                .as_mut()
                .and_then(|executable| executable.storage_snapshot().ok())
                .unwrap_or_else(|| ("[]".into(), "[]".into()));
            self.executable = None;
            let mut resources = resources.clone();
            resources.insert("oryn:local-storage".into(), storage.0);
            resources.insert("oryn:session-storage".into(), storage.1);
            if let Some(determinism) = &self.determinism {
                resources.insert(
                    "oryn:determinism".into(),
                    serde_json::to_string(determinism)
                        .map_err(|error| RuntimeError::Replay(error.to_string()))?,
                );
            }
            #[cfg(unix)]
            let executable = match self.context_client.clone() {
                Some(context_client) => {
                    crate::executable::ExecutableDocument::load_with_context_and_limits(
                        html,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                        (context_client, self.page_id, self.opener_page_id),
                    )
                }
                None => crate::executable::ExecutableDocument::load_with_limits(
                    html,
                    &resources,
                    self.broker.clone(),
                    Url::parse(&self.url).ok(),
                    self.config.max_v8_heap_bytes,
                ),
            }
            .map_err(map_host_error)?;
            #[cfg(not(unix))]
            let executable = crate::executable::ExecutableDocument::load_with_limits(
                html,
                &resources,
                self.broker.clone(),
                Url::parse(&self.url).ok(),
                self.config.max_v8_heap_bytes,
            )
            .map_err(map_host_error)?;
            for diagnostic in &executable.diagnostics {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::ScriptDiagnostic {
                        capability: diagnostic.capability.clone(),
                        detail: diagnostic.detail.clone().unwrap_or_default(),
                    },
                );
            }
            self.document = Some(executable.document.clone());
            self.executable = Some(executable);
            let (console_entries, context_entries) = self
                .executable
                .as_mut()
                .map(|executable| {
                    (
                        executable.take_console_entries().unwrap_or_default(),
                        executable.take_context_trace_entries().unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            for entry in console_entries {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::Console {
                        level: entry.level,
                        message: entry.message,
                    },
                );
            }
            for entry in context_entries {
                if let Some(kind) = context_trace_kind(entry) {
                    self.trace.push(Revision(self.revision), None, kind);
                }
            }
        }
        #[cfg(not(feature = "v8-host"))]
        {
            self.document = Some(parse(html));
        }
        #[cfg(feature = "v8-host")]
        self.refresh_layout()?;
        if push_history {
            if let Some(index) = self.history_index {
                self.history.truncate(index + 1);
            }
            self.history.push(self.url.clone());
            self.history_index = Some(self.history.len() - 1);
        }
        self.record_snapshot();
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::NavigationCommitted {
                url: self.url.clone(),
            },
        );
        for state in [
            LifecycleState::DomContentLoaded,
            LifecycleState::Load,
            LifecycleState::NetworkIdle,
        ] {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::LifecycleChanged { state },
            );
        }
        Ok(())
    }

    fn navigate(
        &mut self,
        requested_url: String,
        history_update: HistoryUpdate,
    ) -> Result<NavigationResult, RuntimeError> {
        let mut current = Url::parse(&requested_url)
            .map_err(|error| RuntimeError::Network(format!("invalid URL: {error}")))?;
        let mut redirects = 0_usize;
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::NavigationStarted {
                url: requested_url.clone(),
            },
        );
        if !matches!(history_update, HistoryUpdate::Reload)
            && self.document.is_some()
            && same_document_url(&self.url, current.as_str())
        {
            self.url = current.to_string();
            if matches!(history_update, HistoryUpdate::Push) {
                if let Some(index) = self.history_index {
                    self.history.truncate(index + 1);
                }
                self.history.push(self.url.clone());
                self.history_index = Some(self.history.len() - 1);
            }
            self.record_snapshot();
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::NavigationCommitted {
                    url: self.url.clone(),
                },
            );
            return Ok(NavigationResult {
                page_id: self.page_id,
                requested_url,
                final_url: self.url.clone(),
                title: self
                    .document
                    .as_ref()
                    .map(document_title)
                    .unwrap_or_default(),
                status: 200,
                document_generation: self.document_generation,
                revision: Revision(self.revision),
                lifecycle: vec![LifecycleState::Commit],
            });
        }
        if current.as_str() == "about:blank" {
            self.load_document(
                current.to_string(),
                "<!doctype html>",
                matches!(history_update, HistoryUpdate::Push),
                &BTreeMap::new(),
                #[cfg(feature = "v8-host")]
                Vec::new(),
            )?;
            return Ok(NavigationResult {
                page_id: self.page_id,
                requested_url,
                final_url: self.url.clone(),
                title: String::new(),
                status: 200,
                document_generation: self.document_generation,
                revision: Revision(self.revision),
                lifecycle: vec![
                    LifecycleState::Commit,
                    LifecycleState::DomContentLoaded,
                    LifecycleState::Load,
                    LifecycleState::NetworkIdle,
                ],
            });
        }
        let (status, body) = loop {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::RequestStarted {
                    method: "GET".into(),
                    url: current.to_string(),
                },
            );
            let broker = self.broker.as_ref().ok_or_else(|| {
                RuntimeError::Network("network transport initialization failed".into())
            })?;
            let response = broker
                .lock()
                .map_err(|_| RuntimeError::Network("network broker lock poisoned".into()))?
                .send(NetworkRequest {
                    request_id: self.revision + redirects as u64 + 1,
                    method: "GET".into(),
                    url: current.to_string(),
                    headers: vec![(
                        "accept".into(),
                        "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8".into(),
                    )],
                    body: Vec::new(),
                    credentials: true,
                    resolved_addrs: Vec::new(),
                })
                .map_err(|error| {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::PolicyDenied {
                            capability: format!("network.navigation: {error}"),
                        },
                    );
                    RuntimeError::Network(error.to_string())
                })?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ResponseReceived {
                    url: current.to_string(),
                    status: response.status,
                    bytes: response.body.len(),
                },
            );
            if (300..400).contains(&response.status) {
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| RuntimeError::Network("redirect omitted Location".into()))?;
                redirects += 1;
                if redirects > self.config.max_redirects {
                    return Err(RuntimeError::Network(format!(
                        "redirect limit ({}) exceeded",
                        self.config.max_redirects
                    )));
                }
                current = current
                    .join(location)
                    .map_err(|error| RuntimeError::Network(format!("invalid redirect: {error}")))?;
                continue;
            }
            break (response.status, response.body);
        };
        let html = String::from_utf8_lossy(&body).into_owned();
        let html = self.inline_external_stylesheets(&current, &html)?;
        #[cfg(feature = "v8-host")]
        let web_fonts = self.load_web_fonts(&current, &html)?;
        let resources = self.load_external_scripts(&current, &html)?;
        self.load_document(
            current.to_string(),
            &html,
            matches!(history_update, HistoryUpdate::Push),
            &resources,
            #[cfg(feature = "v8-host")]
            web_fonts,
        )?;
        Ok(NavigationResult {
            page_id: self.page_id,
            requested_url,
            final_url: self.url.clone(),
            title: self
                .document
                .as_ref()
                .map(document_title)
                .unwrap_or_default(),
            status,
            document_generation: self.document_generation,
            revision: Revision(self.revision),
            lifecycle: vec![
                LifecycleState::Commit,
                LifecycleState::DomContentLoaded,
                LifecycleState::Load,
                LifecycleState::NetworkIdle,
            ],
        })
    }

    #[cfg(feature = "v8-host")]
    fn inline_external_stylesheets(
        &mut self,
        base_url: &Url,
        html: &str,
    ) -> Result<String, RuntimeError> {
        let parsed = parse(html);
        let hrefs = parsed
            .dom
            .iter()
            .filter_map(|(_, node)| match &node.kind {
                crate::dom::NodeKind::Element { name }
                    if name.local.as_ref() == "link"
                        && node.attributes.get("rel").is_some_and(|value| {
                            value
                                .split_whitespace()
                                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                        }) =>
                {
                    node.attributes.get("href").cloned()
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut styles = String::new();
        for href in hrefs {
            let absolute = base_url.join(&href).map_err(|error| {
                RuntimeError::Network(format!("invalid stylesheet URL {href}: {error}"))
            })?;
            let css = self.load_stylesheet_recursive(&absolute, 0, &mut BTreeSet::new())?;
            styles.push_str("\n<style data-oryn-external=\"");
            styles.push_str(&href.replace('&', "&amp;").replace('"', "&quot;"));
            styles.push_str("\">\n");
            styles.push_str(&css);
            styles.push_str("\n</style>");
        }
        let mut expanded = html.to_string();
        let head_end = expanded.to_ascii_lowercase().rfind("</head>");
        if let Some(index) = head_end {
            expanded.insert_str(index, &styles);
        } else {
            expanded.push_str(&styles);
        }
        self.expand_inline_style_imports(base_url, &expanded)
    }

    #[cfg(feature = "v8-host")]
    fn expand_inline_style_imports(
        &mut self,
        base_url: &Url,
        html: &str,
    ) -> Result<String, RuntimeError> {
        let styles = regex::Regex::new(r"(?is)(<style\b[^>]*>)(.*?)(</style\s*>)")
            .expect("constant style element regex");
        let mut output = String::with_capacity(html.len());
        let mut cursor = 0usize;
        for capture in styles.captures_iter(html) {
            let complete = capture.get(0).expect("complete style capture");
            output.push_str(&html[cursor..complete.start()]);
            output.push_str(&capture[1]);
            output.push_str(&self.expand_css_imports(
                base_url,
                &capture[2],
                0,
                &mut BTreeSet::new(),
            )?);
            output.push_str(&capture[3]);
            cursor = complete.end();
        }
        output.push_str(&html[cursor..]);
        Ok(output)
    }

    #[cfg(feature = "v8-host")]
    fn load_stylesheet_recursive(
        &mut self,
        url: &Url,
        depth: usize,
        visited: &mut BTreeSet<String>,
    ) -> Result<String, RuntimeError> {
        if depth > self.config.max_redirects {
            return Err(RuntimeError::Network(format!(
                "stylesheet @import depth exceeded {}",
                self.config.max_redirects
            )));
        }
        let cache_key = url.to_string();
        if !visited.insert(cache_key.clone()) {
            return Ok(String::new());
        }
        let css = if let Some(cached) = self.resource_cache.get(&cache_key) {
            cached.clone()
        } else {
            let css =
                self.fetch_external_resource(url.clone(), "text/css,*/*;q=0.1", "stylesheet")?;
            self.resource_cache.insert(cache_key, css.clone());
            css
        };
        self.expand_css_imports(url, &css, depth, visited)
    }

    #[cfg(feature = "v8-host")]
    fn expand_css_imports(
        &mut self,
        base_url: &Url,
        css: &str,
        depth: usize,
        visited: &mut BTreeSet<String>,
    ) -> Result<String, RuntimeError> {
        if depth > self.config.max_redirects {
            return Err(RuntimeError::Network(format!(
                "stylesheet @import depth exceeded {}",
                self.config.max_redirects
            )));
        }
        let imports = regex::Regex::new(
            r#"(?is)@import\s+(?:url\(\s*)?['\"]?([^'\"\)\s;]+)['\"]?\s*\)?[^;]*;"#,
        )
        .expect("constant import regex");
        let mut expanded = String::new();
        let mut cursor = 0usize;
        for capture in imports.captures_iter(css) {
            let complete = capture.get(0).expect("complete import capture");
            expanded.push_str(&css[cursor..complete.start()]);
            let import_url = base_url.join(&capture[1]).map_err(|error| {
                RuntimeError::Network(format!(
                    "invalid stylesheet @import {}: {error}",
                    &capture[1]
                ))
            })?;
            expanded.push_str(&self.load_stylesheet_recursive(&import_url, depth + 1, visited)?);
            cursor = complete.end();
        }
        expanded.push_str(&css[cursor..]);
        let urls = regex::Regex::new(r#"(?is)url\(\s*(['\"]?)([^'\")]+)['\"]?\s*\)"#)
            .expect("constant CSS URL regex");
        Ok(urls
            .replace_all(&expanded, |capture: &regex::Captures<'_>| {
                let value = capture[2].trim();
                if value.starts_with("data:") || value.starts_with('#') {
                    capture[0].to_string()
                } else {
                    base_url
                        .join(value)
                        .map(|absolute| format!("url(\"{absolute}\")"))
                        .unwrap_or_else(|_| capture[0].to_string())
                }
            })
            .into_owned())
    }

    #[cfg(feature = "v8-host")]
    fn load_web_fonts(
        &mut self,
        base_url: &Url,
        html: &str,
    ) -> Result<Vec<oryn_layout::WebFont>, RuntimeError> {
        let parsed = parse(html);
        let css = parsed
            .dom
            .iter()
            .filter_map(|(id, node)| {
                matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "style")
                    .then(|| parsed.dom.text_content(id).ok())
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let faces =
            regex::Regex::new(r"(?is)@font-face\s*\{([^}]*)\}").expect("constant font-face regex");
        let family = regex::Regex::new(r"(?is)font-family\s*:\s*([^;]+)")
            .expect("constant font-family regex");
        let source = regex::Regex::new(r#"(?is)url\(\s*['\"]?([^'\")\s]+)['\"]?\s*\)"#)
            .expect("constant font source regex");
        let mut fonts = Vec::new();
        let mut total = 0usize;
        for face in faces.captures_iter(&css) {
            let block = &face[1];
            let Some(family) = family
                .captures(block)
                .map(|capture| capture[1].trim().trim_matches(['\'', '"']).to_string())
            else {
                continue;
            };
            let Some(source) = source.captures(block).map(|capture| capture[1].to_string()) else {
                continue;
            };
            if source.starts_with("data:") {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::ScriptDiagnostic {
                        capability: "font.data_url".into(),
                        detail:
                            "data URL fonts are not accepted by the bounded brokered font loader"
                                .into(),
                    },
                );
                continue;
            }
            let absolute = base_url.join(&source).map_err(|error| {
                RuntimeError::Network(format!("invalid web-font URL {source}: {error}"))
            })?;
            let bytes = self.fetch_external_resource_bytes(
                absolute.clone(),
                "font/woff2,font/woff,application/font-sfnt,*/*;q=0.1",
                "web font",
            )?;
            if bytes.len() > self.config.max_font_bytes {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: format!(
                        "web font {} bytes (limit {})",
                        bytes.len(),
                        self.config.max_font_bytes
                    ),
                });
            }
            total = total.saturating_add(bytes.len());
            if total > self.config.max_page_font_bytes {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: format!(
                        "page web fonts {total} bytes (limit {})",
                        self.config.max_page_font_bytes
                    ),
                });
            }
            fonts.push(oryn_layout::WebFont {
                family,
                source_url: absolute.to_string(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                bytes,
            });
        }
        Ok(fonts)
    }

    #[cfg(not(feature = "v8-host"))]
    fn inline_external_stylesheets(
        &mut self,
        _base_url: &Url,
        html: &str,
    ) -> Result<String, RuntimeError> {
        Ok(html.to_string())
    }

    #[cfg(feature = "v8-host")]
    fn load_external_scripts(
        &mut self,
        base_url: &Url,
        html: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        let parsed = parse(html);
        let mut import_map: BTreeMap<String, String> = BTreeMap::new();
        let mut sources = parsed
            .dom
            .iter()
            .filter_map(|(_, node)| match &node.kind {
                crate::dom::NodeKind::Element { name }
                    if matches!(name.local.as_ref(), "script" | "iframe") =>
                {
                    node.attributes.get("src").cloned().map(|source| {
                        (
                            source,
                            name.local.as_ref() == "iframe",
                            name.local.as_ref() == "script"
                                && node.attributes.get("type").map(String::as_str)
                                    == Some("module"),
                        )
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (id, node) in parsed.dom.iter() {
            let crate::dom::NodeKind::Element { name } = &node.kind else {
                continue;
            };
            if name.local.as_ref() == "script" && !node.attributes.contains_key("src") {
                let source = parsed.dom.text_content(id).unwrap_or_default();
                if node.attributes.get("type").map(String::as_str) == Some("importmap")
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&source)
                    && let Some(imports) =
                        value.get("imports").and_then(serde_json::Value::as_object)
                {
                    for (specifier, target) in imports {
                        if let Some(target) = target.as_str() {
                            import_map.insert(specifier.clone(), target.to_string());
                        }
                    }
                }
                sources.extend(
                    module_specifiers(&source)
                        .into_iter()
                        .map(|specifier| (specifier, false, true)),
                );
            }
            if name.local.as_ref() == "link"
                && node.attributes.get("rel").is_some_and(|value| {
                    value.split_whitespace().any(|token| {
                        matches!(
                            token.to_ascii_lowercase().as_str(),
                            "modulepreload" | "preload" | "prefetch"
                        )
                    })
                })
                && node
                    .attributes
                    .get("as")
                    .is_none_or(|value| value.eq_ignore_ascii_case("script"))
                && let Some(href) = node.attributes.get("href")
            {
                let relation = node.attributes.get("rel").map(String::as_str).unwrap_or("");
                let module = relation.split_whitespace().any(|token| {
                    matches!(
                        token.to_ascii_lowercase().as_str(),
                        "modulepreload" | "prefetch"
                    )
                });
                sources.push((href.clone(), false, module));
            }
        }
        // html5ever deliberately treats some script data forms differently
        // from ordinary text nodes. Parse import-map script bodies from the
        // original response as well so aliases remain available to every
        // preloaded module, including Nuxt's generated `#entry` imports.
        let import_map_scripts = regex::Regex::new(
            r#"(?is)<script\b[^>]*\btype\s*=\s*[\"'](?:importmap|application/importmap\+json)[\"'][^>]*>(.*?)</script>"#,
        )
        .expect("valid import-map expression");
        for captures in import_map_scripts.captures_iter(html) {
            let Some(source) = captures.get(1).map(|capture| capture.as_str()) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(source) else {
                continue;
            };
            let Some(imports) = value.get("imports").and_then(serde_json::Value::as_object) else {
                continue;
            };
            for (specifier, target) in imports {
                if let Some(target) = target.as_str() {
                    import_map.insert(specifier.clone(), target.to_string());
                }
            }
        }
        let mut resources = BTreeMap::new();
        let mut pending = sources
            .into_iter()
            .map(|(source, frame, module)| (base_url.clone(), source, frame, module))
            .collect::<Vec<_>>();
        while let Some((parent, source, frame, module)) = pending.pop() {
            if resources.contains_key(&source) {
                continue;
            }
            let absolute = parent.join(&source).map_err(|error| {
                RuntimeError::Network(format!("invalid external script URL {source}: {error}"))
            })?;
            if frame && origin_key(&absolute) != origin_key(base_url) {
                continue;
            }
            let cache_key = absolute.to_string();
            let (request_url, expected_digest) = pinned_resource(&absolute)
                .map(|(url, digest)| (Url::parse(url).expect("valid pinned URL"), Some(digest)))
                .unwrap_or_else(|| (absolute.clone(), None));
            let body = if let Some(cached) = self.resource_cache.get(&cache_key) {
                cached.clone()
            } else {
                let body = self.fetch_external_resource(
                    request_url,
                    "text/javascript,*/*;q=0.1",
                    "script",
                )?;
                if let Some(expected) = expected_digest {
                    let actual = format!("{:x}", Sha256::digest(body.as_bytes()));
                    if actual != expected {
                        return Err(RuntimeError::Network(format!(
                            "pinned resource integrity failure for {absolute}: expected {expected}, got {actual}"
                        )));
                    }
                }
                self.resource_cache.insert(cache_key, body.clone());
                body
            };
            if module {
                for dependency in module_specifiers(&body) {
                    let dependency = import_map.get(&dependency).cloned().unwrap_or(dependency);
                    if !resources.contains_key(&dependency) {
                        pending.push((absolute.clone(), dependency, false, true));
                    }
                }
            }
            resources.insert(source, body);
        }
        for (specifier, target) in import_map {
            if resources.contains_key(&target) {
                resources.insert(format!("oryn:module-alias:{specifier}"), target);
            }
        }
        Ok(resources)
    }

    #[cfg(not(feature = "v8-host"))]
    fn load_external_scripts(
        &mut self,
        _base_url: &Url,
        _html: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        Ok(BTreeMap::new())
    }

    #[cfg(feature = "v8-host")]
    fn fetch_external_resource(
        &mut self,
        url: Url,
        accept: &str,
        resource_kind: &str,
    ) -> Result<String, RuntimeError> {
        self.fetch_external_resource_bytes(url, accept, resource_kind)
            .map(|body| String::from_utf8_lossy(&body).into_owned())
    }

    #[cfg(feature = "v8-host")]
    fn fetch_external_resource_bytes(
        &mut self,
        mut url: Url,
        accept: &str,
        resource_kind: &str,
    ) -> Result<Vec<u8>, RuntimeError> {
        for redirect in 0..=self.config.max_redirects {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::RequestStarted {
                    method: "GET".into(),
                    url: url.to_string(),
                },
            );
            let response = self
                .broker
                .as_ref()
                .ok_or_else(|| {
                    RuntimeError::Network("network transport initialization failed".into())
                })?
                .lock()
                .map_err(|_| RuntimeError::Network("network broker lock poisoned".into()))?
                .send(NetworkRequest {
                    request_id: self.revision + redirect as u64 + 10_000,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers: vec![("accept".into(), accept.into())],
                    body: Vec::new(),
                    credentials: true,
                    resolved_addrs: Vec::new(),
                })
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ResponseReceived {
                    url: url.to_string(),
                    status: response.status,
                    bytes: response.body.len(),
                },
            );
            if (300..400).contains(&response.status) {
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        RuntimeError::Network(format!("{resource_kind} redirect omitted Location"))
                    })?;
                url = url.join(location).map_err(|error| {
                    RuntimeError::Network(format!("invalid {resource_kind} redirect: {error}"))
                })?;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(RuntimeError::Network(format!(
                    "external {resource_kind} returned HTTP {}",
                    response.status,
                )));
            }
            return Ok(response.body);
        }
        Err(RuntimeError::Network(format!(
            "{resource_kind} redirect limit ({}) exceeded",
            self.config.max_redirects,
        )))
    }

    fn traverse_history(&mut self, delta: isize) -> Result<NavigationResult, RuntimeError> {
        let current = self.history_index.ok_or(RuntimeError::HistoryBoundary)? as isize;
        let next = current + delta;
        if next < 0 || next >= self.history.len() as isize {
            return Err(RuntimeError::HistoryBoundary);
        }
        let requested = self.history[next as usize].clone();
        let result = self.navigate(requested, HistoryUpdate::Preserve)?;
        self.history_index = Some(next as usize);
        Ok(result)
    }

    fn refresh(&mut self) -> Result<NavigationResult, RuntimeError> {
        if self.url.is_empty() {
            return Err(RuntimeError::HistoryBoundary);
        }
        self.navigate(self.url.clone(), HistoryUpdate::Reload)
    }

    fn wait_for(&mut self, predicate: &WaitPredicate) -> Result<(), RuntimeError> {
        #[cfg(feature = "v8-host")]
        if matches!(
            predicate,
            WaitPredicate::SelectorVisible(_) | WaitPredicate::SelectorHidden(_)
        ) {
            // Visibility predicates must use the same stylesheet-aware layout
            // state that gates actions. The semantic projection alone only
            // knows structural/inline visibility and can otherwise report a
            // class-hidden element as ready before its timer fires.
            self.refresh_layout()?;
        }
        if self.wait_satisfied(predicate) {
            return Ok(());
        }
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_mut() {
            executable.drain_tasks().map_err(map_host_error)?;
            self.document = Some(executable.document.clone());
            self.refresh_layout()?;
        }
        self.wait_satisfied(predicate)
            .then_some(())
            .ok_or_else(|| RuntimeError::WaitUnsatisfied(format!("{predicate:?}")))
    }

    fn wait_satisfied(&self, predicate: &WaitPredicate) -> bool {
        match predicate {
            WaitPredicate::Lifecycle(_) => self.document.is_some(),
            WaitPredicate::UrlContains(fragment) => self.url.contains(fragment),
            WaitPredicate::SelectorExists(selector) => self
                .document
                .as_ref()
                .is_some_and(|document| selector_nodes(document, selector).next().is_some()),
            WaitPredicate::SelectorGone(selector) => !self
                .document
                .as_ref()
                .is_some_and(|document| selector_nodes(document, selector).next().is_some()),
            WaitPredicate::SelectorVisible(selector) => {
                self.document.as_ref().is_some_and(|document| {
                    selector_nodes(document, selector).any(|id| {
                        #[cfg(feature = "v8-host")]
                        if let Some(geometry) = self
                            .layout
                            .as_ref()
                            .and_then(|layout| layout.geometry.get(&pack_node_id(id)))
                        {
                            // Unsupported geometry is not the same as hidden:
                            // table content, for example, can be visibly
                            // present even though G4 intentionally exposes no
                            // table box geometry.
                            return geometry.availability
                                != oryn_common::v4::GeometryAvailability::Hidden;
                        }
                        project_element(
                            &document.dom,
                            id,
                            self.page_id,
                            self.document_generation,
                            id.slot + 1,
                        )
                        .is_some_and(|node| {
                            node.states.contains(&oryn_common::v3::NodeState::Visible)
                        })
                    })
                })
            }
            WaitPredicate::SelectorHidden(selector) => {
                self.document.as_ref().is_some_and(|document| {
                    let mut nodes = selector_nodes(document, selector).peekable();
                    nodes.peek().is_none()
                        || nodes.all(|id| {
                            #[cfg(feature = "v8-host")]
                            if let Some(geometry) = self
                                .layout
                                .as_ref()
                                .and_then(|layout| layout.geometry.get(&pack_node_id(id)))
                            {
                                return geometry.availability
                                    == oryn_common::v4::GeometryAvailability::Hidden;
                            }
                            project_element(
                                &document.dom,
                                id,
                                self.page_id,
                                self.document_generation,
                                id.slot + 1,
                            )
                            .is_some_and(|node| {
                                node.states.contains(&oryn_common::v3::NodeState::Hidden)
                            })
                        })
                })
            }
        }
    }

    fn evaluate(&mut self, source: &str) -> Result<String, RuntimeError> {
        #[cfg(feature = "v8-host")]
        {
            let executable = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::EvaluationUnavailable)?;
            let value = executable.evaluate(source).map_err(map_host_error)?;
            self.document = Some(executable.document.clone());
            let console_entries = executable.take_console_entries().unwrap_or_default();
            let context_entries = executable.take_context_trace_entries().unwrap_or_default();
            for entry in console_entries {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::Console {
                        level: entry.level,
                        message: entry.message,
                    },
                );
            }
            for entry in context_entries {
                if let Some(kind) = context_trace_kind(entry) {
                    self.trace.push(Revision(self.revision), None, kind);
                }
            }
            self.refresh_layout()?;
            Ok(value)
        }
        #[cfg(not(feature = "v8-host"))]
        {
            let _ = source;
            Err(RuntimeError::EvaluationUnavailable)
        }
    }

    #[cfg(feature = "v8-host")]
    fn refresh_layout(&mut self) -> Result<(), RuntimeError> {
        let layout_document = self
            .executable
            .as_mut()
            .map(|executable| executable.layout_snapshot())
            .transpose()
            .map_err(map_host_error)?;
        let document = layout_document
            .as_ref()
            .or(self.document.as_ref())
            .ok_or_else(|| RuntimeError::LayoutUnavailable("page has no document".into()))?;
        let input = layout_input(
            document,
            self.viewport,
            self.scroll_state,
            self.color_scheme,
            self.reduced_motion,
            &self.web_fonts,
        );
        let limits = oryn_layout::LayoutLimits {
            max_nodes: self.config.max_layout_nodes,
            max_font_bytes: self.config.max_font_bytes,
            max_page_font_bytes: self.config.max_page_font_bytes,
            max_viewport_css_px: f64::from(self.config.max_viewport_css_px),
        };
        let output = oryn_layout::compute(&input, &limits)
            .map_err(|error| RuntimeError::LayoutUnavailable(error.to_string()))?;
        self.style_revision = self.style_revision.saturating_add(1);
        self.layout_revision = self.layout_revision.saturating_add(1);
        self.scroll_state = output.scroll;
        if let Some(executable) = self.executable.as_mut() {
            let payload = serde_json::json!({
                "styles": output.computed_styles,
                "geometry": output.geometry,
                "viewport": self.viewport,
                "scroll": self.scroll_state,
                "media": MediaState {
                    color_scheme: self.color_scheme,
                    reduced_motion: self.reduced_motion,
                },
            });
            let payload = serde_json::to_string(&payload)
                .map_err(|error| RuntimeError::Script(error.to_string()))?;
            executable
                .evaluate(&format!("__oryn_install_layout({payload});undefined"))
                .map_err(map_host_error)?;
            self.document = Some(executable.document.clone());
        }
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::StyleRecalculated {
                revision: self.style_revision,
                nodes: output.computed_styles.len(),
            },
        );
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::LayoutCompleted {
                revision: self.layout_revision,
                nodes: output.geometry.len(),
            },
        );
        for font in &output.loaded_fonts {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::FontLoaded {
                    family: font.family.clone(),
                    sha256: font.sha256.clone(),
                },
            );
        }
        self.layout = Some(output);
        Ok(())
    }

    #[cfg(not(feature = "v8-host"))]
    fn refresh_layout(&mut self) -> Result<(), RuntimeError> {
        Err(RuntimeError::LayoutUnavailable(
            "layout is available only in the V8 page worker".into(),
        ))
    }

    fn validate_target(&self, target: SemanticRef) -> Result<(), RuntimeError> {
        if target.page_id != self.page_id {
            return Err(RuntimeError::WrongPage {
                expected: self.page_id,
                actual: target.page_id,
            });
        }
        if target.document_generation != self.document_generation {
            return Err(RuntimeError::WrongDocumentGeneration {
                expected: self.document_generation,
                actual: target.document_generation,
            });
        }
        let node = crate::semantic::unpack_node_ref(target.node);
        self.document
            .as_ref()
            .and_then(|document| document.dom.get(node).ok())
            .ok_or(RuntimeError::StaleSemanticRef)?;
        Ok(())
    }

    fn computed_style(&mut self, target: SemanticRef) -> Result<ComputedStyle, RuntimeError> {
        self.validate_target(target)?;
        #[cfg(feature = "v8-host")]
        {
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
            let properties = self
                .layout
                .as_ref()
                .and_then(|layout| layout.computed_styles.get(&target.node))
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::LayoutUnavailable("target has no computed style".into())
                })?;
            Ok(ComputedStyle {
                semantic_ref: target,
                style_revision: oryn_common::v4::StyleRevision(self.style_revision),
                properties,
            })
        }
        #[cfg(not(feature = "v8-host"))]
        Err(RuntimeError::LayoutUnavailable(
            "computed style requires the V8 page worker".into(),
        ))
    }

    fn box_model(&mut self, target: SemanticRef) -> Result<BoxModel, RuntimeError> {
        self.validate_target(target)?;
        #[cfg(feature = "v8-host")]
        {
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
            let geometry = self
                .layout
                .as_ref()
                .and_then(|layout| layout.geometry.get(&target.node))
                .ok_or_else(|| RuntimeError::LayoutUnavailable("target has no geometry".into()))?;
            geometry.box_model.clone().ok_or_else(|| {
                RuntimeError::UnsupportedGeometry(
                    geometry
                        .diagnostic
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", geometry.availability)),
                )
            })
        }
        #[cfg(not(feature = "v8-host"))]
        Err(RuntimeError::LayoutUnavailable(
            "box geometry requires the V8 page worker".into(),
        ))
    }

    fn set_viewport(&mut self, viewport: ViewportState) -> Result<ViewportState, RuntimeError> {
        let max = f64::from(self.config.max_viewport_css_px);
        if !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || !viewport.device_pixel_ratio.is_finite()
            || !(1.0..=max).contains(&viewport.width)
            || !(1.0..=max).contains(&viewport.height)
            || !(0.1..=8.0).contains(&viewport.device_pixel_ratio)
        {
            return Err(RuntimeError::InvalidViewport(format!(
                "{}x{} at DPR {} exceeds configured bounds",
                viewport.width, viewport.height, viewport.device_pixel_ratio
            )));
        }
        self.viewport = viewport;
        self.scroll_state.x = 0.0;
        self.scroll_state.y = 0.0;
        self.refresh_layout()?;
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::ViewportChanged {
                width: viewport.width,
                height: viewport.height,
                device_pixel_ratio: viewport.device_pixel_ratio,
            },
        );
        Ok(viewport)
    }

    fn set_media(&mut self, media: MediaState) -> Result<MediaState, RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        let _ = media;
        #[cfg(feature = "v8-host")]
        {
            self.color_scheme = media.color_scheme;
            self.reduced_motion = media.reduced_motion;
            self.refresh_layout()?;
            Ok(media)
        }
        #[cfg(not(feature = "v8-host"))]
        Err(RuntimeError::LayoutUnavailable(
            "media emulation requires the V8 page worker".into(),
        ))
    }

    fn scroll(&mut self, request: ScrollRequest) -> Result<ScrollState, RuntimeError> {
        if !request.x.is_finite() || !request.y.is_finite() {
            return Err(RuntimeError::InvalidViewport(
                "scroll coordinates must be finite".into(),
            ));
        }
        if request.relative {
            self.scroll_state.x += request.x;
            self.scroll_state.y += request.y;
        } else {
            self.scroll_state.x = request.x;
            self.scroll_state.y = request.y;
        }
        self.refresh_layout()?;
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::Scrolled {
                x: self.scroll_state.x,
                y: self.scroll_state.y,
            },
        );
        Ok(self.scroll_state)
    }

    fn scroll_into_view(&mut self, target: SemanticRef) -> Result<ScrollState, RuntimeError> {
        let box_model = self.box_model(target)?;
        if box_model.border_box.y < 0.0 || box_model.border_box.bottom() > self.viewport.height {
            self.scroll_state.y = (self.scroll_state.y + box_model.border_box.y).max(0.0);
            self.refresh_layout()?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::Scrolled {
                    x: self.scroll_state.x,
                    y: self.scroll_state.y,
                },
            );
        }
        Ok(self.scroll_state)
    }

    #[cfg(feature = "v8-host")]
    fn paint_snapshot(&mut self) -> Result<PaintSnapshot, RuntimeError> {
        self.refresh_layout()?;
        let document = self
            .document
            .clone()
            .ok_or_else(|| RuntimeError::PaintUnavailable("page has no document".into()))?;
        let layout = self
            .layout
            .clone()
            .ok_or_else(|| RuntimeError::PaintUnavailable("page has no layout snapshot".into()))?;
        let mut images = Vec::new();
        let mut image_indexes = BTreeMap::<String, usize>::new();
        let mut diagnostics = layout.diagnostics.clone();
        let mut image_bytes = 0usize;
        let mut nodes = Vec::new();

        for (id, node) in document.dom.iter() {
            let crate::dom::NodeKind::Element { name } = &node.kind else {
                continue;
            };
            let node_id = pack_node_id(id);
            let Some(geometry) = layout.geometry.get(&node_id).cloned() else {
                continue;
            };
            let Some(style) = layout.computed_styles.get(&node_id).cloned() else {
                continue;
            };
            let mut image_resource = None;
            if name.local.as_ref() == "img"
                && geometry.in_viewport
                && let Some(source) = node.attributes.get("src")
            {
                let resolved = Url::parse(source)
                    .or_else(|_| Url::parse(&self.url).and_then(|base| base.join(source)));
                match resolved {
                    Ok(url) => {
                        let key = url.as_str().to_string();
                        if let Some(index) = image_indexes.get(&key).copied() {
                            image_resource = Some(index);
                        } else {
                            match self.load_paint_image(&url) {
                                Ok(resource)
                                    if image_bytes.saturating_add(resource.bytes.len())
                                        <= self.config.max_response_bytes =>
                                {
                                    image_bytes += resource.bytes.len();
                                    let index = images.len();
                                    image_indexes.insert(key, index);
                                    images.push(resource);
                                    image_resource = Some(index);
                                }
                                Ok(_) => diagnostics.push(format!(
                                    "image {url} omitted: paint snapshot image budget exceeded"
                                )),
                                Err(error) => diagnostics
                                    .push(format!("image {url} unavailable for paint: {error}")),
                            }
                        }
                    }
                    Err(error) => {
                        diagnostics.push(format!("image source {source:?} is invalid: {error}"))
                    }
                }
            }
            nodes.push(PaintNodeSnapshot {
                node_id,
                tag: name.local.to_string(),
                geometry,
                style,
                text: layout
                    .text_fragments
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_default(),
                image_resource,
            });
        }
        Ok(PaintSnapshot {
            contract_version: oryn_common::v4::CONTRACT_VERSION,
            page: PageInfo {
                page_id: self.page_id,
                opener_page_id: self.opener_page_id,
                url: self.url.clone(),
                title: document_title(&document),
                document_generation: self.document_generation,
            },
            revision: Revision(self.revision),
            style_revision: oryn_common::v4::StyleRevision(self.style_revision),
            layout_revision: oryn_common::v4::LayoutRevision(self.layout_revision),
            viewport: self.viewport,
            scroll: self.scroll_state,
            nodes,
            images,
            diagnostics,
        })
    }

    #[cfg(feature = "v8-host")]
    fn load_paint_image(&mut self, url: &Url) -> Result<PaintImageResource, RuntimeError> {
        if url.scheme() == "data" {
            let value = url.as_str().strip_prefix("data:").unwrap_or_default();
            let (metadata, encoded) = value
                .split_once(',')
                .ok_or_else(|| RuntimeError::PaintUnavailable("malformed data URL".into()))?;
            if !metadata.ends_with(";base64") {
                return Err(RuntimeError::PaintUnavailable(
                    "only base64 image data URLs are supported".into(),
                ));
            }
            let media_type = metadata.trim_end_matches(";base64").to_ascii_lowercase();
            let bytes = BASE64
                .decode(encoded)
                .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
            return Ok(PaintImageResource {
                url: url.as_str().into(),
                media_type,
                bytes,
            });
        }
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RuntimeError::PaintUnavailable(format!(
                "image scheme {} is unsupported",
                url.scheme()
            )));
        }
        let mut current = url.clone();
        let mut response = None;
        for redirect in 0..=self.config.max_redirects {
            self.next_action_id = self.next_action_id.saturating_add(1);
            let next = self
                .broker
                .as_mut()
                .ok_or_else(|| {
                    RuntimeError::PaintUnavailable("network broker is unavailable".into())
                })?
                .send(NetworkRequest {
                    request_id: self.next_action_id,
                    method: "GET".into(),
                    url: current.as_str().into(),
                    headers: vec![("accept".into(), "image/png,image/jpeg,image/*;q=0.8".into())],
                    body: Vec::new(),
                    credentials: true,
                    resolved_addrs: Vec::new(),
                })
                .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
            if (300..400).contains(&next.status) {
                if redirect == self.config.max_redirects {
                    return Err(RuntimeError::PaintUnavailable(
                        "image redirect limit exceeded".into(),
                    ));
                }
                let location = next
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        RuntimeError::PaintUnavailable("image redirect omitted Location".into())
                    })?;
                current = current
                    .join(location)
                    .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
                continue;
            }
            response = Some(next);
            break;
        }
        let response = response.ok_or_else(|| {
            RuntimeError::PaintUnavailable("image redirect loop did not produce a response".into())
        })?;
        if !(200..300).contains(&response.status) {
            return Err(RuntimeError::PaintUnavailable(format!(
                "image request returned HTTP {}",
                response.status
            )));
        }
        let media_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| {
                value
                    .split(';')
                    .next()
                    .unwrap_or(value)
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_else(|| "application/octet-stream".into());
        Ok(PaintImageResource {
            url: current.as_str().into(),
            media_type,
            bytes: response.body,
        })
    }

    #[cfg(not(feature = "v8-host"))]
    fn paint_snapshot(&mut self) -> Result<PaintSnapshot, RuntimeError> {
        Err(RuntimeError::PaintUnavailable(
            "paint snapshots require the sandboxed V8 page worker".into(),
        ))
    }

    fn observation_v4(
        &mut self,
        options: ObservationOptions,
    ) -> Result<oryn_common::v4::Observation, RuntimeError> {
        if options.include_geometry || options.viewport_only {
            #[cfg(feature = "v8-host")]
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
        }
        let v3 = self.observation(options.profile);
        let mut observation = oryn_common::v4::Observation::from_v3(v3, self.viewport);
        observation.style_revision = oryn_common::v4::StyleRevision(self.style_revision);
        observation.layout_revision = oryn_common::v4::LayoutRevision(self.layout_revision);
        observation.scroll = self.scroll_state;
        #[cfg(feature = "v8-host")]
        if let Some(layout) = &self.layout {
            for node in &mut observation.nodes {
                let geometry = layout
                    .geometry
                    .get(&node.semantic.semantic_ref.node)
                    .cloned();
                if geometry.as_ref().is_some_and(|geometry| {
                    geometry.availability == oryn_common::v4::GeometryAvailability::Hidden
                }) {
                    node.semantic
                        .states
                        .remove(&oryn_common::v4::NodeState::Visible);
                    node.semantic
                        .states
                        .insert(oryn_common::v4::NodeState::Hidden);
                    node.semantic.actions.clear();
                }
                if options.include_geometry || options.viewport_only {
                    node.geometry = geometry;
                }
            }
            if options.viewport_only {
                observation.nodes.retain(|node| {
                    node.geometry
                        .as_ref()
                        .is_some_and(|geometry| geometry.in_viewport)
                });
            }
            if !options.include_hidden {
                observation.nodes.retain(|node| {
                    node.geometry.as_ref().is_none_or(|geometry| {
                        geometry.availability != oryn_common::v4::GeometryAvailability::Hidden
                    })
                });
            }
            observation.diagnostics.extend(layout.diagnostics.clone());
        }
        Ok(observation)
    }

    fn capabilities(&self) -> Vec<CapabilityDiagnostic> {
        let legacy_layout = CapabilityDiagnostic {
            capability: "rendered_layout_and_paint".into(),
            support: if cfg!(feature = "v8-host") {
                SupportLevel::Partial
            } else {
                SupportLevel::Unsupported
            },
            alternatives: vec![ExecutionDomain::Chromium, ExecutionDomain::Webkit],
            handoff_lossy: true,
            detail: Some(
                "legacy aggregate capability: native style and layout are separate from unsupported paint"
                    .into(),
            ),
        };
        #[cfg(feature = "v8-host")]
        {
            let mut capabilities = self
                .executable
                .as_ref()
                .map(|executable| executable.diagnostics.clone())
                .unwrap_or_default();
            capabilities.push(legacy_layout);
            capabilities.extend([
                supported_capability(
                    "computed_style",
                    "Stylo-parsed declarations with a worker-owned deterministic bounded cascade",
                ),
                supported_capability("layout.box", "worker-owned CSS box geometry"),
                supported_capability("layout.inline_text", "deterministic inline text fragments"),
                supported_capability(
                    "paint.snapshot",
                    "bounded read-only paint input for an optional renderer",
                ),
                CapabilityDiagnostic {
                    capability: "layout.float".into(),
                    support: SupportLevel::Partial,
                    alternatives: vec![ExecutionDomain::Chromium, ExecutionDomain::Webkit],
                    handoff_lossy: true,
                    detail: Some(
                        "simple left/right floats are supported; complex float exclusion and fragmentation remain unavailable"
                            .into(),
                    ),
                },
                unsupported_capability(
                    "layout.table",
                    "table extraction is available, but table geometry is intentionally unavailable",
                ),
                unsupported_capability(
                    "layout.hit_testing.paint_order",
                    "paint-order hit testing is deferred to G8",
                ),
                unsupported_capability(
                    "layout.transform.3d",
                    "three-dimensional transforms and perspective are outside the G4 geometry contract",
                ),
                unsupported_capability(
                    "layout.float.complex",
                    "complex float exclusion, shape-outside, and fragmentation are outside the bounded G4 float subset",
                ),
                unsupported_capability(
                    "paint",
                    "the native core exports a paint snapshot but intentionally owns no rasterizer; use the optional oryn-render package",
                ),
            ]);
            capabilities.extend([
                unsupported_capability(
                    "cross_origin_dom",
                    "cross-origin frame contents remain opaque",
                ),
                unsupported_capability(
                    "service_worker_and_cache_api",
                    "service workers and Cache API are outside G3",
                ),
                unsupported_capability(
                    "webrtc_and_media",
                    "WebRTC and media capture are outside G3",
                ),
                unsupported_capability(
                    "xhr.synchronous_main_thread",
                    "synchronous main-thread XHR is intentionally unsupported",
                ),
                CapabilityDiagnostic {
                    capability: "indexeddb.advanced".into(),
                    support: SupportLevel::Partial,
                    alternatives: vec![ExecutionDomain::Chromium, ExecutionDomain::Webkit],
                    handoff_lossy: true,
                    detail: Some(
                        "compound keys, multi-entry indexes, key-range intervals, and durability controls are outside the G3 subset"
                            .into(),
                    ),
                },
            ]);
            capabilities
        }
        #[cfg(not(feature = "v8-host"))]
        {
            vec![
                CapabilityDiagnostic {
                    capability: "javascript.execution".into(),
                    support: SupportLevel::Unsupported,
                    alternatives: vec![
                        ExecutionDomain::Chromium,
                        ExecutionDomain::Webkit,
                        ExecutionDomain::UserBrowser,
                    ],
                    handoff_lossy: true,
                    detail: Some(
                        "this binary was built without the v8-host feature and is a probe, not the production native runtime"
                            .into(),
                    ),
                },
                legacy_layout,
            ]
        }
    }

    fn resolve_selector(
        &self,
        selector: &str,
        action: SemanticAction,
    ) -> Result<SemanticRef, RuntimeError> {
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        selector_nodes(document, selector)
            .filter_map(|id| {
                project_element(
                    &document.dom,
                    id,
                    self.page_id,
                    self.document_generation,
                    id.slot + 1,
                )
            })
            .find(|node| node.actions.contains(&action))
            .map(|node| node.semantic_ref)
            .ok_or(RuntimeError::StaleSemanticRef)
    }

    fn extract(
        &self,
        kind: ContentKind,
        selector: Option<&str>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let roots = selector
            .map(|selector| selector_nodes(document, selector).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![document.document]);
        match kind {
            ContentKind::Html => Ok(serde_json::Value::String(
                roots
                    .iter()
                    .map(|id| serialize_dom_node(&document.dom, *id))
                    .collect::<Vec<_>>()
                    .join(""),
            )),
            ContentKind::Text => Ok(serde_json::Value::String(
                roots
                    .iter()
                    .filter_map(|id| visible_text_content(&document.dom, *id).ok())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            )),
            ContentKind::Links => Ok(serde_json::Value::Array(
                document
                    .dom
                    .iter()
                    .filter(|(_, node)| {
                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "a")
                    })
                    .map(|(id, node)| {
                        serde_json::json!({
                            "text": document.dom.text_content(id).unwrap_or_default(),
                            "href": node.attributes.get("href").cloned().unwrap_or_default()
                        })
                    })
                    .collect(),
            )),
            ContentKind::Images => Ok(serde_json::Value::Array(
                document
                    .dom
                    .iter()
                    .filter(|(_, node)| {
                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "img")
                    })
                    .map(|(_, node)| {
                        serde_json::json!({
                            "src": node.attributes.get("src").cloned().unwrap_or_default(),
                            "alt": node.attributes.get("alt").cloned().unwrap_or_default()
                        })
                    })
                    .collect(),
            )),
            ContentKind::Tables => Ok(serde_json::Value::Array(
                document
                    .dom
                    .iter()
                    .filter(|(_, node)| {
                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "table")
                    })
                    .map(|(table, _)| {
                        serde_json::Value::Array(
                            descendants_of(&document.dom, table)
                                .into_iter()
                                .filter(|id| {
                                    document.dom.get(*id).is_ok_and(|node| {
                                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "tr")
                                    })
                                })
                                .map(|row| {
                                    serde_json::Value::Array(
                                        document
                                            .dom
                                            .get(row)
                                            .map(|node| {
                                                node.children
                                                    .iter()
                                                    .filter_map(|cell| {
                                                        document.dom.get(*cell).ok().and_then(|node| {
                                                            matches!(&node.kind, crate::dom::NodeKind::Element { name } if matches!(name.local.as_ref(), "td" | "th"))
                                                                .then(|| serde_json::Value::String(document.dom.text_content(*cell).unwrap_or_default()))
                                                        })
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                    )
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )),
        }
    }

    fn observation(&self, profile: ProjectionProfile) -> Observation {
        let Some(document) = &self.document else {
            let mut observation = empty_observation(
                self.page_id,
                self.opener_page_id,
                "about:blank".into(),
                String::new(),
                self.document_generation,
            );
            observation.profile = profile;
            return observation;
        };
        Observation {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id: self.page_id,
                opener_page_id: self.opener_page_id,
                url: self.url.clone(),
                title: document_title(document),
                document_generation: self.document_generation,
            },
            revision: Revision(self.revision),
            profile,
            source: ObservationSource::NativeDom,
            nodes: match profile {
                ProjectionProfile::Full => {
                    project_document_full(&document.dom, self.page_id, self.document_generation)
                }
                _ => project_document(&document.dom, self.page_id, self.document_generation),
            },
            capabilities: self.capabilities(),
            diagnostics: document.errors.clone(),
            execution_domain: ExecutionDomain::Native,
        }
    }

    fn record_snapshot(&mut self) {
        let nodes = self
            .document
            .as_ref()
            .map(|document| project_document(&document.dom, self.page_id, self.document_generation))
            .unwrap_or_default();
        self.snapshots.insert(self.revision, nodes);
    }

    fn delta_from(&self, from: Revision) -> Result<ObservationDelta, RuntimeError> {
        let old = if from.0 == 0 {
            &[][..]
        } else {
            self.snapshots
                .get(&from.0)
                .map(Vec::as_slice)
                .ok_or(RuntimeError::UnknownRevision(from))?
        };
        let current = self
            .snapshots
            .get(&self.revision)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let old_by_ref: BTreeMap<_, _> = old.iter().map(|node| (node.semantic_ref, node)).collect();
        let current_by_ref: BTreeMap<_, _> = current
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect();
        Ok(ObservationDelta {
            contract_version: CONTRACT_VERSION,
            page_id: self.page_id,
            from_revision: from,
            to_revision: Revision(self.revision),
            upserted: current
                .iter()
                .filter(|node| old_by_ref.get(&node.semantic_ref).copied() != Some(*node))
                .cloned()
                .collect(),
            removed: old_by_ref
                .keys()
                .filter(|semantic_ref| !current_by_ref.contains_key(semantic_ref))
                .copied()
                .collect(),
        })
    }

    fn execute(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        self.execute_with_interactability(action, true)
    }

    fn execute_forced(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        self.execute_with_interactability(action, false)
    }

    fn execute_with_interactability(
        &mut self,
        action: NativeAction,
        check_interactability: bool,
    ) -> Result<ActionResult, RuntimeError> {
        self.validate_target(action.target)?;
        if check_interactability
            && matches!(action.action, SemanticAction::Click | SemanticAction::Hover)
        {
            self.ensure_interactable(action.target)?;
        }
        #[cfg(feature = "v8-host")]
        if self.executable.is_some() {
            return self.execute_in_realm(action);
        }
        let document = self
            .document
            .as_mut()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let node_id = crate::semantic::unpack_node_ref(action.target.node);
        let node = document
            .dom
            .get_mut(node_id)
            .map_err(|_| RuntimeError::StaleSemanticRef)?;
        let crate::dom::NodeKind::Element { name } = &node.kind else {
            return Err(RuntimeError::StaleSemanticRef);
        };
        if node.attributes.contains_key("disabled") {
            return Err(RuntimeError::Disabled);
        }
        let local_name = name.local.as_ref();
        let (summary, mutated) = match action.action {
            SemanticAction::Type if matches!(local_name, "input" | "textarea") => {
                let value = action.value.unwrap_or_default();
                node.attributes.insert("value".into(), value);
                ("updated form control value", true)
            }
            SemanticAction::Clear if matches!(local_name, "input" | "textarea") => {
                node.attributes.insert("value".into(), String::new());
                ("cleared form control value", true)
            }
            SemanticAction::Click if matches!(local_name, "button" | "a" | "input" | "select") => {
                let toggled = local_name == "input"
                    && matches!(
                        node.attributes.get("type").map(String::as_str),
                        Some("checkbox" | "radio")
                    );
                if toggled && node.attributes.remove("checked").is_none() {
                    node.attributes.insert("checked".into(), String::new());
                }
                ("dispatched click activation", toggled)
            }
            SemanticAction::Check if local_name == "input" => {
                node.attributes.insert("checked".into(), String::new());
                ("checked form control", true)
            }
            SemanticAction::Uncheck if local_name == "input" => {
                node.attributes.remove("checked");
                ("unchecked form control", true)
            }
            SemanticAction::Select if local_name == "select" => {
                node.attributes
                    .insert("value".into(), action.value.unwrap_or_default());
                ("selected form control value", true)
            }
            SemanticAction::Focus => {
                node.attributes
                    .insert("data-oryn-focused".into(), String::new());
                ("focused element", true)
            }
            SemanticAction::Hover => ("dispatched pointer hover", false),
            SemanticAction::Submit if matches!(local_name, "form" | "button" | "input") => {
                ("dispatched form submission", false)
            }
            _ => {
                return Err(RuntimeError::UnsupportedAction {
                    action: action.action,
                });
            }
        };

        let revision_before = Revision(self.revision);
        self.revision += 1;
        self.next_action_id += 1;
        let upserted = if mutated {
            project_element(
                &document.dom,
                node_id,
                self.page_id,
                self.document_generation,
                node_id.slot + 1,
            )
            .into_iter()
            .collect()
        } else {
            Vec::new()
        };
        let effect = if mutated {
            Effect::DomMutation {
                summary: summary.into(),
            }
        } else {
            Effect::Event {
                event_type: match action.action {
                    SemanticAction::Hover => "mouseover",
                    SemanticAction::Submit => "submit",
                    _ => "click",
                }
                .into(),
                target: action.target,
            }
        };
        let result = ActionResult {
            contract_version: CONTRACT_VERSION,
            action_id: ActionId(self.next_action_id),
            page_id: self.page_id,
            revision_before,
            revision_after: Revision(self.revision),
            delta: Some(ObservationDelta {
                contract_version: CONTRACT_VERSION,
                page_id: self.page_id,
                from_revision: revision_before,
                to_revision: Revision(self.revision),
                upserted,
                removed: Vec::new(),
            }),
            effects: vec![effect],
            diagnostics: Vec::new(),
            execution_domain: ExecutionDomain::Native,
        };
        self.record_snapshot();
        let trace_kind = if mutated {
            TraceEventKind::Mutation {
                summary: summary.into(),
            }
        } else {
            TraceEventKind::EventDispatched {
                event_type: match action.action {
                    SemanticAction::Hover => "mouseover",
                    SemanticAction::Submit => "submit",
                    _ => "click",
                }
                .into(),
            }
        };
        self.trace
            .push(Revision(self.revision), Some(result.action_id), trace_kind);
        Ok(result)
    }

    fn ensure_interactable(&mut self, target: SemanticRef) -> Result<(), RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        let _ = target;
        #[cfg(feature = "v8-host")]
        {
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
            let first = self
                .layout
                .as_ref()
                .and_then(|layout| layout.geometry.get(&target.node))
                .cloned()
                .ok_or_else(|| RuntimeError::LayoutUnavailable("target has no geometry".into()))?;
            if first.availability == oryn_common::v4::GeometryAvailability::Unsupported {
                let reason = first
                    .diagnostic
                    .unwrap_or_else(|| "target geometry is unsupported".into());
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::InteractabilityRejected {
                        target: target.node,
                        reason: reason.clone(),
                    },
                );
                return Err(RuntimeError::UnsupportedGeometry(reason));
            }
            if !first.in_viewport && first.box_model.is_some() {
                self.scroll_into_view(target)?;
            }
            let current = self
                .layout
                .as_ref()
                .and_then(|layout| layout.geometry.get(&target.node))
                .ok_or_else(|| RuntimeError::LayoutUnavailable("target has no geometry".into()))?;
            if !current.interactable {
                let reason = match current.availability {
                    oryn_common::v4::GeometryAvailability::Hidden => "target is hidden",
                    oryn_common::v4::GeometryAvailability::Unsupported => {
                        "target geometry is unsupported"
                    }
                    oryn_common::v4::GeometryAvailability::Available => {
                        "target has no visible pointer-interactable box"
                    }
                }
                .to_string();
                let reason = format!("{reason} (semantic node {})", target.node);
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::InteractabilityRejected {
                        target: target.node,
                        reason: reason.clone(),
                    },
                );
                return Err(RuntimeError::NotInteractable(reason));
            }
            Ok(())
        }
        #[cfg(not(feature = "v8-host"))]
        Ok(())
    }

    #[cfg(feature = "v8-host")]
    fn execute_in_realm(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        let before = self.observation(ProjectionProfile::Compact);
        let node_id = crate::semantic::unpack_node_ref(action.target.node);
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let node = document
            .dom
            .get(node_id)
            .map_err(|_| RuntimeError::StaleSemanticRef)?;
        if node.attributes.contains_key("disabled") {
            return Err(RuntimeError::Disabled);
        }
        let executable = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        executable
            .apply_action(node_id, action.action, action.value.as_deref())
            .map_err(map_host_error)?;
        let console_entries = executable.take_console_entries().unwrap_or_default();
        let context_entries = executable.take_context_trace_entries().unwrap_or_default();
        let pending_navigation = executable.location().ok().filter(|location| {
            location != &self.url && Url::parse(location).ok() != Url::parse(&self.url).ok()
        });
        self.document = Some(executable.document.clone());
        let diagnostics = executable
            .diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.detail.clone())
            .collect();

        let revision_before = Revision(self.revision);
        self.revision += 1;
        self.next_action_id += 1;
        self.refresh_layout()?;
        let after = self.observation(ProjectionProfile::Compact);
        let before_by_ref = before
            .nodes
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        let after_by_ref = after
            .nodes
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        let delta = ObservationDelta {
            contract_version: CONTRACT_VERSION,
            page_id: self.page_id,
            from_revision: revision_before,
            to_revision: Revision(self.revision),
            upserted: after
                .nodes
                .iter()
                .filter(|node| before_by_ref.get(&node.semantic_ref).copied() != Some(*node))
                .cloned()
                .collect(),
            removed: before_by_ref
                .keys()
                .filter(|reference| !after_by_ref.contains_key(reference))
                .copied()
                .collect(),
        };
        let event_type = match action.action {
            SemanticAction::Hover => "mouseover",
            SemanticAction::Submit => "submit",
            SemanticAction::Focus => "focus",
            SemanticAction::Type => "input",
            SemanticAction::Clear => "input",
            SemanticAction::Check | SemanticAction::Uncheck | SemanticAction::Select => "change",
            _ => "click",
        };
        let mut effects = Vec::new();
        if !delta.upserted.is_empty() || !delta.removed.is_empty() {
            effects.push(Effect::DomMutation {
                summary: "script realm changed the native DOM".into(),
            });
        }
        effects.push(Effect::Event {
            event_type: event_type.into(),
            target: action.target,
        });
        let mut result = ActionResult {
            contract_version: CONTRACT_VERSION,
            action_id: ActionId(self.next_action_id),
            page_id: self.page_id,
            revision_before,
            revision_after: Revision(self.revision),
            delta: Some(delta),
            effects,
            diagnostics,
            execution_domain: ExecutionDomain::Native,
        };
        self.record_snapshot();
        self.trace.push(
            Revision(self.revision),
            Some(result.action_id),
            TraceEventKind::EventDispatched {
                event_type: event_type.into(),
            },
        );
        for entry in console_entries {
            self.trace.push(
                Revision(self.revision),
                Some(result.action_id),
                TraceEventKind::Console {
                    level: entry.level,
                    message: entry.message,
                },
            );
        }
        for entry in context_entries {
            if let Some(effect) = context_trace_effect(&entry) {
                result.effects.push(effect);
            }
            if let Some(kind) = context_trace_kind(entry) {
                self.trace
                    .push(Revision(self.revision), Some(result.action_id), kind);
            }
        }
        if result
            .delta
            .as_ref()
            .is_some_and(|delta| !delta.upserted.is_empty() || !delta.removed.is_empty())
        {
            self.trace.push(
                Revision(self.revision),
                Some(result.action_id),
                TraceEventKind::Mutation {
                    summary: "script realm changed the native DOM".into(),
                },
            );
        }
        if let Some(location) = pending_navigation {
            if same_document_url(&self.url, &location) {
                self.url = location.clone();
                result.effects.push(Effect::Navigation { url: location });
            } else {
                let navigation = self.navigate(location.clone(), HistoryUpdate::Push)?;
                result.revision_after = navigation.revision;
                result.delta = None;
                result.effects.push(Effect::Navigation { url: location });
            }
        }
        Ok(result)
    }
}

#[cfg(feature = "v8-host")]
fn context_trace_effect(entry: &crate::executable::ContextTraceEntry) -> Option<Effect> {
    let detail = &entry.detail;
    let page_id = |key: &str| {
        detail
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .map(PageId)
    };
    let string = |key: &str| {
        detail
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match entry.kind.as_str() {
        "page_created" => Some(Effect::PageOpened {
            page_id: page_id("page_id")?,
            opener_page_id: page_id("opener_page_id"),
            url: string("url"),
        }),
        "page_activated" => Some(Effect::PageActivated {
            page_id: page_id("page_id")?,
        }),
        "page_closed" => Some(Effect::PageClosed {
            page_id: page_id("page_id")?,
        }),
        "opener_message" => Some(Effect::Message {
            source_page_id: page_id("source_page_id")?,
            target_page_id: page_id("target_page_id")?,
            origin: string("origin"),
        }),
        "storage_changed" => Some(Effect::Storage {
            area: string("area"),
            origin: string("origin"),
            operation: if detail.get("key").is_some_and(serde_json::Value::is_null) {
                "clear".into()
            } else {
                "set_or_remove".into()
            },
        }),
        "request_started" => Some(Effect::Request {
            method: string("method"),
            url: string("url"),
        }),
        "response_received" => Some(Effect::Response {
            url: string("url"),
            status: detail
                .get("status")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default() as u16,
        }),
        "web_socket_frame" => Some(Effect::WebSocket {
            operation: format!("{}_{}", string("direction"), string("frame_type")),
            url: string("url"),
        }),
        "capability_failure" => Some(Effect::Capability {
            diagnostic: unsupported_capability(&string("capability"), &string("detail")),
        }),
        _ => None,
    }
}

#[cfg(feature = "v8-host")]
fn context_trace_kind(entry: crate::executable::ContextTraceEntry) -> Option<TraceEventKind> {
    let detail = entry.detail;
    let number = |key: &str| detail.get(key).and_then(serde_json::Value::as_u64);
    let string = |key: &str| {
        detail
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match entry.kind.as_str() {
        "page_created" => Some(TraceEventKind::PageCreated {
            page_id: number("page_id")?,
            opener_page_id: number("opener_page_id"),
        }),
        "page_activated" => Some(TraceEventKind::PageActivated {
            page_id: number("page_id")?,
        }),
        "page_closed" => Some(TraceEventKind::PageClosed {
            page_id: number("page_id")?,
        }),
        "opener_message" => Some(TraceEventKind::OpenerMessage {
            source_page_id: number("source_page_id")?,
            target_page_id: number("target_page_id")?,
            origin: string("origin"),
        }),
        "storage_changed" => Some(TraceEventKind::StorageChanged {
            area: string("area"),
            origin: string("origin"),
            key: detail
                .get("key")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        }),
        "request_started" => Some(TraceEventKind::RequestStarted {
            method: string("method"),
            url: string("url"),
        }),
        "response_received" => Some(TraceEventKind::ResponseReceived {
            url: string("url"),
            status: number("status")? as u16,
            bytes: number("bytes")? as usize,
        }),
        "xhr_state_changed" => Some(TraceEventKind::XhrStateChanged {
            url: string("url"),
            ready_state: number("ready_state")? as u8,
        }),
        "stream_chunk" => Some(TraceEventKind::StreamChunk {
            direction: string("direction"),
            bytes: number("bytes")? as usize,
        }),
        "web_socket_frame" => Some(TraceEventKind::WebSocketFrame {
            socket_id: number("socket_id")?,
            direction: string("direction"),
            frame_type: string("frame_type"),
            bytes: number("bytes")? as usize,
        }),
        "capability_failure" => Some(TraceEventKind::PolicyDenied {
            capability: string("capability"),
        }),
        _ => None,
    }
}

#[cfg(feature = "v8-host")]
fn unsupported_capability(capability: &str, detail: &str) -> CapabilityDiagnostic {
    CapabilityDiagnostic {
        capability: capability.into(),
        support: SupportLevel::Unsupported,
        alternatives: vec![
            ExecutionDomain::Chromium,
            ExecutionDomain::Webkit,
            ExecutionDomain::UserBrowser,
        ],
        handoff_lossy: true,
        detail: Some(detail.into()),
    }
}

#[cfg(feature = "v8-host")]
fn supported_capability(capability: &str, detail: &str) -> CapabilityDiagnostic {
    CapabilityDiagnostic {
        capability: capability.into(),
        support: SupportLevel::Supported,
        alternatives: Vec::new(),
        handoff_lossy: false,
        detail: Some(detail.into()),
    }
}

#[cfg(feature = "v8-host")]
fn layout_input(
    document: &ParsedDocument,
    viewport: ViewportState,
    scroll: ScrollState,
    color_scheme: PreferredColorScheme,
    reduced_motion: ReducedMotion,
    web_fonts: &[oryn_layout::WebFont],
) -> oryn_layout::LayoutInput {
    use crate::dom::NodeKind;

    let elements = document
        .dom
        .iter()
        .filter_map(|(id, node)| matches!(node.kind, NodeKind::Element { .. }).then_some(id))
        .collect::<Vec<_>>();
    let nodes = elements
        .iter()
        .filter_map(|id| {
            let node = document.dom.get(*id).ok()?;
            let NodeKind::Element { name } = &node.kind else {
                return None;
            };
            let packed = pack_node_id(*id);
            let parent = document
                .dom
                .ancestors(*id)
                .ok()?
                .into_iter()
                .find(|ancestor| {
                    document
                        .dom
                        .get(*ancestor)
                        .is_ok_and(|node| matches!(node.kind, NodeKind::Element { .. }))
                })
                .map(pack_node_id);
            let children = node
                .children
                .iter()
                .filter(|child| {
                    document
                        .dom
                        .get(**child)
                        .is_ok_and(|node| matches!(node.kind, NodeKind::Element { .. }))
                })
                .copied()
                .map(pack_node_id)
                .collect();
            let text = node
                .children
                .iter()
                .filter_map(|child| document.dom.get(*child).ok())
                .filter_map(|child| match &child.kind {
                    NodeKind::Text { data } => Some(data.as_str()),
                    _ => None,
                })
                .collect::<String>();
            Some(oryn_layout::LayoutNode {
                id: packed,
                parent,
                children,
                tag: name.local.to_string(),
                attributes: node.attributes.clone(),
                text,
            })
        })
        .collect();
    let stylesheets = document
        .dom
        .iter()
        .filter_map(|(id, node)| {
            matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "style")
                .then(|| document.dom.text_content(id).ok())
                .flatten()
        })
        .collect();
    oryn_layout::LayoutInput {
        nodes,
        stylesheets,
        viewport,
        scroll,
        color_scheme: match color_scheme {
            PreferredColorScheme::Light => oryn_layout::ColorScheme::Light,
            PreferredColorScheme::Dark => oryn_layout::ColorScheme::Dark,
        },
        reduced_motion: reduced_motion == ReducedMotion::Reduce,
        web_fonts: web_fonts.to_vec(),
    }
}

#[cfg(feature = "v8-host")]
fn pack_node_id(id: crate::dom::NodeId) -> u64 {
    (u64::from(id.generation) << 32) | u64::from(id.slot)
}

fn visible_text_content(
    dom: &crate::dom::DomArena,
    id: crate::dom::NodeId,
) -> Result<String, crate::dom::DomError> {
    let node = dom.get(id)?;
    match &node.kind {
        crate::dom::NodeKind::Text { data } => Ok(data.clone()),
        crate::dom::NodeKind::Element { name }
            if matches!(
                name.local.as_ref(),
                "head" | "script" | "style" | "template" | "noscript"
            ) || is_hidden(dom, id) =>
        {
            Ok(String::new())
        }
        _ => {
            let mut result = String::new();
            for child in &node.children {
                result.push_str(&visible_text_content(dom, *child)?);
            }
            Ok(result)
        }
    }
}

fn selector_nodes<'a>(
    document: &'a ParsedDocument,
    selector: &'a str,
) -> impl Iterator<Item = crate::dom::NodeId> + 'a {
    document
        .dom
        .iter()
        .filter_map(move |(id, node)| matches_css_subject(node, selector).then_some(id))
}

fn matches_css_subject(node: &crate::dom::Node, selector: &str) -> bool {
    let crate::dom::NodeKind::Element { name } = &node.kind else {
        return false;
    };
    let subject = css_subject(selector);
    let (subject, excluded_class) = subject
        .split_once(":not(.")
        .map(|(base, excluded)| (base, Some(excluded.trim_end_matches(')'))))
        .unwrap_or((subject, None));
    if excluded_class.is_some_and(|excluded| has_class(node, excluded)) {
        return false;
    }
    let attribute_start = subject.find('[');
    let simple = &subject[..attribute_start.unwrap_or(subject.len())];
    let mut remainder = attribute_start.map(|index| &subject[index..]).unwrap_or("");
    while let Some(open) = remainder.find('[') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find(']') else {
            return false;
        };
        let attribute = &after_open[..close];
        let (key, expected, contains) = if let Some((key, value)) = attribute.split_once("*=") {
            (key, Some(value.trim_matches(['\"', '\''])), true)
        } else if let Some((key, value)) = attribute.split_once('=') {
            (key, Some(value.trim_matches(['\"', '\''])), false)
        } else {
            (attribute, None, false)
        };
        let Some(actual) = node.attributes.get(key) else {
            return false;
        };
        if expected.is_some_and(|expected| {
            if contains {
                !actual.contains(expected)
            } else {
                actual != expected
            }
        }) {
            return false;
        }
        remainder = &after_open[close + 1..];
    }
    let mut tag_end = simple.len();
    for marker in ['#', '.'] {
        if let Some(index) = simple.find(marker) {
            tag_end = tag_end.min(index);
        }
    }
    let tag = &simple[..tag_end];
    if !tag.is_empty() && !name.local.as_ref().eq_ignore_ascii_case(tag) {
        return false;
    }
    if let Some(id_start) = simple.find('#') {
        let id = simple[id_start + 1..].split('.').next().unwrap_or_default();
        if node.attributes.get("id").is_none_or(|value| value != id) {
            return false;
        }
    }
    for class in simple.split('.').skip(1) {
        if !has_class(node, class) {
            return false;
        }
    }
    true
}

fn css_subject(selector: &str) -> &str {
    let mut bracket_depth = 0_u32;
    let mut quote = None;
    let mut subject_start = 0;
    for (index, character) in selector.char_indices() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' if bracket_depth > 0 => quote = Some(character),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            character if character.is_whitespace() && bracket_depth == 0 => {
                subject_start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    selector[subject_start..].trim()
}

fn has_class(node: &crate::dom::Node, class: &str) -> bool {
    node.attributes
        .get("class")
        .is_some_and(|classes| classes.split_whitespace().any(|item| item == class))
}

fn descendants_of(dom: &crate::dom::DomArena, root: crate::dom::NodeId) -> Vec<crate::dom::NodeId> {
    let mut result = Vec::new();
    let mut pending = dom
        .get(root)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    while let Some(id) = pending.pop() {
        result.push(id);
        if let Ok(node) = dom.get(id) {
            pending.extend(node.children.iter().rev().copied());
        }
    }
    result
}

fn serialize_dom_node(dom: &crate::dom::DomArena, id: crate::dom::NodeId) -> String {
    let Ok(node) = dom.get(id) else {
        return String::new();
    };
    match &node.kind {
        crate::dom::NodeKind::Document | crate::dom::NodeKind::DocumentFragment => node
            .children
            .iter()
            .map(|child| serialize_dom_node(dom, *child))
            .collect(),
        crate::dom::NodeKind::DocumentType { name } => format!("<!DOCTYPE {name}>"),
        crate::dom::NodeKind::Text { data } => escape_html(data),
        crate::dom::NodeKind::Comment { data } => format!("<!--{data}-->"),
        crate::dom::NodeKind::Element { name } => {
            let attributes = node
                .attributes
                .iter()
                .map(|(name, value)| format!(" {name}=\"{}\"", escape_html(value)))
                .collect::<String>();
            let children = node
                .children
                .iter()
                .map(|child| serialize_dom_node(dom, *child))
                .collect::<String>();
            format!("<{0}{attributes}>{children}</{0}>", name.local)
        }
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn same_document_url(left: &str, right: &str) -> bool {
    let (Ok(mut left), Ok(mut right)) = (Url::parse(left), Url::parse(right)) else {
        return false;
    };
    left.set_fragment(None);
    right.set_fragment(None);
    left == right
}

#[cfg(feature = "v8-host")]
fn module_specifiers(source: &str) -> Vec<String> {
    static MODULE_SPECIFIER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = MODULE_SPECIFIER.get_or_init(|| {
        regex::Regex::new(
            r#"(?m)(?:\bimport|\bexport)[^;\n]*?\bfrom\s*["']([^"']+)["']|\bimport\s*(?:\(\s*)?["']([^"']+)["']"#,
        )
        .expect("valid module-specifier pattern")
    });
    let source = strip_javascript_comments(source);
    pattern
        .captures_iter(&source)
        .filter_map(|captures| captures.get(1).or_else(|| captures.get(2)))
        .map(|value| value.as_str().to_string())
        .filter(|specifier| {
            specifier.starts_with('.')
                || specifier.starts_with('/')
                || specifier.starts_with('#')
                || Url::parse(specifier).is_ok()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(feature = "v8-host")]
fn strip_javascript_comments(source: &str) -> String {
    let chars = source.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    let mut quote = None;
    let mut block_comment = false;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        if block_comment {
            if current == '*' && next == Some('/') {
                block_comment = false;
                output.push(' ');
                output.push(' ');
                index += 2;
            } else {
                output.push(if current == '\n' { '\n' } else { ' ' });
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            output.push(current);
            if current == '\\'
                && let Some(next) = next
            {
                output.push(next);
                index += 2;
                continue;
            }
            if current == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(current, '\'' | '"' | '`') {
            quote = Some(current);
            output.push(current);
            index += 1;
        } else if current == '/' && next == Some('*') {
            block_comment = true;
            output.push(' ');
            output.push(' ');
            index += 2;
        } else if current == '/' && next == Some('/') {
            output.push(' ');
            output.push(' ');
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                output.push(' ');
                index += 1;
            }
        } else {
            output.push(current);
            index += 1;
        }
    }
    output
}

#[cfg(feature = "v8-host")]
fn pinned_resource(url: &Url) -> Option<(&'static str, &'static str)> {
    match url.as_str() {
        "https://unpkg.com/react@18/umd/react.production.min.js" => Some((
            "https://unpkg.com/react@18.3.1/umd/react.production.min.js",
            "d949f1c3687aedadcedac85261865f29b17cd273997e7f6b2bfc53b2f9d4c4dd",
        )),
        "https://unpkg.com/react-dom@18/umd/react-dom.production.min.js" => Some((
            "https://unpkg.com/react-dom@18.3.1/umd/react-dom.production.min.js",
            "35f4f974f4b2bcd44da73963347f8952e341f83909e4498227d4e26b98f66f0d",
        )),
        "https://unpkg.com/@babel/standalone/babel.min.js" => Some((
            "https://unpkg.com/@babel/standalone@7.28.3/babel.min.js",
            "5c69309d4426c6d981f49291527eec59331a69c2c677eaf96fd10f81f015443f",
        )),
        _ => None,
    }
}

#[cfg(feature = "v8-host")]
fn origin_key(url: &Url) -> (String, Option<String>, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().map(str::to_ascii_lowercase),
        url.port_or_known_default(),
    )
}

fn document_title(document: &ParsedDocument) -> String {
    document
        .dom
        .iter()
        .find_map(|(id, node)| match &node.kind {
            crate::dom::NodeKind::Element { name } if name.local.as_ref() == "title" => {
                document.dom.text_content(id).ok()
            }
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    use oryn_common::v3::{ExecutionDomain, ObservationSource};

    use super::*;
    use crate::security::PolicyRule;

    #[tokio::test(flavor = "current_thread")]
    async fn page_handle_queues_typed_operations() {
        let browser = Browser::new();
        let context = browser.new_context();
        let page = context.new_page();
        let observation = page.observe().await.expect("observe blank page");

        assert_eq!(observation.execution_domain, ExecutionDomain::Native);
        assert_eq!(observation.source, ObservationSource::NativeDom);
        page.close().await.expect("close page");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loaded_html_produces_native_semantics() {
        let page = Browser::new().new_context().new_page();
        let revision = page
            .load_html(
                "https://example.test/form",
                "<title>Form</title><label>Email <input type=email required></label><button>Send</button>",
            )
            .await
            .expect("load HTML");
        let observation = page.observe().await.expect("observe document");

        assert_eq!(revision, Revision(1));
        assert_eq!(observation.page.title, "Form");
        assert_eq!(observation.page.document_generation, 1);
        assert!(observation.nodes.iter().any(|node| node.name == "Send"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_navigation_checks_redirects_and_supports_history() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let response = match path {
                    "/redirect" => concat!(
                        "HTTP/1.1 302 Found\r\n",
                        "Location: /one\r\n",
                        "Content-Length: 0\r\n",
                        "Connection: close\r\n\r\n"
                    )
                    .to_string(),
                    "/one" => fixture_response("<title>One</title><button>First</button>"),
                    "/two" => fixture_response("<title>Two</title><button>Second</button>"),
                    _ => fixture_response("<title>Unknown</title>"),
                };
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        let first = page
            .goto(format!("http://{address}/redirect"))
            .await
            .expect("follow checked redirect");
        assert_eq!(first.status, 200);
        assert!(first.final_url.ends_with("/one"));
        assert_eq!(first.lifecycle.last(), Some(&LifecycleState::NetworkIdle));
        page.goto(format!("http://{address}/two"))
            .await
            .expect("second navigation");
        assert_eq!(page.back().await.expect("back").final_url, first.final_url);
        assert!(
            page.forward()
                .await
                .expect("forward")
                .final_url
                .ends_with("/two")
        );
        assert!(
            page.refresh()
                .await
                .expect("refresh")
                .final_url
                .ends_with("/two")
        );
        server.join().expect("fixture server");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn about_blank_is_internal_and_history_safe() {
        let page = Browser::new().new_context().new_page();
        page.load_html("about:blank", "<!doctype html>")
            .await
            .expect("load initial blank document");
        page.load_html("https://example.test/next", "<title>Next</title>")
            .await
            .expect("load next document");

        let back = page.back().await.expect("return to about:blank");
        assert_eq!(back.final_url, "about:blank");
        assert_eq!(back.status, 200);
        assert_eq!(back.lifecycle.last(), Some(&LifecycleState::NetworkIdle));
    }

    #[cfg(unix)]
    #[test]
    fn internal_fixture_urls_may_commit_but_cannot_escape_as_navigation() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let fixture = "fixture:///tmp/oryn-g3-smoke.html";

        context
            .validate_page_commit_url(page.page_id(), fixture)
            .expect("synthetic fixture document may commit its own URL");
        assert!(context.validate_page_url(page.page_id(), fixture).is_err());
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn fragment_activation_preserves_the_live_document() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/app",
            r##"<a id=route href="#users" onclick="document.querySelector('#view').textContent='Users'">Users</a><output id=view>Dashboard</output>"##,
        )
        .await
        .expect("load app");
        let before = page.observe().await.expect("observe app");
        let target = before
            .nodes
            .iter()
            .find(|node| node.name == "Users")
            .expect("route link")
            .semantic_ref;

        page.execute(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .expect("activate fragment route");
        let after = page.observe().await.expect("observe routed app");

        assert_eq!(
            after.page.document_generation,
            before.page.document_generation
        );
        assert_eq!(after.page.url, "https://example.test/app#users");
        assert!(
            page.extract(ContentKind::Text, None)
                .await
                .expect("extract routed text")
                .as_str()
                .is_some_and(|text| text.contains("Users"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn default_context_denies_loopback_navigation() {
        let page = Browser::new().new_context().new_page();
        let error = page
            .goto("http://127.0.0.1:9/")
            .await
            .expect_err("loopback must be denied before connecting");
        assert!(matches!(error, RuntimeError::Network(message) if message.contains("loopback")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ephemeral_context_shares_cookies_between_its_pages() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let (cookie_sender, cookie_receiver) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                if index == 1 {
                    cookie_sender
                        .send(request.to_string())
                        .expect("capture cookie request");
                }
                let extra = if index == 0 {
                    "Set-Cookie: oryn_session=native; Path=/; SameSite=Lax\r\n"
                } else {
                    ""
                };
                let body = "<title>Cookie</title>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let first = context.new_page();
        first
            .goto(format!("http://{address}/set"))
            .await
            .expect("set cookie");
        first.close().await.expect("close first page");
        let second = context.new_page();
        second
            .goto(format!("http://{address}/check"))
            .await
            .expect("send cookie");
        let request = cookie_receiver.recv().expect("cookie request");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("cookie: oryn_session=native")
        );
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn external_classic_scripts_execute_in_order_and_are_cached() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = if path == "/app.js" {
                    "document.querySelector('#result').textContent='External loaded';"
                } else {
                    "<title>External</title><script src=/app.js></script><output id=result>Pending</output>"
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate with external script");
        assert!(
            page.observe()
                .await
                .expect("observe")
                .nodes
                .iter()
                .any(|node| node.name == "External loaded")
        );
        page.refresh().await.expect("refresh using cached script");
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn external_css_import_and_web_font_are_parent_brokered() {
        struct CssBroker;
        impl crate::network::NetworkBroker for CssBroker {
            fn send(
                &mut self,
                request: NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                let path = Url::parse(&request.url)
                    .expect("fixture URL")
                    .path()
                    .to_string();
                let (content_type, body) = match path.as_str() {
                    "/" => (
                        "text/html",
                        br#"<link rel=stylesheet href=/base.css><style>@import '/inline.css';</style><button id=styled>Styled</button>"#
                            .to_vec(),
                    ),
                    "/base.css" => (
                        "text/css",
                        br#"@import url('/import.css');@font-face{font-family:'Fixture Font';src:url('/font.ttf')}button{width:111px;font-family:'Fixture Font'}"#.to_vec(),
                    ),
                    "/import.css" => ("text/css", b"button{height:33px}".to_vec()),
                    "/inline.css" => ("text/css", b"button{padding-left:7px}".to_vec()),
                    "/font.ttf" => (
                        "font/ttf",
                        oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                    ),
                    _ => return Err(crate::network::NetworkError::Transport(path)),
                };
                Ok(crate::network::NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), content_type.into())],
                    body,
                })
            }
        }

        let context = Browser::new().new_context_with_broker(
            ContextOptions::default(),
            SharedNetworkBroker::new(CssBroker),
        );
        let page = context.new_page();
        page.goto("https://assets.example.test/")
            .await
            .expect("load brokered stylesheet fixture");
        let observation = page.observe().await.expect("observe styled button");
        let target = observation
            .nodes
            .iter()
            .find(|node| node.name == "Styled")
            .expect("styled button")
            .semantic_ref;
        let style = page.computed_style(target).await.expect("computed style");
        assert_eq!(style.properties["width"], "111px");
        assert_eq!(style.properties["height"], "33px");
        assert_eq!(style.properties["padding-left"], "7px");
        assert_eq!(style.properties["font-family"], "\"Fixture Font\"");
        assert!(page.trace().await.expect("trace").iter().any(|event| {
            matches!(
                &event.kind,
                TraceEventKind::FontLoaded { family, sha256, .. }
                    if family == "Fixture Font" && sha256 == oryn_layout::DEFAULT_FONT_SHA256
            )
        }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn external_modules_link_dependencies_and_evaluate() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = match path {
                    "/entry.js" => {
                        "import { label } from './dep.js'; document.querySelector('#module-result').textContent=label;"
                    }
                    "/dep.js" => "export const label = 'Module loaded';",
                    _ => {
                        "<title>Module</title><script type=module src=/entry.js></script><output id=module-result>Pending</output>"
                    }
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate with module graph");
        assert!(
            page.observe()
                .await
                .expect("observe")
                .nodes
                .iter()
                .any(|node| node.name == "Module loaded")
        );
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn promise_fetch_uses_the_policy_broker_and_updates_dom() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = if path == "/data" {
                    r#"{"label":"Fetched"}"#
                } else {
                    r#"<output id=result>Pending</output><script>fetch('/data').then(response=>response.json()).then(data=>document.querySelector('#result').textContent=data.label)</script>"#
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate fetch fixture");
        assert!(
            page.observe()
                .await
                .expect("observe")
                .nodes
                .iter()
                .any(|node| node.name == "Fetched")
        );
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn same_origin_iframe_is_semantic_and_cross_origin_is_diagnostic() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = if path == "/frame" {
                    "<title>Child</title><button id=frame-action onclick=\"this.textContent='Clicked in child'\">Pending</button><script>globalThis.frameRealmMarker='isolated';document.querySelector('#frame-action').textContent='Frame action'</script>"
                } else {
                    "<iframe src=/frame></iframe><iframe src=https://cross-origin.invalid/frame></iframe>"
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate frame fixture");
        let observation = page.observe().await.expect("observe frames");
        assert!(
            observation
                .nodes
                .iter()
                .any(|node| node.name == "Frame action")
        );
        assert!(observation.capabilities.iter().any(|capability| {
            capability.capability == "html.frame.cross_origin"
                && capability.support == SupportLevel::Unsupported
        }));
        assert_eq!(
            page.evaluate("typeof frameRealmMarker")
                .await
                .expect("top-level realm remains isolated"),
            "undefined"
        );
        assert_eq!(
            page.evaluate("document.querySelector('iframe').contentDocument.querySelector('#frame-action').textContent")
                .await
                .expect("same-origin frame DOM is accessible"),
            "Frame action"
        );
        let frame_action = observation
            .nodes
            .iter()
            .find(|node| node.name == "Frame action")
            .expect("frame action target")
            .semantic_ref;
        page.execute(NativeAction {
            target: frame_action,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .expect("execute inside isolated frame realm");
        assert_eq!(
            page.evaluate("document.querySelector('iframe').contentDocument.querySelector('#frame-action').textContent")
                .await
                .expect("updated same-origin frame DOM is reflected"),
            "Clicked in child"
        );
        server.join().expect("fixture server");
    }

    fn fixture_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actions_are_generation_checked_and_report_effects() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/form",
            "<label>Email<input aria-label=Email></label>",
        )
        .await
        .expect("load HTML");
        let observation = page.observe().await.expect("observe document");
        let input = observation
            .nodes
            .iter()
            .find(|node| node.actions.contains(&SemanticAction::Type))
            .expect("input semantic node");
        let result = page
            .execute(NativeAction {
                target: input.semantic_ref,
                action: SemanticAction::Type,
                value: Some("agent@example.test".into()),
            })
            .await
            .expect("type into input");

        assert_eq!(result.revision_before, Revision(1));
        assert_eq!(result.revision_after, Revision(2));
        assert!(matches!(result.effects[0], Effect::DomMutation { .. }));
        let delta = page
            .observe_delta(Revision(1))
            .await
            .expect("observe delta");
        assert_eq!(delta.to_revision, Revision(2));
        assert_eq!(delta.upserted.len(), 1);
        assert_eq!(
            delta.upserted[0].value.as_deref(),
            Some("agent@example.test")
        );
        let trace = page.trace().await.expect("read trace");
        assert!(matches!(
            trace.last().map(|event| &event.kind),
            Some(TraceEventKind::Mutation { .. })
        ));

        let error = page
            .execute(NativeAction {
                target: SemanticRef {
                    page_id: input.semantic_ref.page_id,
                    document_generation: 0,
                    node: input.semantic_ref.node,
                },
                action: SemanticAction::Clear,
                value: None,
            })
            .await
            .expect_err("old generation must fail");
        assert!(matches!(
            error,
            RuntimeError::WrongDocumentGeneration { .. }
        ));
    }

    #[tokio::test]
    async fn consequential_navigation_requires_scoped_one_use_confirmation() {
        let mut options = ContextOptions::default();
        options.action_policy.rules.insert(
            ConsequentialAction::CrossOriginNavigation,
            PolicyRule::RequireConfirmation,
        );
        let context = Browser::new().new_context_with_options(options);
        let page = context.new_page();
        page.load_html("https://app.example/", "<p>ready</p>")
            .await
            .unwrap();
        let request = PolicyRequest {
            action: ConsequentialAction::CrossOriginNavigation,
            source_origin: "https://app.example".into(),
            target_origin: Some("https://other.example".into()),
            detail: Some("https://other.example/".into()),
        };
        assert!(matches!(
            page.goto("https://other.example/").await,
            Err(RuntimeError::ConfirmationRequired {
                action: ConsequentialAction::CrossOriginNavigation
            })
        ));
        let confirmation = context.prepare_confirmation(request);
        // The policy decision is consumed before network activity. A second
        // use can therefore never authorize another navigation.
        let _ = page
            .goto_authorized("https://other.example/", Some(confirmation))
            .await;
        assert!(matches!(
            page.goto_authorized("https://other.example/", Some(confirmation))
                .await,
            Err(RuntimeError::ConfirmationRequired { .. })
        ));
    }

    #[tokio::test]
    async fn secret_fill_is_origin_scoped_confirmed_and_observation_redacted() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        page.load_html(
            "https://app.example/",
            r#"<label for="pw">Password</label><input id="pw" type="password">"#,
        )
        .await
        .unwrap();
        let target = page
            .resolve_selector("#pw", SemanticAction::Type)
            .await
            .unwrap();
        let handle = context.import_secret(
            "https://app.example",
            "account password",
            b"do-not-observe".to_vec(),
        );
        let request = PolicyRequest {
            action: ConsequentialAction::CredentialFill,
            source_origin: "https://app.example".into(),
            target_origin: None,
            detail: Some("opaque secret fill".into()),
        };
        let confirmation = context.prepare_confirmation(request);
        page.fill_secret(target, handle, Some(confirmation))
            .await
            .unwrap();
        let observation = page.observe().await.unwrap();
        let password = observation
            .nodes
            .iter()
            .find(|node| node.semantic_ref == target)
            .unwrap();
        assert_eq!(password.value.as_deref(), Some("[REDACTED]"));
        assert!(
            !serde_json::to_string(&observation)
                .unwrap()
                .contains("do-not-observe")
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn typed_page_actions_dispatch_persistent_script_handlers() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/svelte",
            include_str!("../../../test-harness/scenarios/spa/svelte-tasks.html"),
        )
        .await
        .expect("load executable fixture");
        let before = page.observe().await.expect("observe");
        let increment = before
            .nodes
            .iter()
            .find(|node| node.name == "Add item")
            .expect("increment button");
        page.execute(NativeAction {
            target: increment.semantic_ref,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .expect("click through page actor");
        let after = page.observe().await.expect("observe mutation");
        assert!(after.nodes.iter().any(|node| node.name == "Items: 1"));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn removed_dom_nodes_invalidate_semantic_references_before_layout_or_action() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/stale-node",
            "<button id=target>Remove me</button>",
        )
        .await
        .expect("load stale-reference fixture");
        let reference = page
            .observe()
            .await
            .expect("observe target")
            .nodes
            .into_iter()
            .find(|node| node.name == "Remove me")
            .expect("semantic target")
            .semantic_ref;
        page.evaluate("document.querySelector('#target').remove();'removed'")
            .await
            .expect("remove target");

        let error = page
            .execute(NativeAction {
                target: reference,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect_err("removed reference must be stale");

        assert!(matches!(error, RuntimeError::StaleSemanticRef));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn page_group_shares_local_storage_clones_session_and_rejects_foreign_refs() {
        let context = Browser::new().new_context();
        let opener = context.new_page();
        opener
            .load_html(
                "https://app.example.test/start",
                r#"<button id=open>Open</button><script>
                  localStorage.setItem('shared','from-opener');
                  sessionStorage.setItem('session','initial');
                  document.querySelector('#open').onclick=()=>window.open('https://app.example.test/callback');
                </script>"#,
            )
            .await
            .expect("load opener");
        opener
            .evaluate("String(window.open('https://app.example.test/callback').__orynPageId)")
            .await
            .expect("open popup");
        let popup = context
            .wait_for_page(
                PagePredicate::Opener {
                    page_id: opener.page_id(),
                },
                Duration::from_secs(1),
            )
            .await
            .expect("popup");
        let popup = context
            .activate_page(popup.page_id())
            .expect("activate popup");
        popup
            .load_html(
                "https://app.example.test/callback",
                r#"<output id=result></output><script>
                  document.querySelector('#result').textContent=localStorage.getItem('shared')+'|'+sessionStorage.getItem('session');
                  sessionStorage.setItem('session','popup-only');
                </script>"#,
            )
            .await
            .expect("load popup");
        assert_eq!(
            popup
                .evaluate("document.querySelector('#result').textContent")
                .await
                .expect("read popup storage"),
            "from-opener|initial"
        );
        assert_eq!(
            opener
                .evaluate("sessionStorage.getItem('session')")
                .await
                .expect("read opener session"),
            "initial"
        );

        let foreign = opener
            .observe()
            .await
            .expect("observe opener")
            .nodes
            .into_iter()
            .find(|node| node.name == "Open")
            .expect("button")
            .semantic_ref;
        let error = popup
            .execute(NativeAction {
                target: foreign,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect_err("foreign page reference must fail");
        assert!(matches!(error, RuntimeError::WrongPage { .. }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn web_crypto_file_and_stream_subset_is_executable() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        page.load_html(
            "https://app.example.test/apis",
            r#"<output id=result>pending</output><script>
              const file=new File(['hello'],'hello.txt',{type:'text/plain',lastModified:7});
              const transform=new TransformStream({transform(chunk,controller){controller.enqueue(chunk.toUpperCase());}});
              const chunks=[];
              const sink=new WritableStream({write(chunk){chunks.push(chunk);}});
              new ReadableStream({start(controller){controller.enqueue('oryn');controller.close();}}).pipeThrough(transform).pipeTo(sink).then(async()=>{
                const digest=new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode('abc')));
                const uuid=crypto.randomUUID();
                document.querySelector('#result').textContent=(await file.text())+'|'+chunks.join('')+'|'+digest.length+'|'+uuid.length;
              });
            </script>"#,
        )
        .await
        .expect("load API fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read API result"),
            "hello|ORYN|32|36"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn streams_expose_cancellation_and_bounded_writer_readiness() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://app.example.test/stream-control",
            r#"<output id=result>pending</output><script>
              let cancelled='';
              const readable=new ReadableStream({cancel(reason){cancelled=reason;}});
              const writer=new WritableStream({write(){}}).getWriter();
              Promise.all([readable.cancel('stop'),writer.ready]).then(()=>{
                document.querySelector('#result').textContent=cancelled+'|'+writer.desiredSize+'|ready';
              });
            </script>"#,
        )
        .await
        .expect("load controlled stream fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read stream control result"),
            "stop|1|ready"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn indexed_db_upgrade_transaction_and_cursor_subset_is_executable() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        page.load_html(
            "https://app.example.test/idb",
            r#"<output id=result>pending</output><script>
              const open=indexedDB.open('app',1);
              open.onupgradeneeded=event=>event.target.result.createObjectStore('items',{keyPath:'id'}).createIndex('by-name','name',{unique:true});
              open.onsuccess=event=>{
                const db=event.target.result;
                const write=db.transaction('items','readwrite').objectStore('items').put({id:7,name:'oryn'});
                write.onsuccess=()=>{
                  const request=db.transaction('items').objectStore('items').index('by-name').get('oryn');
                  request.onsuccess=()=>document.querySelector('#result').textContent=request.result.id+'|'+request.result.name;
                };
              };
            </script>"#,
        )
        .await
        .expect("load IndexedDB fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read IndexedDB result"),
            "7|oryn"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn parent_brokered_websocket_delivers_text_binary_and_close_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind WebSocket fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept WebSocket");
            let mut socket = tungstenite::accept(stream).expect("upgrade WebSocket");
            let text = socket.read().expect("read text WebSocket frame");
            assert_eq!(text.into_text().expect("text frame"), "hello");
            socket
                .send(tungstenite::Message::Text("hello".into()))
                .expect("echo text WebSocket frame");
            let binary = socket.read().expect("read binary WebSocket frame");
            assert_eq!(binary.into_data().as_ref(), &[0, 255, 7]);
            socket
                .send(tungstenite::Message::Binary(vec![7, 255, 0].into()))
                .expect("echo binary WebSocket frame");
            socket
                .close(Some(tungstenite::protocol::CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::Library(4001),
                    reason: "fixture-complete".into(),
                }))
                .expect("close WebSocket fixture");
        });
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        page.load_html(
            format!("http://{address}/fixture"),
            &format!(
                r#"<output id=result>pending</output><script>
                  const socket=new WebSocket('ws://{address}/socket');socket.binaryType='arraybuffer';
                  const events=[];
                  socket.onopen=()=>socket.send('hello');
                  socket.onmessage=event=>{{
                    if(typeof event.data==='string'){{events.push(event.data);socket.send(new Uint8Array([0,255,7]));}}
                    else events.push([...new Uint8Array(event.data)].join(','));
                  }};
                  socket.onclose=event=>document.querySelector('#result').textContent=events.join('|')+'|'+event.code+'|'+event.reason;
                </script>"#
            ),
        )
        .await
        .expect("load WebSocket fixture");
        // The socket is parent-thread brokered, so a busy parallel test run
        // may not complete the OS handshake within exactly two JavaScript
        // checkpoints. Poll the observable page state with a strict bound
        // instead of racing the broker and then tearing down a healthy socket.
        let mut result = "pending".to_string();
        for _ in 0..100 {
            result = page
                .evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read WebSocket result");
            if result != "pending" {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(result, "hello|7,255,0|4001|fixture-complete");
        let archive = context.trace_archive(Vec::new()).await.expect("task trace");
        assert!(
            archive
                .nondeterministic_regions
                .contains(&crate::replay::NondeterministicRegion::WebSocket)
        );
        server.join().expect("WebSocket fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn popup_opener_post_message_and_storage_events_are_context_scoped() {
        let context = Browser::new().new_context();
        let opener = context.new_page();
        opener
            .load_html(
                "https://app.example.test/start",
                "<script>globalThis.storageEvents=0;addEventListener('storage',()=>storageEvents++);globalThis.child=window.open()</script>",
            )
            .await
            .expect("load opener");
        let popup = context
            .wait_for_page(
                PagePredicate::Opener {
                    page_id: opener.page_id(),
                },
                Duration::from_secs(1),
            )
            .await
            .expect("popup");
        popup
            .load_html(
                "https://app.example.test/popup",
                r#"<output id=result>pending</output><script>
                  addEventListener('message',event=>document.querySelector('#result').textContent=event.origin+'|'+event.data);
                  addEventListener('storage',event=>document.querySelector('#result').setAttribute('data-storage',event.key+'|'+event.newValue));
                </script>"#,
            )
            .await
            .expect("load popup");
        assert_eq!(
            opener
                .evaluate("child.focus();String(child.closed)")
                .await
                .expect("focus popup through opener proxy"),
            "false"
        );
        assert_eq!(
            context.active_page().expect("focused popup").page_id(),
            popup.page_id()
        );
        opener
            .evaluate("let rejected=false;try{child.postMessage('blocked','https://wrong.example.test')}catch(_error){rejected=true}if(!rejected)throw new Error('wrong target origin was accepted');child.postMessage('code','https://app.example.test');localStorage.setItem('token','ready');'sent'")
            .await
            .expect("send opener effects");
        assert_eq!(
            popup
                .evaluate("__oryn_drain(false);document.querySelector('#result').textContent+'|'+document.querySelector('#result').getAttribute('data-storage')")
                .await
                .expect("poll popup context events"),
            "https://app.example.test|code|token|ready"
        );
        assert_eq!(
            opener
                .evaluate("__oryn_drain(false);String(storageEvents)")
                .await
                .expect("poll source page storage events"),
            "0",
            "storage events must not be delivered back to their source realm"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn popup_policy_denial_returns_null_without_creating_a_page() {
        let context =
            Browser::new().new_context_with_options(ContextOptions::default().with_popups(false));
        let page = context.new_page();
        page.load_html(
            "https://app.example.test/recovery",
            r#"<button id=open>Open popup</button><output id=result>ready</output><script>
              document.querySelector('#open').onclick=()=>document.querySelector('#result').textContent=window.open('/authorize')===null?'denied':'opened';
            </script>"#,
        )
        .await
        .expect("load denial fixture");
        let target = page
            .observe()
            .await
            .expect("observe denial fixture")
            .nodes
            .into_iter()
            .find(|node| node.name == "Open popup")
            .expect("popup button")
            .semantic_ref;
        let result = page
            .execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect("execute denied popup");

        assert_eq!(context.pages().len(), 1);
        assert!(result.effects.iter().any(|effect| {
            matches!(effect, Effect::Capability { diagnostic } if diagnostic.capability == "popup.open")
        }));
        assert_eq!(
            page.extract(ContentKind::Text, Some("#result".into()))
                .await
                .expect("read denial result"),
            serde_json::json!("denied")
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn fragment_link_preserves_document_and_script_effects() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/results",
            r##"<a href="#" onclick="document.querySelector('#status').textContent='reward:1'">Result</a><output id=status>reward:-</output>"##,
        )
        .await
        .expect("load fragment link fixture");
        let before = page.observe().await.expect("observe fixture");
        let target = before
            .nodes
            .iter()
            .find(|node| node.name == "Result")
            .expect("result link")
            .semantic_ref;

        let result = page
            .execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect("click fragment link");
        let after = page.observe().await.expect("observe retained document");

        assert!(
            result.delta.is_some(),
            "fragment navigation must not reload"
        );
        assert_eq!(
            before.page.document_generation,
            after.page.document_generation
        );
        assert!(after.page.url.ends_with('#'));
        assert_eq!(
            page.extract(ContentKind::Text, Some("#status".into()))
                .await
                .expect("extract retained reward"),
            serde_json::json!("reward:1")
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn target_blank_links_and_forms_create_opener_scoped_pages() {
        let context = Browser::new().new_context();
        let opener = context.new_page();
        opener
            .load_html(
                "https://app.example.test/start",
                r#"<a href='/link-target' target='_blank'>Open link</a>
                <form action='/form-target' target='_blank'><button>Open form</button></form>"#,
            )
            .await
            .expect("load target fixture");
        let observation = opener.observe().await.expect("observe target fixture");
        for name in ["Open link", "Open form"] {
            let target = observation
                .nodes
                .iter()
                .find(|node| {
                    node.name == name
                        && node.actions.contains(&SemanticAction::Click)
                        && matches!(node.role.as_str(), "link" | "button")
                })
                .expect("target control")
                .semantic_ref;
            opener
                .execute(NativeAction {
                    target,
                    action: SemanticAction::Click,
                    value: None,
                })
                .await
                .expect("open target page");
        }
        let popups = context
            .pages()
            .into_iter()
            .filter(|page| page.page.opener_page_id == Some(opener.page_id()))
            .collect::<Vec<_>>();
        assert_eq!(popups.len(), 2);
        assert!(
            popups
                .iter()
                .all(|page| page.status == PageStatus::AttentionRequired)
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn asynchronous_xhr_reports_states_headers_and_json() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind XHR fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept XHR");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).expect("read XHR");
            let body = r#"{"name":"oryn"}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-G3: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .expect("write XHR response");
        });
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        page.load_html(
            format!("http://{address}/app"),
            r#"<output id=result>pending</output><script>
              const states=[];const xhr=new XMLHttpRequest();xhr.responseType='json';
              xhr.onreadystatechange=()=>states.push(xhr.readyState);
              xhr.onload=()=>document.querySelector('#result').textContent=xhr.response.name+'|'+xhr.getResponseHeader('x-g3')+'|'+states.join(',');
              xhr.open('GET','/api');xhr.send();
            </script>"#,
        )
        .await
        .expect("load XHR fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read XHR result"),
            "oryn|yes|1,2,3,4"
        );
        server.join().expect("XHR fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn xhr_sync_main_thread_is_truthfully_unsupported() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://app.example.test/xhr-sync",
            r#"<output id=result>pending</output><script>
              try { const xhr=new XMLHttpRequest();xhr.open('GET','/api',false); }
              catch(error) { document.querySelector('#result').textContent=String(error.message); }
            </script>"#,
        )
        .await
        .expect("load synchronous XHR diagnostic fixture");
        assert_eq!(
            page.evaluate("document.querySelector('#result').textContent")
                .await
                .expect("read synchronous XHR diagnostic"),
            "synchronous main-thread XHR is unsupported"
        );
        assert!(page.trace().await.expect("XHR trace").iter().any(|entry| {
            matches!(
                &entry.kind,
                TraceEventKind::PolicyDenied { capability }
                    if capability == "xhr.synchronous_main_thread"
            )
        }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn xhr_abort_and_timeout_are_observable() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind XHR timeout fixture");
        let address = listener.local_addr().expect("XHR timeout fixture address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept timed XHR");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).expect("read timed XHR");
            std::thread::sleep(std::time::Duration::from_millis(20));
            stream
                .write_all(fixture_response("late").as_bytes())
                .expect("write timed XHR response");
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.load_html(
            format!("http://{address}/app"),
            &format!(
                r#"<output id=result>pending</output><script>
                  const result=document.querySelector('#result');
                  const aborted=new XMLHttpRequest();
                  aborted.onabort=()=>result.textContent='abort';
                  aborted.open('GET','http://{address}/must-not-dispatch');aborted.send();aborted.abort();
                  const timed=new XMLHttpRequest();timed.timeout=1;
                  timed.ontimeout=()=>result.textContent+='|timeout';
                  timed.open('GET','http://{address}/slow');timed.send();
                </script>"#
            ),
        )
        .await
        .expect("load XHR abort and timeout fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read XHR abort/timeout result"),
            "abort|timeout"
        );
        server.join().expect("XHR timeout fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn file_formdata_request_response_and_headers_are_executable() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://app.example.test/file-fetch",
            r#"<form id=form><input name=name value=oryn><input name=enabled type=checkbox value=yes checked></form><output id=result>pending</output><script>
              const headers=new Headers({'X-G3':'one'});headers.append('x-g3','two');
              const form=new FormData(document.querySelector('#form'));
              form.append('upload',new Blob(['payload'],{type:'text/plain'}),'proof.txt');
              const request=new Request('/submit',{method:'POST',headers,body:form});
              const response=Response.json({accepted:true},{status:201,headers:{'X-Result':'ready'}});
              const file=new File(['hello'],'hello.txt',{type:'text/plain',lastModified:7});
              Promise.all([response.json(),file.text()]).then(([body,text])=>{
                document.querySelector('#result').textContent=[request.method,request.headers.get('x-g3'),request.body.get('name'),request.body.get('enabled'),request.body.get('upload').name,response.status,response.headers.get('x-result'),body.accepted,file.name,file.type,file.lastModified,text].join('|');
              });
            </script>"#,
        )
        .await
        .expect("load file/fetch primitives fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read file/fetch primitives result"),
            "POST|one, two|oryn|yes|proof.txt|201|ready|true|hello.txt|text/plain|7|hello"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn fetch_preserves_binary_request_bytes_and_abort_prevents_dispatch() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind byte fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept binary fetch");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            let (header_end, content_length) = loop {
                let length = stream.read(&mut buffer).expect("read binary fetch");
                assert_ne!(length, 0, "request closed before headers");
                request.extend_from_slice(&buffer[..length]);
                if let Some(index) = request.windows(4).position(|item| item == b"\r\n\r\n") {
                    let header_end = index + 4;
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':').and_then(|(name, value)| {
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                        })
                        .unwrap_or(0);
                    break (header_end, content_length);
                }
            };
            while request.len() < header_end + content_length {
                let length = stream.read(&mut buffer).expect("read binary body");
                assert_ne!(length, 0, "request closed before body");
                request.extend_from_slice(&buffer[..length]);
            }
            assert_eq!(
                &request[header_end..header_end + content_length],
                &[0, 255, 1]
            );
            stream
                .write_all(fixture_response("ok").as_bytes())
                .expect("write byte response");
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.load_html(
            format!("http://{address}/app"),
            &format!(
                r#"<output id=result>pending</output><script>
                  fetch('http://{address}/bytes',{{method:'POST',body:new Uint8Array([0,255,1])}})
                    .then(response=>response.text()).then(value=>document.querySelector('#result').textContent=value);
                </script>"#
            ),
        )
        .await
        .expect("load binary fetch fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read binary fetch result"),
            "ok"
        );
        server.join().expect("binary fixture server");

        page.load_html(
            "https://app.example.test/abort",
            r#"<output id=result>pending</output><script>
              const controller=new AbortController();
              fetch('https://network-must-not-run.invalid/',{signal:controller.signal})
                .then(()=>document.querySelector('#result').textContent='wrong')
                .catch(()=>document.querySelector('#result').textContent='aborted');
              controller.abort(new Error('cancelled'));
            </script>"#,
        )
        .await
        .expect("load abort fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read aborted fetch result"),
            "aborted"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn response_body_streams_in_bounded_parent_owned_chunks() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stream fixture");
        let address = listener.local_addr().expect("stream fixture address");
        let body = vec![b'x'; 150_000];
        let server_body = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept stream request");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).expect("read stream request");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                server_body.len()
            )
            .expect("write stream headers");
            stream.write_all(&server_body).expect("write stream body");
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.load_html(
            format!("http://{address}/app"),
            format!(
                r#"<output id=result>pending</output><script>
                  fetch('http://{address}/stream').then(async response=>{{
                    const reader=response.body.getReader();let chunks=0,total=0;
                    for(;;){{const item=await reader.read();if(item.done)break;chunks++;total+=item.value.length;}}
                    document.querySelector('#result').textContent=chunks+':'+total;
                  }});
                </script>"#
            ),
        )
        .await
        .expect("load stream fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read stream result"),
            "3:150000"
        );
        let trace = page.trace().await.expect("stream trace");
        assert!(trace.iter().any(|entry| matches!(
            entry.kind,
            TraceEventKind::RequestStarted { ref url, .. } if url.ends_with("/stream")
        )));
        assert_eq!(
            trace
                .iter()
                .filter(|entry| matches!(entry.kind, TraceEventKind::StreamChunk { .. }))
                .count(),
            3
        );
        server.join().expect("stream fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn cross_origin_credentials_obey_cookie_and_cors_rules() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind CORS fixture");
        let address = listener.local_addr().expect("CORS fixture address");
        let reserved = TcpListener::bind("127.0.0.1:0").expect("reserve page origin");
        let page_origin = format!("http://{}", reserved.local_addr().unwrap());
        drop(reserved);
        let expected_origin = page_origin.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept CORS request");
                let mut request = [0_u8; 4096];
                let length = stream.read(&mut request).expect("read CORS request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let cookie = request
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("cookie")
                                .then(|| value.trim().to_string())
                        })
                    })
                    .unwrap_or_else(|| "none".into());
                let allowed_origin = if path == "/wild" {
                    "*"
                } else {
                    expected_origin.as_str()
                };
                let set_cookie = if path == "/set" {
                    "Set-Cookie: session=ready; Path=/; HttpOnly\r\n"
                } else {
                    ""
                };
                let body = cookie;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: {allowed_origin}\r\nAccess-Control-Allow-Credentials: true\r\n{set_cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write CORS response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.load_html(
            format!("{page_origin}/app"),
            &format!(
                r#"<output id=result>pending</output><script>
                  fetch('http://{address}/set',{{credentials:'include'}})
                    .then(()=>fetch('http://{address}/omit',{{credentials:'omit'}})).then(response=>response.text())
                    .then(omit=>fetch('http://{address}/include',{{credentials:'include'}}).then(response=>response.text()).then(include=>[omit,include]))
                    .then(values=>fetch('http://{address}/wild',{{credentials:'include'}}).then(()=>{{throw new Error('wildcard accepted')}}).catch(()=>values))
                    .then(values=>document.querySelector('#result').textContent=values.join('|'));
                </script>"#
            ),
        )
        .await
        .expect("load credential fixture");
        let result = page
            .evaluate("__oryn_drain(true);__oryn_drain(true);__oryn_drain(true);document.querySelector('#result').textContent")
            .await
            .expect("read credential result");
        assert_eq!(result, "none|session=ready");
        server.join().expect("CORS fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn indexed_db_structured_clone_roundtrips_binary_blob_and_file() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://app.example.test/clone",
            r#"<output id=result>pending</output><script>
              const open=indexedDB.open('clone',1);
              open.onupgradeneeded=event=>event.target.result.createObjectStore('items',{keyPath:'id'});
              open.onsuccess=event=>{
                const db=event.target.result;
                const value={id:1,bytes:new Uint16Array([513,1027]),blob:new Blob(['blob']),file:new File(['file'],'proof.txt',{lastModified:7})};
                const put=db.transaction('items','readwrite').objectStore('items').put(value);
                put.onsuccess=()=>{
                  const get=db.transaction('items').objectStore('items').get(1);
                  get.onsuccess=async()=>{
                    const item=get.result;
                    document.querySelector('#result').textContent=[item.bytes.constructor.name,item.bytes[0],item.bytes[1],await item.blob.text(),item.file.name,item.file.lastModified,await item.file.text()].join('|');
                  };
                };
              };
            </script>"#,
        )
        .await
        .expect("load structured clone fixture");
        assert_eq!(
            page.evaluate("__oryn_drain(true);document.querySelector('#result').textContent")
                .await
                .expect("read structured clone result"),
            "Uint16Array|513|1027|blob|proof.txt|7|file"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn page_close_does_not_invalidate_an_unrelated_page_reference() {
        let context = Browser::new().new_context();
        let first = context.new_page();
        let second = context.new_page();
        first
            .load_html("https://one.example.test", "<button>First</button>")
            .await
            .expect("load first page");
        second
            .load_html("https://two.example.test", "<button>Second</button>")
            .await
            .expect("load second page");
        let target = second
            .observe()
            .await
            .expect("observe second")
            .nodes
            .into_iter()
            .find(|node| node.name == "Second")
            .expect("second button")
            .semantic_ref;
        context
            .activate_page(first.page_id())
            .expect("activate first");
        context
            .close_page(first.page_id())
            .await
            .expect("close first");
        let result = second
            .execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect("unrelated reference survives");
        assert_eq!(result.page_id, second.page_id());
        assert_eq!(
            context.active_page().expect("replacement active").page_id(),
            second.page_id()
        );
        assert!(context.page_events().iter().any(|event| {
            matches!(event, PageLifecycleEvent::Closed { page_id } if *page_id == first.page_id())
        }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn contract_v4_exposes_computed_style_and_relational_geometry() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://layout.example.test",
            r#"<style>
                .panel { width: 300px; padding: 10px; }
                .panel button { display: block; width: 120px; height: 30px; margin: 4px; }
              </style>
              <div class=panel><button id=first>First</button><button id=second>Second</button></div>"#,
        )
        .await
        .expect("load layout fixture");
        let observation = page
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: false,
            })
            .await
            .expect("observe geometry");
        assert_eq!(
            observation.contract_version,
            oryn_common::v4::CONTRACT_VERSION
        );
        let first = observation
            .nodes
            .iter()
            .find(|node| node.semantic.name == "First")
            .expect("first button");
        let second = observation
            .nodes
            .iter()
            .find(|node| node.semantic.name == "Second")
            .expect("second button");
        let first_rect = first
            .geometry
            .as_ref()
            .unwrap()
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        let second_rect = second
            .geometry
            .as_ref()
            .unwrap()
            .box_model
            .as_ref()
            .unwrap()
            .border_box;
        assert_eq!(first_rect.width, 120.0);
        assert!(second_rect.y > first_rect.y);
        let style = page
            .computed_style(first.semantic.semantic_ref)
            .await
            .expect("computed style");
        assert_eq!(style.properties["width"], "120px");
        assert_eq!(
            page.evaluate("getComputedStyle(document.querySelector('#first')).width")
                .await
                .expect("script computed style"),
            "120px"
        );
        assert_eq!(
            page.evaluate("document.querySelector('#first').getBoundingClientRect().width")
                .await
                .expect("script geometry"),
            "120"
        );
        page.evaluate("document.styleSheets[0].insertRule('#first { width: 140px }');'updated'")
            .await
            .expect("mutate CSSOM");
        assert_eq!(
            page.evaluate("getComputedStyle(document.querySelector('#first')).width")
                .await
                .expect("updated computed style"),
            "140px"
        );
        page.set_media(MediaState {
            color_scheme: PreferredColorScheme::Dark,
            reduced_motion: ReducedMotion::Reduce,
        })
        .await
        .expect("set emulated media");
        assert_eq!(
            page.evaluate(
                "String(matchMedia('(prefers-color-scheme: dark)').matches)+'|'+String(matchMedia('(prefers-reduced-motion: reduce)').matches)",
            )
            .await
            .expect("evaluate media query"),
            "true|true"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn viewport_scroll_and_table_geometry_are_truthful() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://layout.example.test",
            "<style>#spacer{height:900px}button{display:block;width:100px;height:30px}</style><div id=spacer></div><button id=end>End</button><table id=data><tr><td>Cell</td></tr></table>",
        )
        .await
        .expect("load scroll fixture");
        page.set_viewport(ViewportState {
            width: 320.0,
            height: 200.0,
            device_pixel_ratio: 1.0,
        })
        .await
        .expect("set viewport");
        let observation = page
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await
            .expect("observe");
        let end = observation
            .nodes
            .iter()
            .find(|node| node.semantic.name == "End")
            .expect("end button")
            .semantic
            .semantic_ref;
        let table = observation
            .nodes
            .iter()
            .find(|node| node.semantic.selector.as_deref() == Some("#data"))
            .expect("table")
            .semantic
            .semantic_ref;
        let scroll = page.scroll_into_view(end).await.expect("scroll into view");
        assert!(scroll.y > 0.0);
        assert!(matches!(
            page.box_model(table).await,
            Err(RuntimeError::UnsupportedGeometry(_))
        ));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn top_level_navigation_resets_scroll_before_interactability_checks() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://layout.example.test/first",
            "<style>#spacer{height:900px}button{display:block;width:100px;height:30px}</style><div id=spacer></div><button>Far</button>",
        )
        .await
        .expect("load first document");
        page.set_viewport(ViewportState {
            width: 320.0,
            height: 200.0,
            device_pixel_ratio: 1.0,
        })
        .await
        .expect("set viewport");
        let far = page
            .observe()
            .await
            .expect("observe first document")
            .nodes
            .into_iter()
            .find(|node| node.name == "Far")
            .expect("far button")
            .semantic_ref;
        assert!(page.scroll_into_view(far).await.expect("scroll down").y > 0.0);

        page.load_html(
            "https://layout.example.test/second",
            "<button style='width:100px;height:30px'>Top</button>",
        )
        .await
        .expect("load second document");
        let observation = page
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: false,
            })
            .await
            .expect("observe second document");
        assert_eq!(observation.scroll.x, 0.0);
        assert_eq!(observation.scroll.y, 0.0);
        let top = observation
            .nodes
            .iter()
            .find(|node| node.semantic.name == "Top")
            .expect("top button")
            .semantic
            .semantic_ref;
        page.execute(NativeAction {
            target: top,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .expect("top control remains interactable after navigation");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn viewport_filter_interactability_and_closed_shadow_are_truthful() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://layout.example.test",
            r#"<style>
                button { display:block; width:100px; height:30px }
                #hidden { display:none }
                #zero { width:0; height:0 }
                #blocked { pointer-events:none }
                #spacer { height:900px }
              </style>
              <button id=hidden>Hidden</button>
              <button id=zero>Zero</button>
              <button id=blocked>Blocked</button>
              <div id=host></div><div id=spacer></div><button id=far>Far</button>
              <script>document.querySelector('#host').attachShadow({mode:'closed'}).innerHTML='<button>Secret</button>';</script>"#,
        )
        .await
        .expect("load interactability fixture");
        page.set_viewport(ViewportState {
            width: 320.0,
            height: 180.0,
            device_pixel_ratio: 1.0,
        })
        .await
        .expect("set viewport");
        let all = page
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await
            .expect("observe all geometry");
        assert!(all.nodes.iter().all(|node| node.semantic.name != "Secret"));
        let find = |name: &str| {
            all.nodes
                .iter()
                .find(|node| node.semantic.name == name)
                .expect("fixture semantic node")
        };
        assert_eq!(
            find("Hidden").geometry.as_ref().unwrap().availability,
            oryn_common::v4::GeometryAvailability::Hidden
        );
        for name in ["Hidden", "Zero", "Blocked"] {
            let result = page
                .execute(NativeAction {
                    target: find(name).semantic.semantic_ref,
                    action: SemanticAction::Click,
                    value: None,
                })
                .await;
            assert!(matches!(result, Err(RuntimeError::NotInteractable(_))));
        }
        let visible = page
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: true,
                include_hidden: false,
            })
            .await
            .expect("observe viewport");
        assert!(visible.nodes.iter().all(|node| node.semantic.name != "Far"));
        let far = find("Far").semantic.semantic_ref;
        let result = page
            .execute_v4(NativeAction {
                target: far,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect("auto-scroll and click");
        assert_eq!(result.contract_version, oryn_common::v4::CONTRACT_VERSION);
        assert!(
            page.observe_delta_v4(result.revision_before)
                .await
                .expect("v4 delta")
                .scroll
                .is_some()
        );
        assert!(
            page.observe_with(ObservationOptions {
                profile: ProjectionProfile::Full,
                include_geometry: true,
                viewport_only: true,
                include_hidden: false,
            })
            .await
            .expect("observe scrolled viewport")
            .nodes
            .iter()
            .any(|node| node.semantic.name == "Far")
        );
    }

    #[cfg(all(unix, feature = "in-process-probe"))]
    #[test]
    fn worker_replacement_reopens_a_blank_generation_without_touching_other_pages() {
        let context = Browser::in_process_probe().new_context();
        let replaced = context.new_page();
        let unrelated = context.new_page();
        context
            .activate_page(replaced.page_id())
            .expect("activate replaced page");
        context
            .inner
            .group
            .crash(replaced.page_id(), "fixture crash".into());

        let replacement_generation = 7_u64 << 32;
        context
            .inner
            .group
            .worker_started(replaced.page_id(), replacement_generation);
        let pages = context.pages();
        let replaced_descriptor = pages
            .iter()
            .find(|page| page.page.page_id == replaced.page_id())
            .expect("replaced descriptor");
        let unrelated_descriptor = pages
            .iter()
            .find(|page| page.page.page_id == unrelated.page_id())
            .expect("unrelated descriptor");

        assert_eq!(replaced_descriptor.status, PageStatus::Open);
        assert!(replaced_descriptor.active);
        assert_eq!(replaced_descriptor.page.url, "about:blank");
        assert_eq!(
            replaced_descriptor.page.document_generation,
            replacement_generation
        );
        assert_eq!(unrelated_descriptor.status, PageStatus::Open);
        assert_eq!(unrelated_descriptor.page.document_generation, 0);
        assert!(context.page_events().iter().any(|event| {
            matches!(event, PageLifecycleEvent::Crashed { page_id, .. } if *page_id == replaced.page_id())
        }));
        assert!(context.page_events().iter().any(|event| {
            matches!(event, PageLifecycleEvent::Navigated { page_id, url, document_generation }
                if *page_id == replaced.page_id()
                    && url == "about:blank"
                    && *document_generation == replacement_generation)
        }));
    }

    #[tokio::test]
    async fn checkpoint_restore_uses_fresh_page_identity_and_restores_storage() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_context();
        let page = context.new_page();
        page.goto("about:blank").await.expect("blank page");
        let scope = StorageScope {
            origin: "null".into(),
            top_level_page_id: page.page_id(),
        };
        context
            .inner
            .store
            .execute(StorageRequest::Set {
                area: StorageArea::Session,
                scope,
                key: "step".into(),
                value: "shipping".into(),
            })
            .expect("set session state");
        let checkpoint = context
            .checkpoint(
                "shipping-ready",
                BTreeMap::from([("task".into(), "checkout".into())]),
                Vec::new(),
            )
            .await
            .expect("checkpoint");
        checkpoint.verify().expect("sealed checkpoint");

        let (restored, receipt) = browser
            .restore_checkpoint(ContextOptions::default(), &checkpoint)
            .await
            .expect("restore checkpoint");
        assert_eq!(receipt.pages.len(), 1);
        assert_ne!(
            receipt.pages[0].source_page_id,
            receipt.pages[0].destination_page_id
        );
        let response = restored
            .inner
            .store
            .execute(StorageRequest::Get {
                area: StorageArea::Session,
                scope: StorageScope {
                    origin: "null".into(),
                    top_level_page_id: receipt.pages[0].destination_page_id,
                },
                key: "step".into(),
            })
            .expect("read restored session state");
        assert!(matches!(
            response,
            StorageResponse::Value { value: Some(value) } if value == "shipping"
        ));
    }

    #[tokio::test]
    async fn recorded_navigation_checkpoint_restores_form_state_without_live_network() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).expect("read request");
            stream
                .write_all(fixture_response("<input id=name aria-label=Name>").as_bytes())
                .expect("write response");
        });
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_recording_context(ContextOptions::loopback_test(), true);
        let page = context.new_page();
        page.goto(format!("http://{address}/form"))
            .await
            .expect("record navigation");
        let target = page
            .resolve_selector("#name", SemanticAction::Type)
            .await
            .expect("resolve form field");
        page.execute(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some("restored-value".into()),
        })
        .await
        .expect("type value");
        let checkpoint = context
            .checkpoint("form-filled", BTreeMap::new(), Vec::new())
            .await
            .expect("checkpoint");
        server.join().expect("fixture server");

        let (restored, receipt) = browser
            .restore_checkpoint(ContextOptions::loopback_test(), &checkpoint)
            .await
            .expect("offline replay restore");
        let restored_page = restored
            .inner
            .group
            .handle(receipt.pages[0].destination_page_id)
            .expect("restored page");
        let observation = restored_page
            .observe()
            .await
            .expect("observe restored form");
        assert!(observation.nodes.iter().any(|node| {
            node.selector.as_deref() == Some("#name")
                && node.value.as_deref() == Some("restored-value")
        }));
        assert_eq!(restored.replayed_network_exchanges(), 1);
    }

    #[tokio::test]
    async fn task_trace_replays_post_checkpoint_intents_and_verifies_network_consumption() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_recording_context(ContextOptions::default(), true);
        context
            .new_page()
            .goto("about:blank")
            .await
            .expect("blank page");
        let checkpoint = context
            .checkpoint("before-intent", BTreeMap::new(), Vec::new())
            .await
            .expect("checkpoint");
        let mut session = crate::oil::NativeContextOilSession::new(context.clone());
        session
            .execute("page current")
            .await
            .expect("record intent");
        let trace = context.trace_archive(Vec::new()).await.expect("trace");

        let (_restored, replay) = browser
            .replay_task_trace(ContextOptions::default(), &checkpoint, &trace)
            .await
            .expect("replay task");
        assert_eq!(replay.attempted_intents, 1);
        assert_eq!(replay.replayed_intents, 1);
        assert!(replay.divergences.is_empty(), "{replay:#?}");
        assert!(replay.deterministic);
    }

    #[tokio::test]
    async fn replay_branch_records_parent_lineage_and_fresh_page_identity() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_context();
        let source = context.new_page();
        source.goto("about:blank").await.expect("blank page");
        let checkpoint = context
            .checkpoint("branch-root", BTreeMap::new(), Vec::new())
            .await
            .expect("checkpoint");
        let (_branch_context, branch) = browser
            .branch_from_checkpoint(ContextOptions::default(), &checkpoint, "alternate-payment")
            .await
            .expect("branch");
        assert_eq!(branch.parent_checkpoint_sha256, checkpoint.sha256);
        assert_eq!(branch.strategy, "alternate-payment");
        assert_ne!(
            branch.restored_pages[0].destination_page_id,
            source.page_id()
        );
    }

    #[tokio::test]
    async fn replay_reports_redacted_intent_as_nondeterministic_divergence() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_context();
        let source = context.new_page();
        source.goto("about:blank").await.expect("blank page");
        let checkpoint = context
            .checkpoint("before-secret", BTreeMap::new(), Vec::new())
            .await
            .expect("checkpoint");
        let trace = crate::replay::TaskTraceArchive {
            schema_version: crate::replay::TRACE_ARCHIVE_VERSION,
            context_id: context.id(),
            determinism: None,
            page_events: Vec::new(),
            page_traces: BTreeMap::new(),
            intents: vec![crate::replay::IntentTraceRecord {
                sequence: checkpoint.payload.intent_cursor,
                page_id: Some(source.page_id()),
                input: "type \"Password\" \"[REDACTED]\"".into(),
                revision_before: None,
                revision_after: None,
                outcome: "completed".into(),
            }],
            domain_decisions: Vec::new(),
            network: crate::replay::NetworkArchive::default(),
            nondeterministic_regions: Vec::new(),
        };
        let (_restored, replay) = browser
            .replay_task_trace(ContextOptions::default(), &checkpoint, &trace)
            .await
            .expect("replay");
        assert!(!replay.deterministic);
        assert!(replay.divergences.iter().any(|divergence| {
            divergence.kind == crate::replay::ReplayDivergenceKind::RedactedInput
        }));
    }

    #[tokio::test]
    async fn handoff_captures_only_explicit_transferable_state_and_reports_losses() {
        let browser = Browser::in_process_with_config(BrowserConfig::default());
        let context = browser.new_context();
        let page = context.new_page();
        page.load_html(
            "https://example.test/form",
            "<input id=name aria-label=Name value=Oryn><input id=password aria-label=Password type=password value=secret>",
        )
        .await
        .expect("load handoff page");
        let scope = StorageScope {
            origin: "https://example.test".into(),
            top_level_page_id: page.page_id(),
        };
        context
            .inner
            .store
            .execute(StorageRequest::Set {
                area: StorageArea::Local,
                scope,
                key: "theme".into(),
                value: "dark".into(),
            })
            .expect("set local storage");
        context.import_secret(
            "https://example.test",
            "api-token",
            b"do-not-transfer".to_vec(),
        );

        let bundle = context
            .prepare_handoff(
                page.page_id(),
                ExecutionDomain::Chromium,
                HandoffOptions {
                    allow_lossy: true,
                    ..HandoffOptions::default()
                },
                None,
            )
            .await
            .expect("prepare handoff");
        assert_eq!(
            bundle.storage[0].local,
            vec![("theme".into(), "dark".into())]
        );
        assert!(
            bundle.form_values.iter().any(|field| field.value == "Oryn"),
            "{bundle:#?}"
        );
        assert!(
            bundle
                .form_values
                .iter()
                .all(|field| field.value != "secret")
        );
        assert!(bundle.losses.contains(&HandoffLoss::JavascriptHeap));
        assert!(
            bundle
                .losses
                .contains(&HandoffLoss::HttpOnlyCookiesNotTransferred)
        );
        assert!(
            bundle
                .losses
                .contains(&HandoffLoss::SecretHandlesRemainAtSource)
        );
        assert_eq!(context.secret_labels(), vec!["api-token"]);
    }

    #[tokio::test]
    async fn handoff_cookie_transfer_requires_exact_parent_policy_confirmation() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html("https://example.test/account", "<button>Account</button>")
            .await
            .expect("load account");
        let options = HandoffOptions {
            transfer_cookie_header: true,
            allow_lossy: true,
            ..HandoffOptions::default()
        };
        let error = context
            .prepare_handoff(
                page.page_id(),
                ExecutionDomain::Chromium,
                options.clone(),
                None,
            )
            .await
            .expect_err("cookie handoff must require confirmation");
        assert!(matches!(
            error,
            RuntimeError::ConfirmationRequired {
                action: ConsequentialAction::ProtectedRequest
            }
        ));
        let request = PolicyRequest {
            action: ConsequentialAction::ProtectedRequest,
            source_origin: "https://example.test".into(),
            target_origin: None,
            detail: Some("handoff Chromium with cookies".into()),
        };
        let confirmation = context.prepare_confirmation(request);
        context
            .prepare_handoff(
                page.page_id(),
                ExecutionDomain::Chromium,
                options,
                Some(confirmation),
            )
            .await
            .expect("exact confirmation permits cookie handoff");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test]
    async fn paint_snapshot_is_worker_neutral_and_broker_complete() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html(
            "https://paint.example.test/",
            r#"<!doctype html><style>
              body{margin:0;background-color:#fefefe}
              #card{width:120px;height:40px;background-color:#123456;color:#ffffff;border:2px solid #000000}
              img{width:8px;height:8px}
            </style><div id=card>Paint me</div>
            <img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=">"#,
        )
        .await
        .expect("load paint fixture");
        let snapshot = page.paint_snapshot().await.expect("paint snapshot");
        assert_eq!(snapshot.contract_version, oryn_common::v4::CONTRACT_VERSION);
        assert_eq!(snapshot.images.len(), 1);
        assert_eq!(snapshot.images[0].media_type, "image/png");
        assert!(snapshot.nodes.iter().any(|node| {
            node.style.contains_key("background-color")
                && node
                    .text
                    .iter()
                    .any(|fragment| fragment.text.contains("Paint me"))
        }));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test]
    async fn deterministic_context_freezes_clock_and_random_stream() {
        let config = crate::replay::DeterminismConfig {
            unix_time_ms: 1_700_000_000_123,
            random_seed: 42,
        };
        async fn sample(config: crate::replay::DeterminismConfig) -> String {
            let context = Browser::in_process_with_config(BrowserConfig::default())
                .new_context_with_options(ContextOptions::default().with_determinism(config));
            let page = context.new_page();
            page.load_html(
                "fixture://determinism",
                "<script>document.body.textContent=Date.now()+':'+Math.random()+':'+crypto.randomUUID()</script>",
            )
            .await
            .expect("load deterministic page");
            page.extract(ContentKind::Text, None)
                .await
                .expect("extract")
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
        assert_eq!(sample(config.clone()).await, sample(config).await);
    }
}
