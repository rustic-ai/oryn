use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

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

use crate::challenge::{ChallengeAssessment, assess_challenge};
use crate::html::{ParsedDocument, parse};
use crate::network::NetworkBroker;
use crate::semantic::{empty_observation, is_hidden, project_element};
use crate::trace::{CausalRelation, TraceEvent, TraceEventKind, TraceLog};
use crate::{
    download::{DownloadDescriptor, DownloadId, DownloadStore},
    execution::{
        FormValueState, HandoffBundle, HandoffLoss, HandoffOptions, OriginStorageState,
        SensitiveCookieHeader,
    },
    network::{
        CookieBroker, MemoryCachingBroker, NetworkRequest, PersistentCachingBroker, PolicyBroker,
        RustlsTransport, SharedNetworkBroker, collect_operation_bounded,
    },
    security::{
        ActionPolicy, ConfirmationToken, ConsequentialAction, NetworkPolicy, PolicyDecision,
        PolicyEngine, PolicyRequest, SecretError, SecretHandle, SecretStore, SecurityAuditRecord,
    },
    storage::{
        PersistentProfile, SharedContextStore, StorageArea, StorageLimits, StorageRequest,
        StorageResponse, StorageScope,
    },
};

#[cfg(feature = "v8-host")]
use crate::network::NetworkResponse;

static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_WORKER_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_CSP_REPORT_REQUEST_ID: AtomicU64 = AtomicU64::new(1_u64 << 61);
#[cfg(feature = "v8-host")]
static NEXT_PAINT_IMAGE_REQUEST_ID: AtomicU64 = AtomicU64::new(1_u64 << 60);
static NEXT_OPAQUE_ORIGIN_ID: AtomicU64 = AtomicU64::new(1);
const NETWORK_IDLE_QUIET_PERIOD: Duration = Duration::from_millis(500);

#[derive(Default)]
struct CspReportingState {
    pending: usize,
    deliveries: BTreeMap<String, VecDeque<Instant>>,
}

#[cfg(feature = "v8-host")]
#[derive(Default)]
struct NavigationBodyCapture {
    source: String,
    bytes: usize,
    completed: bool,
}

/// Demand-driven bridge from the parent-owned network operation into the live
/// html5ever parser. `next()` is called only when the parser needs more input,
/// coupling response demand to parser progress; once html5ever publishes a
/// blocking boundary, JavaScript runs against the same `DomRuntime` before the
/// retained source suffix is consumed.
#[cfg(feature = "v8-host")]
struct NavigationBodyChunks {
    operation: crate::network::NetworkOperation,
    broker: SharedNetworkBroker,
    decoder: oryn_net::IncrementalTextDecoder,
    capture: Arc<Mutex<NavigationBodyCapture>>,
    deadline: Instant,
    finished: bool,
}

#[cfg(feature = "v8-host")]
impl Iterator for NavigationBodyChunks {
    type Item = Result<String, crate::host::HostError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            self.operation.request_body_chunk();
            let event = loop {
                match self.operation.events.try_recv() {
                    Ok(event) => break Some(event),
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break None,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        if Instant::now() >= self.deadline {
                            let _ = self.broker.cancel_operation(self.operation.request_id);
                            self.finished = true;
                            return Some(Err(crate::host::HostError::LimitExceeded {
                                resource: "navigation wall time".into(),
                            }));
                        }
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                }
            };
            match event {
                Some(crate::network::NetworkEvent::Chunk { request_id, bytes })
                    if request_id == self.operation.request_id =>
                {
                    let decoded = self.decoder.push(&bytes);
                    let mut capture = self.capture.lock().expect("navigation capture");
                    capture.bytes = capture.bytes.saturating_add(bytes.len());
                    capture.source.push_str(&decoded);
                    drop(capture);
                    if !decoded.is_empty() {
                        return Some(Ok(decoded));
                    }
                }
                Some(crate::network::NetworkEvent::Complete { request_id })
                    if request_id == self.operation.request_id =>
                {
                    self.finished = true;
                    let tail = self.decoder.finish();
                    let mut capture = self.capture.lock().expect("navigation capture");
                    capture.source.push_str(&tail);
                    capture.completed = true;
                    drop(capture);
                    return (!tail.is_empty()).then_some(Ok(tail));
                }
                Some(crate::network::NetworkEvent::Failed { request_id, error })
                    if request_id == self.operation.request_id =>
                {
                    self.finished = true;
                    return Some(Err(crate::host::HostError::Execution(format!(
                        "navigation body failed: {error}"
                    ))));
                }
                Some(crate::network::NetworkEvent::Headers { request_id, .. })
                    if request_id == self.operation.request_id =>
                {
                    self.finished = true;
                    return Some(Err(crate::host::HostError::Execution(
                        "network operation emitted duplicate response headers".into(),
                    )));
                }
                Some(_) => {
                    let _ = self.broker.cancel_operation(self.operation.request_id);
                    self.finished = true;
                    return Some(Err(crate::host::HostError::Execution(
                        "navigation operation received a stale or unmatched completion".into(),
                    )));
                }
                None => {
                    self.finished = true;
                    return Some(Err(crate::host::HostError::Execution(
                        "network operation ended before response completion".into(),
                    )));
                }
            }
        }
    }
}

#[cfg(feature = "v8-host")]
impl Drop for NavigationBodyChunks {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.broker.cancel_operation(self.operation.request_id);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SecurityOrigin {
    Tuple(String),
    Opaque(u64),
}

impl SecurityOrigin {
    fn from_url(url: &str) -> Self {
        Url::parse(url)
            .map(|url| Self::Tuple(url.origin().ascii_serialization()))
            .unwrap_or_else(|_| Self::opaque())
    }

    fn opaque() -> Self {
        Self::Opaque(NEXT_OPAQUE_ORIGIN_ID.fetch_add(1, Ordering::Relaxed))
    }

    fn serialize(&self) -> String {
        match self {
            Self::Tuple(value) => value.clone(),
            Self::Opaque(_) => "null".into(),
        }
    }

    fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque(_))
    }
}

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
    uploads: Arc<crate::upload::UploadStore>,
    virtual_clipboard: Mutex<String>,
    downloads: Arc<DownloadStore>,
    network_replay: crate::replay::NetworkReplayController,
    intent_trace: Mutex<Vec<crate::replay::IntentTraceRecord>>,
    domain_decisions: Mutex<Vec<crate::execution::DomainDecision>>,
    document_security: Mutex<BTreeMap<PageId, crate::csp::CspPolicySet>>,
    document_referrer_policy: Mutex<BTreeMap<PageId, crate::referrer::ReferrerPolicy>>,
    security_origins: Mutex<BTreeMap<PageId, SecurityOrigin>>,
    expected_meta_security: Mutex<BTreeMap<PageId, std::collections::VecDeque<String>>>,
    csp_reporting: Arc<Mutex<CspReportingState>>,
    /// One-shot parent authorizations consumed only by the corresponding
    /// worker-originated form effect.
    form_submission_grants: Mutex<BTreeMap<PageId, usize>>,
    navigation_grants: Mutex<BTreeSet<PageId>>,
    #[cfg(unix)]
    context_events: Mutex<Vec<ContextEventRecord>>,
    #[cfg(unix)]
    next_context_event_id: AtomicU64,
    #[cfg(unix)]
    parent_cookies: Option<std::sync::Arc<crate::cookies::ContextCookieStore>>,
    #[cfg(all(unix, feature = "v8-host"))]
    context_client: Option<crate::context_service::SharedContextClient>,
}

#[derive(Clone)]
pub struct PageHandle {
    id: u64,
    opener_page_id: Option<PageId>,
    context: Weak<BrowserContextInner>,
    // Public page handles retain their owning context. The context registry
    // stores detached handles to avoid an Arc cycle.
    context_guard: Option<Arc<BrowserContextInner>>,
    sender: Option<mpsc::Sender<PageCommand>>,
    #[cfg(unix)]
    worker: Option<std::sync::Arc<WorkerPage>>,
}

impl std::fmt::Debug for PageHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PageHandle")
            .field("id", &self.id)
            .field("opener_page_id", &self.opener_page_id)
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
struct WorkerPage {
    page_id: PageId,
    opener_page_id: Option<PageId>,
    context: Weak<BrowserContextInner>,
    client: std::sync::Mutex<Option<crate::worker_client::WorkerClient>>,
    config: BrowserConfig,
    options: ContextOptions,
    pending: AtomicUsize,
    context_wakeups: AtomicUsize,
    process_id: AtomicU32,
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
                handle: handle.detached_clone(),
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
            .map(|record| record.handle.retained_clone())
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
            .map(|record| record.handle.retained_clone())
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
            .map(|record| record.handle.retained_clone())
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
    fn interrupt_inflight(&self) -> bool {
        if self.pending.load(Ordering::Acquire) == 0 {
            return false;
        }
        let process_id = self.process_id.load(Ordering::Acquire);
        if process_id == 0 {
            return false;
        }
        // Production page workers are disposable security boundaries. Killing
        // a busy worker is the only operation permitted to bypass its serial
        // page command channel; replacement starts with a blank generation.
        unsafe { libc::kill(process_id.cast_signed(), libc::SIGKILL) == 0 }
    }

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
            let spawned = crate::worker_client::WorkerClient::spawn(
                &config,
                &self.options,
                self.page_id,
                self.opener_page_id,
                context_owner,
            )?;
            self.process_id
                .store(spawned.process_id(), Ordering::Release);
            *client = Some(spawned);
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
            self.process_id.store(0, Ordering::Release);
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub runtime_profile: RuntimeProfile,
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
    /// Maximum number of semantic revisions retained for delta queries.
    pub max_revision_snapshots: usize,
    /// Approximate serialized byte ceiling for retained semantic revisions.
    pub max_revision_bytes: usize,
    /// Maximum number of pinned script/style resources cached by a page.
    pub max_resource_cache_entries: usize,
    /// Combined UTF-8 byte ceiling for pinned page resources.
    pub max_resource_cache_bytes: usize,
    /// Maximum size of one cacheable response body.
    pub max_resource_cache_entry_bytes: usize,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            runtime_profile: RuntimeProfile::compiled(),
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
            max_revision_snapshots: 128,
            max_revision_bytes: 16 * 1024 * 1024,
            max_resource_cache_entries: 256,
            max_resource_cache_bytes: 32 * 1024 * 1024,
            max_resource_cache_entry_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    #[default]
    Full,
    LiteEnglish,
}

impl RuntimeProfile {
    pub const fn compiled() -> Self {
        if cfg!(feature = "core-lite") {
            Self::LiteEnglish
        } else {
            Self::Full
        }
    }

    pub const fn available_in_build(self) -> bool {
        matches!(
            (self, Self::compiled()),
            (Self::Full, Self::Full) | (Self::LiteEnglish, Self::LiteEnglish)
        )
    }

    pub const fn supported_locales(self) -> &'static [&'static str] {
        match self {
            Self::Full => &[],
            Self::LiteEnglish => &["root", "en", "en-US"],
        }
    }

    pub const fn capability_name(self) -> &'static str {
        match self {
            Self::Full => "core_full",
            Self::LiteEnglish => "core_lite_english",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CacheMode {
    Disabled,
    #[default]
    Memory,
    Persistent,
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
    /// Parent-owned HTTP cache policy. Persistent mode requires an explicitly
    /// opened persistent profile; workers never receive a cache path.
    #[serde(default)]
    pub cache_mode: CacheMode,
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
            cache_mode: CacheMode::Memory,
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
            cache_mode: CacheMode::Memory,
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
    #[serde(default)]
    pub challenge: ChallengeAssessment,
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
    PollContextEvents,
    Capabilities {
        reply: oneshot::Sender<Vec<CapabilityDiagnostic>>,
    },
    ChallengeAssessment {
        reply: oneshot::Sender<ChallengeAssessment>,
    },
    ResolveSelector {
        selector: String,
        action: SemanticAction,
        reply: oneshot::Sender<Result<SemanticRef, RuntimeError>>,
    },
    ResolveSemantic {
        target: oryn_common::agent::Target,
        action: SemanticAction,
        reply: oneshot::Sender<Result<oryn_common::v6::ResolutionResult, RuntimeError>>,
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
    ObserveSemantic {
        profile: ProjectionProfile,
        reply: oneshot::Sender<oryn_common::v5::SemanticObservation>,
    },
    ObserveSemanticV6 {
        options: oryn_common::v6::SemanticObservationOptions,
        reply: oneshot::Sender<Result<oryn_common::v6::SemanticObservation, RuntimeError>>,
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
    ObserveSemanticDelta {
        from: Revision,
        reply: oneshot::Sender<Result<oryn_common::v6::SemanticDelta, RuntimeError>>,
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
    ExecuteSensitive {
        action: NativeAction,
        reply: oneshot::Sender<Result<ActionResult, RuntimeError>>,
    },
    SetFiles {
        target: SemanticRef,
        files: Vec<crate::upload::UploadFile>,
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
        mut options: ContextOptions,
        store: SharedContextStore,
        persistent_profile: Option<PersistentProfile>,
        network_replay: crate::replay::NetworkReplayController,
    ) -> BrowserContext {
        let context_id = NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed);
        if options.cache_mode == CacheMode::Persistent && persistent_profile.is_none() {
            options.cache_mode = CacheMode::Memory;
        }
        let policy = Arc::new(PolicyEngine::new(options.action_policy.clone()));
        let secrets = Arc::new(SecretStore::default());
        let context_cookies = persistent_profile
            .as_ref()
            .map(PersistentProfile::cookie_store)
            .unwrap_or_else(crate::cookies::ContextCookieStore::memory);
        let timeout = self.config.request_timeout;
        let max_response_bytes = self.config.max_response_bytes;
        let network_policy = options.network_policy.clone();
        let transport_cookies = context_cookies.clone();
        let replay = network_replay.clone();
        let cache_mode = options.cache_mode.clone();
        let persistent_cache_path = persistent_profile
            .as_ref()
            .map(|profile| profile.path().to_path_buf());
        let cache_config = self.config.clone();
        let broker = std::thread::spawn(move || {
            RustlsTransport::new(timeout, max_response_bytes)
                .ok()
                .and_then(|transport| {
                    let transport = replay.wrap(transport);
                    let transport: Box<dyn crate::network::NetworkBroker> = match cache_mode {
                        CacheMode::Disabled => Box::new(transport),
                        CacheMode::Memory => Box::new(MemoryCachingBroker::new(
                            transport,
                            format!("context-{context_id}"),
                            cache_config.max_resource_cache_entries,
                            cache_config.max_resource_cache_bytes,
                            cache_config.max_resource_cache_entry_bytes,
                        )),
                        CacheMode::Persistent => Box::new(
                            PersistentCachingBroker::open(
                                transport,
                                persistent_cache_path.as_deref()?,
                                "profile",
                                cache_config.max_resource_cache_entries,
                                cache_config.max_resource_cache_bytes,
                                cache_config.max_resource_cache_entry_bytes,
                            )
                            .ok()?,
                        ),
                    };
                    Some(SharedNetworkBroker::new(PolicyBroker::new(
                        network_policy,
                        CookieBroker::new(transport, transport_cookies),
                    )))
                })
        })
        .join()
        .unwrap_or(None);
        BrowserContext {
            inner: Arc::new(BrowserContextInner {
                id: context_id,
                config: self.config.clone(),
                broker,
                worker_process: self.worker_process,
                options,
                group: ContextGroup::new(),
                store,
                persistent_profile,
                policy,
                secrets,
                uploads: Arc::new(crate::upload::UploadStore::default()),
                virtual_clipboard: Mutex::new(String::new()),
                downloads: Arc::new(DownloadStore::default()),
                network_replay,
                intent_trace: Mutex::new(Vec::new()),
                domain_decisions: Mutex::new(Vec::new()),
                document_security: Mutex::new(BTreeMap::new()),
                document_referrer_policy: Mutex::new(BTreeMap::new()),
                security_origins: Mutex::new(BTreeMap::new()),
                expected_meta_security: Mutex::new(BTreeMap::new()),
                csp_reporting: Arc::new(Mutex::new(CspReportingState::default())),
                form_submission_grants: Mutex::new(BTreeMap::new()),
                navigation_grants: Mutex::new(BTreeSet::new()),
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
                uploads: Arc::new(crate::upload::UploadStore::default()),
                virtual_clipboard: Mutex::new(String::new()),
                downloads: Arc::new(DownloadStore::default()),
                network_replay: crate::replay::NetworkReplayController::live(),
                intent_trace: Mutex::new(Vec::new()),
                domain_decisions: Mutex::new(Vec::new()),
                document_security: Mutex::new(BTreeMap::new()),
                document_referrer_policy: Mutex::new(BTreeMap::new()),
                security_origins: Mutex::new(BTreeMap::new()),
                expected_meta_security: Mutex::new(BTreeMap::new()),
                csp_reporting: Arc::new(Mutex::new(CspReportingState::default())),
                form_submission_grants: Mutex::new(BTreeMap::new()),
                navigation_grants: Mutex::new(BTreeSet::new()),
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
                uploads: Arc::new(crate::upload::UploadStore::default()),
                virtual_clipboard: Mutex::new(String::new()),
                downloads: Arc::new(DownloadStore::default()),
                network_replay: crate::replay::NetworkReplayController::live(),
                intent_trace: Mutex::new(Vec::new()),
                domain_decisions: Mutex::new(Vec::new()),
                document_security: Mutex::new(BTreeMap::new()),
                document_referrer_policy: Mutex::new(BTreeMap::new()),
                security_origins: Mutex::new(BTreeMap::new()),
                expected_meta_security: Mutex::new(BTreeMap::new()),
                csp_reporting: Arc::new(Mutex::new(CspReportingState::default())),
                form_submission_grants: Mutex::new(BTreeMap::new()),
                navigation_grants: Mutex::new(BTreeSet::new()),
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

    pub fn secret_handle(&self, origin: &str, label: &str) -> Result<SecretHandle, RuntimeError> {
        self.inner
            .secrets
            .handle_for_label(origin, label)
            .map_err(|error| RuntimeError::Secret(error.to_string()))
    }

    /// Import bytes into the parent-owned upload store and return an opaque
    /// handle safe to place in agent contracts and traces.
    pub fn import_upload(
        &self,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> crate::upload::UploadHandle {
        self.inner.uploads.insert(crate::upload::UploadFile {
            filename: filename.into(),
            media_type: media_type.into(),
            bytes,
        })
    }

    pub fn import_upload_as(
        &self,
        label: impl Into<String>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> crate::upload::UploadHandle {
        self.inner.uploads.insert_named(
            label.into(),
            crate::upload::UploadFile {
                filename: filename.into(),
                media_type: media_type.into(),
                bytes,
            },
        )
    }

    /// Read a host path in the privileged parent. The path is never sent to a
    /// worker or serialized into an intent.
    pub fn import_upload_path(
        &self,
        path: impl AsRef<std::path::Path>,
        media_type: Option<&str>,
    ) -> Result<crate::upload::UploadHandle, RuntimeError> {
        self.inner
            .uploads
            .import_path(path.as_ref(), media_type)
            .map_err(|error| RuntimeError::Upload(error.to_string()))
    }

    pub fn upload_descriptor(
        &self,
        handle: crate::upload::UploadHandle,
    ) -> Result<crate::upload::UploadDescriptor, RuntimeError> {
        self.inner
            .uploads
            .descriptor(handle)
            .map_err(|error| RuntimeError::Upload(error.to_string()))
    }

    pub fn upload_handle(&self, label: &str) -> Result<crate::upload::UploadHandle, RuntimeError> {
        self.inner
            .uploads
            .handle_for_label(label)
            .map_err(|error| RuntimeError::Upload(error.to_string()))
    }

    pub fn revoke_upload(&self, handle: crate::upload::UploadHandle) -> bool {
        self.inner.uploads.revoke(handle)
    }

    /// Read the text-only clipboard owned by this browser context. No host
    /// clipboard API is called and values are not shared across contexts.
    pub fn read_clipboard_text(
        &self,
        page_id: PageId,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<String, RuntimeError> {
        self.require_policy(
            PolicyRequest {
                action: ConsequentialAction::Clipboard,
                source_origin: self.page_origin(page_id)?,
                target_origin: None,
                detail: Some("read context virtual clipboard".into()),
            },
            confirmation,
        )?;
        Ok(self
            .inner
            .virtual_clipboard
            .lock()
            .expect("virtual clipboard lock")
            .clone())
    }

    /// Replace the context-owned, text-only virtual clipboard.
    pub fn write_clipboard_text(
        &self,
        page_id: PageId,
        value: impl Into<String>,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<(), RuntimeError> {
        self.require_policy(
            PolicyRequest {
                action: ConsequentialAction::Clipboard,
                source_origin: self.page_origin(page_id)?,
                target_origin: None,
                detail: Some("write context virtual clipboard".into()),
            },
            confirmation,
        )?;
        *self
            .inner
            .virtual_clipboard
            .lock()
            .expect("virtual clipboard lock") = value.into();
        Ok(())
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
        if let Some(page_id) = self.pages().into_iter().find_map(|page| {
            same_document_url(&page.page.url, &source_url).then_some(page.page.page_id)
        }) && self
            .csp_sandbox(page_id)?
            .is_some_and(|sandbox| !sandbox.allow_downloads)
        {
            return Err(RuntimeError::ContextService(
                "Content-Security-Policy sandbox denied download".into(),
            ));
        }
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

    #[cfg(unix)]
    pub(crate) fn grant_form_submission(&self, page_id: PageId) {
        *self
            .inner
            .form_submission_grants
            .lock()
            .expect("form submission grant lock")
            .entry(page_id)
            .or_default() += 1;
    }

    #[cfg(unix)]
    pub(crate) fn consume_form_submission_grant(&self, page_id: PageId) -> bool {
        let mut grants = self
            .inner
            .form_submission_grants
            .lock()
            .expect("form submission grant lock");
        let Some(remaining) = grants.get_mut(&page_id) else {
            return false;
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining == 0 {
            grants.remove(&page_id);
        }
        true
    }

    #[cfg(unix)]
    pub(crate) fn clear_form_submission_grant(&self, page_id: PageId) {
        self.inner
            .form_submission_grants
            .lock()
            .expect("form submission grant lock")
            .remove(&page_id);
    }

    #[cfg(unix)]
    pub(crate) fn grant_navigation_chain(&self, page_id: PageId) {
        self.inner
            .navigation_grants
            .lock()
            .expect("navigation grant lock")
            .insert(page_id);
    }

    #[cfg(unix)]
    pub(crate) fn navigation_chain_is_preconfirmed(&self, page_id: PageId) -> bool {
        self.inner
            .navigation_grants
            .lock()
            .expect("navigation grant lock")
            .contains(&page_id)
    }

    #[cfg(unix)]
    pub(crate) fn clear_navigation_chain_grant(&self, page_id: PageId) {
        self.inner
            .navigation_grants
            .lock()
            .expect("navigation grant lock")
            .remove(&page_id);
    }

    #[cfg(unix)]
    pub(crate) fn require_preconfirmed_policy(
        &self,
        request: PolicyRequest,
    ) -> Result<(), RuntimeError> {
        match self.inner.policy.authorize_preconfirmed(request.clone()) {
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
            .and_then(|cookies| cookies.document_header(url))
    }

    #[cfg(unix)]
    pub(crate) fn cookie_header_for_request(
        &self,
        page_id: PageId,
        url: &Url,
        method: &str,
        kind: crate::cookies::CookieRequestKind,
    ) -> Result<Option<String>, RuntimeError> {
        let top_level_url = self.page_url(page_id)?;
        Ok(self.inner.parent_cookies.as_ref().and_then(|cookies| {
            cookies
                .header_for_method(
                    url,
                    method,
                    &crate::cookies::CookieRequestContext {
                        kind,
                        initiator_url: Some(top_level_url.clone()),
                        top_level_url: Some(top_level_url),
                        ..Default::default()
                    },
                )
                .0
        }))
    }

    #[cfg(unix)]
    pub(crate) fn document_cookie(&self, page_id: PageId) -> Result<String, RuntimeError> {
        if self.is_opaque_origin(page_id)? {
            return Err(RuntimeError::ContextService(
                "SecurityError: sandboxed opaque origins cannot access document.cookie".into(),
            ));
        }
        let url = Url::parse(&self.page_url(page_id)?)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        Ok(self
            .inner
            .parent_cookies
            .as_ref()
            .and_then(|cookies| cookies.document_header(&url))
            .unwrap_or_default())
    }

    #[cfg(unix)]
    pub(crate) fn set_document_cookie(
        &self,
        page_id: PageId,
        value: String,
    ) -> Result<(), RuntimeError> {
        if self.is_opaque_origin(page_id)? {
            return Err(RuntimeError::ContextService(
                "SecurityError: sandboxed opaque origins cannot access document.cookie".into(),
            ));
        }
        let url = Url::parse(&self.page_url(page_id)?)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        self.inner
            .parent_cookies
            .as_ref()
            .ok_or_else(|| RuntimeError::ContextService("cookie store is unavailable".into()))?
            .store_document_cookie(&url, value)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))
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

    /// Wake the target page actor after a parent-owned context event arrives.
    /// Browser event loops do not require an API call on the receiving page to
    /// deliver `postMessage`; without this bounded wakeup an idle popup can
    /// retain a queued event forever.
    #[cfg(unix)]
    pub(crate) fn wake_context_events(&self, page_id: PageId) {
        let Some(page) = self.inner.group.handle(page_id) else {
            return;
        };
        if let Some(sender) = page.sender.as_ref() {
            // Context events are durable in the parent journal. A full actor
            // queue therefore needs no unbounded retry: commands already ahead
            // of this wake run scheduler checkpoints, and a later page command
            // will observe any event not covered by this notification.
            let _ = sender.try_send(PageCommand::PollContextEvents);
            return;
        }

        let Some(worker) = page.worker.as_ref().cloned() else {
            return;
        };
        let became_leader = worker
            .context_wakeups
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                Some(pending.saturating_add(1).min(32))
            })
            .is_ok_and(|previous| previous == 0);
        if !became_leader {
            return;
        }
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                worker.context_wakeups.store(0, Ordering::Release);
                return;
            };
            loop {
                // Internal event-loop progress is not raw user inspection and
                // must remain available even when `evaluate` is policy-denied.
                let _ =
                    runtime.block_on(page.worker_request(crate::worker::WorkerRequest::Evaluate {
                        source: "__oryn_drain(false);''".into(),
                    }));
                // Coalesce events which arrived while the worker was draining.
                // The zero transition lets a racing event elect a new leader,
                // so no wakeup can be stranded between iterations.
                let observed = worker.context_wakeups.swap(0, Ordering::AcqRel);
                if observed <= 1 {
                    break;
                }
                if worker
                    .context_wakeups
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    break;
                }
            }
        });
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
                ContextEvent::CspViolation { target_page_id, .. } => *target_page_id == page_id,
                ContextEvent::CspReport { target_page_id, .. } => *target_page_id == page_id,
                ContextEvent::PageLifecycle { .. } => true,
            })
            .take(max_events)
            .cloned()
            .collect())
    }

    pub(crate) fn page_origin(&self, page_id: PageId) -> Result<String, RuntimeError> {
        self.security_origin(page_id)
            .map(|origin| origin.serialize())
    }

    fn security_origin(&self, page_id: PageId) -> Result<SecurityOrigin, RuntimeError> {
        self.inner
            .security_origins
            .lock()
            .map_err(|_| RuntimeError::ContextService("security origin lock poisoned".into()))?
            .get(&page_id)
            .cloned()
            .ok_or(RuntimeError::PageNotFound(page_id))
    }

    pub(crate) fn is_opaque_origin(&self, page_id: PageId) -> Result<bool, RuntimeError> {
        self.security_origin(page_id)
            .map(|origin| origin.is_opaque())
    }

    pub(crate) fn page_url(&self, page_id: PageId) -> Result<String, RuntimeError> {
        self.pages()
            .into_iter()
            .find(|page| page.page.page_id == page_id)
            .map(|descriptor| descriptor.page.url)
            .ok_or(RuntimeError::PageNotFound(page_id))
    }

    pub(crate) fn page_referrer_policy(
        &self,
        page_id: PageId,
    ) -> Result<crate::referrer::ReferrerPolicy, RuntimeError> {
        if !self.pages().iter().any(|page| page.page.page_id == page_id) {
            return Err(RuntimeError::PageNotFound(page_id));
        }
        Ok(self
            .inner
            .document_referrer_policy
            .lock()
            .map_err(|_| RuntimeError::ContextService("referrer policy lock poisoned".into()))?
            .get(&page_id)
            .copied()
            .unwrap_or_default())
    }

    pub(crate) fn install_meta_referrer(
        &self,
        page_id: PageId,
        serialized: &str,
    ) -> Result<(), RuntimeError> {
        if !self.pages().iter().any(|page| page.page.page_id == page_id) {
            return Err(RuntimeError::PageNotFound(page_id));
        }
        self.inner
            .document_referrer_policy
            .lock()
            .map_err(|_| RuntimeError::ContextService("referrer policy lock poisoned".into()))?
            .insert(
                page_id,
                crate::referrer::ReferrerPolicy::parse_list(serialized),
            );
        Ok(())
    }

    pub(crate) fn install_document_security(
        &self,
        page_id: PageId,
        document_url: String,
        headers: &[(String, String)],
    ) -> Result<(), RuntimeError> {
        if !self.pages().iter().any(|page| page.page.page_id == page_id) {
            return Err(RuntimeError::PageNotFound(page_id));
        }
        let policy = crate::csp::CspPolicySet::from_headers(document_url.clone(), headers);
        let referrer = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
            .map(|(_, value)| crate::referrer::ReferrerPolicy::parse_list(value))
            .next_back()
            .unwrap_or_default();
        let origin = if policy
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_same_origin)
        {
            SecurityOrigin::opaque()
        } else {
            SecurityOrigin::from_url(&document_url)
        };
        self.inner
            .security_origins
            .lock()
            .map_err(|_| RuntimeError::ContextService("security origin lock poisoned".into()))?
            .insert(page_id, origin);
        self.inner
            .document_security
            .lock()
            .map_err(|_| RuntimeError::ContextService("document security lock poisoned".into()))?
            .insert(page_id, policy);
        self.inner
            .document_referrer_policy
            .lock()
            .map_err(|_| RuntimeError::ContextService("referrer policy lock poisoned".into()))?
            .insert(page_id, referrer);
        Ok(())
    }

    pub(crate) fn prepare_document_security(
        &self,
        page_id: PageId,
        document_url: String,
        headers: &[(String, String)],
        html: &str,
    ) -> Result<(), RuntimeError> {
        self.install_document_security(page_id, document_url, headers)?;
        self.inner
            .expected_meta_security
            .lock()
            .map_err(|_| RuntimeError::ContextService("meta security lock poisoned".into()))?
            .insert(
                page_id,
                crate::csp::CspPolicySet::meta_policies_from_html(html).into(),
            );
        Ok(())
    }

    #[cfg(not(test))]
    pub(crate) fn verify_document_security(
        &self,
        page_id: PageId,
        document_url: &str,
        headers: &[(String, String)],
    ) -> Result<(), RuntimeError> {
        let expected = crate::csp::CspPolicySet::from_headers(document_url, headers);
        let policies =
            self.inner.document_security.lock().map_err(|_| {
                RuntimeError::ContextService("document security lock poisoned".into())
            })?;
        let installed = policies.get(&page_id).ok_or_else(|| {
            RuntimeError::ProtocolViolation(
                "worker attempted to confirm document security before the parent installed it"
                    .into(),
            )
        })?;
        if !installed.equivalent_policy(&expected) {
            return Err(RuntimeError::ProtocolViolation(format!(
                "worker document-security snapshot did not match parent response headers: parent={installed:?}, worker={expected:?}"
            )));
        }
        Ok(())
    }

    pub(crate) fn authorize_csp(
        &self,
        page_id: PageId,
        request: &crate::csp::CspRequest,
    ) -> Result<crate::csp::CspDecision, RuntimeError> {
        let pending = self
            .inner
            .expected_meta_security
            .lock()
            .map_err(|_| RuntimeError::ContextService("meta security lock poisoned".into()))?
            .remove(&page_id)
            .unwrap_or_default();
        let mut policies =
            self.inner.document_security.lock().map_err(|_| {
                RuntimeError::ContextService("document security lock poisoned".into())
            })?;
        if let Some(policy) = policies.get_mut(&page_id) {
            for serialized in pending {
                policy.install_meta(&serialized);
            }
        }
        Ok(policies.get(&page_id).map_or(
            crate::csp::CspDecision {
                allowed: true,
                violations: Vec::new(),
            },
            |policy| policy.authorize(request),
        ))
    }

    pub(crate) fn handle_csp_violations(
        &self,
        page_id: PageId,
        violations: Vec<crate::csp::CspViolation>,
    ) -> Result<(), RuntimeError> {
        for violation in violations {
            #[cfg(unix)]
            self.push_context_event(ContextEvent::CspViolation {
                target_page_id: page_id,
                violation: violation.clone(),
            });
            let targets = self
                .inner
                .document_security
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("document security lock poisoned".into())
                })?
                .get(&page_id)
                .map(|policy| policy.report_targets(&violation))
                .unwrap_or_default();
            for target in targets {
                self.queue_csp_report(page_id, target, violation.clone())?;
            }
        }
        Ok(())
    }

    fn queue_csp_report(
        &self,
        page_id: PageId,
        target: crate::csp::CspReportTarget,
        violation: crate::csp::CspViolation,
    ) -> Result<(), RuntimeError> {
        let Some(broker) = self.inner.broker.clone() else {
            return Ok(());
        };
        let now = Instant::now();
        {
            let mut state =
                self.inner.csp_reporting.lock().map_err(|_| {
                    RuntimeError::ContextService("CSP reporting lock poisoned".into())
                })?;
            if state.pending >= 128 {
                #[cfg(unix)]
                self.push_context_event(ContextEvent::CspReport {
                    target_page_id: page_id,
                    endpoint: target.url,
                    format: target.format,
                    outcome: "queue_limited".into(),
                });
                return Ok(());
            }
            let deliveries = state.deliveries.entry(target.url.clone()).or_default();
            while deliveries
                .front()
                .is_some_and(|instant| now.duration_since(*instant) >= Duration::from_secs(60))
            {
                deliveries.pop_front();
            }
            if deliveries.len() >= 10 {
                #[cfg(unix)]
                self.push_context_event(ContextEvent::CspReport {
                    target_page_id: page_id,
                    endpoint: target.url,
                    format: target.format,
                    outcome: "rate_limited".into(),
                });
                return Ok(());
            }
            deliveries.push_back(now);
            state.pending += 1;
        }
        let reporting = self.inner.csp_reporting.clone();
        let top_level_url = self.page_url(page_id)?;
        let context = self.clone();
        std::thread::spawn(move || {
            let (content_type, body) = serialize_csp_report(target.format, &violation);
            let endpoint = target.url.clone();
            let outcome = if body.len() > 32 * 1024 {
                "size_limited".into()
            } else {
                match collect_operation_bounded(
                    &broker,
                    NetworkRequest {
                        request_id: NEXT_CSP_REPORT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
                        method: "POST".into(),
                        url: endpoint.clone(),
                        headers: vec![
                            ("content-type".into(), content_type.into()),
                            ("cache-control".into(), "no-store".into()),
                        ],
                        body,
                        credentials: false,
                        security: crate::cookies::CookieRequestContext {
                            credentials: crate::cookies::CookieCredentials::Omit,
                            kind: crate::cookies::CookieRequestKind::Other,
                            initiator_url: Some(violation.document_uri),
                            top_level_url: Some(top_level_url),
                            ..Default::default()
                        },
                        resolved_addrs: Vec::new(),
                    },
                    Duration::from_secs(10),
                    64 * 1024,
                ) {
                    Ok(response) if (200..300).contains(&response.status) => "delivered".into(),
                    Ok(response) if response.status == 410 => "endpoint_removed".into(),
                    Ok(response) => format!("http_{}", response.status),
                    Err(error) => format!("failed:{error}"),
                }
            };
            #[cfg(unix)]
            context.push_context_event(ContextEvent::CspReport {
                target_page_id: page_id,
                endpoint,
                format: target.format,
                outcome,
            });
            if let Ok(mut state) = reporting.lock() {
                state.pending = state.pending.saturating_sub(1);
            }
        });
        Ok(())
    }

    pub(crate) fn upgrade_csp_url(
        &self,
        page_id: PageId,
        url: &Url,
    ) -> Result<Option<Url>, RuntimeError> {
        let policies =
            self.inner.document_security.lock().map_err(|_| {
                RuntimeError::ContextService("document security lock poisoned".into())
            })?;
        Ok(policies
            .get(&page_id)
            .and_then(|policy| policy.upgraded_url(url)))
    }

    pub(crate) fn csp_sandbox(
        &self,
        page_id: PageId,
    ) -> Result<Option<crate::csp::SandboxFlags>, RuntimeError> {
        let policies =
            self.inner.document_security.lock().map_err(|_| {
                RuntimeError::ContextService("document security lock poisoned".into())
            })?;
        Ok(policies
            .get(&page_id)
            .and_then(crate::csp::CspPolicySet::sandbox))
    }

    pub(crate) fn install_meta_security(
        &self,
        page_id: PageId,
        serialized: &str,
    ) -> Result<(), RuntimeError> {
        #[cfg(not(test))]
        {
            let mut expected =
                self.inner.expected_meta_security.lock().map_err(|_| {
                    RuntimeError::ContextService("meta security lock poisoned".into())
                })?;
            let next = expected
                .get_mut(&page_id)
                .and_then(std::collections::VecDeque::pop_front)
                .ok_or_else(|| {
                    RuntimeError::ProtocolViolation(
                        "worker reported a CSP meta policy not present in the parent response"
                            .into(),
                    )
                })?;
            if next != serialized {
                return Err(RuntimeError::ProtocolViolation(
                    "worker CSP meta policy did not match parent response order".into(),
                ));
            }
        }
        let mut policies =
            self.inner.document_security.lock().map_err(|_| {
                RuntimeError::ContextService("document security lock poisoned".into())
            })?;
        let policy = policies.get_mut(&page_id).ok_or_else(|| {
            RuntimeError::ContextService("document policy is not installed".into())
        })?;
        policy.install_meta(serialized);
        Ok(())
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
        let inherited_policy = opener_page_id.and_then(|opener| {
            self.inner
                .document_security
                .lock()
                .ok()?
                .get(&opener)
                .cloned()
                .filter(|policy| {
                    policy
                        .sandbox()
                        .is_some_and(|sandbox| !sandbox.allow_popups_to_escape_sandbox)
                })
        });
        let initial_origin = if inherited_policy
            .as_ref()
            .and_then(crate::csp::CspPolicySet::sandbox)
            .is_some_and(|sandbox| !sandbox.allow_same_origin)
        {
            SecurityOrigin::opaque()
        } else {
            opener_page_id
                .and_then(|opener| {
                    self.inner
                        .security_origins
                        .lock()
                        .ok()?
                        .get(&opener)
                        .cloned()
                })
                .unwrap_or_else(SecurityOrigin::opaque)
        };
        self.inner
            .security_origins
            .lock()
            .expect("security origin lock")
            .insert(page_id, initial_origin);
        if let Some(policy) = inherited_policy {
            self.inner
                .document_security
                .lock()
                .expect("document security lock")
                .insert(page_id, policy);
        }
        #[cfg(unix)]
        if self.inner.worker_process {
            let handle = PageHandle {
                id: page_id.0,
                opener_page_id,
                context: Arc::downgrade(&self.inner),
                context_guard: Some(self.inner.clone()),
                sender: None,
                worker: Some(std::sync::Arc::new(WorkerPage {
                    page_id,
                    opener_page_id,
                    context: Arc::downgrade(&self.inner),
                    client: std::sync::Mutex::new(None),
                    config: self.inner.config.clone(),
                    options: self.inner.options.clone(),
                    pending: AtomicUsize::new(0),
                    context_wakeups: AtomicUsize::new(0),
                    process_id: AtomicU32::new(0),
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
        let context_owner = BrowserContext {
            inner: self.inner.clone(),
        };
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
                        state.set_challenge_assessment(assess_challenge(200, &[], &html));
                        state.csp = crate::csp::CspPolicySet::new(url.clone());
                        if let Err(error) =
                            context_owner.install_document_security(state.page_id, url.clone(), &[])
                        {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                        #[cfg(all(unix, feature = "v8-host"))]
                        if let Some(client) = &state.context_client
                            && let Err(error) = client.request(
                                crate::context_service::ContextServiceRequest::InstallDocumentSecurity {
                                    page_id: state.page_id,
                                    document_url: url.clone(),
                                    headers: Vec::new(),
                                },
                            )
                        {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                        if let Err(error) = state.load_document(
                            url.clone(),
                            &html,
                            None,
                            None,
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
                    PageCommand::PollContextEvents => {
                        let _ = state.evaluate("__oryn_drain(false);''");
                    }
                    PageCommand::Capabilities { reply } => {
                        let _ = reply.send(state.capabilities());
                    }
                    PageCommand::ChallengeAssessment { reply } => {
                        let _ = reply.send(state.last_challenge.clone());
                    }
                    PageCommand::ResolveSelector {
                        selector,
                        action,
                        reply,
                    } => {
                        let _ = reply.send(state.resolve_selector(&selector, action));
                    }
                    PageCommand::ResolveSemantic {
                        target,
                        action,
                        reply,
                    } => {
                        let _ = reply.send(state.resolve_semantic(&target, action));
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
                    PageCommand::ObserveSemantic { profile, reply } => {
                        let _ = reply.send(state.semantic_observation(profile));
                    }
                    PageCommand::ObserveSemanticV6 { options, reply } => {
                        let _ = reply.send(state.semantic_observation_v6(options));
                    }
                    PageCommand::ComputedStyle { target, reply } => {
                        let _ = reply.send(state.computed_style(target));
                    }
                    PageCommand::BoxModel { target, reply } => {
                        let _ = reply.send(state.box_model(target));
                    }
                    PageCommand::PaintSnapshot { reply } => {
                        #[cfg(feature = "v8-host")]
                        match state.paint_snapshot_plan() {
                            Ok(plan) => {
                                // Image I/O is parent-brokered and independent
                                // of DOM/V8 ownership. Complete it away from the
                                // page actor so observe/action/close commands do
                                // not queue behind network latency.
                                std::thread::spawn(move || {
                                    let _ = reply.send(complete_paint_snapshot(plan));
                                });
                            }
                            Err(error) => {
                                let _ = reply.send(Err(error));
                            }
                        }
                        #[cfg(not(feature = "v8-host"))]
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
                    PageCommand::ObserveSemanticDelta { from, reply } => {
                        let _ = reply.send(state.semantic_delta_from(from));
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
                    PageCommand::Close => break,
                }
            }
        });
        let handle = PageHandle {
            id: page_id.0,
            opener_page_id,
            context: Arc::downgrade(&self.inner),
            context_guard: Some(self.inner.clone()),
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
    pub(crate) fn owning_context(&self) -> Option<BrowserContext> {
        self.context.upgrade().map(|inner| BrowserContext { inner })
    }

    fn detached_clone(&self) -> Self {
        Self {
            id: self.id,
            opener_page_id: self.opener_page_id,
            context: self.context.clone(),
            context_guard: None,
            sender: self.sender.clone(),
            #[cfg(unix)]
            worker: self.worker.clone(),
        }
    }

    fn retained_clone(&self) -> Self {
        let mut handle = self.clone();
        if handle.context_guard.is_none() {
            handle.context_guard = handle.context.upgrade();
        }
        handle
    }

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
    fn interrupt_inflight_worker(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.interrupt_inflight())
    }

    #[cfg(unix)]
    async fn worker_request(
        &self,
        request: crate::worker::WorkerRequest,
    ) -> Option<Result<crate::worker::WorkerResponse, RuntimeError>> {
        let worker = self.worker.clone()?;
        let result = tokio::task::spawn_blocking(move || worker.request(request))
            .await
            .unwrap_or_else(|error| {
                Err(RuntimeError::WorkerCrashed(format!(
                    "worker request task failed: {error}"
                )))
            });
        if let Err(RuntimeError::WorkerCrashed(detail) | RuntimeError::ProtocolViolation(detail)) =
            &result
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
        Some(result)
    }

    pub async fn load_html(
        &self,
        url: impl Into<String>,
        html: impl Into<String>,
    ) -> Result<Revision, RuntimeError> {
        let url = url.into();
        let html = html.into();
        #[cfg(unix)]
        self.interrupt_inflight_worker();
        // The privileged parent establishes the initial document security
        // state before an untrusted worker is allowed to confirm the snapshot.
        // This keeps local HTML on the same fail-closed ordering as a
        // brokered navigation without granting the worker policy authority.
        if let Some(context) = self.context.upgrade() {
            BrowserContext { inner: context }.prepare_document_security(
                self.page_id(),
                url.clone(),
                &[],
                &html,
            )?;
        }
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::LoadHtml {
                url: url.clone(),
                html: html.clone(),
            })
            .await
        {
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
        #[cfg(unix)]
        let mut granted_context = None;
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
                #[cfg(unix)]
                {
                    context.grant_navigation_chain(self.page_id());
                    granted_context = Some(context.clone());
                }
            }
        }
        #[cfg(unix)]
        self.interrupt_inflight_worker();
        let outcome = async {
            #[cfg(unix)]
            if let Some(response) = self
                .worker_request(crate::worker::WorkerRequest::Goto { url: url.clone() })
                .await
            {
                return worker_navigation(response?);
            }
            let (reply, response) = oneshot::channel();
            self.local_sender()?
                .send(PageCommand::Goto { url, reply })
                .await
                .map_err(|_| RuntimeError::Closed)?;
            response.await.map_err(|_| RuntimeError::Closed)?
        }
        .await;
        #[cfg(unix)]
        if let Some(context) = granted_context {
            context.clear_navigation_chain_grant(self.page_id());
        }
        if let Ok(navigation) = &outcome {
            self.record_navigation(navigation);
        }
        outcome
    }

    pub async fn back(&self) -> Result<NavigationResult, RuntimeError> {
        #[cfg(unix)]
        self.interrupt_inflight_worker();
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Back)
            .await
        {
            let navigation = worker_navigation(response?)?;
            self.record_navigation(&navigation);
            return Ok(navigation);
        }
        self.navigation_command(|reply| PageCommand::Back { reply })
            .await
    }

    pub async fn forward(&self) -> Result<NavigationResult, RuntimeError> {
        #[cfg(unix)]
        self.interrupt_inflight_worker();
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Forward)
            .await
        {
            let navigation = worker_navigation(response?)?;
            self.record_navigation(&navigation);
            return Ok(navigation);
        }
        self.navigation_command(|reply| PageCommand::Forward { reply })
            .await
    }

    pub async fn refresh(&self) -> Result<NavigationResult, RuntimeError> {
        #[cfg(unix)]
        self.interrupt_inflight_worker();
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Refresh)
            .await
        {
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
            let keep_opaque = context
                .document_security
                .lock()
                .ok()
                .and_then(|policies| policies.get(&navigation.page_id).cloned())
                .and_then(|policy| policy.sandbox())
                .is_some_and(|sandbox| !sandbox.allow_same_origin);
            if let Ok(mut origins) = context.security_origins.lock() {
                if keep_opaque {
                    if !origins
                        .get(&navigation.page_id)
                        .is_some_and(SecurityOrigin::is_opaque)
                    {
                        origins.insert(navigation.page_id, SecurityOrigin::opaque());
                    }
                } else {
                    origins.insert(
                        navigation.page_id,
                        SecurityOrigin::from_url(&navigation.final_url),
                    );
                }
            }
        }
    }

    pub async fn wait_for(&self, predicate: WaitPredicate) -> Result<(), RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::WaitFor {
                predicate: predicate.clone(),
            })
            .await
        {
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
        self.audit_raw_inspection("evaluate", Some(&source))?;
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Evaluate {
                source: source.clone(),
            })
            .await
        {
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Capabilities)
            .await
        {
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

    /// Return the latest deterministic challenge assessment for this page.
    /// Only `Confirmed` assessments are eligible to influence domain routing.
    pub async fn challenge_assessment(&self) -> Result<ChallengeAssessment, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ChallengeAssessment)
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::ChallengeAssessment { assessment } => Ok(assessment),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ChallengeAssessment { reply })
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
        self.audit_raw_inspection("selector", Some(&selector))?;
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ResolveSelector {
                selector: selector.clone(),
                action,
            })
            .await
        {
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

    pub async fn resolve_semantic(
        &self,
        target: oryn_common::agent::Target,
        action: SemanticAction,
    ) -> Result<oryn_common::v6::ResolutionResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ResolveSemantic {
                target: target.clone(),
                action,
            })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::Resolution { result } => Ok(result),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ResolveSemantic {
                target,
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
        self.audit_raw_inspection("extract", selector.as_deref())?;
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Extract {
                kind,
                selector: selector.clone(),
            })
            .await
        {
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Observe {
                profile: Some(profile),
            })
            .await
        {
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ObserveWith { options })
            .await
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

    /// Observe first-class semantic structures and supported accessibility
    /// rules using additive contract v5. Contracts v2-v4 remain unchanged.
    pub async fn observe_semantics(
        &self,
        profile: ProjectionProfile,
    ) -> Result<oryn_common::v5::SemanticObservation, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ObserveSemantic { profile })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::SemanticObservation { observation } => {
                    if let Some(context) = self.context.upgrade() {
                        context.group.update_observation(&observation.observation);
                    }
                    Ok(observation)
                }
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ObserveSemantic { profile, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        let observation = response.await.map_err(|_| RuntimeError::Closed)?;
        if let Some(context) = self.context.upgrade() {
            context.group.update_observation(&observation.observation);
        }
        Ok(observation)
    }

    /// Observe the canonical incremental semantic graph through contract v6.
    pub async fn observe_semantics_v6(
        &self,
        options: oryn_common::v6::SemanticObservationOptions,
    ) -> Result<oryn_common::v6::SemanticObservation, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ObserveSemanticV6 {
                options: options.clone(),
            })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::SemanticObservationV6 { observation } => {
                    Ok(observation)
                }
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ObserveSemanticV6 { options, reply })
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ComputedStyle { target })
            .await
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::BoxModel { target })
            .await
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::PaintSnapshot)
            .await
        {
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::SetViewport { viewport })
            .await
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::SetMedia { media })
            .await
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Scroll { request })
            .await
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ScrollIntoView { target })
            .await
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ObserveDelta { from })
            .await
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

    pub async fn observe_semantic_delta(
        &self,
        from: Revision,
    ) -> Result<oryn_common::v6::SemanticDelta, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::ObserveSemanticDelta { from })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::SemanticDelta { delta } => Ok(delta),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ObserveSemanticDelta { from, reply })
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
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Trace)
            .await
        {
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

    fn audit_raw_inspection(
        &self,
        operation: &str,
        page_controlled_value: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let Some(context) = self.context.upgrade() else {
            return Err(RuntimeError::Closed);
        };
        let context = BrowserContext { inner: context };
        let detail = page_controlled_value.map_or_else(
            || operation.to_string(),
            |value| {
                use sha2::Digest as _;
                let digest = sha2::Sha256::digest(value.as_bytes());
                format!("{operation}:sha256:{digest:x}")
            },
        );
        context.require_policy(
            PolicyRequest {
                action: ConsequentialAction::RawPageInspection,
                source_origin: context
                    .page_origin(self.page_id())
                    .unwrap_or_else(|_| "null".into()),
                target_origin: None,
                detail: Some(detail),
            },
            None,
        )
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
            #[cfg(unix)]
            context.grant_form_submission(self.page_id());
        }
        let clear_form_grant = action.action == SemanticAction::Submit;
        let outcome = self.execute_after_authorization(action).await;
        #[cfg(unix)]
        if clear_form_grant && let Some(context) = self.context.upgrade() {
            BrowserContext { inner: context }.clear_form_submission_grant(self.page_id());
        }
        outcome
    }

    async fn execute_after_authorization(
        &self,
        action: NativeAction,
    ) -> Result<ActionResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Execute {
                action: crate::worker::NativeActionRequest {
                    target: action.target,
                    action: action.action,
                    value: action.value.clone(),
                    force: false,
                    sensitive: false,
                },
            })
            .await
        {
            let outcome = match response? {
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
            return outcome;
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
        self.execute_sensitive(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some(value),
        })
        .await
    }

    /// Resolve an origin-scoped parent-owned secret by label and fill it
    /// without exposing the value to OIL, JSON, Python, MCP or trace records.
    pub async fn fill_secret_label(
        &self,
        target: SemanticRef,
        label: &str,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<ActionResult, RuntimeError> {
        let context = self.context.upgrade().ok_or(RuntimeError::Closed)?;
        let context = BrowserContext { inner: context };
        let origin = context.page_origin(self.page_id())?;
        let handle = context.secret_handle(&origin, label)?;
        self.fill_secret(target, handle, confirmation).await
    }

    /// Select parent-imported files for an input. Opaque handles and host
    /// paths remain in the parent; the worker receives only selected payloads.
    pub async fn set_files(
        &self,
        target: SemanticRef,
        handles: &[crate::upload::UploadHandle],
        confirmation: Option<ConfirmationToken>,
    ) -> Result<ActionResult, RuntimeError> {
        let context = self.context.upgrade().ok_or(RuntimeError::Closed)?;
        let context = BrowserContext { inner: context };
        context.require_policy(
            PolicyRequest {
                action: ConsequentialAction::FileUpload,
                source_origin: context.page_origin(self.page_id())?,
                target_origin: None,
                detail: Some(format!("select {} parent-owned upload(s)", handles.len())),
            },
            confirmation,
        )?;
        if handles.is_empty() {
            return Err(RuntimeError::Upload(
                "at least one upload handle is required".into(),
            ));
        }
        let files = handles
            .iter()
            .map(|handle| context.inner.uploads.materialize(*handle))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RuntimeError::Upload(error.to_string()))?;
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::SetFiles {
                target,
                files: files.clone(),
            })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::Action { result } => Ok(result),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::SetFiles {
                target,
                files,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub(crate) async fn set_files_materialized(
        &self,
        target: SemanticRef,
        files: Vec<crate::upload::UploadFile>,
    ) -> Result<ActionResult, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::SetFiles {
                target,
                files,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub(crate) async fn execute_sensitive(
        &self,
        action: NativeAction,
    ) -> Result<ActionResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Execute {
                action: crate::worker::NativeActionRequest {
                    target: action.target,
                    action: action.action,
                    value: action.value.clone(),
                    force: false,
                    sensitive: true,
                },
            })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::Action { result } => Ok(result),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ExecuteSensitive { action, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub(crate) async fn execute_forced(
        &self,
        action: NativeAction,
    ) -> Result<ActionResult, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Execute {
                action: crate::worker::NativeActionRequest {
                    target: action.target,
                    action: action.action,
                    value: action.value.clone(),
                    force: true,
                    sensitive: false,
                },
            })
            .await
        {
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
        if self.interrupt_inflight_worker() {
            if let Some(context) = self.context.upgrade() {
                context.group.close(self.page_id());
                BrowserContext { inner: context }.push_context_event(ContextEvent::PageLifecycle {
                    event: PageLifecycleEvent::Closed {
                        page_id: self.page_id(),
                    },
                });
            }
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::Close)
            .await
        {
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
    #[error("semantic target is ambiguous: {candidates:?}")]
    AmbiguousTarget { candidates: Vec<SemanticRef> },
    #[error("{action:?} is not supported by this element")]
    UnsupportedAction { action: SemanticAction },
    #[error("semantic target is disabled")]
    Disabled,
    #[error("revision {0:?} is not available for delta observation")]
    UnknownRevision(Revision),
    #[error("native script realm failed: {0}")]
    Script(String),
    #[error("typed agent intent failed: {0}")]
    Intent(String),
    #[error("semantic state failed: {0}")]
    Semantic(String),
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
    #[error("upload handle failed: {0}")]
    Upload(String),
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

fn target_requires_geometry(target: &oryn_common::agent::Target) -> bool {
    use oryn_common::agent::Target;
    match target {
        Target::Near { .. } => true,
        Target::Inside { target, container } => {
            target_requires_geometry(target) || target_requires_geometry(container)
        }
        Target::After { target, anchor } | Target::Before { target, anchor } => {
            target_requires_geometry(target) || target_requires_geometry(anchor)
        }
        Target::Contains { target, content } => {
            target_requires_geometry(target) || target_requires_geometry(content)
        }
        _ => false,
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
    lifecycle: Vec<LifecycleState>,
    network_quiet_since: Option<Instant>,
    document: Option<ParsedDocument>,
    semantic_graph: crate::semantic::SemanticGraph,
    #[cfg(feature = "v8-host")]
    executable: Option<crate::executable::RustExecutableDocument>,
    semantic_deltas: BTreeMap<u64, crate::semantic::SemanticGraphChanges>,
    semantic_v6_deltas: BTreeMap<u64, oryn_common::v6::SemanticGraphChanges>,
    semantic_delta_bytes: usize,
    trace: TraceLog,
    history: Vec<String>,
    history_index: Option<usize>,
    broker: Option<SharedNetworkBroker>,
    #[cfg(all(unix, feature = "v8-host"))]
    context_client: Option<crate::context_service::SharedContextClient>,
    #[cfg(feature = "v8-host")]
    web_fonts: Vec<oryn_layout::WebFont>,
    #[cfg(feature = "v8-host")]
    layout: Option<oryn_layout::LayoutOutput>,
    #[cfg(feature = "v8-host")]
    style_engine: oryn_layout::StyleEngine,
    #[cfg(feature = "v8-host")]
    color_scheme: PreferredColorScheme,
    #[cfg(feature = "v8-host")]
    reduced_motion: ReducedMotion,
    #[cfg(feature = "v8-host")]
    determinism: Option<crate::replay::DeterminismConfig>,
    last_challenge: ChallengeAssessment,
    csp: crate::csp::CspPolicySet,
    referrer_policy: crate::referrer::ReferrerPolicy,
}

#[cfg(feature = "v8-host")]
struct PaintSnapshotPlan {
    snapshot: PaintSnapshot,
    pending: Vec<(Url, Vec<usize>)>,
    broker: Option<SharedNetworkBroker>,
    config: BrowserConfig,
    referrer_policy: crate::referrer::ReferrerPolicy,
    document_url: String,
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
            lifecycle: Vec::new(),
            network_quiet_since: None,
            document: None,
            semantic_graph: crate::semantic::SemanticGraph::new(page_id, document_generation),
            #[cfg(feature = "v8-host")]
            executable: None,
            semantic_deltas: BTreeMap::new(),
            semantic_v6_deltas: BTreeMap::new(),
            semantic_delta_bytes: 0,
            trace: TraceLog::new(page_id),
            history: Vec::new(),
            history_index: None,
            broker,
            #[cfg(all(unix, feature = "v8-host"))]
            context_client,
            #[cfg(feature = "v8-host")]
            web_fonts: Vec::new(),
            #[cfg(feature = "v8-host")]
            layout: None,
            #[cfg(feature = "v8-host")]
            style_engine: oryn_layout::StyleEngine::default(),
            #[cfg(feature = "v8-host")]
            color_scheme: context_options.color_scheme,
            #[cfg(feature = "v8-host")]
            reduced_motion: context_options.reduced_motion,
            #[cfg(feature = "v8-host")]
            determinism: context_options.determinism,
            last_challenge: ChallengeAssessment::default(),
            csp: crate::csp::CspPolicySet::new("about:blank"),
            referrer_policy: crate::referrer::ReferrerPolicy::default(),
        }
    }

    fn set_challenge_assessment(&mut self, assessment: ChallengeAssessment) {
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::ChallengeAssessed {
                disposition: assessment.disposition,
                provider: assessment.provider.clone(),
                signals: assessment
                    .signals
                    .iter()
                    .map(|signal| signal.marker.clone())
                    .collect(),
            },
        );
        self.last_challenge = assessment;
    }

    #[allow(clippy::too_many_arguments)]
    fn load_document(
        &mut self,
        url: String,
        html: &str,
        parsed: Option<ParsedDocument>,
        live_source: Option<Box<dyn Iterator<Item = Result<String, crate::host::HostError>>>>,
        push_history: bool,
        resources: &BTreeMap<String, String>,
        #[cfg(feature = "v8-host")] web_fonts: Vec<oryn_layout::WebFont>,
    ) -> Result<(), RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        let _ = (resources, live_source);
        self.document_generation += 1;
        self.revision += 1;
        self.lifecycle.clear();
        self.lifecycle.push(LifecycleState::Commit);
        self.network_quiet_since = None;
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
            let trusted = self.csp.trusted_types();
            resources.insert(
                "oryn:trusted-types".into(),
                serde_json::to_string(&oryn_js::trusted_types::TrustedTypesConfig {
                    require_script: trusted.require_script,
                    allowed_policy_names: trusted.policy_names,
                    allow_any_policy: trusted.allow_any_policy,
                    allow_duplicates: trusted.allow_duplicates,
                    deny_policy_creation: trusted.deny_policy_creation,
                })
                .map_err(|error| RuntimeError::Script(error.to_string()))?,
            );
            resources.insert(
                "oryn:csp-allow-eval".into(),
                self.csp.allows_eval(false).allowed.to_string(),
            );
            resources.insert(
                "oryn:csp-policy".into(),
                serde_json::to_string(&self.csp)
                    .map_err(|error| RuntimeError::Script(error.to_string()))?,
            );
            resources.insert(
                "oryn:execution-wall-ms".into(),
                self.config.command_timeout.as_millis().to_string(),
            );
            #[cfg(unix)]
            let mut executable = match (self.context_client.clone(), parsed, live_source) {
                (Some(context_client), None, Some(source)) => {
                    crate::executable::RustExecutableDocument::load_stream_with_context_and_limits(
                        source,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                        (context_client, self.page_id, self.opener_page_id),
                    )
                }
                (Some(context_client), Some(parsed), _) => {
                    crate::executable::RustExecutableDocument::load_parsed_with_context_and_limits(
                        parsed,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                        (context_client, self.page_id, self.opener_page_id),
                    )
                }
                (Some(context_client), None, None) => {
                    crate::executable::RustExecutableDocument::load_with_context_and_limits(
                        html,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                        (context_client, self.page_id, self.opener_page_id),
                    )
                }
                (None, Some(parsed), _) => {
                    crate::executable::RustExecutableDocument::load_parsed_with_limits(
                        parsed,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                    )
                }
                (None, None, Some(source)) => {
                    crate::executable::RustExecutableDocument::load_stream_with_limits(
                        source,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                    )
                }
                (None, None, None) => crate::executable::RustExecutableDocument::load_with_limits(
                    html,
                    &resources,
                    self.broker.clone(),
                    Url::parse(&self.url).ok(),
                    self.config.max_v8_heap_bytes,
                ),
            }
            .map_err(map_host_error)?;
            #[cfg(not(unix))]
            let mut executable = match (parsed, live_source) {
                (Some(parsed), _) => {
                    crate::executable::RustExecutableDocument::load_parsed_with_limits(
                        parsed,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                    )
                }
                (None, Some(source)) => {
                    crate::executable::RustExecutableDocument::load_stream_with_limits(
                        source,
                        &resources,
                        self.broker.clone(),
                        Url::parse(&self.url).ok(),
                        self.config.max_v8_heap_bytes,
                    )
                }
                (None, None) => crate::executable::RustExecutableDocument::load_with_limits(
                    html,
                    &resources,
                    self.broker.clone(),
                    Url::parse(&self.url).ok(),
                    self.config.max_v8_heap_bytes,
                ),
            }
            .map_err(map_host_error)?;
            executable.poll_context_events().map_err(map_host_error)?;
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
            if let Some(document) = &self.document {
                self.semantic_graph
                    .reset(&document.dom, self.document_generation);
                self.semantic_graph
                    .set_frame_ids(executable.semantic_frame_ids());
            }
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
            self.document = Some(parsed.unwrap_or_else(|| parse(html)));
            if let Some(document) = &self.document {
                self.semantic_graph
                    .reset(&document.dom, self.document_generation);
            }
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
        for state in [LifecycleState::DomContentLoaded, LifecycleState::Load] {
            self.lifecycle.push(state);
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::LifecycleChanged { state },
            );
        }
        self.network_quiet_since = Some(Instant::now());
        Ok(())
    }

    fn navigate(
        &mut self,
        requested_url: String,
        history_update: HistoryUpdate,
    ) -> Result<NavigationResult, RuntimeError> {
        let mut current = Url::parse(&requested_url)
            .map_err(|error| RuntimeError::Network(format!("invalid URL: {error}")))?;
        let navigation_site = crate::cookies::SchemefulSite::from_url(&current);
        let mut redirect_chain_cross_site = false;
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
            #[cfg(feature = "v8-host")]
            let previous_url = self.url.clone();
            self.url = current.to_string();
            self.revision = self.revision.saturating_add(1);
            if matches!(history_update, HistoryUpdate::Push) {
                if let Some(index) = self.history_index {
                    self.history.truncate(index + 1);
                }
                self.history.push(self.url.clone());
                self.history_index = Some(self.history.len() - 1);
            }
            #[cfg(feature = "v8-host")]
            if let Some(executable) = self.executable.as_mut() {
                let url = serde_json::to_string(&self.url)
                    .map_err(|error| RuntimeError::Script(error.to_string()))?;
                let previous = serde_json::to_string(&previous_url)
                    .map_err(|error| RuntimeError::Script(error.to_string()))?;
                let traversed = matches!(history_update, HistoryUpdate::Preserve);
                executable.execute(&format!(
                    "Object.assign(location,new URL({url}));if(new URL({previous}).hash!==new URL({url}).hash)dispatchEvent(new Event('hashchange'));{}",
                    if traversed {
                        "dispatchEvent(new Event('popstate'))"
                    } else {
                        ""
                    }
                )).map_err(map_host_error)?;
                self.sync_executable_semantics();
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
                challenge: self.last_challenge.clone(),
            });
        }
        if current.as_str() == "about:blank" {
            self.set_challenge_assessment(ChallengeAssessment::default());
            self.load_document(
                current.to_string(),
                "<!doctype html>",
                None,
                None,
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
                lifecycle: self.lifecycle.clone(),
                challenge: self.last_challenge.clone(),
            });
        }
        let (status, response_headers, operation, content_type) = loop {
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
            let mut headers = vec![(
                "accept".into(),
                "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8".into(),
            )];
            if let Ok(source) = Url::parse(&self.url)
                && let Some(referrer) = self.referrer_policy.header(&source, &current)
            {
                headers.push(("referer".into(), referrer));
            }
            let mut operation = broker
                .start_operation(NetworkRequest {
                    request_id: self.revision + redirects as u64 + 1,
                    method: "GET".into(),
                    url: current.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        redirect_chain_cross_site,
                        ..crate::cookies::CookieRequestContext::navigation(Some(self.url.clone()))
                    },
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
            let header_deadline = Instant::now() + self.config.navigation_timeout;
            let header_event = loop {
                match operation.events.try_recv() {
                    Ok(event) => break Some(event),
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break None,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        if Instant::now() >= header_deadline {
                            let _ = broker.cancel_operation(operation.request_id);
                            return Err(RuntimeError::WorkerLimitExceeded {
                                resource: "navigation wall time".into(),
                            });
                        }
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                }
            };
            let expected_request_id = operation.request_id;
            let (response_status, response_headers, _content_length) = match header_event {
                Some(crate::network::NetworkEvent::Headers {
                    request_id,
                    status,
                    headers,
                    content_length,
                }) if request_id == expected_request_id => (status, headers, content_length),
                Some(crate::network::NetworkEvent::Failed { request_id, error })
                    if request_id == expected_request_id =>
                {
                    return Err(RuntimeError::Network(error));
                }
                Some(crate::network::NetworkEvent::Chunk { .. })
                | Some(crate::network::NetworkEvent::Complete { .. }) => {
                    let _ = broker.cancel_operation(expected_request_id);
                    return Err(RuntimeError::ProtocolViolation(
                        "network body arrived before accepted response headers".into(),
                    ));
                }
                Some(_) => {
                    let _ = broker.cancel_operation(expected_request_id);
                    return Err(RuntimeError::ProtocolViolation(
                        "navigation received a stale or unmatched completion".into(),
                    ));
                }
                None => {
                    let _ = broker.cancel_operation(expected_request_id);
                    return Err(RuntimeError::Network(
                        "network operation ended before response headers".into(),
                    ));
                }
            };
            if (300..400).contains(&response_status) {
                let _ = broker.cancel_operation(expected_request_id);
                let location = response_headers
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
                #[cfg(all(unix, feature = "v8-host"))]
                if let Some(client) = &self.context_client {
                    match client.request(
                        crate::context_service::ContextServiceRequest::AuthorizeNavigation {
                            page_id: self.page_id,
                            url: current.to_string(),
                        },
                    )? {
                        crate::context_service::ContextServiceResponse::NavigationAuthorized => {}
                        crate::context_service::ContextServiceResponse::PageDenied { reason } => {
                            self.trace.push(
                                Revision(self.revision),
                                None,
                                TraceEventKind::PolicyDenied {
                                    capability: format!(
                                        "action.cross_origin_navigation.redirect: {reason}"
                                    ),
                                },
                            );
                            return Err(RuntimeError::PolicyDenied {
                                action: ConsequentialAction::CrossOriginNavigation,
                            });
                        }
                        _ => {
                            return Err(RuntimeError::ProtocolViolation(
                                "parent returned the wrong redirect-authorization response".into(),
                            ));
                        }
                    }
                }
                redirect_chain_cross_site |=
                    crate::cookies::SchemefulSite::from_url(&current) != navigation_site;
                continue;
            }
            let content_type = response_headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.clone());
            break (response_status, response_headers, operation, content_type);
        };
        self.csp = crate::csp::CspPolicySet::from_headers(current.to_string(), &response_headers);
        if let Some(value) = response_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
            .map(|(_, value)| value)
        {
            self.referrer_policy = crate::referrer::ReferrerPolicy::parse_list(value);
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if let Some(client) = &self.context_client {
            match client.request(
                crate::context_service::ContextServiceRequest::InstallDocumentSecurity {
                    page_id: self.page_id,
                    document_url: current.to_string(),
                    headers: response_headers.clone(),
                },
            )? {
                crate::context_service::ContextServiceResponse::DocumentSecurityInstalled => {}
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "parent returned the wrong document-security acknowledgement".into(),
                    ));
                }
            }
        }
        if !self.csp.is_empty() {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::CspPolicyInstalled {
                    disposition: "enforce_or_report".into(),
                    source: "response_header".into(),
                },
            );
        }
        #[cfg(feature = "v8-host")]
        {
            let capture = Arc::new(Mutex::new(NavigationBodyCapture::default()));
            let source = NavigationBodyChunks {
                operation,
                broker: self
                    .broker
                    .clone()
                    .ok_or_else(|| RuntimeError::Network("network broker is unavailable".into()))?,
                decoder: oryn_net::IncrementalTextDecoder::new(
                    content_type.as_deref(),
                    oryn_net::TextResourceKind::Html,
                ),
                capture: capture.clone(),
                deadline: Instant::now() + self.config.navigation_timeout,
                finished: false,
            };
            self.load_document(
                current.to_string(),
                "",
                None,
                Some(Box::new(source)),
                matches!(history_update, HistoryUpdate::Push),
                &BTreeMap::new(),
                Vec::new(),
            )?;
            let capture = capture.lock().expect("navigation capture");
            if !capture.completed {
                return Err(RuntimeError::Network(
                    "navigation parser stopped before response completion".into(),
                ));
            }
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ResponseReceived {
                    url: current.to_string(),
                    status,
                    bytes: capture.bytes,
                },
            );
            if let Some(policy) = meta_referrer_policy(&capture.source) {
                self.referrer_policy = crate::referrer::ReferrerPolicy::parse_list(&policy);
                #[cfg(unix)]
                if let Some(client) = &self.context_client {
                    client.request(
                        crate::context_service::ContextServiceRequest::InstallReferrerPolicy {
                            page_id: self.page_id,
                            serialized: policy,
                        },
                    )?;
                }
            }
            self.set_challenge_assessment(assess_challenge(
                status,
                &response_headers,
                &capture.source,
            ));
            // Re-evaluate the completed source for parent trace publication and
            // non-script resource discovery. Script execution itself was
            // authorized at its parser boundary and is never replayed here.
            let _ = self.enforce_csp_html(&capture.source);
            self.load_auxiliary_resources(&current, &capture.source);
            let stylesheet_source = self.inline_external_stylesheets(&current, &capture.source)?;
            self.web_fonts = self.load_web_fonts(&current, &stylesheet_source)?;
            self.layout = None;
            self.style_revision = self.style_revision.saturating_add(1);
        }
        #[cfg(not(feature = "v8-host"))]
        {
            let mut operation = operation;
            let mut decoder = oryn_net::IncrementalTextDecoder::new(
                content_type.as_deref(),
                oryn_net::TextResourceKind::Html,
            );
            let mut html = String::new();
            let mut bytes = 0_usize;
            operation.request_body_chunk();
            while let Some(event) = operation.events.blocking_recv() {
                match event {
                    crate::network::NetworkEvent::Chunk { bytes: chunk, .. } => {
                        bytes = bytes.saturating_add(chunk.len());
                        html.push_str(&decoder.push(&chunk));
                    }
                    crate::network::NetworkEvent::Complete { .. } => break,
                    crate::network::NetworkEvent::Failed { error, .. } => {
                        return Err(RuntimeError::Network(error));
                    }
                    crate::network::NetworkEvent::Headers { .. } => {
                        return Err(RuntimeError::ProtocolViolation(
                            "network operation emitted duplicate response headers".into(),
                        ));
                    }
                }
                operation.request_body_chunk();
            }
            html.push_str(&decoder.finish());
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ResponseReceived {
                    url: current.to_string(),
                    status,
                    bytes,
                },
            );
            let original_html = html;
            let html = self.enforce_csp_html(&original_html);
            if let Some(policy) = meta_referrer_policy(&html) {
                self.referrer_policy = crate::referrer::ReferrerPolicy::parse_list(&policy);
            }
            self.set_challenge_assessment(assess_challenge(status, &response_headers, &html));
            let html = self.inline_external_stylesheets(&current, &html)?;
            self.load_auxiliary_resources(&current, &html);
            let resources = self.load_external_scripts(&current, &html)?;
            self.load_document(
                current.to_string(),
                &html,
                None,
                None,
                matches!(history_update, HistoryUpdate::Push),
                &resources,
            )?;
        }
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
            lifecycle: self.lifecycle.clone(),
            challenge: self.last_challenge.clone(),
        })
    }

    #[cfg(feature = "v8-host")]
    fn enforce_csp_html(&mut self, html: &str) -> String {
        let token = regex::Regex::new(
            r#"(?is)<meta\b[^>]*http-equiv\s*=\s*['\"]?content-security-policy['\"]?[^>]*>|<script\b([^>]*)>(.*?)</script\s*>|<style\b([^>]*)>(.*?)</style\s*>"#,
        )
        .expect("constant CSP token regex");
        let content = regex::Regex::new(r#"(?is)\bcontent\s*=\s*(?:\"([^\"]*)\"|'([^']*)')"#)
            .expect("constant CSP meta content regex");
        let nonce = regex::Regex::new(r#"(?is)\bnonce\s*=\s*['\"]([^'\"]*)['\"]"#)
            .expect("constant nonce regex");
        let source = regex::Regex::new(r#"(?is)\bsrc\s*=\s*['\"]([^'\"]*)['\"]"#)
            .expect("constant script src regex");
        let mut output = String::with_capacity(html.len());
        let mut cursor = 0;
        for capture in token.captures_iter(html) {
            let whole = capture.get(0).expect("whole CSP token");
            output.push_str(&html[cursor..whole.start()]);
            let rendered = whole.as_str();
            if rendered.to_ascii_lowercase().starts_with("<meta") {
                if let Some(policy) = content.captures(rendered).and_then(|capture| {
                    capture
                        .get(1)
                        .or_else(|| capture.get(2))
                        .map(|value| value.as_str().to_string())
                }) {
                    self.csp.install_meta(&policy);
                    #[cfg(all(unix, feature = "v8-host"))]
                    if let Some(client) = &self.context_client {
                        let _ = client.request(
                            crate::context_service::ContextServiceRequest::InstallMetaSecurity {
                                page_id: self.page_id,
                                serialized: policy.clone(),
                            },
                        );
                    }
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::CspPolicyInstalled {
                            disposition: "enforce".into(),
                            source: "meta".into(),
                        },
                    );
                }
                output.push_str(rendered);
            } else {
                let is_script = rendered.to_ascii_lowercase().starts_with("<script");
                let attributes = capture
                    .get(if is_script { 1 } else { 3 })
                    .map_or("", |value| value.as_str());
                let inline = capture
                    .get(if is_script { 2 } else { 4 })
                    .map(|value| value.as_str());
                let external = source
                    .captures(attributes)
                    .map(|capture| capture[1].to_string());
                let request = crate::csp::CspRequest {
                    destination: if is_script {
                        crate::csp::CspDestination::Script
                    } else {
                        crate::csp::CspDestination::Style
                    },
                    url: external
                        .as_ref()
                        .and_then(|value| Url::parse(&self.url).ok()?.join(value).ok())
                        .map_or_else(|| "inline".into(), |url| url.to_string()),
                    nonce: nonce
                        .captures(attributes)
                        .map(|capture| capture[1].to_string()),
                    inline_source: external
                        .is_none()
                        .then(|| inline.unwrap_or_default().to_string()),
                    parser_inserted: true,
                    initiated_by_trusted_script: false,
                };
                let decision = self.csp.authorize(&request);
                self.record_csp_decision(&decision);
                let sandbox_blocks_script = is_script
                    && self
                        .csp
                        .sandbox()
                        .is_some_and(|sandbox| !sandbox.allow_scripts);
                if sandbox_blocks_script {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::CspViolation {
                            disposition: "enforce".into(),
                            effective_directive: "sandbox".into(),
                            blocked_uri: "inline".into(),
                        },
                    );
                }
                if decision.allowed && !sandbox_blocks_script {
                    output.push_str(rendered);
                }
            }
            cursor = whole.end();
        }
        output.push_str(&html[cursor..]);
        self.enforce_csp_attributes_and_base(&output)
    }

    #[cfg(feature = "v8-host")]
    fn enforce_csp_attributes_and_base(&mut self, html: &str) -> String {
        let base = regex::Regex::new(
            r#"(?is)<base\b[^>]*\bhref\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))[^>]*>"#,
        )
        .expect("constant base element regex");
        let mut with_base = String::with_capacity(html.len());
        let mut cursor = 0;
        for capture in base.captures_iter(html) {
            let whole = capture.get(0).expect("whole base element");
            with_base.push_str(&html[cursor..whole.start()]);
            let href = capture
                .get(1)
                .or_else(|| capture.get(2))
                .or_else(|| capture.get(3))
                .map_or("", |value| value.as_str());
            let resolved = Url::parse(&self.url)
                .ok()
                .and_then(|document| document.join(href).ok())
                .map_or_else(|| href.to_string(), |url| url.to_string());
            let decision = self.csp.authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Base,
                url: resolved,
                nonce: None,
                inline_source: None,
                parser_inserted: true,
                initiated_by_trusted_script: false,
            });
            self.record_csp_decision(&decision);
            if decision.allowed {
                with_base.push_str(whole.as_str());
            }
            cursor = whole.end();
        }
        with_base.push_str(&html[cursor..]);

        let attribute = regex::Regex::new(
            r#"(?is)\s+(on[\w:-]+|style)\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))"#,
        )
        .expect("constant CSP attribute regex");
        let sandbox_blocks_scripts = self
            .csp
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_scripts);
        let tag = regex::Regex::new(r#"(?is)<[^>]+>"#).expect("constant HTML tag regex");
        let mut output = String::with_capacity(with_base.len());
        let mut cursor = 0;
        for tag_match in tag.find_iter(&with_base) {
            output.push_str(&with_base[cursor..tag_match.start()]);
            let rendered = tag_match.as_str();
            let mut rewritten = String::with_capacity(rendered.len());
            let mut attribute_cursor = 0;
            for capture in attribute.captures_iter(rendered) {
                let whole = capture.get(0).expect("whole inline attribute");
                rewritten.push_str(&rendered[attribute_cursor..whole.start()]);
                let name = capture.get(1).expect("attribute name").as_str();
                let source = capture
                    .get(2)
                    .or_else(|| capture.get(3))
                    .or_else(|| capture.get(4))
                    .map_or("", |value| value.as_str());
                let script = name.to_ascii_lowercase().starts_with("on");
                let decision = self.csp.authorize(&crate::csp::CspRequest {
                    destination: if script {
                        crate::csp::CspDestination::ScriptAttribute
                    } else {
                        crate::csp::CspDestination::StyleAttribute
                    },
                    url: "inline".into(),
                    nonce: None,
                    inline_source: Some(source.to_string()),
                    parser_inserted: true,
                    initiated_by_trusted_script: false,
                });
                self.record_csp_decision(&decision);
                if decision.allowed && !(script && sandbox_blocks_scripts) {
                    rewritten.push_str(whole.as_str());
                } else if script && sandbox_blocks_scripts {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::CspViolation {
                            disposition: "enforce".into(),
                            effective_directive: "sandbox".into(),
                            blocked_uri: "inline".into(),
                        },
                    );
                }
                attribute_cursor = whole.end();
            }
            rewritten.push_str(&rendered[attribute_cursor..]);
            output.push_str(&rewritten);
            cursor = tag_match.end();
        }
        output.push_str(&with_base[cursor..]);
        output
    }

    #[cfg(not(feature = "v8-host"))]
    fn enforce_csp_html(&mut self, html: &str) -> String {
        html.to_string()
    }

    #[cfg(feature = "v8-host")]
    fn record_csp_decision(&mut self, decision: &crate::csp::CspDecision) {
        for violation in &decision.violations {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::CspViolation {
                    disposition: format!("{:?}", violation.disposition).to_ascii_lowercase(),
                    effective_directive: violation.effective_directive.clone(),
                    blocked_uri: violation.blocked_uri.clone(),
                },
            );
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if !decision.violations.is_empty()
            && let Some(client) = &self.context_client
        {
            let _ = client.request(
                crate::context_service::ContextServiceRequest::ReportCspViolations {
                    page_id: self.page_id,
                    violations: decision.violations.clone(),
                },
            );
        }
    }

    #[cfg(feature = "v8-host")]
    fn csp_allows_resource(&mut self, url: &Url, resource_kind: &str) -> bool {
        let destination = match resource_kind {
            "script" | "module" => crate::csp::CspDestination::Script,
            "stylesheet" => crate::csp::CspDestination::Style,
            "font" | "web font" => crate::csp::CspDestination::Font,
            "frame" => crate::csp::CspDestination::Frame,
            "image" => crate::csp::CspDestination::Image,
            "media" => crate::csp::CspDestination::Media,
            "object" => crate::csp::CspDestination::Object,
            "manifest" => crate::csp::CspDestination::Manifest,
            "worker" => crate::csp::CspDestination::Worker,
            _ => crate::csp::CspDestination::Connect,
        };
        let decision = self.csp.authorize(&crate::csp::CspRequest {
            destination,
            url: url.to_string(),
            nonce: None,
            inline_source: None,
            parser_inserted: true,
            initiated_by_trusted_script: false,
        });
        self.record_csp_decision(&decision);
        decision.allowed
    }

    #[cfg(feature = "v8-host")]
    fn load_auxiliary_resources(&mut self, base_url: &Url, html: &str) {
        let parsed = parse(html);
        let resources = parsed.dom.iter().filter_map(|(_, node)| {
            let crate::dom::NodeKind::Element { name } = &node.kind else {
                return None;
            };
            match name.local.as_ref() {
                "audio" | "video" | "source" | "track" => node
                    .attributes
                    .get("src")
                    .map(|url| (url.clone(), "media", "audio/*,video/*,text/vtt,*/*;q=0.1")),
                "object" => node
                    .attributes
                    .get("data")
                    .map(|url| (url.clone(), "object", "*/*")),
                "embed" => node
                    .attributes
                    .get("src")
                    .map(|url| (url.clone(), "object", "*/*")),
                "link"
                    if node.attributes.get("rel").is_some_and(|value| {
                        value
                            .split_ascii_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("manifest"))
                    }) =>
                {
                    node.attributes.get("href").map(|url| {
                        (
                            url.clone(),
                            "manifest",
                            "application/manifest+json,application/json,*/*;q=0.1",
                        )
                    })
                }
                _ => None,
            }
        });
        for (source, kind, accept) in resources {
            let result = base_url
                .join(&source)
                .map_err(|error| RuntimeError::Network(error.to_string()))
                .and_then(|url| self.fetch_external_resource_response(url, accept, kind));
            if let Err(error) = result {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::ScriptDiagnostic {
                        capability: format!("html.resource.{kind}"),
                        detail: error.to_string(),
                    },
                );
            }
        }
    }

    #[cfg(not(feature = "v8-host"))]
    fn load_auxiliary_resources(&mut self, _base_url: &Url, _html: &str) {}

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
            if !self.csp_allows_resource(&absolute, "stylesheet") {
                continue;
            }
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
        if !visited.insert(url.to_string()) {
            return Ok(String::new());
        }
        if !self.csp_allows_resource(url, "stylesheet") {
            return Ok(String::new());
        }
        let css = self.fetch_external_resource(url.clone(), "text/css,*/*;q=0.1", "stylesheet")?;
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
            if !self.csp_allows_resource(&absolute, "font") {
                continue;
            }
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
    #[cfg_attr(feature = "v8-host", allow(dead_code))]
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
            let absolute = parent.join(&source).map_err(|error| {
                RuntimeError::Network(format!("invalid external script URL {source}: {error}"))
            })?;
            let canonical = absolute.to_string();
            if resources.contains_key(&canonical) {
                if module {
                    resources.insert(format!("oryn:module-alias:{source}"), canonical);
                }
                continue;
            }
            if frame && origin_key(&absolute) != origin_key(base_url) {
                continue;
            }
            let (request_url, expected_digest) = pinned_resource(&absolute)
                .map(|(url, digest)| (Url::parse(url).expect("valid pinned URL"), Some(digest)))
                .unwrap_or_else(|| (absolute.clone(), None));
            if !self.csp_allows_resource(
                &request_url,
                if frame {
                    "frame"
                } else if module {
                    "module"
                } else {
                    "script"
                },
            ) {
                continue;
            }
            let body =
                self.fetch_external_resource(request_url, "text/javascript,*/*;q=0.1", "script")?;
            if let Some(expected) = expected_digest {
                let actual = format!("{:x}", Sha256::digest(body.as_bytes()));
                if actual != expected {
                    return Err(RuntimeError::Network(format!(
                        "pinned resource integrity failure for {absolute}: expected {expected}, got {actual}"
                    )));
                }
            }
            if module {
                for dependency in module_specifiers(&body) {
                    let dependency = import_map.get(&dependency).cloned().unwrap_or(dependency);
                    if !resources.contains_key(&dependency) {
                        pending.push((absolute.clone(), dependency, false, true));
                    }
                }
            }
            resources.insert(canonical.clone(), body.clone());
            if module {
                resources.insert(format!("oryn:module-alias:{source}"), canonical);
            } else {
                resources.insert(source, body);
            }
        }
        for (specifier, target) in import_map {
            let canonical = resources
                .get(&format!("oryn:module-alias:{target}"))
                .cloned()
                .or_else(|| resources.contains_key(&target).then_some(target));
            if let Some(canonical) = canonical {
                resources.insert(format!("oryn:module-alias:{specifier}"), canonical);
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
        let response = self.fetch_external_resource_response(url, accept, resource_kind)?;
        let content_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str());
        let kind = if resource_kind == "stylesheet" {
            oryn_net::TextResourceKind::Css
        } else {
            oryn_net::TextResourceKind::JavaScript
        };
        Ok(oryn_net::decode_resource_text(&response.body, content_type, kind).0)
    }

    #[cfg(feature = "v8-host")]
    fn fetch_external_resource_bytes(
        &mut self,
        url: Url,
        accept: &str,
        resource_kind: &str,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.fetch_external_resource_response(url, accept, resource_kind)
            .map(|response| response.body)
    }

    #[cfg(feature = "v8-host")]
    fn fetch_external_resource_response(
        &mut self,
        mut url: Url,
        accept: &str,
        resource_kind: &str,
    ) -> Result<NetworkResponse, RuntimeError> {
        if let Some(upgraded) = self.csp.upgraded_url(&url) {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::InsecureRequestUpgraded {
                    from: url.to_string(),
                    to: upgraded.to_string(),
                },
            );
            url = upgraded;
        }
        if !self.csp_allows_resource(&url, resource_kind) {
            return Err(RuntimeError::Network(format!(
                "Content-Security-Policy blocked {resource_kind} resource"
            )));
        }
        for redirect in 0..=self.config.max_redirects {
            if redirect > 0 && !self.csp_allows_resource(&url, resource_kind) {
                return Err(RuntimeError::Network(format!(
                    "Content-Security-Policy blocked redirected {resource_kind} resource"
                )));
            }
            self.network_quiet_since = None;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::RequestStarted {
                    method: "GET".into(),
                    url: url.to_string(),
                },
            );
            let mut headers = vec![("accept".into(), accept.into())];
            if let Ok(source) = Url::parse(&self.url)
                && let Some(referrer) = self.referrer_policy.header(&source, &url)
            {
                headers.push(("referer".into(), referrer));
            }
            let broker = self.broker.as_ref().ok_or_else(|| {
                RuntimeError::Network("network transport initialization failed".into())
            })?;
            let response = collect_operation_bounded(
                broker,
                NetworkRequest {
                    request_id: self.revision + redirect as u64 + 10_000,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        kind: match resource_kind {
                            "script" | "module" => crate::cookies::CookieRequestKind::Script,
                            "stylesheet" => crate::cookies::CookieRequestKind::Style,
                            "font" | "web font" => crate::cookies::CookieRequestKind::Font,
                            "frame" => crate::cookies::CookieRequestKind::Frame,
                            "media" => crate::cookies::CookieRequestKind::Media,
                            "object" => crate::cookies::CookieRequestKind::Object,
                            "manifest" => crate::cookies::CookieRequestKind::Manifest,
                            "worker" => crate::cookies::CookieRequestKind::Worker,
                            _ => crate::cookies::CookieRequestKind::Other,
                        },
                        initiator_url: Some(self.url.clone()),
                        top_level_url: Some(self.url.clone()),
                        redirect_chain_cross_site: redirect > 0
                            && Url::parse(&self.url).ok().is_some_and(|source| {
                                crate::cookies::SchemefulSite::from_url(&source)
                                    != crate::cookies::SchemefulSite::from_url(&url)
                            }),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                self.config.request_timeout,
                self.config.max_resource_cache_entry_bytes,
            )
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
            self.network_quiet_since = Some(Instant::now());
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
                if let Some(upgraded) = self.csp.upgraded_url(&url) {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::InsecureRequestUpgraded {
                            from: url.to_string(),
                            to: upgraded.to_string(),
                        },
                    );
                    url = upgraded;
                }
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(RuntimeError::Network(format!(
                    "external {resource_kind} returned HTTP {}",
                    response.status,
                )));
            }
            return Ok(response);
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
        let network_idle = matches!(
            predicate,
            WaitPredicate::Lifecycle(LifecycleState::NetworkIdle)
        );
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
        if !network_idle && self.wait_satisfied(predicate) {
            return Ok(());
        }
        #[cfg(feature = "v8-host")]
        if self.executable.is_some() {
            let before = self.semantic_graph.semantic_revision();
            let context_entries = {
                let executable = self.executable.as_mut().expect("checked above");
                executable.drain_tasks().map_err(map_host_error)?;
                executable
                    .take_context_trace_entries()
                    .map_err(map_host_error)?
            };
            self.sync_executable_semantics();
            self.refresh_layout()?;
            let after = self.semantic_graph.semantic_revision();
            if before != after {
                self.revision += 1;
                self.record_snapshot();
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::Mutation {
                        summary: "asynchronous page checkpoint".into(),
                    },
                );
            }
            for entry in context_entries {
                match entry.kind.as_str() {
                    "request_started" => self.network_quiet_since = None,
                    "response_received" => self.network_quiet_since = Some(Instant::now()),
                    _ => {}
                }
                if let Some(kind) = context_trace_kind(entry) {
                    self.trace.push(Revision(self.revision), None, kind);
                }
            }
        }
        if network_idle && let Some(quiet_since) = self.network_quiet_since {
            let elapsed = quiet_since.elapsed();
            if elapsed < NETWORK_IDLE_QUIET_PERIOD {
                std::thread::sleep(NETWORK_IDLE_QUIET_PERIOD - elapsed);
            }
            if !self.lifecycle.contains(&LifecycleState::NetworkIdle) {
                self.lifecycle.push(LifecycleState::NetworkIdle);
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::LifecycleChanged {
                        state: LifecycleState::NetworkIdle,
                    },
                );
            }
        }
        self.wait_satisfied(predicate)
            .then_some(())
            .ok_or_else(|| RuntimeError::WaitUnsatisfied(format!("{predicate:?}")))
    }

    fn wait_satisfied(&self, predicate: &WaitPredicate) -> bool {
        match predicate {
            WaitPredicate::Lifecycle(LifecycleState::NetworkIdle) => {
                self.lifecycle.contains(&LifecycleState::NetworkIdle)
                    || self
                        .network_quiet_since
                        .is_some_and(|since| since.elapsed() >= NETWORK_IDLE_QUIET_PERIOD)
            }
            WaitPredicate::Lifecycle(state) => self.lifecycle.contains(state),
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
                            // present even though bounded layout intentionally exposes no
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
            let before = self.semantic_graph.semantic_revision();
            let (value, console_entries, context_entries) = {
                let executable = self
                    .executable
                    .as_mut()
                    .ok_or(RuntimeError::EvaluationUnavailable)?;
                let value = executable.evaluate(source).map_err(map_host_error)?;
                (
                    value,
                    executable.take_console_entries().unwrap_or_default(),
                    executable.take_context_trace_entries().unwrap_or_default(),
                )
            };
            self.sync_executable_semantics();
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
            let after = self.semantic_graph.semantic_revision();
            if before != after {
                self.revision += 1;
                self.record_snapshot();
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::Mutation {
                        summary: "evaluated script changed page state".into(),
                    },
                );
            }
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
        if let Some(executable) = self.executable.as_mut() {
            executable
                .run_animation_frame_phase()
                .map_err(map_host_error)?;
        }
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
        let output = self
            .style_engine
            .compute(&input, &limits)
            .map_err(|error| RuntimeError::LayoutUnavailable(error.to_string()))?;
        self.style_revision = self.style_revision.saturating_add(1);
        self.layout_revision = self.layout_revision.saturating_add(1);
        self.scroll_state = output.scroll;
        if self.executable.is_some() {
            {
                let executable = self.executable.as_mut().expect("checked above");
                executable
                    .install_layout(&output, self.viewport)
                    .map_err(map_host_error)?;
                executable
                    .install_media(
                        match self.color_scheme {
                            PreferredColorScheme::Light => "light",
                            PreferredColorScheme::Dark => "dark",
                        },
                        match self.reduced_motion {
                            ReducedMotion::Reduce => "reduce",
                            ReducedMotion::NoPreference => "no_preference",
                        },
                    )
                    .map_err(map_host_error)?;
            }
            self.sync_executable_semantics();
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
        self.semantic_graph
            .apply_geometry_visibility(&output.geometry);
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
    fn paint_snapshot_plan(&mut self) -> Result<PaintSnapshotPlan, RuntimeError> {
        self.refresh_layout()?;
        let document = self
            .document
            .clone()
            .ok_or_else(|| RuntimeError::PaintUnavailable("page has no document".into()))?;
        let layout = self
            .layout
            .clone()
            .ok_or_else(|| RuntimeError::PaintUnavailable("page has no layout snapshot".into()))?;
        let mut pending_images = BTreeMap::<String, (Url, Vec<usize>)>::new();
        let mut diagnostics = layout.diagnostics.clone();
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
            if name.local.as_ref() == "img"
                && geometry.in_viewport
                && let Some(source) = node.attributes.get("src")
            {
                let resolved = Url::parse(source)
                    .or_else(|_| Url::parse(&self.url).and_then(|base| base.join(source)));
                match resolved {
                    Ok(url) => {
                        if url.scheme() == "data" || self.csp_allows_resource(&url, "image") {
                            pending_images
                                .entry(url.as_str().to_string())
                                .or_insert_with(|| (url, Vec::new()))
                                .1
                                .push(nodes.len());
                        } else {
                            diagnostics.push(format!(
                                "image {url} unavailable for paint: parent policy denied it"
                            ));
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
                image_resource: None,
            });
        }
        let snapshot = PaintSnapshot {
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
            images: Vec::new(),
            diagnostics,
        };
        Ok(PaintSnapshotPlan {
            snapshot,
            pending: pending_images.into_values().collect(),
            broker: self.broker.clone(),
            config: self.config.clone(),
            referrer_policy: self.referrer_policy,
            document_url: self.url.clone(),
        })
    }

    #[cfg(feature = "v8-host")]
    fn load_paint_image(
        broker: Option<&SharedNetworkBroker>,
        config: &BrowserConfig,
        referrer_policy: crate::referrer::ReferrerPolicy,
        document_url: &str,
        url: &Url,
    ) -> Result<PaintImageResource, RuntimeError> {
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
        for redirect in 0..=config.max_redirects {
            let request_id = NEXT_PAINT_IMAGE_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
            let mut headers = vec![("accept".into(), "image/png,image/jpeg,image/*;q=0.8".into())];
            if let Ok(source) = Url::parse(document_url)
                && let Some(referrer) = referrer_policy.header(&source, &current)
            {
                headers.push(("referer".into(), referrer));
            }
            let broker = broker.ok_or_else(|| {
                RuntimeError::PaintUnavailable("network broker is unavailable".into())
            })?;
            let next = collect_operation_bounded(
                broker,
                NetworkRequest {
                    request_id,
                    method: "GET".into(),
                    url: current.as_str().into(),
                    headers,
                    body: Vec::new(),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        kind: crate::cookies::CookieRequestKind::Image,
                        initiator_url: Some(document_url.to_owned()),
                        top_level_url: Some(document_url.to_owned()),
                        redirect_chain_cross_site: redirect > 0
                            && Url::parse(document_url).ok().is_some_and(|source| {
                                crate::cookies::SchemefulSite::from_url(&source)
                                    != crate::cookies::SchemefulSite::from_url(&current)
                            }),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                config.request_timeout,
                config.max_resource_cache_entry_bytes,
            )
            .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
            if (300..400).contains(&next.status) {
                if redirect == config.max_redirects {
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
                    "semantic.structures.v5",
                    "first-class forms, tables, lists, navigation, dialogs, menus, trees, tab sets, grids, landmarks, live regions, ARIA relationships, ownership, and explicit accessibility evidence",
                ),
                supported_capability(
                    "semantic.graph.v6",
                    "persistent Rust-owned semantic and accessibility graph with scoped observations, revision journals, field evidence, graph-native resolution, and explicit ambiguity/geometry failures",
                ),
                supported_capability(
                    "computed_style",
                    "Stylo-parsed stylesheets, selectors, specificity, declarations, and media rules with a worker-owned bounded computed-style projection",
                ),
                supported_capability("layout.box", "worker-owned CSS box geometry"),
                supported_capability("layout.inline_text", "deterministic inline text fragments"),
                supported_capability(
                    "paint.snapshot",
                    "bounded read-only paint input for an optional renderer",
                ),
                supported_capability(
                    "network.cors.parent_enforced",
                    "the parent broker enforces CORS response and preflight policy before bytes enter the worker",
                ),
                supported_capability(
                    "security.mixed_content",
                    "the parent blocks active HTTP and WebSocket mixed content",
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
                supported_capability(
                    "layout.table",
                    "bounded common-web table, row, and cell geometry plus structured table extraction",
                ),
                unsupported_capability(
                    "layout.hit_testing.paint_order",
                    "paint-order hit testing requires the optional rendering capability",
                ),
                unsupported_capability(
                    "layout.transform.3d",
                    "three-dimensional transforms and perspective are outside the bounded geometry contract",
                ),
                unsupported_capability(
                    "layout.float.complex",
                    "complex float exclusion, shape-outside, and fragmentation are outside the bounded float subset",
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
                    "service workers and Cache API are outside the application-compatibility subset",
                ),
                unsupported_capability(
                    "webrtc_and_media",
                    "WebRTC and media capture are outside the application-compatibility subset",
                ),
                unsupported_capability(
                    "xhr.synchronous_main_thread",
                    "synchronous main-thread XHR is intentionally unsupported",
                ),
                supported_capability(
                    "cookie.same_site",
                    "parent-owned schemeful SameSite enforcement with Lax-by-default and bounded Lax-allowing-unsafe compatibility",
                ),
                supported_capability(
                    "security.csp",
                    "parent-enforced CSP3 agent-browser profile with reporting endpoints, violation DOM events, opaque sandbox origins, worker/media/object/manifest fetch directives, upgrades, and Trusted Types sinks",
                ),
                unsupported_capability(
                    "media.decode_and_playback",
                    "media resources are policy-checked and brokered, but audio/video decoding and playback remain unavailable",
                ),
                supported_capability(
                    "worker.dedicated",
                    "parent-brokered worker-src enforcement with one persistent independent V8 context per dedicated worker and realm-owned message queues",
                ),
                unsupported_capability(
                    "worker.shared_and_service",
                    "SharedWorker and ServiceWorker remain outside the native profile",
                ),
                supported_capability(
                    "file_input.upload",
                    "parent-owned opaque upload handles populate native FileList/FormData after explicit policy authorization; host paths never enter the worker",
                ),
                supported_capability(
                    "clipboard.context_virtual_text",
                    "policy-controlled text clipboard isolated to the parent-owned BrowserContext; host system clipboard access remains unavailable",
                ),
                unsupported_capability(
                    "clipboard.system",
                    "host operating-system clipboard access is intentionally unavailable",
                ),
                CapabilityDiagnostic {
                    capability: "indexeddb.advanced".into(),
                    support: SupportLevel::Partial,
                    alternatives: vec![ExecutionDomain::Chromium, ExecutionDomain::Webkit],
                    handoff_lossy: true,
                    detail: Some(
                        "compound keys, multi-entry indexes, key-range intervals, and durability controls are outside the IndexedDB compatibility subset"
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
                CapabilityDiagnostic {
                    capability: "semantic.structures.v5".into(),
                    support: SupportLevel::Supported,
                    alternatives: Vec::new(),
                    handoff_lossy: false,
                    detail: Some(
                        "Rust DOM projection of first-class structures and supported accessibility rules"
                            .into(),
                    ),
                },
                CapabilityDiagnostic {
                    capability: "semantic.graph.v6".into(),
                    support: SupportLevel::Supported,
                    alternatives: Vec::new(),
                    handoff_lossy: false,
                    detail: Some(
                        "persistent Rust DOM semantic graph, scoped observations, deltas, evidence, and fail-closed resolution"
                            .into(),
                    ),
                },
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

    fn resolve_semantic(
        &mut self,
        target: &oryn_common::agent::Target,
        action: SemanticAction,
    ) -> Result<oryn_common::v6::ResolutionResult, RuntimeError> {
        if let oryn_common::agent::Target::Css { selector } = target {
            let selected = self.resolve_selector(selector, action)?;
            return Ok(oryn_common::v6::ResolutionResult {
                revision: Revision(self.revision),
                selected: Some(selected),
                candidates: vec![oryn_common::v6::ResolutionCandidate {
                    semantic_ref: selected,
                    score: 1_000,
                    actionable: true,
                    factors: vec!["rust_dom_css_selector".into()],
                }],
                diagnostics: Vec::new(),
            });
        }
        if target_requires_geometry(target) {
            #[cfg(feature = "v8-host")]
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
        }
        #[cfg(feature = "v8-host")]
        let geometry = self.layout.as_ref().map(|layout| &layout.geometry);
        #[cfg(not(feature = "v8-host"))]
        let geometry = None;
        self.semantic_graph
            .resolve(target, action, Revision(self.revision), geometry)
            .map_err(|error| match error {
                crate::semantic::SemanticResolutionError::NotFound => {
                    RuntimeError::StaleSemanticRef
                }
                crate::semantic::SemanticResolutionError::Ambiguous(candidates) => {
                    RuntimeError::AmbiguousTarget { candidates }
                }
                crate::semantic::SemanticResolutionError::GeometryUnavailable => {
                    RuntimeError::UnsupportedGeometry(
                        "spatial target resolution requires authoritative geometry".into(),
                    )
                }
                crate::semantic::SemanticResolutionError::UnsupportedSelector(detail) => {
                    RuntimeError::Semantic(detail)
                }
            })
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

    #[cfg(feature = "v8-host")]
    fn sync_executable_semantics(&mut self) {
        let Some(executable) = self.executable.as_mut() else {
            return;
        };
        let (mutations, truncated) = executable.take_semantic_mutations();
        let document = executable.document.clone();
        if truncated {
            self.semantic_graph
                .reset(&document.dom, self.document_generation);
        } else if !mutations.is_empty() {
            self.semantic_graph
                .apply_mutations(&document.dom, &mutations);
        }
        self.document = Some(document);
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
            nodes: self.semantic_graph.nodes(profile),
            capabilities: self.capabilities(),
            diagnostics: document.errors.clone(),
            execution_domain: ExecutionDomain::Native,
        }
    }

    fn semantic_observation(
        &self,
        profile: ProjectionProfile,
    ) -> oryn_common::v5::SemanticObservation {
        let observation = self.observation(profile);
        self.semantic_graph.observation_v5(observation)
    }

    fn semantic_observation_v6(
        &mut self,
        options: oryn_common::v6::SemanticObservationOptions,
    ) -> Result<oryn_common::v6::SemanticObservation, RuntimeError> {
        if options.geometry != oryn_common::v6::GeometryPolicy::Omit {
            #[cfg(feature = "v8-host")]
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
        }
        let base = self.observation(options.profile);
        #[cfg(feature = "v8-host")]
        let geometry = self.layout.as_ref().map(|layout| &layout.geometry);
        #[cfg(not(feature = "v8-host"))]
        let geometry = None;
        self.semantic_graph
            .observation_v6(base, &options, geometry)
            .map_err(RuntimeError::Semantic)
    }

    fn record_snapshot(&mut self) {
        let v6_changes = self.semantic_graph.take_v6_changes();
        let changes = crate::semantic::SemanticGraphChanges {
            upserted: v6_changes
                .upserted_nodes
                .iter()
                .map(|node| node.semantic.clone())
                .collect(),
            removed: v6_changes.removed_nodes.clone(),
        };
        let bytes = serde_json::to_vec(&v6_changes).map_or(0, |encoded| encoded.len());
        self.semantic_deltas.insert(self.revision, changes);
        if let Some(previous) = self.semantic_v6_deltas.insert(self.revision, v6_changes) {
            self.semantic_delta_bytes = self
                .semantic_delta_bytes
                .saturating_sub(serde_json::to_vec(&previous).map_or(0, |encoded| encoded.len()));
        }
        self.semantic_delta_bytes = self.semantic_delta_bytes.saturating_add(bytes);
        while self.semantic_deltas.len() > self.config.max_revision_snapshots
            || self.semantic_delta_bytes > self.config.max_revision_bytes
        {
            let Some(oldest) = self.semantic_deltas.keys().next().copied() else {
                break;
            };
            // Retain the current revision even when one unusually large
            // observation exceeds the byte ceiling. Older delta bases fail
            // explicitly rather than allowing memory to grow without limit.
            if oldest == self.revision && self.semantic_deltas.len() == 1 {
                break;
            }
            self.semantic_deltas.remove(&oldest);
            if let Some(removed) = self.semantic_v6_deltas.remove(&oldest) {
                self.semantic_delta_bytes = self.semantic_delta_bytes.saturating_sub(
                    serde_json::to_vec(&removed).map_or(0, |encoded| encoded.len()),
                );
            }
        }
    }

    fn semantic_delta_from(
        &self,
        from: Revision,
    ) -> Result<oryn_common::v6::SemanticDelta, RuntimeError> {
        if from.0 > self.revision {
            return Err(RuntimeError::UnknownRevision(from));
        }
        if from.0 < self.revision
            && self
                .semantic_v6_deltas
                .keys()
                .next()
                .is_none_or(|oldest| from.0.saturating_add(1) < *oldest)
        {
            return Err(RuntimeError::UnknownRevision(from));
        }
        let mut nodes = BTreeMap::new();
        let mut removed_nodes = BTreeSet::new();
        let mut accessibility = BTreeMap::new();
        let mut removed_accessibility = BTreeSet::new();
        let mut structures = BTreeMap::new();
        let mut removed_structures = BTreeSet::new();
        for changes in self
            .semantic_v6_deltas
            .range(from.0.saturating_add(1)..=self.revision)
            .map(|(_, changes)| changes)
        {
            for reference in &changes.removed_nodes {
                nodes.remove(reference);
                removed_nodes.insert(*reference);
            }
            for node in &changes.upserted_nodes {
                removed_nodes.remove(&node.semantic.semantic_ref);
                nodes.insert(node.semantic.semantic_ref, node.clone());
            }
            for reference in &changes.removed_accessibility {
                accessibility.remove(reference);
                removed_accessibility.insert(*reference);
            }
            for node in &changes.upserted_accessibility {
                removed_accessibility.remove(&node.semantic_ref);
                accessibility.insert(node.semantic_ref, node.clone());
            }
            for id in &changes.removed_structures {
                structures.remove(id);
                removed_structures.insert(id.clone());
            }
            for structure in &changes.upserted_structures {
                removed_structures.remove(&structure.structure_id);
                structures.insert(structure.structure_id.clone(), structure.clone());
            }
        }
        Ok(oryn_common::v6::SemanticDelta {
            contract_version: oryn_common::v6::CONTRACT_VERSION,
            page_id: self.page_id,
            from_revision: from,
            to_revision: Revision(self.revision),
            semantic_revision: self.semantic_graph.semantic_revision(),
            changes: oryn_common::v6::SemanticGraphChanges {
                upserted_nodes: nodes.into_values().collect(),
                removed_nodes: removed_nodes.into_iter().collect(),
                upserted_accessibility: accessibility.into_values().collect(),
                removed_accessibility: removed_accessibility.into_iter().collect(),
                upserted_structures: structures.into_values().collect(),
                removed_structures: removed_structures.into_iter().collect(),
            },
            diagnostics: Vec::new(),
        })
    }

    fn delta_from(&self, from: Revision) -> Result<ObservationDelta, RuntimeError> {
        if from.0 > self.revision {
            return Err(RuntimeError::UnknownRevision(from));
        }
        if from.0 == 0 {
            return Ok(ObservationDelta {
                contract_version: CONTRACT_VERSION,
                page_id: self.page_id,
                from_revision: from,
                to_revision: Revision(self.revision),
                upserted: self.semantic_graph.nodes(ProjectionProfile::Compact),
                removed: Vec::new(),
            });
        }
        if from.0 < self.revision
            && self
                .semantic_deltas
                .keys()
                .next()
                .is_none_or(|oldest| from.0.saturating_add(1) < *oldest)
        {
            return Err(RuntimeError::UnknownRevision(from));
        }
        let mut upserted = BTreeMap::new();
        let mut removed = BTreeSet::new();
        for (_, changes) in self
            .semantic_deltas
            .range(from.0.saturating_add(1)..=self.revision)
        {
            for semantic_ref in &changes.removed {
                upserted.remove(semantic_ref);
                removed.insert(*semantic_ref);
            }
            for node in &changes.upserted {
                removed.remove(&node.semantic_ref);
                upserted.insert(node.semantic_ref, node.clone());
            }
        }
        let mut upserted = upserted.into_values().collect::<Vec<_>>();
        upserted.sort_by_key(|node| node.document_order);
        Ok(ObservationDelta {
            contract_version: CONTRACT_VERSION,
            page_id: self.page_id,
            from_revision: from,
            to_revision: Revision(self.revision),
            upserted,
            removed: removed.into_iter().collect(),
        })
    }

    fn execute(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        self.execute_with_interactability(action, true, false)
    }

    fn execute_forced(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        self.execute_with_interactability(action, false, false)
    }

    fn execute_sensitive(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        self.execute_with_interactability(action, true, true)
    }

    fn set_files(
        &mut self,
        target: SemanticRef,
        files: Vec<crate::upload::UploadFile>,
    ) -> Result<ActionResult, RuntimeError> {
        self.validate_target(target)?;
        #[cfg(not(feature = "v8-host"))]
        let _ = &files;
        #[cfg(feature = "v8-host")]
        if self.executable.is_some() {
            let node = crate::semantic::unpack_node_ref(target.node);
            self.executable
                .as_mut()
                .expect("checked above")
                .set_files(node, &files)
                .map_err(map_host_error)?;
            self.sync_executable_semantics();
            return self.execute_in_realm(
                NativeAction {
                    target,
                    action: SemanticAction::Type,
                    value: files.first().map(|file| file.filename.clone()),
                },
                false,
            );
        }
        Err(RuntimeError::EvaluationUnavailable)
    }

    fn execute_with_interactability(
        &mut self,
        action: NativeAction,
        check_interactability: bool,
        sensitive: bool,
    ) -> Result<ActionResult, RuntimeError> {
        self.validate_target(action.target)?;
        if check_interactability
            && matches!(action.action, SemanticAction::Click | SemanticAction::Hover)
        {
            self.ensure_interactable(action.target)?;
        }
        #[cfg(feature = "v8-host")]
        if self.executable.is_some() {
            return self.execute_in_realm(action, sensitive);
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
                node.value = Some(value);
                node.sensitive_value = sensitive;
                ("updated form control value", true)
            }
            SemanticAction::Clear if matches!(local_name, "input" | "textarea") => {
                node.value = Some(String::new());
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
            self.semantic_graph.apply_mutations(
                &document.dom,
                &[crate::dom::Mutation::FormState { node: node_id }],
            );
            self.semantic_graph
                .nodes(ProjectionProfile::Compact)
                .into_iter()
                .filter(|node| node.semantic_ref == action.target)
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
        self.trace.push_causal(
            Revision(self.revision),
            Some(result.action_id),
            CausalRelation::Direct,
            trace_kind,
        );
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
    fn execute_in_realm(
        &mut self,
        action: NativeAction,
        sensitive: bool,
    ) -> Result<ActionResult, RuntimeError> {
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
        let mut connected = node_id == document.document;
        let mut ancestor = node.parent;
        while let Some(parent) = ancestor {
            if parent == document.document {
                connected = true;
                break;
            }
            ancestor = document
                .dom
                .get(parent)
                .map_err(|_| RuntimeError::StaleSemanticRef)?
                .parent;
        }
        if !connected {
            return Err(RuntimeError::StaleSemanticRef);
        }
        if node.attributes.contains_key("disabled") {
            return Err(RuntimeError::Disabled);
        }
        let (console_entries, context_entries, pending_navigation, diagnostics) = {
            let executable = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::StaleSemanticRef)?;
            if sensitive {
                executable
                    .apply_sensitive_action(node_id, action.action, action.value.as_deref())
                    .map_err(map_host_error)?;
            } else {
                executable
                    .apply_action(node_id, action.action, action.value.as_deref())
                    .map_err(map_host_error)?;
            }
            let pending_navigation = executable.location().ok().filter(|location| {
                location != &self.url && Url::parse(location).ok() != Url::parse(&self.url).ok()
            });
            (
                executable.take_console_entries().unwrap_or_default(),
                executable.take_context_trace_entries().unwrap_or_default(),
                pending_navigation,
                executable
                    .diagnostics
                    .iter()
                    .filter_map(|diagnostic| diagnostic.detail.clone())
                    .collect(),
            )
        };
        self.sync_executable_semantics();

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
        self.trace.push_causal(
            Revision(self.revision),
            Some(result.action_id),
            CausalRelation::Direct,
            TraceEventKind::EventDispatched {
                event_type: event_type.into(),
            },
        );
        for entry in console_entries {
            self.trace.push_causal(
                Revision(self.revision),
                Some(result.action_id),
                CausalRelation::Correlated,
                TraceEventKind::Console {
                    level: entry.level,
                    message: entry.message,
                },
            );
        }
        for entry in context_entries {
            match entry.kind.as_str() {
                "request_started" => self.network_quiet_since = None,
                "response_received" => self.network_quiet_since = Some(Instant::now()),
                _ => {}
            }
            if let Some(effect) = context_trace_effect(&entry) {
                result.effects.push(effect);
            }
            if let Some(kind) = context_trace_kind(entry) {
                self.trace.push_causal(
                    Revision(self.revision),
                    Some(result.action_id),
                    CausalRelation::Correlated,
                    kind,
                );
            }
        }
        if result
            .delta
            .as_ref()
            .is_some_and(|delta| !delta.upserted.is_empty() || !delta.removed.is_empty())
        {
            self.trace.push_causal(
                Revision(self.revision),
                Some(result.action_id),
                CausalRelation::Derived,
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
        "csp_report" => Some(TraceEventKind::CspReport {
            endpoint: string("endpoint"),
            format: string("format"),
            outcome: string("outcome"),
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
            let children = elements
                .iter()
                .copied()
                .filter(|candidate| {
                    document
                        .dom
                        .ancestors(*candidate)
                        .ok()
                        .and_then(|ancestors| {
                            ancestors.into_iter().find(|ancestor| {
                                document
                                    .dom
                                    .get(*ancestor)
                                    .is_ok_and(|node| matches!(node.kind, NodeKind::Element { .. }))
                            })
                        })
                        == Some(*id)
                })
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
fn complete_paint_snapshot(mut plan: PaintSnapshotPlan) -> Result<PaintSnapshot, RuntimeError> {
    for batch in plan.pending.chunks(32) {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            for (offset, (url, node_indexes)) in batch.iter().enumerate() {
                let sender = sender.clone();
                let broker = plan.broker.clone();
                let config = plan.config.clone();
                let url = url.clone();
                let node_indexes = node_indexes.clone();
                let document_url = plan.document_url.clone();
                let referrer_policy = plan.referrer_policy;
                scope.spawn(move || {
                    let result = PageState::load_paint_image(
                        broker.as_ref(),
                        &config,
                        referrer_policy,
                        &document_url,
                        &url,
                    );
                    let _ = sender.send((offset, url, node_indexes, result));
                });
            }
        });
        drop(sender);
        let mut completed = receiver.into_iter().collect::<Vec<_>>();
        completed.sort_by_key(|(offset, ..)| *offset);
        for (_, url, node_indexes, result) in completed {
            match result {
                Ok(resource)
                    if plan
                        .snapshot
                        .images
                        .iter()
                        .map(|image| image.bytes.len())
                        .sum::<usize>()
                        .saturating_add(resource.bytes.len())
                        <= plan.config.max_response_bytes =>
                {
                    let index = plan.snapshot.images.len();
                    plan.snapshot.images.push(resource);
                    for node_index in node_indexes {
                        if let Some(node) = plan.snapshot.nodes.get_mut(node_index) {
                            node.image_resource = Some(index);
                        }
                    }
                }
                Ok(_) => plan.snapshot.diagnostics.push(format!(
                    "image {url} omitted: paint snapshot image budget exceeded"
                )),
                Err(error) => plan
                    .snapshot
                    .diagnostics
                    .push(format!("image {url} unavailable for paint: {error}")),
            }
        }
    }
    Ok(plan.snapshot)
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
        crate::dom::NodeKind::Document
        | crate::dom::NodeKind::DocumentFragment
        | crate::dom::NodeKind::ShadowRoot { .. } => node
            .children
            .iter()
            .map(|child| serialize_dom_node(dom, *child))
            .collect(),
        crate::dom::NodeKind::DocumentType { name, .. } => format!("<!DOCTYPE {name}>"),
        crate::dom::NodeKind::Text { data } => escape_html(data),
        crate::dom::NodeKind::CDataSection { data } => escape_html(data),
        crate::dom::NodeKind::ProcessingInstruction { target, data } => {
            format!("<?{target} {data}?>")
        }
        crate::dom::NodeKind::Comment { data } => format!("<!--{data}-->"),
        crate::dom::NodeKind::Attribute { value, .. } => escape_html(value),
        crate::dom::NodeKind::Element { name } => {
            let attributes = node
                .attributes
                .iter()
                .map(|(name, value)| format!(" {name}=\"{}\"", escape_html(value)))
                .collect::<String>();
            let children = node
                .children
                .iter()
                .filter(|child| {
                    !matches!(
                        dom.get(**child).map(|node| &node.kind),
                        Ok(crate::dom::NodeKind::ShadowRoot { .. })
                    )
                })
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

fn meta_referrer_policy(html: &str) -> Option<String> {
    let meta = regex::Regex::new(r#"(?is)<meta\b[^>]*>"#).expect("constant meta regex");
    let attribute = regex::Regex::new(
        r#"(?is)\b([a-z][a-z0-9_-]*)\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))"#,
    )
    .expect("constant meta attribute regex");
    meta.find_iter(html).find_map(|element| {
        let attributes = attribute
            .captures_iter(element.as_str())
            .filter_map(|capture| {
                Some((
                    capture.get(1)?.as_str().to_ascii_lowercase(),
                    capture
                        .get(2)
                        .or_else(|| capture.get(3))
                        .or_else(|| capture.get(4))?
                        .as_str()
                        .to_string(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        attributes
            .get("name")
            .is_some_and(|value| value.eq_ignore_ascii_case("referrer"))
            .then(|| attributes.get("content").cloned())
            .flatten()
    })
}

#[cfg(feature = "v8-host")]
#[cfg_attr(feature = "v8-host", allow(dead_code))]
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
#[cfg_attr(feature = "v8-host", allow(dead_code))]
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
#[cfg_attr(feature = "v8-host", allow(dead_code))]
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

fn serialize_csp_report(
    format: crate::csp::CspReportFormat,
    violation: &crate::csp::CspViolation,
) -> (&'static str, Vec<u8>) {
    let disposition = format!("{:?}", violation.disposition).to_ascii_lowercase();
    match format {
        crate::csp::CspReportFormat::ReportingApi => (
            "application/reports+json",
            serde_json::to_vec(&serde_json::json!([{
                "age": 0,
                "type": "csp-violation",
                "url": violation.document_uri,
                "body": {
                    "documentURL": violation.document_uri,
                    "referrer": violation.referrer,
                    "blockedURL": violation.blocked_uri,
                    "effectiveDirective": violation.effective_directive,
                    "originalPolicy": violation.original_policy,
                    "sourceFile": violation.source_file,
                    "sample": violation.sample,
                    "disposition": disposition,
                    "statusCode": violation.status_code,
                    "lineNumber": violation.line_number,
                    "columnNumber": violation.column_number,
                }
            }]))
            .unwrap_or_default(),
        ),
        crate::csp::CspReportFormat::LegacyReportUri => (
            "application/csp-report",
            serde_json::to_vec(&serde_json::json!({
                "csp-report": {
                    "document-uri": violation.document_uri,
                    "referrer": violation.referrer,
                    "blocked-uri": violation.blocked_uri,
                    "effective-directive": violation.effective_directive,
                    "violated-directive": violation.effective_directive,
                    "original-policy": violation.original_policy,
                    "source-file": violation.source_file,
                    "script-sample": violation.sample,
                    "disposition": disposition,
                    "status-code": violation.status_code,
                    "line-number": violation.line_number,
                    "column-number": violation.column_number,
                }
            }))
            .unwrap_or_default(),
        ),
    }
}

#[cfg(feature = "v8-host")]
#[cfg_attr(feature = "v8-host", allow(dead_code))]
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

    #[test]
    fn meta_referrer_policy_is_order_and_quote_independent() {
        assert_eq!(
            super::meta_referrer_policy(
                "<head><meta content='no-referrer' data-x=1 NAME=referrer></head>"
            )
            .as_deref(),
            Some("no-referrer")
        );
        assert_eq!(
            super::meta_referrer_policy("<meta name=viewport content=x>"),
            None
        );
    }

    use oryn_common::v3::{ExecutionDomain, ObservationSource};

    use super::*;

    #[test]
    fn runtime_profile_default_matches_the_compiled_icu_payload() {
        assert_eq!(
            BrowserConfig::default().runtime_profile,
            RuntimeProfile::compiled()
        );
        assert!(RuntimeProfile::compiled().available_in_build());
        assert_ne!(
            RuntimeProfile::Full.available_in_build(),
            RuntimeProfile::LiteEnglish.available_in_build()
        );
    }
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

    #[cfg(feature = "v8-host")]
    #[test]
    fn navigation_enforces_csp_before_inline_script_execution() {
        struct CspFixture;
        impl NetworkBroker for CspFixture {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                Ok(NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/html".into()),
                        ("content-security-policy".into(), "script-src 'none'".into()),
                    ],
                    body: b"<main id='state'>blocked</main><script>document.getElementById('state').textContent='ran'</script>".to_vec(),
                })
            }
        }
        let mut page = PageState::new(
            PageId(91),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            Some(SharedNetworkBroker::new(CspFixture)),
            #[cfg(all(unix, feature = "v8-host"))]
            None,
        );
        page.navigate("https://app.example/".into(), HistoryUpdate::Push)
            .expect("CSP-protected navigation");
        let state = page
            .document
            .as_ref()
            .unwrap()
            .dom
            .iter()
            .find(|(_, node)| node.attributes.get("id").is_some_and(|id| id == "state"))
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(
            page.document
                .as_ref()
                .unwrap()
                .dom
                .text_content(state)
                .unwrap(),
            "blocked"
        );
        assert!(page.trace.events().iter().any(|event| matches!(
            event.kind,
            TraceEventKind::CspViolation { ref effective_directive, .. }
                if effective_directive == "script-src"
        )));
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn navigation_cancels_timeout_and_stale_header_operations() {
        struct MalformedNavigation {
            stale: bool,
            cancelled: Arc<AtomicUsize>,
        }
        impl NetworkBroker for MalformedNavigation {
            fn send(
                &self,
                _request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                unreachable!("navigation uses the operation path")
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<crate::network::NetworkOperation, crate::network::NetworkError>
            {
                let (sender, events) = tokio::sync::mpsc::channel(2);
                if self.stale {
                    sender
                        .blocking_send(crate::network::NetworkEvent::Headers {
                            request_id: request.request_id + 1,
                            status: 200,
                            headers: Vec::new(),
                            content_length: Some(0),
                        })
                        .unwrap();
                } else {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(1));
                        drop(sender);
                    });
                }
                Ok(crate::network::NetworkOperation {
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
            fn cancel_operation(
                &self,
                _request_id: u64,
            ) -> Result<(), crate::network::NetworkError> {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        for stale in [false, true] {
            let cancelled = Arc::new(AtomicUsize::new(0));
            let config = BrowserConfig {
                navigation_timeout: Duration::from_millis(10),
                ..BrowserConfig::default()
            };
            let mut page = PageState::new(
                PageId(190),
                None,
                config,
                ContextOptions::default(),
                Some(SharedNetworkBroker::new(MalformedNavigation {
                    stale,
                    cancelled: cancelled.clone(),
                })),
                #[cfg(all(unix, feature = "v8-host"))]
                None,
            );
            let error = page
                .navigate("https://timeout.example/".into(), HistoryUpdate::Push)
                .expect_err("malformed navigation must fail closed");
            if stale {
                assert!(matches!(error, RuntimeError::ProtocolViolation(_)));
            } else {
                assert!(matches!(error, RuntimeError::WorkerLimitExceeded { .. }));
            }
            assert_eq!(cancelled.load(Ordering::SeqCst), 1);
        }
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn csp_meta_order_guards_base_and_inline_attributes() {
        let mut page = PageState::new(
            PageId(92),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(all(unix, feature = "v8-host"))]
            None,
        );
        page.url = "https://app.example/".into();
        page.csp = crate::csp::CspPolicySet::new(page.url.clone());
        let filtered = page.enforce_csp_html(
            r#"<script>globalThis.beforeMeta=true</script>
               <meta http-equiv="content-security-policy" content="base-uri 'none'; script-src-attr 'none'; style-src-attr 'none'">
               <base href="https://evil.example/">
               <button onclick="attack()" style="display:none">Safe</button>"#,
        );
        assert!(filtered.contains("beforeMeta"));
        assert!(!filtered.contains("evil.example"));
        assert!(!filtered.contains("onclick"));
        assert!(!filtered.contains("display:none"));
        let directives = page
            .trace
            .events()
            .iter()
            .filter_map(|event| match &event.kind {
                TraceEventKind::CspViolation {
                    effective_directive,
                    ..
                } => Some(effective_directive.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(directives.contains(&"base-uri"));
        assert!(directives.contains(&"script-src-attr"));
        assert!(directives.contains(&"style-src-attr"));
    }

    #[test]
    fn csp_sandbox_uses_unique_opaque_security_origins() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let first = context.new_page();
        let second = context.new_page();
        let headers = vec![(
            "content-security-policy".into(),
            "sandbox allow-scripts".into(),
        )];
        context
            .install_document_security(first.page_id(), "https://app.example/one".into(), &headers)
            .unwrap();
        context
            .install_document_security(second.page_id(), "https://app.example/two".into(), &headers)
            .unwrap();
        assert_eq!(context.page_origin(first.page_id()).unwrap(), "null");
        assert_eq!(context.page_origin(second.page_id()).unwrap(), "null");
        assert_ne!(
            context.security_origin(first.page_id()).unwrap(),
            context.security_origin(second.page_id()).unwrap()
        );
        #[cfg(unix)]
        assert!(context.document_cookie(first.page_id()).is_err());
    }

    #[test]
    fn csp_allow_same_origin_preserves_tuple_origin() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        context
            .install_document_security(
                page.page_id(),
                "https://app.example/path".into(),
                &[(
                    "content-security-policy".into(),
                    "sandbox allow-scripts allow-same-origin".into(),
                )],
            )
            .unwrap();
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://app.example"
        );
    }

    #[test]
    fn csp_report_serializers_do_not_expose_credentials() {
        let violation = crate::csp::CspViolation {
            disposition: crate::csp::CspDisposition::Report,
            effective_directive: "script-src".into(),
            blocked_uri: "https://cdn.example/app.js".into(),
            document_uri: "https://app.example/private".into(),
            source: "header".into(),
            original_policy: "script-src 'none'; report-uri /csp".into(),
            referrer: String::new(),
            source_file: String::new(),
            sample: String::new(),
            status_code: 200,
            line_number: 0,
            column_number: 0,
        };
        for format in [
            crate::csp::CspReportFormat::ReportingApi,
            crate::csp::CspReportFormat::LegacyReportUri,
        ] {
            let (_, body) = serialize_csp_report(format, &violation);
            let body = String::from_utf8(body).unwrap();
            assert!(body.contains("script-src"));
            assert!(!body.contains("token="));
        }
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn report_only_policy_dispatches_dom_event_and_parent_report() {
        #[derive(Clone)]
        struct ReportingFixture {
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
        }
        impl NetworkBroker for ReportingFixture {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                if request.url.ends_with("/csp-report") {
                    return Ok(NetworkResponse {
                        request_id: request.request_id,
                        status: 204,
                        headers: Vec::new(),
                        body: Vec::new(),
                    });
                }
                Ok(NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/html".into()),
                        (
                            "content-security-policy-report-only".into(),
                            "script-src 'none'; report-uri /csp-report".into(),
                        ),
                    ],
                    body: br#"<script>globalThis.cspEvents=[];addEventListener('securitypolicyviolation',event=>cspEvents.push(event.effectiveDirective+':'+event.disposition))</script>"#.to_vec(),
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_broker(
                ContextOptions::default(),
                SharedNetworkBroker::new(ReportingFixture {
                    requests: requests.clone(),
                }),
            );
        let page = context.new_page();
        page.goto("https://app.example/")
            .await
            .expect("report-only navigation");
        assert_eq!(
            page.evaluate("JSON.stringify(cspEvents)").await.unwrap(),
            r#"["script-src:report"]"#
        );
        for _ in 0..50 {
            if requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.url.ends_with("/csp-report"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let requests = requests.lock().unwrap();
        let report = requests
            .iter()
            .find(|request| request.url.ends_with("/csp-report"))
            .expect("parent CSP report request");
        assert!(!report.credentials);
        assert!(String::from_utf8_lossy(&report.body).contains("script-src"));
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn media_object_and_manifest_resources_use_their_csp_directives() {
        struct ResourceFixture;
        impl NetworkBroker for ResourceFixture {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                Ok(NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![(
                        "content-security-policy".into(),
                        "media-src 'none'; object-src 'none'; manifest-src 'none'".into(),
                    )],
                    body: br#"<video src='/movie.mp4'></video><object data='/item.bin'></object><link rel='manifest' href='/app.webmanifest'>"#.to_vec(),
                })
            }
        }
        let mut page = PageState::new(
            PageId(94),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            Some(SharedNetworkBroker::new(ResourceFixture)),
            #[cfg(all(unix, feature = "v8-host"))]
            None,
        );
        page.navigate("https://app.example/".into(), HistoryUpdate::Push)
            .unwrap();
        let directives = page
            .trace
            .events()
            .iter()
            .filter_map(|event| match &event.kind {
                TraceEventKind::CspViolation {
                    effective_directive,
                    ..
                } => Some(effective_directive.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(directives.contains(&"media-src"));
        assert!(directives.contains(&"object-src"));
        assert!(directives.contains(&"manifest-src"));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn dedicated_worker_is_brokered_through_worker_src() {
        struct WorkerFixture;
        impl NetworkBroker for WorkerFixture {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                if request.url.ends_with("/worker.js") {
                    return Ok(NetworkResponse {
                        request_id: request.request_id,
                        status: 200,
                        headers: vec![("content-type".into(), "text/javascript".into())],
                        body: b"globalThis.workerOnly='worker-realm';self.onmessage=event=>postMessage({reply:event.data+'-reply',documentType:typeof document,pageLeak:typeof pageOnly,workerOnly})".to_vec(),
                    });
                }
                Ok(NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/html".into()),
                        (
                            "content-security-policy".into(),
                            "script-src 'unsafe-inline' 'unsafe-eval'; worker-src 'self'".into(),
                        ),
                    ],
                    body: br#"<output id='result'></output><script>globalThis.pageOnly='page-realm';const worker=new Worker('/worker.js');worker.onmessage=event=>document.querySelector('#result').textContent=JSON.stringify(event.data);worker.postMessage('worker')</script>"#.to_vec(),
                })
            }
        }
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_broker(
                ContextOptions::default(),
                SharedNetworkBroker::new(WorkerFixture),
            );
        let page = context.new_page();
        page.goto("https://app.example/").await.unwrap();
        let result = page
            .evaluate("document.querySelector('#result').textContent")
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!({
                "reply": "worker-reply",
                "documentType": "undefined",
                "pageLeak": "undefined",
                "workerOnly": "worker-realm",
            })
        );
        assert_eq!(
            page.evaluate("typeof workerOnly").await.unwrap(),
            "undefined"
        );
        assert_eq!(
            page.evaluate("typeof worker.__scope").await.unwrap(),
            "undefined"
        );
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
        let navigation_started = Instant::now();
        let first = page
            .goto(format!("http://{address}/redirect"))
            .await
            .expect("follow checked redirect");
        assert_eq!(first.status, 200);
        assert!(first.final_url.ends_with("/one"));
        assert_eq!(first.lifecycle.last(), Some(&LifecycleState::Load));
        page.wait_for(WaitPredicate::Lifecycle(LifecycleState::NetworkIdle))
            .await
            .expect("wait for a measured quiet period");
        assert!(navigation_started.elapsed() >= NETWORK_IDLE_QUIET_PERIOD);
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
        assert_eq!(back.lifecycle.last(), Some(&LifecycleState::Load));
        page.wait_for(WaitPredicate::Lifecycle(LifecycleState::NetworkIdle))
            .await
            .expect("wait for blank document quiet period");
    }

    #[cfg(unix)]
    #[test]
    fn internal_fixture_urls_may_commit_but_cannot_escape_as_navigation() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let fixture = "fixture:///tmp/oryn-app-compatibility-smoke.html";

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
                    "<title>External</title><output id=result>Pending</output><script src=/app.js></script>"
                };
                let mut response = fixture_response(body);
                if path == "/app.js" {
                    response =
                        response.replacen("\r\n", "\r\nCache-Control: public, max-age=60\r\n", 1);
                }
                stream
                    .write_all(response.as_bytes())
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
    async fn async_and_deferred_resources_fetch_concurrently_and_execute_by_contract() {
        struct ScheduledResourceBroker {
            active: Arc<AtomicUsize>,
            peak: Arc<AtomicUsize>,
        }

        impl crate::network::NetworkBroker for ScheduledResourceBroker {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                let path = Url::parse(&request.url)
                    .expect("fixture URL")
                    .path()
                    .to_string();
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(active, Ordering::SeqCst);
                let (content_type, body, delay) = match path.as_str() {
                    "/" => (
                        "text/html",
                        concat!(
                            "<output id=result></output>",
                            "<script async src=/async-slow.js></script>",
                            "<script async src=/async-fast.js></script>",
                            "<script defer src=/defer-slow.js></script>",
                            "<script defer src=/defer-fast.js></script>",
                            "<script type=module src=/module.js></script>"
                        ),
                        0,
                    ),
                    "/async-slow.js" => (
                        "text/javascript",
                        "document.querySelector('#result').textContent+='A'",
                        40,
                    ),
                    "/async-fast.js" => (
                        "text/javascript",
                        "document.querySelector('#result').textContent+='B'",
                        5,
                    ),
                    "/defer-slow.js" => (
                        "text/javascript",
                        "document.querySelector('#result').textContent+='D'",
                        40,
                    ),
                    "/defer-fast.js" => (
                        "text/javascript",
                        "document.querySelector('#result').textContent+='E'",
                        5,
                    ),
                    "/module.js" => (
                        "text/javascript",
                        "import {left} from './left.js';import {right} from './right.js';document.querySelector('#result').textContent+=left+right",
                        5,
                    ),
                    "/left.js" => ("text/javascript", "export const left='M'", 20),
                    "/right.js" => ("text/javascript", "export const right=''", 20),
                    _ => return Err(crate::network::NetworkError::Transport(path)),
                };
                std::thread::sleep(Duration::from_millis(delay));
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(crate::network::NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), content_type.into())],
                    body: body.as_bytes().to_vec(),
                })
            }
        }

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let context = Browser::new().new_context_with_broker(
            ContextOptions::default(),
            SharedNetworkBroker::new(ScheduledResourceBroker {
                active,
                peak: peak.clone(),
            }),
        );
        let page = context.new_page();
        page.goto("https://scheduler.example.test/")
            .await
            .expect("load scheduled resources");
        let value = page
            .evaluate("document.querySelector('#result').textContent")
            .await
            .expect("read execution order");
        assert_eq!(value, "BADEM");
        assert_eq!(peak.load(Ordering::SeqCst), 5);
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn external_css_import_and_web_font_are_parent_brokered() {
        struct CssBroker;
        impl crate::network::NetworkBroker for CssBroker {
            fn send(
                &self,
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
    async fn evaluate_publishes_one_revision_for_one_semantic_dom_change() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/evaluate-revision",
            "<button id=target aria-label=Pending></button>",
        )
        .await
        .expect("load evaluation fixture");
        let before = page.observe().await.expect("observe initial state");

        assert_eq!(
            page.evaluate(
                "document.querySelector('#target').setAttribute('aria-label','Ready'); 'done'",
            )
            .await
            .expect("evaluate mutation"),
            "done"
        );

        let after = page.observe().await.expect("observe evaluated state");
        assert_eq!(after.revision, Revision(before.revision.0 + 1));
        let delta = page
            .observe_delta(before.revision)
            .await
            .expect("observe evaluated delta");
        assert_eq!(delta.from_revision, before.revision);
        assert_eq!(delta.to_revision, after.revision);
        assert!(delta.upserted.iter().any(|node| {
            node.semantic_ref
                == before
                    .nodes
                    .iter()
                    .find(|node| node.name == "Pending")
                    .expect("initial target")
                    .semantic_ref
                && node.name == "Ready"
        }));

        page.evaluate("'no mutation'")
            .await
            .expect("evaluate no-op");
        assert_eq!(
            page.observe().await.expect("observe after no-op").revision,
            after.revision,
            "an evaluation without an observable DOM change must not mint a revision"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn wait_task_drain_publishes_one_revision_and_one_delta() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/wait-revision",
            r#"<button id=pending aria-label=Pending></button>
               <script>
                 setTimeout(() => {
                   const target = document.querySelector('#pending');
                   target.id = 'ready';
                   target.setAttribute('aria-label', 'Ready');
                 }, 25);
               </script>"#,
        )
        .await
        .expect("load delayed mutation fixture");
        let before = page.observe().await.expect("observe pending state");
        let target = before
            .nodes
            .iter()
            .find(|node| node.name == "Pending")
            .expect("pending target")
            .semantic_ref;

        page.wait_for(WaitPredicate::SelectorExists("#ready".into()))
            .await
            .expect("drain delayed task");

        let after = page.observe().await.expect("observe ready state");
        assert_eq!(after.revision, Revision(before.revision.0 + 1));
        let delta = page
            .observe_delta(before.revision)
            .await
            .expect("observe asynchronous delta");
        assert_eq!(delta.to_revision, after.revision);
        assert!(
            delta
                .upserted
                .iter()
                .any(|node| node.semantic_ref == target && node.name == "Ready")
        );

        page.wait_for(WaitPredicate::SelectorExists("#ready".into()))
            .await
            .expect("already-satisfied wait");
        assert_eq!(
            page.observe()
                .await
                .expect("observe after repeated wait")
                .revision,
            after.revision,
            "an already-satisfied wait must not publish the same task change twice"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn same_origin_iframe_is_semantic_and_cross_origin_is_diagnostic() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let cross_origin = format!("http://localhost:{}", address.port());
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
                let body = if path == "/frame" {
                    "<title>Child</title><button id=frame-action onclick=\"this.textContent='Clicked in child'\">Pending</button><script>globalThis.frameRealmMarker='isolated';document.querySelector('#frame-action').textContent='Frame action';addEventListener('message',async event=>{await indexedDB.databases();event.source.postMessage({reply:event.data},'*')})</script>".to_owned()
                } else {
                    format!(
                        "<iframe src=/frame></iframe><iframe src=\"http://localhost:{}/frame\"></iframe>",
                        address.port()
                    )
                };
                stream
                    .write_all(fixture_response(&body).as_bytes())
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
            observation.capabilities.iter().any(|capability| {
                capability.capability == "html.frame.cross_origin"
                    && capability.support == SupportLevel::Partial
            }),
            "frame diagnostics: {:#?}",
            observation.capabilities
        );
        assert!(
            observation
                .nodes
                .iter()
                .any(|node| node.name == "Frame action")
        );
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
        page.evaluate("globalThis.frameReply='';addEventListener('message',event=>frameReply=event.data.reply);document.querySelector('iframe').contentWindow.postMessage('ping','*');'sent'")
            .await
            .expect("send same-origin frame message");
        assert_eq!(
            page.evaluate("frameReply")
                .await
                .expect("same-origin frame postMessage round trip"),
            "ping"
        );
        page.evaluate(&format!("globalThis.crossFrameReply='';addEventListener('message',event=>crossFrameReply=event.data.reply);document.querySelectorAll('iframe')[1].contentWindow.postMessage('opaque',{cross_origin:?});'sent'"))
            .await
            .expect("send opaque frame message");
        assert_eq!(
            page.evaluate("crossFrameReply")
                .await
                .expect("opaque frame postMessage round trip"),
            "opaque"
        );
        assert_eq!(
            page.evaluate("document.querySelectorAll('iframe')[1].contentDocument===null")
                .await
                .expect("opaque frame document is inaccessible"),
            "true"
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

    #[tokio::test(flavor = "current_thread")]
    async fn semantic_revision_history_is_bounded_and_eviction_is_explicit() {
        let config = BrowserConfig {
            max_revision_snapshots: 2,
            max_revision_bytes: usize::MAX,
            ..BrowserConfig::default()
        };
        let page = Browser::in_process_with_config(config)
            .new_context()
            .new_page();
        page.load_html("https://example.test/", "<input id=value>")
            .await
            .unwrap();
        let target = page
            .resolve_selector("#value", SemanticAction::Type)
            .await
            .unwrap();
        page.execute(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some("first".into()),
        })
        .await
        .unwrap();
        page.execute(NativeAction {
            target,
            action: SemanticAction::Clear,
            value: None,
        })
        .await
        .unwrap();
        page.execute(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some("second".into()),
        })
        .await
        .unwrap();

        assert!(matches!(
            page.observe_delta(Revision(1)).await,
            Err(RuntimeError::UnknownRevision(Revision(1)))
        ));
        assert_eq!(
            page.observe_delta(Revision(2)).await.unwrap().to_revision,
            Revision(4)
        );
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

    #[cfg(all(unix, feature = "v8-host", feature = "in-process-probe"))]
    #[tokio::test(flavor = "current_thread")]
    async fn confirmed_form_command_is_rechecked_at_the_concrete_effect_boundary() {
        let mut options = ContextOptions::default();
        options.action_policy.rules.insert(
            ConsequentialAction::FormSubmission,
            PolicyRule::RequireConfirmation,
        );
        let context = Browser::in_process_probe().new_context_with_options(options);
        let page = context.new_page();
        page.load_html(
            "https://app.example/",
            "<form id=checkout action=/charged><button>Pay</button></form>",
        )
        .await
        .unwrap();
        let request = PolicyRequest {
            action: ConsequentialAction::FormSubmission,
            source_origin: "https://app.example".into(),
            target_origin: None,
            detail: Some("semantic form submission".into()),
        };
        let confirmation = context.prepare_confirmation(request.clone());

        context.require_policy(request, Some(confirmation)).unwrap();
        context.grant_form_submission(page.page_id());
        let service =
            crate::context_service::ParentContextService::new(context.clone(), page.page_id());
        assert!(matches!(
            service
                .handle(
                    crate::context_service::ContextServiceRequest::AuthorizeFormSubmission {
                        page_id: page.page_id(),
                        url: "https://app.example/charged".into(),
                        method: "post".into(),
                    },
                )
                .unwrap(),
            crate::context_service::ContextServiceResponse::FormSubmissionAuthorized
        ));
        let form_audit = context
            .security_audit()
            .into_iter()
            .filter(|record| record.request.action == ConsequentialAction::FormSubmission)
            .collect::<Vec<_>>();
        assert_eq!(form_audit.len(), 2);
        assert!(form_audit[0].confirmation_consumed);
        assert!(!form_audit[1].confirmation_consumed);
        assert_eq!(form_audit[1].decision, PolicyDecision::Allowed);
        assert_eq!(
            form_audit[1].request.target_origin.as_deref(),
            Some("https://app.example")
        );
    }

    #[tokio::test]
    async fn secret_fill_is_origin_scoped_confirmed_and_observation_redacted() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        page.load_html(
            "https://app.example/",
            r#"<label for="pw">API key</label><input id="pw" type="text">"#,
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
        let html = page.extract(ContentKind::Html, None).await.unwrap();
        assert!(!html.to_string().contains("do-not-observe"));
    }

    #[cfg(all(feature = "v8-host", feature = "in-process-probe"))]
    #[tokio::test(flavor = "current_thread")]
    async fn raw_page_inspection_is_audited_without_recording_page_content() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://app.example/", "<p>ready</p>")
            .await
            .unwrap();
        page.evaluate("'page-controlled-sensitive-marker'")
            .await
            .unwrap();

        let record = context.security_audit().pop().unwrap();
        assert_eq!(
            record.request.action,
            ConsequentialAction::RawPageInspection
        );
        let detail = record.request.detail.unwrap();
        assert!(detail.starts_with("evaluate:sha256:"));
        assert!(!detail.contains("page-controlled-sensitive-marker"));
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
    async fn popup_global_close_alias_closes_its_current_page() {
        let context = Browser::new().new_context();
        let opener = context.new_page();
        opener
            .load_html(
                "https://app.example.test/start",
                "<script>globalThis.child=window.open()</script>",
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
                "<button onclick='close()'>Close popup</button>",
            )
            .await
            .expect("load popup");
        assert_eq!(
            popup
                .evaluate("typeof close+'|'+closed")
                .await
                .expect("inspect global close alias"),
            "function|false"
        );
        let target = popup
            .observe()
            .await
            .expect("observe popup")
            .nodes
            .into_iter()
            .find(|node| node.name == "Close popup")
            .expect("close control")
            .semantic_ref;
        let result = popup
            .execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .expect("close popup through global alias");
        assert!(result.effects.iter().any(
            |effect| matches!(effect, Effect::PageClosed { page_id } if *page_id == popup.page_id())
        ));
        assert_eq!(
            opener
                .evaluate("__oryn_drain(false);String(child.closed)")
                .await
                .expect("poll popup closure in opener"),
            "true"
        );
        assert!(
            context
                .pages()
                .iter()
                .find(|page| page.page.page_id == popup.page_id())
                .is_some_and(|page| page.status == PageStatus::Closed)
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
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Fixture: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
              xhr.onload=()=>document.querySelector('#result').textContent=xhr.response.name+'|'+xhr.getResponseHeader('x-fixture')+'|'+states.join(',');
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
              const headers=new Headers({'X-Fixture':'one'});headers.append('x-fixture','two');
              const form=new FormData(document.querySelector('#form'));
              form.append('upload',new Blob(['payload'],{type:'text/plain'}),'proof.txt');
              const request=new Request('/submit',{method:'POST',headers,body:form});
              const response=Response.json({accepted:true},{status:201,headers:{'X-Result':'ready'}});
              const file=new File(['hello'],'hello.txt',{type:'text/plain',lastModified:7});
              Promise.all([response.json(),file.text()]).then(([body,text])=>{
                document.querySelector('#result').textContent=[request.method,request.headers.get('x-fixture'),request.body.get('name'),request.body.get('enabled'),request.body.get('upload').name,response.status,response.headers.get('x-result'),body.accepted,file.name,file.type,file.lastModified,text].join('|');
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
        assert_eq!(first_rect.width, 134.0);
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
            "134"
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
        let table_box = page.box_model(table).await.expect("bounded table geometry");
        assert!(table_box.border_box.width > 0.0);
        assert!(table_box.border_box.height > 0.0);
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
                #zero { box-sizing:border-box; width:0; height:0; padding:0; border:0 }
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
        let semantic = page
            .observe_semantics(ProjectionProfile::Full)
            .await
            .expect("observe structured semantics");
        assert!(
            semantic
                .accessibility
                .iter()
                .all(|node| node.name != "Secret")
        );
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_paint_images_do_not_hold_the_page_actor() {
        struct SlowImage {
            state: Arc<(std::sync::Mutex<(bool, bool)>, std::sync::Condvar)>,
        }
        impl NetworkBroker for SlowImage {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                let (lock, wake) = &*self.state;
                let mut state = lock.lock().unwrap();
                state.0 = true;
                wake.notify_all();
                while !state.1 {
                    state = wake.wait(state).unwrap();
                }
                Ok(NetworkResponse {
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "image/png".into())],
                    body: BASE64
                        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
                        .unwrap(),
                })
            }
        }

        let state = Arc::new((
            std::sync::Mutex::new((false, false)),
            std::sync::Condvar::new(),
        ));
        let context = Browser::new().new_context_with_broker(
            ContextOptions::default(),
            SharedNetworkBroker::new(SlowImage {
                state: state.clone(),
            }),
        );
        let page = context.new_page();
        page.load_html(
            "https://paint.example.test/",
            "<style>img{width:8px;height:8px}</style><img alt=slow src=/slow.png>",
        )
        .await
        .unwrap();
        let paint_page = page.clone();
        let paint = tokio::spawn(async move { paint_page.paint_snapshot().await });
        tokio::task::spawn_blocking({
            let state = state.clone();
            move || {
                let (lock, wake) = &*state;
                let mut value = lock.lock().unwrap();
                while !value.0 {
                    value = wake.wait(value).unwrap();
                }
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_millis(250), page.observe())
            .await
            .expect("observe must not queue behind image I/O")
            .expect("observe while paint is pending");
        {
            let (lock, wake) = &*state;
            let mut value = lock.lock().unwrap();
            value.1 = true;
            wake.notify_all();
        }
        assert_eq!(paint.await.unwrap().unwrap().images.len(), 1);
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

    #[tokio::test]
    async fn page_exposes_confirmed_only_challenge_assessment_and_trace() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html(
            "https://challenge.test/",
            r#"<script src="https://challenges.cloudflare.com/turnstile/v0/api.js"></script><input name="cf-turnstile-response">"#,
        )
        .await
        .expect("load challenge fixture");
        let assessment = page.challenge_assessment().await.expect("assessment");
        assert_eq!(
            assessment.disposition,
            crate::challenge::ChallengeDisposition::Confirmed
        );
        assert!(
            page.trace()
                .await
                .expect("trace")
                .iter()
                .any(|event| matches!(
                    event.kind,
                    TraceEventKind::ChallengeAssessed {
                        disposition: crate::challenge::ChallengeDisposition::Confirmed,
                        ..
                    }
                ))
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test]
    async fn parent_owned_upload_populates_file_list_and_form_data() {
        let mut options = ContextOptions::default();
        options.action_policy.rules.insert(
            ConsequentialAction::FileUpload,
            crate::security::PolicyRule::Allow,
        );
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_options(options);
        let page = context.new_page();
        page.load_html(
            "https://upload.test/",
            "<form id=form><input id=file type=file name=proof></form>",
        )
        .await
        .expect("load upload fixture");
        let handle =
            context.import_upload_as("proof", "proof.txt", "text/plain", b"verified".to_vec());
        let target = page
            .resolve_selector("#file", SemanticAction::Type)
            .await
            .expect("resolve file input");
        page.set_files(target, &[handle], None)
            .await
            .expect("select upload");
        assert_eq!(
            page.evaluate("document.querySelector('#file').files[0].name+':'+new FormData(document.querySelector('#form')).get('proof').name")
                .await
                .expect("inspect selected file"),
            "proof.txt:proof.txt"
        );
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test]
    async fn rendering_opportunity_orders_animation_resize_and_intersection_observers() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        page.load_html(
            "https://observer.test/",
            r#"<div id=box style="width:40px;height:20px"></div><script>
            globalThis.order=[];const box=document.querySelector('#box');
            requestAnimationFrame(()=>order.push('raf'));
            new ResizeObserver(()=>order.push('resize')).observe(box);
            new IntersectionObserver(()=>order.push('intersection')).observe(box);
            </script>"#,
        )
        .await
        .expect("load observer fixture");
        assert_eq!(
            page.evaluate("order.join(',')")
                .await
                .expect("observer order"),
            "raf,resize,intersection"
        );
    }
}
