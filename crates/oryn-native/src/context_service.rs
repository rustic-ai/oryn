#![cfg(unix)]

//! Typed parent services available to a sandboxed page worker.
//!
//! This channel deliberately has no generic filesystem, environment, shell,
//! socket, or secret operation. Networking continues to use the separately
//! policy-enforced network broker; this service coordinates context state.

/// Inputs for load page script classified traced.
struct TracedPageScriptInputs<'a> {
    page_id: PageId,
    requested_url: &'a str,
    module: bool,
    nonce: Option<&'a str>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    integrity: Option<&'a str>,
    cross_origin: Option<&'a str>,
    referrer_source: Option<&'a str>,
    kind: Option<crate::ModuleType>,
    cancellation: Option<&'a AtomicBool>,
    creator: Option<&'a oryn_js::worker_wire::Creator>,
    script_policy: Option<&'a str>,
    parser_inserted: bool,
    fallback_encoding: Option<&'a str>,
    trace: Option<&'a mut Vec<String>>,
    hash_evidence: Option<&'a mut Option<crate::csp::ScriptHashEvidence>>,
}
/// Inputs for load page script with cancel.
struct PageScriptInputs<'a> {
    page_id: PageId,
    requested_url: &'a str,
    module: bool,
    nonce: Option<&'a str>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    integrity: Option<&'a str>,
    cross_origin: Option<&'a str>,
    referrer_source: Option<&'a str>,
    kind: Option<crate::ModuleType>,
    cancellation: Option<&'a AtomicBool>,
    creator: Option<&'a oryn_js::worker_wire::Creator>,
    script_policy: Option<&'a str>,
    parser_inserted: bool,
    fallback_encoding: Option<&'a str>,
}
/// Inputs for load page script for type.
#[cfg(all(test, feature = "in-process-probe"))]
struct TypedScriptInputs<'a> {
    page_id: PageId,
    requested_url: &'a str,
    module: bool,
    nonce: Option<&'a str>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    integrity: Option<&'a str>,
    cross_origin: Option<&'a str>,
    referrer_source: Option<&'a str>,
    kind: Option<crate::ModuleType>,
}
/// Inputs for load page script with referrer.
#[cfg(all(test, feature = "in-process-probe"))]
struct ScriptReferrerInputs<'a> {
    page_id: PageId,
    requested_url: &'a str,
    module: bool,
    nonce: Option<&'a str>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    integrity: Option<&'a str>,
    cross_origin: Option<&'a str>,
    referrer_source: Option<&'a str>,
}
/// Inputs for load page script with options.
#[cfg(all(test, feature = "in-process-probe"))]
struct ScriptOptionsInputs<'a> {
    page_id: PageId,
    requested_url: &'a str,
    module: bool,
    nonce: Option<&'a str>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    integrity: Option<&'a str>,
    cross_origin: Option<&'a str>,
}
/// Inputs for load page script with integrity.
#[cfg(all(test, feature = "in-process-probe"))]
struct IntegrityScriptInputs<'a> {
    page_id: PageId,
    requested_url: &'a str,
    module: bool,
    nonce: Option<&'a str>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    integrity: Option<&'a str>,
}
/// Inputs for authorize inline script kind.
struct InlineScriptAuthorization<'a> {
    page_id: PageId,
    creator: Option<&'a oryn_js::worker_wire::Creator>,
    source: &'a str,
    nonce: Option<&'a str>,
    parser_inserted: bool,
    script_policy: Option<&'a str>,
    speculation_rules: bool,
}
/// Inputs for start http stream with destination.
struct HttpStreamDestinationInputs<'a> {
    page_id: PageId,
    request: HttpStreamRequest,
    destination: crate::csp::CspDestination,
    cors_enabled: bool,
    media_destination: Option<&'a str>,
    xslt: bool,
    preload: bool,
    template_nonce: Option<Option<&'a str>>,
    frozen: Option<ordinary_keepalive::Authority>,
}

use oryn_net::cors::{
    cors_exposed_response_headers, cors_headers_allowed, cors_non_safelisted_headers,
    cors_preflight_required,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpStream},
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub use crate::agent_isolation::{DocumentIsolationPolicy, EmbedderPolicy};
use crate::worker_route::WorkerRoute;
use oryn_common::v3::{PageId, PageLifecycleEvent};
use serde::{Deserialize, Serialize};
use tungstenite::{
    Message, WebSocket,
    client::IntoClientRequest,
    protocol::{CloseFrame, frame::coding::CloseCode},
    stream::MaybeTlsStream,
};
use url::Url;

use crate::{
    network::{
        NetworkBroker, NetworkEvent, NetworkOperation, NetworkRequest, NetworkResponse,
        SharedNetworkBroker, collect_operation_bounded_with_cancel,
        sanitize_parent_response_headers, sanitize_worker_headers,
    },
    runtime::{BrowserContext, RuntimeError},
    security::{ConsequentialAction, PolicyRequest},
    storage::{StorageArea, StorageChange, StorageRequest, StorageResponse, StorageScope},
};

#[path = "context_inventory_wire.rs"]
mod inventory_wire;

pub const CONTEXT_SERVICE_PROTOCOL_VERSION: u32 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NavigationNetworkAuthority {
    Service(oryn_js::navigation::ServiceNavigationAuthority),
    Script(oryn_js::navigation::ScriptNavigationAuthority),
}
impl NavigationNetworkAuthority {
    pub(crate) fn source(
        &self,
        context: &BrowserContext,
        page: PageId,
        target: &oryn_js::worker_wire::Creator,
        url: &str,
    ) -> Result<oryn_js::navigation::FormNavigationSource, RuntimeError> {
        Ok(self.network_source(context, page, target, url)?.0)
    }
    pub(crate) fn network_source(
        &self,
        context: &BrowserContext,
        page: PageId,
        target: &oryn_js::worker_wire::Creator,
        url: &str,
    ) -> Result<(oryn_js::navigation::FormNavigationSource, bool), RuntimeError> {
        match self {
            Self::Service(value) => Ok((
                context.service_navigation_source(value, page, target, url)?,
                true,
            )),
            Self::Script(value) => {
                context.script_navigation_network_source(value, page, target, url)
            }
        }
    }
}

pub const MAX_CONTEXT_FRAME_BYTES: usize = 24 * 1024 * 1024;
// Page bundles use the same ceiling as the default resource-cache entry.
// Wire JSON remains independently bounded, including escape expansion.
const MAX_PAGE_SCRIPT_BYTES: usize = 8 * 1024 * 1024;

fn decode_page_module(
    body: &[u8],
    content_type: Option<&str>,
    module: bool,
    kind: Option<crate::ModuleType>,
    fallback_encoding: Option<&str>,
) -> Result<String, RuntimeError> {
    let Some(kind) = kind else {
        return decode_page_script_with_encoding(body, content_type, module, fallback_encoding);
    };
    let headers = content_type
        .map(|value| vec![("content-type".into(), value.into())])
        .unwrap_or_default();
    if !kind.mime_allowed(&headers) {
        return Err(RuntimeError::Network(
            "module response MIME does not match import type".into(),
        ));
    }
    let (source, _) = oryn_net::decode_resource_text(
        body,
        content_type,
        oryn_net::TextResourceKind::ModuleJavaScript,
    );
    if body.len() > MAX_PAGE_SCRIPT_BYTES || source.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "typed module source".into(),
        });
    }
    Ok(source)
}

#[cfg(all(test, feature = "in-process-probe"))]
fn decode_page_script(
    body: &[u8],
    content_type: Option<&str>,
    module: bool,
) -> Result<String, RuntimeError> {
    decode_page_script_with_encoding(body, content_type, module, None)
}
fn decode_page_script_with_encoding(
    body: &[u8],
    content_type: Option<&str>,
    module: bool,
    fallback_encoding: Option<&str>,
) -> Result<String, RuntimeError> {
    if body.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "page script resource".into(),
        });
    }
    let headers = content_type
        .map(|value| vec![("content-type".into(), value.into())])
        .unwrap_or_default();
    if !oryn_net::script_response_mime_allowed(&headers, module) {
        return Err(RuntimeError::Network(
            "script response blocked by MIME type".into(),
        ));
    }
    let (source, _) = oryn_net::decode_resource_text_with_fallback(
        body,
        content_type,
        if module {
            oryn_net::TextResourceKind::ModuleJavaScript
        } else {
            oryn_net::TextResourceKind::JavaScript
        },
        fallback_encoding,
    );
    if source.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "decoded page script source".into(),
        });
    }
    Ok(source)
}
/// Worker entry scripts require a JavaScript MIME type even for classic
/// workers; the document classic-script permissive MIME rules do not apply.
pub(crate) fn decode_worker_script(
    response: &NetworkResponse,
    _module: bool,
) -> Result<String, RuntimeError> {
    if !(200..=299).contains(&response.status) {
        return Err(RuntimeError::Network(format!(
            "worker script returned HTTP {}",
            response.status
        )));
    }
    if !oryn_net::script_response_mime_allowed(&response.headers, true) {
        return Err(RuntimeError::Network(
            "worker script response requires a JavaScript MIME type".into(),
        ));
    }
    if response.body.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "worker script resource".into(),
        });
    }
    let extracted_type = oryn_common::mime_type::MimeType::from_headers(&response.headers)
        .map(|mime| mime.serialize());
    let content_type = extracted_type.as_deref();
    // Both classic worker scripts and module scripts use UTF-8 decoding;
    // a document's legacy classic-script fallback encoding is not inherited.
    let (source, _) = oryn_net::decode_resource_text(
        &response.body,
        content_type,
        oryn_net::TextResourceKind::ModuleJavaScript,
    );
    if source.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "decoded worker script source".into(),
        });
    }
    Ok(source)
}
const MAX_CONTEXT_OPERATIONS: usize = 32;
// Allow one response-writing cohort in addition to the client's pending cohort:
// a client may receive a reply just before its parent dispatch guard is dropped.
const MAX_PARENT_CONTEXT_OPERATIONS: usize = MAX_CONTEXT_OPERATIONS * 2;

/// Parent-owned limit: worker-side pending tables are not a security boundary.
/// A permit covers dispatch *and* response writing, including error paths.
#[derive(Default)]
struct ContextDispatchLimit(Arc<AtomicUsize>);

struct ContextDispatchPermit(Arc<AtomicUsize>);

impl ContextDispatchLimit {
    fn admit(&self) -> Result<ContextDispatchPermit, RuntimeError> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_PARENT_CONTEXT_OPERATIONS).then_some(active + 1)
            })
            .map_err(|_| RuntimeError::WorkerLimitExceeded {
                resource: "parent context dispatch operations".into(),
            })?;
        Ok(ContextDispatchPermit(self.0.clone()))
    }
}

impl Drop for ContextDispatchPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ParserEncodingPolicyAction {
    Prepare,
    Restore,
    Release,
}

/// Native navigation carries its initiating document across redirects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NavigationPolicySource {
    Document(oryn_js::worker_wire::Creator),
    Prerender {
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    InitialAboutBlank,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ContextServiceRequest {
    XsltTransform {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        request_id: u64,
        request: oryn_js::platform::XsltRequest,
    },
    WebFeatureProfile {
        page_id: PageId,
    },
    ReleaseDeferredContainer {
        page_id: PageId,
        frame: String,
    },
    ReserveDeferredContainer {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        frame: String,
        source: String,
        allow: String,
    },
    PrepareDeferredFetch {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        token: oryn_js::fetch_later::DeferredFetchToken,
        request: oryn_js::platform::PlatformHttpRequest,
        activate_after: Option<std::time::Duration>,
    },
    AcknowledgeDeferredFetch {
        page_id: PageId,
        token: oryn_js::fetch_later::DeferredFetchToken,
    },
    QueryDeferredFetch {
        page_id: PageId,
        token: oryn_js::fetch_later::DeferredFetchToken,
    },
    ActivateDeferredFetch {
        page_id: PageId,
        token: oryn_js::fetch_later::DeferredFetchToken,
    },
    CancelDeferredFetch {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        token: oryn_js::fetch_later::DeferredFetchToken,
    },
    PrepareHyperlinkAudit {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        token: oryn_js::navigation::HyperlinkAuditToken,
        destination: String,
        urls: Vec<String>,
    },
    FinishHyperlinkAudit {
        page_id: PageId,
        token: oryn_js::navigation::HyperlinkAuditToken,
        commit: bool,
    },
    UpdateRealmDocumentUrl {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    },
    ParkInitialDocument {
        page_id: PageId,
    },
    RestoreInitialDocument {
        page_id: PageId,
        token: oryn_js::message_wire::ParkedDocumentToken,
    },
    DiscardInitialDocument {
        page_id: PageId,
        token: oryn_js::message_wire::ParkedDocumentToken,
    },
    PrepareJavascriptNavigation {
        page_id: PageId,
        admission: oryn_js::navigation::ScriptNavigationAdmission,
        javascript_url: String,
        url: String,
        html: String,
    },
    PrepareBlankNavigation {
        page_id: PageId,
        creator: Option<oryn_js::worker_wire::Creator>,
        url: String,
        base_url: Option<String>,
    },
    PrepareServiceLocalNavigation {
        page_id: PageId,
        admission: oryn_js::navigation::ServiceNavigationAdmission,
        url: String,
        blob: bool,
    },
    PrepareInitialLocalNavigation {
        page_id: PageId,
        url: String,
        blob: bool,
    },
    PrepareBlobNavigation {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    },
    PrepareLocalDataNavigation {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    },
    CommitPreparedLocalNavigation {
        page_id: PageId,
        token: String,
    },
    DiscardPreparedLocalNavigation {
        page_id: PageId,
        token: String,
    },
    AuthorizePageDownload {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    },
    QuarantinePageDownload {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
        suggested_filename: String,
        media_type: String,
        body_base64: String,
    },
    NativePlatformQuery {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        request: oryn_js::platform_control::Request,
    },
    SharedMemory {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        request: oryn_js::shared_memory::SharedMemoryRequest,
    },
    DragFeedback {
        page_id: PageId,
        feedback: oryn_js::drag_image::FeedbackBatch,
    },
    NotifyUserActivation {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    TransientUserActivation {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        consume: bool,
    },
    StickyUserActivation {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    ClipboardAuthorizeScript {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        write: bool,
    },
    ClipboardRead {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    ClipboardWrite {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        data: oryn_js::clipboard::ClipboardData,
    },
    /// Trusted automation only: the page JavaScript bridge must reject this operation.
    NativeWindowGroup {
        source_page_id: PageId,
        target_page_id: PageId,
        expected: Option<crate::browsing_groups::GroupToken>,
    },
    CheckNativeTargetNavigation {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        target_page_id: PageId,
        target: oryn_js::worker_wire::Creator,
    },
    UpdateFrameContainerPolicy {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        parent: oryn_js::worker_wire::Creator,
        allow: String,
        source: String,
    },
    CheckFormNavigationPolicy {
        page_id: PageId,
        authority: oryn_js::navigation::FormNavigationAuthority,
        url: String,
    },
    InstallDocumentSourceMetadata {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
        referrer: String,
        meta_csp: Option<String>,
    },
    CaptureDocumentSource {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
        referrer: String,
    },
    DeriveInertDocumentSource {
        page_id: PageId,
        source: oryn_dom::DocumentProvenance,
        url: String,
    },
    DocumentFormAuthority {
        page_id: PageId,
        source: oryn_dom::DocumentProvenance,
    },
    CheckDocumentTargetNavigation {
        page_id: PageId,
        #[serde(default)]
        target_page_id: Option<PageId>,
        source: oryn_dom::DocumentProvenance,
        target: oryn_js::worker_wire::Creator,
    },
    RetargetFormNavigationAuthority {
        page_id: PageId,
        target_page_id: PageId,
        authority: oryn_js::navigation::FormNavigationAuthority,
    },
    IssueFormNavigationAuthority {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        referrer_policy: String,
        source_url: String,
    },
    IssueScriptNavigationAuthority {
        page_id: PageId,
        #[serde(default)]
        target_page_id: Option<PageId>,
        creator: oryn_js::worker_wire::Creator,
        target: oryn_js::worker_wire::Creator,
        url: String,
        referrer_policy: String,
        source_url: String,
    },
    IssueDocumentScriptNavigationAuthority {
        page_id: PageId,
        source: oryn_dom::DocumentProvenance,
        target: oryn_js::worker_wire::Creator,
        url: String,
        referrer_policy: Option<String>,
    },
    ServiceNavigationToScript {
        page_id: PageId,
        admission: oryn_js::navigation::ServiceNavigationAdmission,
        url: String,
    },
    AcquireNavigationNetworkSource {
        page_id: PageId,
        nonce: String,
        target: oryn_js::worker_wire::Creator,
        url: String,
        authority: NavigationNetworkAuthority,
    },
    DiscardNavigationNetworkSource {
        page_id: PageId,
        nonce: String,
    },
    ReadServiceNavigationSource {
        page_id: PageId,
        target: oryn_js::worker_wire::Creator,
        url: String,
        authority: oryn_js::navigation::ServiceNavigationAuthority,
    },
    ReadScriptNavigationSource {
        page_id: PageId,
        target: oryn_js::worker_wire::Creator,
        url: String,
        authority: oryn_js::navigation::ScriptNavigationAuthority,
    },
    NativeWindowTargetName {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        value: Option<String>,
    },
    DisownNativeOpener {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    NativeWindowOpener {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    DiscardNativeFrameOpener {
        page_id: PageId,
        frame: String,
    },
    NavigateRelatedWindow {
        source_page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        target_page_id: PageId,
        target: oryn_js::message_wire::WindowTarget,
        expected: Option<crate::browsing_groups::GroupToken>,
        url: String,
        replace: bool,
    },
    PublishNativeWindowInventory {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        #[serde(deserialize_with = "inventory_wire::deserialize")]
        inventory: BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    },
    ReadRelatedWindow {
        source_page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        target_page_id: PageId,
        target: oryn_js::message_wire::WindowTarget,
        expected: Option<crate::browsing_groups::GroupToken>,
        #[serde(deserialize_with = "inventory_wire::deserialize")]
        local_inventory: BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    },
    ReadNativeWindowOrigin {
        source_page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        target_page_id: PageId,
        target: oryn_js::message_wire::WindowTarget,
        expected: Option<crate::browsing_groups::GroupToken>,
    },
    AutomationWindowStatus {
        source_page_id: PageId,
        target_page_id: PageId,
    },
    MessageWire {
        source_page_id: PageId,
        request: oryn_js::message_wire::MessageRequest,
    },
    WorkerService {
        source_page_id: PageId,
        request: oryn_js::worker_wire::AgentServiceRequest,
    },
    BindFrameWorkerPolicy {
        page_id: PageId,
        frame: String,
        document_generation: u64,
        stream_epoch: u64,
        url: String,
        token: String,
        parent: oryn_js::message_wire::WindowTarget,
        parent_generation: u64,
        parent_epoch: u64,
        container_allow: String,
        container_source: String,
        container_sandbox: Option<String>,
    },
    InheritFrameWorkerPolicy {
        document_url: String,
        page_id: PageId,
        frame: String,
        document_generation: u64,
        stream_epoch: u64,
        parent: oryn_js::message_wire::WindowTarget,
        parent_generation: u64,
        parent_epoch: u64,
        container_allow: String,
        container_source: String,
        container_sandbox: Option<String>,
    },
    NavigationCheckpoint {
        page_id: PageId,
        id: crate::navigation_control::NavigationId,
    },
    NavigationEvent {
        page_id: PageId,
        id: crate::navigation_control::NavigationId,
        event: crate::navigation_control::NavigationEvent,
    },
    UserPrompt {
        page_id: PageId,
        request: oryn_js::prompts::PromptRequest,
    },
    CancelUserPrompt {
        page_id: PageId,
        token: oryn_js::prompts::PromptToken,
    },
    GetReferrerPolicy {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    PrepareOrdinaryKeepalive {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    StartOrdinaryKeepalive {
        page_id: PageId,
        token: String,
    },
    ReleaseOrdinaryKeepalive {
        page_id: PageId,
        token: String,
    },
    HttpStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    ImageStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
        #[serde(default)]
        cors_enabled: bool,
    },
    HttpStreamRead {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        stream_id: u64,
        max_bytes: usize,
    },
    HttpStreamAbort {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        stream_id: u64,
    },
    HttpRequestAbort {
        page_id: PageId,
        request_id: u64,
    },
    Storage {
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        request: StorageRequest,
    },
    OpenPage {
        #[serde(default)]
        hyperlink_audit: Option<oryn_js::navigation::HyperlinkAuditToken>,
        #[serde(default)]
        #[serde(deserialize_with = "inventory_wire::deserialize")]
        local_inventory: BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
        #[serde(default)]
        link_navigation: bool,
        #[serde(default)]
        user_initiated: bool,
        #[serde(default)]
        defer_navigation: bool,
        /// Native Window.open can initialize a newly admitted related root locally.
        #[serde(default)]
        local_related_navigation: bool,
        #[serde(default)]
        selected_target: Option<oryn_js::navigation::NavigationTargetReceipt>,
        #[serde(default)]
        document_source: Option<oryn_dom::DocumentProvenance>,
        opener_page_id: PageId,
        noopener: bool,
        target_name: String,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        url: Option<String>,
        #[serde(default)]
        post: Option<oryn_js::navigation::PostResource>,
        #[serde(default)]
        referrer_policy: Option<String>,
    },
    ActivatePage {
        source_page_id: PageId,
        target_page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        expected: Option<crate::browsing_groups::GroupToken>,
    },
    CommitPage {
        page_id: PageId,
        url: String,
    },
    CommitPreparedLoadHtml {
        page_id: PageId,
        token: String,
        document_url: String,
        headers: Vec<(String, String)>,
        body_sha256: String,
    },
    CommitNavigationResponsePolicy {
        page_id: PageId,
        token: String,
        document_url: String,
        headers: Vec<(String, String)>,
    },
    DiscardNavigationResponsePolicy {
        page_id: PageId,
        token: String,
    },
    InstallDocumentSecurity {
        page_id: PageId,
        document_url: String,
        headers: Vec<(String, String)>,
    },
    ReportRealmCspRequest {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        request: crate::csp::CspRequest,
        #[serde(default)]
        redirected: bool,
        #[serde(default)]
        original_url: Option<String>,
    },
    ParserEncodingPolicy {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        token: String,
        action: ParserEncodingPolicyAction,
    },
    InstallRealmMetadata {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        meta_csp: Option<String>,
        meta_referrer: Option<String>,
    },
    InstallMetaSecurity {
        page_id: PageId,
        serialized: String,
    },
    InstallReferrerPolicy {
        page_id: PageId,
        serialized: String,
    },
    ReportCspViolations {
        page_id: PageId,
        violations: Vec<crate::csp::CspViolation>,
    },
    GetDocumentCookie {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
    },
    SetDocumentCookie {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        value: String,
    },
    GetNativeDocumentAuthority {
        page_id: PageId,
        document_url: String,
    },
    GetSecurityOrigin {
        page_id: PageId,
    },
    /// Materialize one explicitly imported, parent-owned upload. This is a
    /// consequential operation and succeeds only when the context policy has
    /// already allowed file upload; no path or generic file operation exists.
    MaterializeUpload {
        page_id: PageId,
        label: String,
    },
    NavigatePage {
        page_id: PageId,
        url: String,
    },
    /// Authorize a page-originated navigation before the worker mutates its
    /// location. The worker remains responsible for the resulting document
    /// navigation; this call is only the parent policy boundary.
    CaptureCommandNavigationSource {
        page_id: PageId,
        initial_only: bool,
    },
    AuthorizeNavigation {
        page_id: PageId,
        source: NavigationPolicySource,
        url: String,
    },
    /// Authorize the actual default form-submission effect after validation
    /// and the cancelable submit event have completed.
    AuthorizeFormSubmission {
        #[serde(default)]
        report_source: Option<String>,
        page_id: PageId,
        authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        url: String,
        method: String,
    },
    ReadScriptCloseState {
        source_page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        target_page_id: PageId,
        expected: Option<crate::browsing_groups::GroupToken>,
    },
    BeginScriptCloseHistory {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        revision: u64,
        count: usize,
    },
    PublishScriptCloseHistory {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        revision: u64,
        count: usize,
    },
    AbortScriptCloseHistory {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        revision: u64,
        count: usize,
    },
    RollbackAuxiliaryRoot {
        page_id: PageId,
        expected_generation: Option<u64>,
    },
    BeginActionCloseScope {
        page_id: PageId,
        action_id: u64,
    },
    EndActionCloseScope {
        page_id: PageId,
        action_id: u64,
        retain: bool,
    },
    ClosePage {
        source_page_id: PageId,
        target_page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        expected: Option<crate::browsing_groups::GroupToken>,
    },
    PollEvents {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        after_event_id: u64,
        max_events: usize,
    },
    LoadWorkerScript {
        page_id: PageId,
        url: String,
        module: bool,
    },
    CaptureScriptPolicy {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    CaptureScriptSourcePolicy {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        source: oryn_dom::DocumentProvenance,
    },
    ReserveScriptLoad {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    CancelScriptLoad {
        page_id: PageId,
        token: u64,
    },
    ReleaseFrameResponseStream {
        page_id: PageId,
        stream_id: u64,
        policy: String,
    },
    /// Native decoder selection of one already admitted parent occurrence.
    SelectSupportFrameResponse {
        page_id: PageId,
        stream_id: u64,
        creator: oryn_js::worker_wire::Creator,
    },
    ReleaseNetworkResult {
        page_id: PageId,
        creator: Option<oryn_js::worker_wire::Creator>,
        stream_id: Option<u64>,
        socket_id: Option<u64>,
    },
    ValidateStylesheetResponse {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        id: u64,
        url: String,
        body_sha256: [u8; 32],
    },
    ReleaseStylesheetResponse {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        id: u64,
    },
    RunScriptLoad {
        #[serde(default)]
        render_blocking: bool,
        page_id: PageId,
        token: u64,
        request: Box<ContextServiceRequest>,
    },
    LoadScript {
        #[serde(default)]
        fallback_encoding: Option<String>,
        #[serde(default)]
        parser_inserted: bool,
        #[serde(default)]
        script_policy: Option<String>,
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
        module: bool,
        nonce: Option<String>,
        #[serde(default)]
        integrity: Option<String>,
        #[serde(default)]
        cross_origin: Option<String>,
        #[serde(default)]
        initiator_url: Option<String>,
        #[serde(default)]
        referrer_policy: Option<String>,
        #[serde(default)]
        referrer_source: Option<String>,
    },
    LoadTypedModule {
        #[serde(default)]
        nonce: Option<String>,
        #[serde(default)]
        parser_inserted: bool,
        #[serde(default)]
        script_policy: Option<String>,
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
        kind: crate::ModuleType,
        integrity: Option<String>,
        cross_origin: Option<String>,
        initiator_url: Option<String>,
        referrer_policy: Option<String>,
        referrer_source: Option<String>,
    },
    LoadPageResource {
        #[serde(default)]
        download: Option<String>,
        script_authority: Option<oryn_js::navigation::ScriptNavigationAdmission>,
        page_id: PageId,
        initiator: Box<ResourceInitiator>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        url: String,
        destination: String,
        #[serde(default)]
        parent_encoding: Option<String>,
        #[serde(default)]
        initiator_url: Option<String>,
        #[serde(default)]
        referrer_policy: Option<String>,
        #[serde(default)]
        post: Option<oryn_js::navigation::PostResource>,
    },
    AuthorizeTrustedTypes {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        check: oryn_js::trusted_types::PolicyCheck,
    },
    AuthorizeInlineStyle {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        source: String,
        nonce: Option<String>,
        attribute: bool,
    },
    AuthorizeInlineScript {
        #[serde(default)]
        speculation_rules: bool,
        #[serde(default)]
        script_policy: Option<String>,
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        #[serde(default)]
        parser_inserted: bool,
        source: String,
        nonce: Option<String>,
    },
    WebSocketOpen {
        #[serde(default)]
        request_id: u64,
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        url: String,
        protocols: Vec<String>,
    },
    WebSocketSend {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        socket_id: u64,
        frame: WebSocketFrame,
    },
    WebSocketPoll {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        socket_id: u64,
        max_frames: usize,
    },
    WebSocketClose {
        page_id: PageId,
        #[serde(default)]
        creator: Option<oryn_js::worker_wire::Creator>,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    },
    MediaStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
        cors_enabled: bool,
        destination: String,
    },
    ImagePreloadStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
        cors_enabled: bool,
    },
    XsltStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    PrefetchStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    TemplateStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
        nonce: Option<String>,
    },
    GetDocumentDomain {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    },
    PrepareDocumentDomain {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        candidate: String,
    },
    CommitDocumentDomain {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        candidate: String,
        expected_domain: Option<String>,
        expected_affected: Vec<(PageId, oryn_js::worker_wire::Creator)>,
    },
    SettleNavigationExecution {
        page_id: PageId,
        receipt: oryn_js::navigation::NavigationExecutionToken,
        outcome: oryn_js::navigation::NavigationExecutionOutcome,
    },
    ClaimNavigationExecution {
        page_id: PageId,
        receipt: oryn_js::navigation::NavigationExecutionToken,
    },
    AcceptNavigationExecution {
        page_id: PageId,
        receipt: oryn_js::navigation::NavigationExecutionToken,
    },
    RegisterPrerenderFrame {
        page_id: PageId,
        parent: oryn_js::worker_wire::Creator,
        frame: String,
        generation: u64,
        epoch: u64,
        origin: String,
        inherit_origin: bool,
    },
    ReadPrerenderAuthority {
        page_id: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    MarkPrerenderReady {
        page_id: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    MarkPrerenderLoaded {
        page_id: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    ActivatePrerender {
        page_id: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
        predecessor: oryn_js::message_wire::ParkedDocumentToken,
    },
    ActivateBlankPrerender {
        page_id: PageId,
        activation: BlankPrerenderActivation,
    },
    PublishBlankPrerender {
        page_id: PageId,
        activation: BlankPrerenderActivation,
    },
    AcknowledgeBlankPrerender {
        page_id: PageId,
        activation: BlankPrerenderActivation,
    },
    ReservePrerender {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    },
    ReserveBlankPrerender {
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    },
    CancelBlankPrerender {
        page_id: PageId,
        destination: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    CommitPrerenderResponse {
        #[serde(default)]
        response_policy: Option<String>,
        page_id: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
        url: String,
        headers: Vec<(String, String)>,
    },
    CancelPrerender {
        page_id: PageId,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    PreloadResource {
        page_id: PageId,
        token: u64,
        request: Box<ContextServiceRequest>,
    },
    AuthorizePreloadConsumption {
        proof: String,
        request: Box<ContextServiceRequest>,
    },
}

impl ContextServiceRequest {
    /// Literal variant only: never serialize payloads, grants, URLs, or text.
    fn diagnostic_operation(&self) -> &'static str {
        match self {
            Self::XsltTransform { .. } => "XsltTransform",
            Self::WebFeatureProfile { .. } => "WebFeatureProfile",
            Self::ReleaseDeferredContainer { .. } => "ReleaseDeferredContainer",
            Self::ReserveDeferredContainer { .. } => "ReserveDeferredContainer",
            Self::PrepareDeferredFetch { .. } => "PrepareDeferredFetch",
            Self::AcknowledgeDeferredFetch { .. } => "AcknowledgeDeferredFetch",
            Self::QueryDeferredFetch { .. } => "QueryDeferredFetch",
            Self::ActivateDeferredFetch { .. } => "ActivateDeferredFetch",
            Self::CancelDeferredFetch { .. } => "CancelDeferredFetch",
            Self::PrepareHyperlinkAudit { .. } => "PrepareHyperlinkAudit",
            Self::FinishHyperlinkAudit { .. } => "FinishHyperlinkAudit",
            Self::UpdateRealmDocumentUrl { .. } => "UpdateRealmDocumentUrl",
            Self::ParkInitialDocument { .. } => "ParkInitialDocument",
            Self::RestoreInitialDocument { .. } => "RestoreInitialDocument",
            Self::DiscardInitialDocument { .. } => "DiscardInitialDocument",
            Self::PrepareJavascriptNavigation { .. } => "PrepareJavascriptNavigation",
            Self::PrepareBlankNavigation { .. } => "PrepareBlankNavigation",
            Self::PrepareServiceLocalNavigation { .. } => "PrepareServiceLocalNavigation",
            Self::PrepareInitialLocalNavigation { .. } => "PrepareInitialLocalNavigation",
            Self::PrepareBlobNavigation { .. } => "PrepareBlobNavigation",
            Self::PrepareLocalDataNavigation { .. } => "PrepareLocalDataNavigation",
            Self::CommitPreparedLocalNavigation { .. } => "CommitPreparedLocalNavigation",
            Self::DiscardPreparedLocalNavigation { .. } => "DiscardPreparedLocalNavigation",
            Self::AuthorizePageDownload { .. } => "AuthorizePageDownload",
            Self::QuarantinePageDownload { .. } => "QuarantinePageDownload",
            Self::NativePlatformQuery { request, .. } => match request {
                oryn_js::platform_control::Request::StorageAccess { .. } => {
                    "NativePlatformQuery.StorageAccess"
                }
                oryn_js::platform_control::Request::Storage { .. } => "NativePlatformQuery.Storage",
                oryn_js::platform_control::Request::StorageEvents { .. } => {
                    "NativePlatformQuery.StorageEvents"
                }
                oryn_js::platform_control::Request::CspEvents { .. } => {
                    "NativePlatformQuery.CspEvents"
                }
                oryn_js::platform_control::Request::BluetoothDevices => {
                    "NativePlatformQuery.BluetoothDevices"
                }
                oryn_js::platform_control::Request::BluetoothTestingState => {
                    "NativePlatformQuery.BluetoothTestingState"
                }
                oryn_js::platform_control::Request::InstallEligibility => {
                    "NativePlatformQuery.InstallEligibility"
                }
                oryn_js::platform_control::Request::UiLocaleData { .. } => {
                    "NativePlatformQuery.UiLocaleData"
                }
                oryn_js::platform_control::Request::UiLocale => "NativePlatformQuery.UiLocale",
                oryn_js::platform_control::Request::PromptBegin { .. } => {
                    "NativePlatformQuery.PromptBegin"
                }
                oryn_js::platform_control::Request::PromptAnswer { .. } => {
                    "NativePlatformQuery.PromptAnswer"
                }
                oryn_js::platform_control::Request::PromptCancel { .. } => {
                    "NativePlatformQuery.PromptCancel"
                }
                oryn_js::platform_control::Request::Media { .. } => "NativePlatformQuery.Media",
                oryn_js::platform_control::Request::Permission { .. } => {
                    "NativePlatformQuery.Permission"
                }
                oryn_js::platform_control::Request::Geolocation => {
                    "NativePlatformQuery.Geolocation"
                }
                oryn_js::platform_control::Request::DocumentFocus => {
                    "NativePlatformQuery.DocumentFocus"
                }
                oryn_js::platform_control::Request::WindowState => {
                    "NativePlatformQuery.WindowState"
                }
                oryn_js::platform_control::Request::RegisterProtocol { .. } => {
                    "NativePlatformQuery.RegisterProtocol"
                }
                oryn_js::platform_control::Request::UnregisterProtocol { .. } => {
                    "NativePlatformQuery.UnregisterProtocol"
                }
                oryn_js::platform_control::Request::WindowPosition => {
                    "NativePlatformQuery.WindowPosition"
                }
                oryn_js::platform_control::Request::PermissionsPolicySnapshot => {
                    "NativePlatformQuery.PermissionsPolicySnapshot"
                }
                oryn_js::platform_control::Request::PromptResolve { .. } => {
                    "NativePlatformQuery.PromptResolve"
                }
            },
            Self::SharedMemory { .. } => "SharedMemory",
            Self::DragFeedback { .. } => "DragFeedback",
            Self::NotifyUserActivation { .. } => "NotifyUserActivation",
            Self::TransientUserActivation { .. } => "TransientUserActivation",
            Self::StickyUserActivation { .. } => "StickyUserActivation",
            Self::ClipboardAuthorizeScript { .. } => "ClipboardAuthorizeScript",
            Self::ClipboardRead { .. } => "ClipboardRead",
            Self::ClipboardWrite { .. } => "ClipboardWrite",
            Self::NativeWindowGroup { .. } => "NativeWindowGroup",
            Self::CheckNativeTargetNavigation { .. } => "CheckNativeTargetNavigation",
            Self::UpdateFrameContainerPolicy { .. } => "UpdateFrameContainerPolicy",
            Self::CheckFormNavigationPolicy { .. } => "CheckFormNavigationPolicy",
            Self::InstallDocumentSourceMetadata { .. } => "InstallDocumentSourceMetadata",
            Self::CaptureDocumentSource { .. } => "CaptureDocumentSource",
            Self::DeriveInertDocumentSource { .. } => "DeriveInertDocumentSource",
            Self::DocumentFormAuthority { .. } => "DocumentFormAuthority",
            Self::CheckDocumentTargetNavigation { .. } => "CheckDocumentTargetNavigation",
            Self::RetargetFormNavigationAuthority { .. } => "RetargetFormNavigationAuthority",
            Self::IssueFormNavigationAuthority { .. } => "IssueFormNavigationAuthority",
            Self::IssueScriptNavigationAuthority { .. } => "IssueScriptNavigationAuthority",
            Self::IssueDocumentScriptNavigationAuthority { .. } => {
                "IssueDocumentScriptNavigationAuthority"
            }
            Self::ServiceNavigationToScript { .. } => "ServiceNavigationToScript",
            Self::AcquireNavigationNetworkSource { .. } => "AcquireNavigationNetworkSource",
            Self::DiscardNavigationNetworkSource { .. } => "DiscardNavigationNetworkSource",
            Self::ReadServiceNavigationSource { .. } => "ReadServiceNavigationSource",
            Self::ReadScriptNavigationSource { .. } => "ReadScriptNavigationSource",
            Self::NativeWindowTargetName { .. } => "NativeWindowTargetName",
            Self::DisownNativeOpener { .. } => "DisownNativeOpener",
            Self::NativeWindowOpener { .. } => "NativeWindowOpener",
            Self::DiscardNativeFrameOpener { .. } => "DiscardNativeFrameOpener",
            Self::NavigateRelatedWindow { .. } => "NavigateRelatedWindow",
            Self::PublishNativeWindowInventory { .. } => "PublishNativeWindowInventory",
            Self::ReadRelatedWindow { .. } => "ReadRelatedWindow",
            Self::ReadNativeWindowOrigin { .. } => "ReadNativeWindowOrigin",
            Self::AutomationWindowStatus { .. } => "AutomationWindowStatus",
            Self::MessageWire { .. } => "MessageWire",
            Self::WorkerService { .. } => "WorkerService",
            Self::BindFrameWorkerPolicy { .. } => "BindFrameWorkerPolicy",
            Self::InheritFrameWorkerPolicy { .. } => "InheritFrameWorkerPolicy",
            Self::NavigationCheckpoint { .. } => "NavigationCheckpoint",
            Self::NavigationEvent { .. } => "NavigationEvent",
            Self::UserPrompt { .. } => "UserPrompt",
            Self::CancelUserPrompt { .. } => "CancelUserPrompt",
            Self::GetReferrerPolicy { .. } => "GetReferrerPolicy",
            Self::PrepareOrdinaryKeepalive { .. } => "PrepareOrdinaryKeepalive",
            Self::StartOrdinaryKeepalive { .. } => "StartOrdinaryKeepalive",
            Self::ReleaseOrdinaryKeepalive { .. } => "ReleaseOrdinaryKeepalive",
            Self::HttpStreamStart { .. } => "HttpStreamStart",
            Self::ImageStreamStart { .. } => "ImageStreamStart",
            Self::HttpStreamRead { .. } => "HttpStreamRead",
            Self::HttpStreamAbort { .. } => "HttpStreamAbort",
            Self::HttpRequestAbort { .. } => "HttpRequestAbort",
            Self::Storage { .. } => "Storage",
            Self::OpenPage { .. } => "OpenPage",
            Self::ActivatePage { .. } => "ActivatePage",
            Self::CommitPage { .. } => "CommitPage",
            Self::CommitPreparedLoadHtml { .. } => "CommitPreparedLoadHtml",
            Self::CommitNavigationResponsePolicy { .. } => "CommitNavigationResponsePolicy",
            Self::DiscardNavigationResponsePolicy { .. } => "DiscardNavigationResponsePolicy",
            Self::InstallDocumentSecurity { .. } => "InstallDocumentSecurity",
            Self::ReportRealmCspRequest { .. } => "ReportRealmCspRequest",
            Self::ParserEncodingPolicy { .. } => "ParserEncodingPolicy",
            Self::InstallRealmMetadata { .. } => "InstallRealmMetadata",
            Self::InstallMetaSecurity { .. } => "InstallMetaSecurity",
            Self::InstallReferrerPolicy { .. } => "InstallReferrerPolicy",
            Self::ReportCspViolations { .. } => "ReportCspViolations",
            Self::GetDocumentCookie { .. } => "GetDocumentCookie",
            Self::SetDocumentCookie { .. } => "SetDocumentCookie",
            Self::GetNativeDocumentAuthority { .. } => "GetNativeDocumentAuthority",
            Self::GetSecurityOrigin { .. } => "GetSecurityOrigin",
            Self::MaterializeUpload { .. } => "MaterializeUpload",
            Self::NavigatePage { .. } => "NavigatePage",
            Self::CaptureCommandNavigationSource { .. } => "CaptureCommandNavigationSource",
            Self::AuthorizeNavigation { .. } => "AuthorizeNavigation",
            Self::AuthorizeFormSubmission { .. } => "AuthorizeFormSubmission",
            Self::ReadScriptCloseState { .. } => "ReadScriptCloseState",
            Self::BeginScriptCloseHistory { .. } => "BeginScriptCloseHistory",
            Self::PublishScriptCloseHistory { .. } => "PublishScriptCloseHistory",
            Self::AbortScriptCloseHistory { .. } => "AbortScriptCloseHistory",
            Self::RollbackAuxiliaryRoot { .. } => "RollbackAuxiliaryRoot",
            Self::BeginActionCloseScope { .. } => "BeginActionCloseScope",
            Self::EndActionCloseScope { .. } => "EndActionCloseScope",
            Self::ClosePage { .. } => "ClosePage",
            Self::PollEvents { .. } => "PollEvents",
            Self::LoadWorkerScript { .. } => "LoadWorkerScript",
            Self::CaptureScriptPolicy { .. } => "CaptureScriptPolicy",
            Self::CaptureScriptSourcePolicy { .. } => "CaptureScriptSourcePolicy",
            Self::ReserveScriptLoad { .. } => "ReserveScriptLoad",
            Self::CancelScriptLoad { .. } => "CancelScriptLoad",
            Self::ReleaseFrameResponseStream { .. } => "ReleaseFrameResponseStream",
            Self::SelectSupportFrameResponse { .. } => "SelectSupportFrameResponse",
            Self::ReleaseNetworkResult { .. } => "ReleaseNetworkResult",
            Self::ValidateStylesheetResponse { .. } => "ValidateStylesheetResponse",
            Self::ReleaseStylesheetResponse { .. } => "ReleaseStylesheetResponse",
            Self::RunScriptLoad { .. } => "RunScriptLoad",
            Self::LoadScript { .. } => "LoadScript",
            Self::LoadTypedModule { .. } => "LoadTypedModule",
            Self::LoadPageResource { .. } => "LoadPageResource",
            Self::AuthorizeTrustedTypes { .. } => "AuthorizeTrustedTypes",
            Self::AuthorizeInlineStyle { .. } => "AuthorizeInlineStyle",
            Self::AuthorizeInlineScript { .. } => "AuthorizeInlineScript",
            Self::WebSocketOpen { .. } => "WebSocketOpen",
            Self::WebSocketSend { .. } => "WebSocketSend",
            Self::WebSocketPoll { .. } => "WebSocketPoll",
            Self::WebSocketClose { .. } => "WebSocketClose",
            Self::MediaStreamStart { .. } => "MediaStreamStart",
            Self::ImagePreloadStreamStart { .. } => "ImagePreloadStreamStart",
            Self::XsltStreamStart { .. } => "XsltStreamStart",
            Self::PrefetchStreamStart { .. } => "PrefetchStreamStart",
            Self::TemplateStreamStart { .. } => "TemplateStreamStart",
            Self::GetDocumentDomain { .. } => "GetDocumentDomain",
            Self::PrepareDocumentDomain { .. } => "PrepareDocumentDomain",
            Self::CommitDocumentDomain { .. } => "CommitDocumentDomain",
            Self::SettleNavigationExecution { .. } => "SettleNavigationExecution",
            Self::ClaimNavigationExecution { .. } => "ClaimNavigationExecution",
            Self::AcceptNavigationExecution { .. } => "AcceptNavigationExecution",
            Self::RegisterPrerenderFrame { .. } => "RegisterPrerenderFrame",
            Self::ReadPrerenderAuthority { .. } => "ReadPrerenderAuthority",
            Self::MarkPrerenderReady { .. } => "MarkPrerenderReady",
            Self::MarkPrerenderLoaded { .. } => "MarkPrerenderLoaded",
            Self::ActivatePrerender { .. } => "ActivatePrerender",
            Self::ActivateBlankPrerender { .. } => "ActivateBlankPrerender",
            Self::PublishBlankPrerender { .. } => "PublishBlankPrerender",
            Self::AcknowledgeBlankPrerender { .. } => "AcknowledgeBlankPrerender",
            Self::ReservePrerender { .. } => "ReservePrerender",
            Self::ReserveBlankPrerender { .. } => "ReserveBlankPrerender",
            Self::CancelBlankPrerender { .. } => "CancelBlankPrerender",
            Self::CommitPrerenderResponse { .. } => "CommitPrerenderResponse",
            Self::CancelPrerender { .. } => "CancelPrerender",
            Self::PreloadResource { .. } => "PreloadResource",
            Self::AuthorizePreloadConsumption { .. } => "AuthorizePreloadConsumption",
        }
    }

    /// Only legacy script shims use JSON dispatch. Every other operation must
    /// arrive through a typed native producer; adding a protocol variant never
    /// silently makes it author-callable.
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn author_compatibility_operation(&self) -> bool {
        matches!(
            self,
            Self::Storage { .. }
                | Self::GetDocumentCookie { .. }
                | Self::SetDocumentCookie { .. }
                | Self::GetSecurityOrigin { .. }
                | Self::PollEvents { .. }
                | Self::WebSocketOpen { .. }
                | Self::WebSocketSend { .. }
                | Self::WebSocketPoll { .. }
                | Self::WebSocketClose { .. }
                | Self::HttpStreamRead { .. }
                | Self::HttpStreamAbort { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebSocketFrame {
    Text { value: String },
    Binary { base64: String },
    Close { code: Option<u16>, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpStreamRequest {
    #[serde(default)]
    pub upgrade_mixed_image: bool,
    #[serde(default)]
    pub fetch: Option<oryn_js::platform::FetchOptions>,
    #[serde(default)]
    pub client: Option<oryn_js::worker_wire::Creator>,
    #[serde(default)]
    pub use_cors_preflight: bool,
    pub request_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_base64: String,
    #[serde(default = "default_http_credentials")]
    pub credentials: bool,
    #[serde(default)]
    pub credentials_mode: Option<String>,
    #[serde(default)]
    pub initiator_url: Option<String>,
    #[serde(default)]
    pub referrer_policy: Option<String>,
}

const fn default_http_credentials() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ContextServiceResponse {
    SupportFrameResponseSelected { selected: bool },
    XsltTransformed {
        transform: oryn_js::platform::XsltResult,
    },
    WebFeatureProfile {
        profile: crate::feature_profile::NativeFeatureProfile,
    },
    DeferredFetchState {
        #[serde(rename = "outcome")]
        result: Result<
            oryn_js::fetch_later::DeferredFetchState,
            oryn_js::fetch_later::DeferredFetchError,
        >,
    },
    DeferredFetchResult {
        #[serde(rename = "outcome")]
        result: Result<(), oryn_js::fetch_later::DeferredFetchError>,
    },
    HyperlinkAuditPrepared,
    HyperlinkAuditFinished,
    NativeWindowInventoryPublished,
    RelatedWindow {
        snapshot: RelatedWindowSnapshot,
    },
    InitialDocumentParked {
        token: oryn_js::message_wire::ParkedDocumentToken,
    },
    RealmDocumentUrlUpdated,
    InitialDocumentRestored,
    InitialDocumentDiscarded,
    BlankNavigationPrepared {
        token: String,
        body_base64: String,
        media_type: String,
        headers: Vec<(String, String)>,
        serialized: String,
        opaque_key: Option<String>,
        base_url: String,
    },
    BlobNavigationPrepared {
        token: String,
        body_base64: String,
        media_type: String,
        headers: Vec<(String, String)>,
        serialized: String,
        opaque_key: Option<String>,
    },
    LocalDataNavigationPrepared {
        token: String,
        body_base64: String,
        media_type: String,
        headers: Vec<(String, String)>,
        serialized: String,
        opaque_key: Option<String>,
    },
    LocalNavigationCommitted {
        serialized: String,
        opaque_key: Option<String>,
        policy: crate::csp::CspPolicySet,
    },
    LocalNavigationDiscarded,
    PageDownloadAuthorized,
    DownloadQuarantined {
        download: crate::download::DownloadDescriptor,
    },
    NativePlatformResult {
        #[serde(rename = "outcome")]
        result: Result<oryn_js::platform_control::Response, String>,
    },
    SharedMemory(oryn_js::shared_memory::SharedMemoryResponse),
    CapturedScriptPolicy {
        authority: String,
    },
    ScriptLoadReserved {
        token: u64,
    },
    ScriptLoadCancelled,
    ScriptLoadCancellationPending,
    NetworkResultReleased,
    DragFeedbackPublished,
    UserActivationRecorded {
        #[serde(rename = "outcome")]
        result: Result<(), String>,
    },
    TransientUserActivation {
        #[serde(rename = "outcome")]
        result: Result<bool, String>,
    },
    ClipboardScriptAuthorized {
        #[serde(rename = "outcome")]
        result: Result<(), oryn_js::clipboard::ClipboardError>,
    },
    ClipboardRead {
        #[serde(rename = "outcome")]
        result: Result<oryn_js::clipboard::ClipboardData, oryn_js::clipboard::ClipboardError>,
    },
    ClipboardWrite {
        #[serde(rename = "outcome")]
        result: Result<(), oryn_js::clipboard::ClipboardError>,
    },
    OpenerDisowned,
    WindowTargetName {
        value: String,
    },
    WindowOpener {
        opener: Option<(PageId, oryn_js::message_wire::WindowTarget)>,
    },
    FormNavigationPolicyChecked {
        source: oryn_js::navigation::FormNavigationSource,
    },
    ScriptNavigationAuthority {
        authority: oryn_js::navigation::ScriptNavigationAuthority,
    },
    NavigationNetworkSource {
        source: oryn_js::navigation::FormNavigationSource,
        bound: bool,
    },
    NavigationNetworkSourceDiscarded,
    ScriptNavigationSource {
        source: oryn_js::navigation::FormNavigationSource,
    },
    DocumentSource {
        source: oryn_dom::DocumentProvenance,
    },
    FormNavigationAuthority {
        authority: oryn_js::navigation::FormNavigationAuthority,
    },
    NativeWindowGroup {
        token: Option<crate::browsing_groups::GroupToken>,
        opener: Option<PageId>,
        opener_target: oryn_js::message_wire::WindowTarget,
        live: bool,
    },
    AutomationWindowStatus {
        live: bool,
    },
    MessageWire {
        #[serde(rename = "outcome")]
        result: Result<oryn_js::message_wire::MessageResponse, String>,
    },
    WorkerService {
        #[serde(rename = "outcome")]
        result:
            Result<oryn_js::worker_wire::AgentServiceResponse, oryn_js::worker_wire::WorkerError>,
    },
    FrameWorkerPolicyBound {
        serialized: String,
        opaque_key: Option<String>,
        hide_nonces: bool,
        #[serde(default)]
        automatic_features_allowed: bool,
    },
    NavigationControl {
        #[serde(rename = "outcome")]
        result: Result<(), crate::navigation_control::NavigationStop>,
    },
    UserPrompt {
        response: Result<oryn_js::prompts::PromptResponse, oryn_js::prompts::PromptError>,
    },
    UserPromptCancelled,
    ReferrerPolicy {
        value: String,
    },
    OrdinaryKeepalivePrepared {
        token: String,
    },
    OrdinaryKeepaliveReleased,
    HttpStreamStarted {
        #[serde(default)]
        timing: Option<oryn_js::platform::ResourceTimingMetadata>,
        #[serde(default)]
        internal_response: Option<oryn_js::platform::InternalResponseMetadata>,
        #[serde(default)]
        response_type: Option<oryn_js::platform::FetchResponseType>,
        upload: Option<oryn_net::UploadProgress>,
        cors_filtered: bool,
        url: String,
        redirected: bool,
        stream_id: u64,
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        total_bytes: Option<usize>,
    },
    HttpStreamChunk {
        encoded_body_size: Option<u64>,
        stream_id: u64,
        body_base64: String,
        done: bool,
    },
    HttpStreamAborted,
    HttpRequestAborted,
    Storage {
        response: Box<StorageResponse>,
    },
    PageOpened {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blank_prerender: Option<BlankPrerenderActivation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_frame: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_target: Option<oryn_js::navigation::NavigationTargetReceipt>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        related_root: Option<RelatedRootAdmission>,
        /// Present only when this new related root owns its initial navigation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        related_navigation_url: Option<String>,
        page_id: PageId,
        created: bool,
        opener_page_id: Option<PageId>,
    },
    PageDenied {
        reason: String,
    },
    PageActivated {
        page_id: PageId,
    },
    ScriptCloseState {
        closing: bool,
    },
    ScriptCloseHistoryPublished,
    ActionCloseScopeUpdated,
    PageClosed,
    PageCommitted,
    DocumentSecurityInstalled,
    NavigationResponsePolicyDiscarded,
    CspViolationsAccepted,
    DocumentCookie {
        value: String,
    },
    DocumentCookieSet,
    CommandNavigationSource {
        creator: oryn_js::worker_wire::Creator,
    },
    NativeDocumentAuthority {
        serialized: String,
        opaque_key: Option<String>,
        secure_context: bool,
        document_generation: u64,
        initial_document: bool,
    },
    SecurityOrigin {
        serialized: String,
        opaque: bool,
    },
    UploadMaterialized {
        filename: String,
        media_type: String,
        body_base64: String,
        last_modified: u64,
    },
    NavigationQueued,
    NavigationAuthorized,
    FormSubmissionAuthorized,
    Events {
        events: Vec<ContextEventRecord>,
    },
    WorkerScript {
        url: String,
        source: String,
        module: bool,
        #[serde(default)]
        referrer_policy: String,
    },
    ScriptLoadFailed {
        failure: crate::script_load_failure::ScriptLoadFailure,
    },
    ScriptSource {
        #[serde(default)]
        timing: Option<oryn_js::platform::ResourceTimingMetadata>,
        url: String,
        source: String,
        module: bool,
        muted_errors: bool,
        referrer_policy: String,
    },
    ScriptAuthorized,
    TrustedTypesAuthorization {
        allowed: bool,
    },
    InlineStyleAuthorization {
        allowed: bool,
    },
    EmbeddedResourceUnavailable,
    PageResourceNoCommit {
        reason: String,
    },
    PageBinaryResource {
        #[serde(default)]
        timing: Option<oryn_js::platform::ResourceTimingMetadata>,
        url: String,
        body_base64: String,
        content_type: Option<String>,
        destination: String,
    },
    PageResource {
        #[serde(default)]
        stylesheet_response: Option<crate::stylesheet_response_proof::StylesheetResponseProof>,
        #[serde(default)]
        inherited_encoding: Option<String>,
        #[serde(default)]
        response_headers: Vec<(String, String)>,
        #[serde(default)]
        raw_body_base64: Option<String>,
        #[serde(default)]
        request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
        #[serde(default)]
        timing: Option<Box<oryn_js::platform::ResourceTimingMetadata>>,
        #[serde(default)]
        content_language: Option<String>,
        #[serde(default)]
        stream_id: Option<u64>,
        #[serde(default)]
        native_origin: Option<oryn_js::worker_wire::AgentOrigin>,
        #[serde(default)]
        effective_post: Option<oryn_js::navigation::PostResource>,
        #[serde(default)]
        worker_policy_token: Option<String>,
        url: String,
        source: String,
        encoding: String,
        #[serde(default)]
        modals_allowed: bool,
        destination: String,
        #[serde(default)]
        content_type: Option<String>,
        #[serde(default)]
        document_referrer: String,
        #[serde(default)]
        referrer_policy: String,
        #[serde(default)]
        refresh_url: Option<String>,
    },
    StylesheetResponseValidated {
        origin_clean: Option<bool>,
    },
    StylesheetResponseReleased,
    WebSocketOpened {
        socket_id: u64,
        protocol: String,
    },
    WebSocketSent,
    WebSocketEvents {
        frames: Vec<WebSocketFrame>,
    },
    WebSocketClosed {
        code: u16,
        reason: String,
        was_clean: bool,
    },
    DocumentDomainState {
        relaxed_domain: Option<String>,
        access_token_key: String,
        origin_keyed: bool,
        alias_key: String,
    },
    DocumentDomainPrepared {
        #[serde(rename = "outcome")]
        result: Result<crate::runtime::document_domain::DocumentDomainPreparation, String>,
    },
    NavigationExecutionSettled,
    NavigationExecutionClaimed,
    NavigationExecutionAccepted,
    NavigationExecutionClaimRejected,
    NavigationExecutionRetired,
    InitialDocumentRestorationObsolete,
    PrerenderReserved {
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    BlankPrerenderReserved {
        page_id: PageId,
        admission: RelatedRootAdmission,
        token: oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    },
    PrerenderResponseCommitted {
        policy: oryn_js::worker_wire::AgentPolicy,
        origin: oryn_js::worker_wire::AgentOrigin,
    },
    PrerenderCancelled,
    PrerenderAuthority {
        initiator_page: PageId,
        url: String,
        policy: oryn_js::worker_wire::AgentPolicy,
        origin: oryn_js::worker_wire::AgentOrigin,
    },
    PrerenderReady,
    PrerenderLoaded,
    PrerenderActivated,
    PrerenderPublished,
    PrerenderAcknowledged,
    PrerenderFrameRegistered,
    PrerenderActivationObsolete,
    PreloadedResource {
        proof: Option<String>,
        response: Box<ContextServiceResponse>,
    },
    PreloadConsumptionAuthorized {
        allowed: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEventRecord {
    pub event_id: u64,
    pub event: ContextEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextEvent {
    Storage {
        source_page_id: PageId,
        source_creator: oryn_js::worker_wire::Creator,
        source_url: String,
        targets: Vec<(PageId, oryn_js::worker_wire::Creator)>,
        change: StorageChange,
    },
    Message {
        source_page_id: PageId,
        target_page_id: PageId,
        source_origin: String,
        data: serde_json::Value,
    },
    CspViolation {
        target_page_id: PageId,
        target_creator: Option<oryn_js::worker_wire::Creator>,
        violation: crate::csp::CspViolation,
    },
    CspReport {
        target_page_id: PageId,
        endpoint: String,
        format: crate::csp::CspReportFormat,
        outcome: String,
    },
    PageLifecycle {
        event: PageLifecycleEvent,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct RequestEnvelope {
    /// Diagnostic correlation only. Never consulted for routing or authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagnostic: Option<crate::ipc_progress::context_phases::Stamp>,
    protocol_version: u32,
    route: Option<WorkerRoute>,
    message_id: u64,
    request: ContextServiceRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseReceipt {
    protocol_version: u32,
    route: Option<WorkerRoute>,
    received_id: u64,
    accepted: bool,
}
fn write_response_receipt(
    writer: &mut UnixStream,
    receipt: &ResponseReceipt,
) -> Result<(), RuntimeError> {
    write_context_json_line(writer, receipt)
}

/// Coalesce JSON tokens while the caller owns the transport write lock. Keep
/// buffering bounded even for large requests, and never retry buffered bytes
/// implicitly on drop after a serialization or transport failure.
fn write_context_json_line(
    writer: &mut impl Write,
    value: &impl Serialize,
) -> Result<(), RuntimeError> {
    let mut buffered = std::io::BufWriter::with_capacity(8192, writer);
    let result = (|| {
        serde_json::to_writer(&mut buffered, value)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        buffered
            .write_all(b"\n")
            .and_then(|_| buffered.flush())
            .map_err(|error| RuntimeError::ContextService(error.to_string()))
    })();
    let _ = buffered.into_parts();
    result
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseEnvelope {
    protocol_version: u32,
    route: Option<WorkerRoute>,
    correlation_id: u64,
    response: Option<ContextServiceResponse>,
    error: Option<String>,
}

struct BoundedContextFrame {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedContextFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|size| *size <= self.limit)
            .ok_or_else(|| std::io::Error::other("context response frame limit exceeded"))?;
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, feature = "in-process-probe"))]
fn encode_context_response(
    response: ResponseEnvelope,
    limit: usize,
) -> Result<Vec<u8>, RuntimeError> {
    encode_context_response_status(response, limit).map(|(bytes, _)| bytes)
}
fn encode_context_response_status(
    mut response: ResponseEnvelope,
    limit: usize,
) -> Result<(Vec<u8>, bool), RuntimeError> {
    let mut preserved = true;
    let mut frame = BoundedContextFrame {
        bytes: Vec::new(),
        limit: limit.saturating_sub(1),
    };
    if serde_json::to_writer(&mut frame, &response).is_err() {
        // Nothing has reached the socket. Replace the entire oversized result,
        // retaining authenticated route and correlation rather than truncating JSON.
        preserved = false;
        frame.bytes.clear();
        response.response = None;
        response.error = Some("context-service response exceeded IPC frame budget".into());
        serde_json::to_writer(&mut frame, &response)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
    }
    if limit == 0 {
        return Err(RuntimeError::ProtocolViolation(
            "empty context frame budget".into(),
        ));
    }
    frame
        .bytes
        .try_reserve_exact(1)
        .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
    frame.bytes.push(b'\n');
    Ok((frame.bytes, preserved))
}

fn shared_memory_response(
    response: &mut ContextServiceResponse,
) -> Option<&mut oryn_js::shared_memory::SharedMemoryResponse> {
    match response {
        ContextServiceResponse::SharedMemory(memory) => Some(memory),
        ContextServiceResponse::WorkerService {
            result: Ok(oryn_js::worker_wire::AgentServiceResponse::SharedMemory(memory)),
        } => Some(memory),
        _ => None,
    }
}
fn receive_context_attachment(
    reader: &mut crate::shared_memory_fd::DescriptorReader,
    envelope: &mut ResponseEnvelope,
) -> Result<(), RuntimeError> {
    if let Some(memory) = envelope.response.as_mut().and_then(shared_memory_response)
        && memory.descriptor.is_some()
    {
        memory.native_descriptor = Some(Arc::new(
            reader
                .attachment(envelope.correlation_id)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?,
        ));
    }
    Ok(())
}

pub struct IpcContextClient {
    stream: UnixStream,
    next_message_id: u64,
}

impl IpcContextClient {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            next_message_id: 1,
        }
    }

    pub fn request(
        &mut self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let message_id = self.next_message_id;
        self.next_message_id = self.next_message_id.checked_add(1).ok_or_else(|| {
            RuntimeError::ProtocolViolation("context message identifiers exhausted".into())
        })?;
        let envelope = RequestEnvelope {
            diagnostic: None,
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: None,
            message_id,
            request,
        };
        let encoded = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if encoded.len() > MAX_CONTEXT_FRAME_BYTES {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "context-service IPC frame".into(),
            });
        }
        self.stream
            .write_all(&encoded)
            .and_then(|_| self.stream.write_all(b"\n"))
            .and_then(|_| self.stream.flush())
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
        let mut reader = crate::shared_memory_fd::DescriptorReader::new(
            self.stream
                .try_clone()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?,
        );
        let mut line = Vec::new();
        read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        let mut response: ResponseEnvelope = serde_json::from_slice(&line)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        receive_context_attachment(&mut reader, &mut response)?;
        if response.protocol_version != CONTEXT_SERVICE_PROTOCOL_VERSION
            || response.route.is_some()
            || response.correlation_id != message_id
        {
            return Err(RuntimeError::ProtocolViolation(
                "context-service response correlation mismatch".into(),
            ));
        }
        write_response_receipt(
            &mut self.stream,
            &ResponseReceipt {
                protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                route: None,
                received_id: message_id,
                accepted: true,
            },
        )?;
        if let Some(error) = response.error {
            return Err(RuntimeError::ContextService(error));
        }
        response.response.ok_or_else(|| {
            RuntimeError::ProtocolViolation("context service returned no response".into())
        })
    }
}

#[path = "context_response_custody.rs"]
mod response_custody;
pub(crate) use response_custody::{
    ResponseCleanup, ResponseReclaims, ResponseScope, ResponseSource, agent_actions,
};

pub(crate) trait ContextTransport: Send + Sync {
    fn response_scope(
        &self,
        _route: Option<&WorkerRoute>,
        _request: Option<&ContextServiceRequest>,
    ) -> Result<Option<ResponseScope>, RuntimeError> {
        Ok(None)
    }
    fn request_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        if route.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "context transport does not accept route authority".into(),
            ));
        }
        self.request(request)
    }
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError>;
}

struct ContextDelivery {
    decode_ns: u128,
    ready_at: Instant,
    response: Result<ContextServiceResponse, String>,
}
type ContextReply = Result<ContextDelivery, String>;
type PublicationReply = Result<(), String>;
struct PendingContextReply {
    route: Option<WorkerRoute>,
    response: mpsc::Sender<ContextReply>,
    publication: mpsc::Sender<PublicationReply>,
}
type PendingContextReplies = BTreeMap<u64, PendingContextReply>;
struct ContextReplyWait {
    response: mpsc::Receiver<ContextReply>,
    publication: mpsc::Receiver<PublicationReply>,
    #[cfg(test)]
    before_wait: Option<mpsc::Sender<()>>,
    #[cfg(test)]
    response_consumed: Option<Arc<std::sync::atomic::AtomicBool>>,
}
impl ContextReplyWait {
    fn new(route: Option<WorkerRoute>) -> (PendingContextReply, Self) {
        let (response, received) = mpsc::channel();
        let (publication, published) = mpsc::channel();
        (
            PendingContextReply {
                route,
                response,
                publication,
            },
            Self {
                response: received,
                publication: published,
                #[cfg(test)]
                before_wait: None,
                #[cfg(test)]
                response_consumed: None,
            },
        )
    }
    fn receive(self) -> Result<(ContextDelivery, Result<(), RuntimeError>), RuntimeError> {
        #[cfg(test)]
        if let Some(entered) = self.before_wait {
            let _ = entered.send(());
        }
        // The response is enqueued before ACK publication, but do not wake the
        // actor to consume it until publication is settled. No ACK is omitted.
        let publication = self
            .publication
            .recv()
            .map_err(|_| RuntimeError::ContextService("response receipt channel closed".into()))
            .and_then(|value| value.map_err(RuntimeError::ContextService));
        // Preserve the original outer transport/protocol error precedence.
        let delivered = self
            .response
            .recv()
            .map_err(|_| RuntimeError::ContextService("context response channel closed".into()))?
            .map_err(RuntimeError::ContextService)?;
        #[cfg(test)]
        if let Some(consumed) = self.response_consumed {
            consumed.store(true, Ordering::SeqCst);
        }
        Ok((delivered, publication))
    }
}

#[cfg(test)]
#[path = "context_reply_wait_tests.rs"]
mod context_reply_wait_tests;

struct IpcContextTransport {
    writer: Arc<Mutex<UnixStream>>,
    cancel: UnixStream,
    reader: Option<std::thread::JoinHandle<()>>,
    next_message_id: AtomicU64,
    pending: Arc<Mutex<PendingContextReplies>>,
}

impl IpcContextTransport {
    fn new(client: IpcContextClient) -> Result<Self, RuntimeError> {
        let reader_stream = client
            .stream
            .try_clone()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let cancel = client
            .stream
            .try_clone()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let reader_cancel = cancel
            .try_clone()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let pending = Arc::new(Mutex::new(PendingContextReplies::new()));
        let reader_pending = pending.clone();
        let writer = Arc::new(Mutex::new(client.stream));
        let receipt_writer = writer.clone();
        let reader = std::thread::Builder::new()
            .name("oryn-context-response-reader".into())
            .spawn(move || {
                let mut reader = crate::shared_memory_fd::DescriptorReader::new(reader_stream);
                loop {
                    let mut line = Vec::new();
                    let read = read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES);
                    let Ok(read) = read else { break };
                    if read == 0 {
                        break;
                    }
                    let decode_started = Instant::now();
                    let Ok(mut response) = serde_json::from_slice::<ResponseEnvelope>(&line) else {
                        break;
                    };
                    if receive_context_attachment(&mut reader, &mut response).is_err() {
                        break;
                    }
                    let id = response.correlation_id;
                    let receipt_route = response.route.clone();
                    let sender = reader_pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.remove(&id));
                    let mut accepted = false;
                    let mut publication = None;
                    if let Some(PendingContextReply {
                        route,
                        response: sender,
                        publication: completed,
                    }) = sender
                    {
                        publication = Some(completed);
                        let valid = response.protocol_version == CONTEXT_SERVICE_PROTOCOL_VERSION
                            && response.route == route;
                        let result = if !valid {
                            Err("context response protocol mismatch".into())
                        } else if let Some(error) = response.error {
                            Err(error)
                        } else {
                            response
                                .response
                                .ok_or_else(|| "context service returned no response".into())
                        };
                        accepted = if valid {
                            sender
                                .send(Ok(ContextDelivery {
                                    response: result,
                                    decode_ns: decode_started.elapsed().as_nanos(),
                                    ready_at: Instant::now(),
                                }))
                                .is_ok()
                        } else {
                            let _ = sender.send(Err("context response protocol mismatch".into()));
                            // Protocol failures historically return before receipt I/O.
                            // Wake the failed consumer now; the rejected ACK still follows.
                            if let Some(completed) = publication.take() {
                                let _ = completed
                                    .send(Err("context response protocol mismatch".into()));
                            }
                            false
                        };
                    }
                    let receipt = ResponseReceipt {
                        protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                        route: receipt_route,
                        received_id: id,
                        accepted,
                    };
                    let sent = receipt_writer
                        .lock()
                        .map_err(|_| ())
                        .and_then(|mut writer| {
                            write_response_receipt(&mut writer, &receipt).map_err(|_| ())
                        });
                    if let Some(publication) = publication {
                        let _ = publication.send(
                            sent.map_err(|_| "response receipt was not published".to_string()),
                        );
                    }
                    if sent.is_err() {
                        break;
                    }
                }
                let _ = reader_cancel.shutdown(Shutdown::Both);
                if let Ok(mut pending) = reader_pending.lock() {
                    for (_, reply) in std::mem::take(&mut *pending) {
                        let _ = reply
                            .response
                            .send(Err("context service connection closed".into()));
                        // Dropping publication wakes its waiter with the same outer error.
                    }
                }
            })
            .map_err(|error| {
                RuntimeError::ContextService(format!("context response reader: {error}"))
            })?;
        Ok(Self {
            writer,
            cancel,
            reader: Some(reader),
            next_message_id: AtomicU64::new(client.next_message_id),
            pending,
        })
    }
}

impl Drop for IpcContextTransport {
    fn drop(&mut self) {
        let _ = self.cancel.shutdown(Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl ContextTransport for IpcContextTransport {
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.request_routed(None, request)
    }

    fn request_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let mut rpc =
            crate::ipc_progress::context_rpc::Scope::start(route, request.diagnostic_operation());
        let (reply, wait) = ContextReplyWait::new(route.cloned());
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| RuntimeError::ContextService("context writer lock poisoned".into()))?;
        let message_id = self
            .next_message_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| {
                RuntimeError::ProtocolViolation("context message identifiers exhausted".into())
            })?;
        rpc.admitted(message_id);
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RuntimeError::ContextService("context pending lock poisoned".into()))?;
        if pending.len() >= MAX_CONTEXT_OPERATIONS {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending context operations".into(),
            });
        }
        pending.insert(message_id, reply);
        drop(pending);
        let envelope = RequestEnvelope {
            diagnostic: rpc.diagnostic_stamp(),
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: route.cloned(),
            message_id,
            request,
        };
        let written = write_context_json_line(&mut *writer, &envelope);
        drop(writer);
        if let Err(error) = written {
            let _ = self.cancel.shutdown(Shutdown::Both);
            self.pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&message_id));
            return Err(error);
        }
        rpc.written();
        let (delivered, publication) = wait.receive()?;
        rpc.decoded(delivered.decode_ns);
        rpc.response_at(delivered.ready_at);
        publication?;
        rpc.published();
        let result = delivered.response.map_err(RuntimeError::ContextService);
        rpc.finished(result.is_ok());
        result
    }
}

#[path = "preload_authorization.rs"]
mod preload_authorization;
#[path = "prepared_resource_preload.rs"]
mod prepared_resource_preload;
#[path = "resource_job_scope.rs"]
mod resource_job_scope;
#[path = "resource_timing_scope.rs"]
mod resource_timing_scope;
pub(crate) use prepared_resource_preload::Entry as PreparedResourcePreload;
#[cfg(feature = "v8-host")]
pub(crate) use prepared_resource_preload::Snapshot as ResourcePreloadSnapshot;
pub(crate) use resource_job_scope::ResourceJob;
#[derive(Clone)]
pub struct SharedContextClient(
    Arc<dyn ContextTransport>,
    Option<WorkerRoute>,
    Arc<resource_job_scope::Registry>,
    Arc<prepared_resource_preload::Store>,
    #[cfg(feature = "v8-host")] Option<Arc<DirectActorEndpoint>>,
);
#[cfg(feature = "v8-host")]
struct DirectActorEndpoint {
    context: crate::runtime::WeakBrowserContext,
    route: WorkerRoute,
    actor: WorkerRoute,
}

#[path = "script_cleanup_custody.rs"]
pub(crate) mod script_cleanup_custody;
pub(crate) use crate::http_cleanup_custody;
pub(crate) use crate::stylesheet_response_proof::StylesheetResponseProof;
/// Worker-side ownership of a parent script operation. Dropping the handle
/// queues cancellation on owned IO lanes and never waits for the broker reply.
pub(crate) struct ContextScriptLoad {
    #[cfg(any(test, feature = "v8-host"))]
    pub client: SharedContextClient,
    #[cfg(any(test, feature = "v8-host"))]
    pub page_id: PageId,
    #[cfg(any(test, feature = "v8-host"))]
    pub token: u64,
    _cleanup: script_cleanup_custody::Ticket,
}
impl ContextScriptLoad {
    pub(crate) fn cancel(&self) {
        self._cleanup.cancel();
    }
    #[cfg(test)]
    pub(crate) fn from_reserved(client: SharedContextClient, page_id: PageId, token: u64) -> Self {
        let cleanup = script_cleanup_custody::Ticket::reserve(client.clone(), page_id).unwrap();
        cleanup.track_network_activity().unwrap();
        cleanup.attach(token);
        Self {
            client,
            page_id,
            token,
            _cleanup: cleanup,
        }
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub fn run(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.client.request(ContextServiceRequest::RunScriptLoad {
            render_blocking: false,
            page_id: self.page_id,
            token: self.token,
            request: Box::new(request),
        })
    }
}

/// Caller-owned cancellation spans a complete resource graph. Weak leases do
/// not retain finished requests; cancellation reaches already admitted native IO.
#[derive(Default)]
#[cfg(feature = "v8-host")]
pub(crate) struct ResourceCancellation {
    pub(crate) flag: Arc<AtomicBool>,
    loads: Mutex<Vec<std::sync::Weak<ContextScriptLoad>>>,
}
#[cfg(feature = "v8-host")]
impl ResourceCancellation {
    pub(crate) fn run_resource(
        &self,
        client: &SharedContextClient,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let (page, creator) = match &request {
            ContextServiceRequest::LoadPageResource {
                page_id, initiator, ..
            } => (*page_id, initiator.creator.clone()),
            _ => {
                return Err(RuntimeError::ProtocolViolation(
                    "owned resource request type".into(),
                ));
            }
        };
        self.run(client, page, creator, request)
    }

    pub(crate) fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        let loads = self.loads.lock().unwrap_or_else(|error| error.into_inner());
        for load in loads.iter().filter_map(std::sync::Weak::upgrade) {
            load.cancel();
        }
    }
    pub(crate) fn run(
        &self,
        client: &SharedContextClient,
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        if self.flag.load(Ordering::Acquire) {
            return Err(RuntimeError::Network("resource owner cancelled".into()));
        }
        let load = Arc::new(client.reserve_script_load(page_id, creator)?);
        {
            let mut loads = self.loads.lock().unwrap_or_else(|error| error.into_inner());
            loads.retain(|load| load.strong_count() > 0);
            if self.flag.load(Ordering::Acquire) {
                load.cancel();
                return Err(RuntimeError::Network(
                    "resource cancelled before admission".into(),
                ));
            }
            loads.push(Arc::downgrade(&load));
        }
        let result = load.run(request);
        if self.flag.load(Ordering::Acquire) {
            load.cancel();
        }
        result
    }
}

#[cfg(feature = "v8-host")]
pub(crate) struct ContextHttpCancellation {
    ticket: script_cleanup_custody::Ticket,
}
#[cfg(feature = "v8-host")]
impl ContextHttpCancellation {
    pub(crate) fn cancel(&self) {
        self.ticket.cancel();
    }
    pub(crate) fn finish(&self) {
        self.ticket.finish_if_not_cancelled();
    }
    pub(crate) fn finish_support_network_activity(&self) {
        self.ticket.finish_network_activity();
    }
}
#[cfg(any(test, feature = "v8-host"))]
pub(crate) struct ContextNetworkReply {
    #[cfg(feature = "v8-host")]
    socket: Mutex<Option<u64>>,
    stream: Mutex<Option<u64>>,
    ticket: script_cleanup_custody::Ticket,
    page: PageId,
    creator: Option<oryn_js::worker_wire::Creator>,
}
#[cfg(any(test, feature = "v8-host"))]
impl ContextNetworkReply {
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn status(&self) -> (bool, bool) {
        self.ticket.status()
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn attach(&self, result: &Result<oryn_js::platform::PlatformHostResult, String>) {
        use oryn_js::platform::PlatformHostResult;
        let (stream_id, socket_id) = match result {
            Ok(PlatformHostResult::HttpStarted { stream_id, .. }) => (Some(*stream_id), None),
            Ok(PlatformHostResult::WebSocketOpened { socket_id, .. }) => (None, Some(*socket_id)),
            _ => return,
        };
        *self
            .stream
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = stream_id;
        *self
            .socket
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = socket_id;
        self.ticket
            .attach_request(ContextServiceRequest::ReleaseNetworkResult {
                page_id: self.page,
                creator: self.creator.clone(),
                stream_id,
                socket_id,
            });
    }
    pub(crate) fn attach_stream(&self, stream_id: u64) {
        *self
            .stream
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(stream_id);
        self.ticket
            .attach_request(ContextServiceRequest::ReleaseNetworkResult {
                page_id: self.page,
                creator: self.creator.clone(),
                stream_id: Some(stream_id),
                socket_id: None,
            });
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn socket_id(&self) -> Option<u64> {
        *self
            .socket
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn stream_id(&self) -> Option<u64> {
        *self
            .stream
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub(crate) fn cancel(&self) {
        self.ticket.cancel();
    }
    pub(crate) fn finish(&self) {
        self.ticket.finish_if_not_cancelled();
    }
}

#[cfg(any(test, feature = "v8-host"))]
pub(crate) struct ContextDeferredContainerLease {
    _ticket: script_cleanup_custody::Ticket,
}
impl SharedContextClient {
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn reserve_deferred_container_cleanup(
        &self,
        page: PageId,
        frame: String,
    ) -> Result<ContextDeferredContainerLease, RuntimeError> {
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page)
            .map_err(RuntimeError::ContextService)?;
        ticket.attach_request(ContextServiceRequest::ReleaseDeferredContainer {
            page_id: page,
            frame,
        });
        Ok(ContextDeferredContainerLease { _ticket: ticket })
    }
}
#[cfg(feature = "v8-host")]
pub(crate) struct ContextPrerenderLease {
    ticket: script_cleanup_custody::Ticket,
    page: PageId,
}
#[cfg(feature = "v8-host")]
impl ContextPrerenderLease {
    pub(crate) fn attach(
        &self,
        token: &oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    ) {
        self.ticket
            .attach_request(ContextServiceRequest::CancelPrerender {
                page_id: self.page,
                token: token.clone(),
                generation,
                epoch,
            });
    }
    pub(crate) fn attach_blank(
        &self,
        destination: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    ) {
        self.ticket
            .attach_request(ContextServiceRequest::CancelBlankPrerender {
                page_id: self.page,
                destination,
                token: token.clone(),
                generation,
                epoch,
            });
    }
    pub(crate) fn activated(&self) {
        self.ticket.finish_if_not_cancelled();
    }
}
#[cfg(feature = "v8-host")]
pub(crate) struct ContextLocalNavigationLease {
    ticket: script_cleanup_custody::Ticket,
    page: PageId,
}
#[cfg(feature = "v8-host")]
pub(crate) struct ContextNavigationSourceLease {
    pub(crate) nonce: String,
    _ticket: script_cleanup_custody::Ticket,
}
pub(crate) struct ContextResponsePolicyLease {
    _ticket: script_cleanup_custody::Ticket,
}
#[cfg(feature = "v8-host")]
impl ContextLocalNavigationLease {
    pub(crate) fn attach(&self, token: &str) {
        self.ticket
            .attach_request(ContextServiceRequest::DiscardPreparedLocalNavigation {
                page_id: self.page,
                token: token.into(),
            });
    }
}
#[cfg(any(test, feature = "v8-host"))]
pub(crate) struct ContextFrameResponseLease {
    ticket: script_cleanup_custody::Ticket,
    page: PageId,
}
#[cfg(any(test, feature = "v8-host"))]
impl ContextFrameResponseLease {
    pub(crate) fn attach(&self, stream_id: u64, policy: &str) {
        self.ticket
            .attach_request(ContextServiceRequest::ReleaseFrameResponseStream {
                page_id: self.page,
                stream_id,
                policy: policy.into(),
            });
    }
    pub(crate) fn cancel(&self) {
        self.ticket.cancel();
    }
    pub(crate) fn finish(&self) {
        self.ticket.finish_if_not_cancelled();
    }
}
#[cfg(any(test, feature = "v8-host"))]
impl std::fmt::Debug for ContextFrameResponseLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContextFrameResponseLease")
    }
}
#[cfg(feature = "v8-host")]
pub(crate) struct ContextAuxiliaryRollback {
    ticket: script_cleanup_custody::Ticket,
    target: Option<(SharedContextClient, PageId, Option<u64>)>,
}
#[cfg(feature = "v8-host")]
impl ContextAuxiliaryRollback {
    pub(crate) fn bind(&mut self, client: SharedContextClient, page: PageId) {
        self.ticket.bind_auxiliary(client.clone(), page, None);
        self.target = Some((client, page, None));
    }
    pub(crate) fn set_generation(&mut self, generation: u64) {
        let (client, page, expected) = self.target.as_mut().expect("bound auxiliary");
        self.ticket
            .bind_auxiliary(client.clone(), *page, Some(generation));
        *expected = Some(generation);
    }
    pub(crate) fn revoke(&mut self) -> Result<(), RuntimeError> {
        let (client, page, expected) = self.target.as_ref().expect("bound auxiliary");
        match client.request(ContextServiceRequest::RollbackAuxiliaryRoot {
            page_id: *page,
            expected_generation: *expected,
        })? {
            ContextServiceResponse::PageClosed => {
                self.ticket.finish_if_not_cancelled();
                Ok(())
            }
            _ => Err(RuntimeError::ProtocolViolation(
                "auxiliary rollback response mismatch".into(),
            )),
        }
    }
    pub(crate) fn commit(self) {
        self.ticket.finish_if_not_cancelled();
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelatedRootAdmission {
    #[serde(default)]
    pub direct: bool,
    pub route: WorkerRoute,
    pub actor_route: WorkerRoute,
    pub policy: Option<crate::csp::CspPolicySet>,
    pub isolation: DocumentIsolationPolicy,
}

impl SharedContextClient {
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn drain_resource_timings(&self) -> Vec<resource_timing_scope::Record> {
        self.2.drain_timings(self.1.as_ref())
    }
    #[cfg(test)]
    pub(crate) fn for_test_transport(
        transport: Arc<dyn ContextTransport>,
        route: WorkerRoute,
    ) -> Self {
        Self(
            transport,
            Some(route),
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        )
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn current_route(&self) -> Option<WorkerRoute> {
        self.1.clone()
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn environment_route(&self) -> Option<WorkerRoute> {
        self.1
            .clone()
            .or_else(|| self.4.as_ref().map(|direct| direct.route.clone()))
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn actor_route(&self) -> Option<WorkerRoute> {
        self.4
            .as_ref()
            .map(|direct| direct.actor.clone())
            .or_else(|| self.1.clone())
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn related_network_broker(
        &self,
        admission: &RelatedRootAdmission,
    ) -> Result<Option<crate::network::SharedNetworkBroker>, RuntimeError> {
        if !admission.direct {
            return Ok(None);
        }
        // Inspect the admitted context without creating a second page service.
        // Dropping a temporary service retires that page's navigation controller.
        let context = self.direct_related_context(admission)?;
        Ok(context.shared_network_broker().map(|inner| {
            crate::network::SharedNetworkBroker::new(
                crate::service_network::ServiceNavigationBroker {
                    context: context.downgrade(),
                    page: PageId(admission.route.environment),
                    inner,
                },
            )
        }))
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn related_endpoint(
        &self,
        admission: &RelatedRootAdmission,
    ) -> Result<Self, RuntimeError> {
        if !admission.direct {
            if self.4.is_some() {
                return Err(RuntimeError::ProtocolViolation(
                    "routed admission on direct endpoint".into(),
                ));
            }
            return Ok(self.for_route(admission.route.clone()));
        }
        Ok(Self::direct(
            self.direct_related_context(admission)?,
            PageId(admission.route.environment),
        ))
    }

    #[cfg(feature = "v8-host")]
    fn direct_related_context(
        &self,
        admission: &RelatedRootAdmission,
    ) -> Result<BrowserContext, RuntimeError> {
        let direct = self.4.as_ref().ok_or_else(|| {
            RuntimeError::ProtocolViolation("direct admission on routed endpoint".into())
        })?;
        let context = direct.context.upgrade().ok_or(RuntimeError::Closed)?;
        let page = PageId(admission.route.environment);
        let expected = context.related_root_admission(page).ok_or_else(|| {
            RuntimeError::ProtocolViolation("direct related page admission unavailable".into())
        })?;
        if !admission.direct
            || direct.actor != admission.actor_route
            || expected.route != admission.route
            || expected.actor_route != admission.actor_route
            || !expected.direct
        {
            return Err(RuntimeError::ProtocolViolation(
                "direct related actor capability mismatch".into(),
            ));
        }
        Ok(context)
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn reserve_auxiliary_rollback(
        &self,
        source_page: PageId,
    ) -> Result<ContextAuxiliaryRollback, RuntimeError> {
        Ok(ContextAuxiliaryRollback {
            ticket: script_cleanup_custody::Ticket::reserve(self.clone(), source_page)
                .map_err(RuntimeError::ContextService)?,
            target: None,
        })
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn reserve_prerender_cleanup(
        &self,
        page: PageId,
    ) -> Result<ContextPrerenderLease, RuntimeError> {
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page)
            .map_err(RuntimeError::ContextService)?;
        Ok(ContextPrerenderLease { ticket, page })
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn reserve_local_navigation(
        &self,
        page: PageId,
    ) -> Result<ContextLocalNavigationLease, RuntimeError> {
        Ok(ContextLocalNavigationLease {
            ticket: script_cleanup_custody::Ticket::reserve(self.clone(), page)
                .map_err(RuntimeError::ContextService)?,
            page,
        })
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn reserve_navigation_source(
        &self,
        page: PageId,
    ) -> Result<Arc<ContextNavigationSourceLease>, RuntimeError> {
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page)
            .map_err(RuntimeError::ContextService)?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let nonce = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        // Arm before submitting the publishing query: cancellation may win its reply.
        ticket.attach_request(ContextServiceRequest::DiscardNavigationNetworkSource {
            page_id: page,
            nonce: nonce.clone(),
        });
        Ok(Arc::new(ContextNavigationSourceLease {
            nonce,
            _ticket: ticket,
        }))
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn retain_response_policy(
        &self,
        page: PageId,
        token: &str,
    ) -> Result<Arc<ContextResponsePolicyLease>, RuntimeError> {
        let request = ContextServiceRequest::DiscardNavigationResponsePolicy {
            page_id: page,
            token: token.into(),
        };
        let ticket = match script_cleanup_custody::Ticket::reserve(self.clone(), page) {
            Ok(ticket) => ticket,
            Err(error) => {
                // The response is already admitted. A full cleanup queue must
                // release that exact token before rejecting its consumer.
                match self.request(request)? {
                    ContextServiceResponse::NavigationResponsePolicyDiscarded => {}
                    _ => {
                        return Err(RuntimeError::ProtocolViolation(
                            "unexpected response policy cleanup acknowledgement".into(),
                        ));
                    }
                }
                return Err(RuntimeError::ContextService(error));
            }
        };
        ticket.attach_request(request);
        Ok(Arc::new(ContextResponsePolicyLease { _ticket: ticket }))
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn reserve_navigation_stop(
        &self,
        page: PageId,
        id: crate::navigation_control::NavigationId,
    ) -> Result<ContextNavigationStop, RuntimeError> {
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page)
            .map_err(RuntimeError::ContextService)?;
        ticket.attach_request(ContextServiceRequest::NavigationEvent {
            page_id: page,
            id,
            event: crate::navigation_control::NavigationEvent::Stopped(
                crate::navigation_control::NavigationStop::Failed(
                    "navigation publisher retired before terminal acknowledgement".into(),
                ),
            ),
        });
        Ok(ContextNavigationStop { ticket })
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn reserve_frame_response(
        &self,
        page: PageId,
    ) -> Result<Arc<ContextFrameResponseLease>, RuntimeError> {
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page)
            .map_err(RuntimeError::ContextService)?;
        ticket
            .track_network_activity()
            .map_err(RuntimeError::ContextService)?;
        Ok(Arc::new(ContextFrameResponseLease { ticket, page }))
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn reserve_http_cancellation(
        &self,
        page: PageId,
        request_id: u64,
    ) -> Result<Arc<ContextHttpCancellation>, RuntimeError> {
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page)
            .map_err(RuntimeError::ContextService)?;
        ticket.attach_request(ContextServiceRequest::HttpRequestAbort {
            page_id: page,
            request_id,
        });
        Ok(Arc::new(ContextHttpCancellation { ticket }))
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn reserve_http_network_reply(
        &self,
        page: PageId,
        creator: Option<oryn_js::worker_wire::Creator>,
    ) -> Result<ContextNetworkReply, RuntimeError> {
        let reply = self.reserve_network_reply(page, creator)?;
        reply
            .ticket
            .track_network_activity()
            .map_err(RuntimeError::ContextService)?;
        Ok(reply)
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn reserve_network_reply(
        &self,
        page: PageId,
        creator: Option<oryn_js::worker_wire::Creator>,
    ) -> Result<ContextNetworkReply, RuntimeError> {
        Ok(ContextNetworkReply {
            #[cfg(feature = "v8-host")]
            socket: Mutex::new(None),
            stream: Mutex::new(None),
            ticket: script_cleanup_custody::Ticket::reserve(self.clone(), page)
                .map_err(RuntimeError::ContextService)?,
            page,
            creator,
        })
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn resource_job_activity(&self) -> (usize, Option<std::time::Instant>) {
        self.2.activity(self.1.as_ref())
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn for_document_activity(&self) -> Self {
        Self(
            self.0.clone(),
            self.1.clone(),
            Arc::new(self.2.for_document()),
            Default::default(),
            #[cfg(feature = "v8-host")]
            self.4.clone(),
        )
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn for_child_document_activity(&self) -> Self {
        Self(
            self.0.clone(),
            self.1.clone(),
            Arc::new(self.2.for_child_document()),
            Default::default(),
            #[cfg(feature = "v8-host")]
            self.4.clone(),
        )
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn track_control_job(&self) -> Result<ResourceJob, RuntimeError> {
        self.2
            .acquire_control(self.1.as_ref())
            .map_err(RuntimeError::ContextService)
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn track_resource_job(&self) -> Result<ResourceJob, RuntimeError> {
        self.2
            .acquire(self.1.as_ref())
            .map_err(RuntimeError::ContextService)
    }
    pub(crate) fn retire_resource_jobs(&self, budget: Duration) -> Result<(), RuntimeError> {
        script_cleanup_custody::release_deferred_containers(self);
        self.3.retire();
        self.2
            .retire_and_drain(self.1.as_ref(), budget)
            .map_err(RuntimeError::ContextService)
    }
    pub(crate) fn drain_resource_teardown(&self, budget: Duration) -> Result<(), RuntimeError> {
        let started = std::time::Instant::now();
        let jobs = self.retire_resource_jobs(budget);
        let cleanup = self.drain_script_cleanup(budget.saturating_sub(started.elapsed()));
        jobs.and(cleanup)
    }
    pub(crate) fn drain_script_cleanup(&self, budget: Duration) -> Result<(), RuntimeError> {
        script_cleanup_custody::drain(self, budget).map_err(RuntimeError::ContextService)
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn prepare_parser_encoding_policy(
        &self,
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    ) -> Result<ContextParserEncodingPolicy, RuntimeError> {
        let cleanup = script_cleanup_custody::Ticket::reserve(self.clone(), page_id)
            .map_err(RuntimeError::ContextService)?;
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy)
            .map_err(|_| RuntimeError::ContextService("parser policy entropy".into()))?;
        let token = entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        cleanup.attach_request(ContextServiceRequest::ParserEncodingPolicy {
            page_id,
            creator: creator.clone(),
            token: token.clone(),
            action: ParserEncodingPolicyAction::Release,
        });
        match self.request(ContextServiceRequest::ParserEncodingPolicy {
            page_id,
            creator: creator.clone(),
            token: token.clone(),
            action: ParserEncodingPolicyAction::Prepare,
        })? {
            ContextServiceResponse::DocumentSecurityInstalled => Ok(ContextParserEncodingPolicy {
                client: self.clone(),
                page_id,
                creator,
                token,
                _cleanup: cleanup,
            }),
            _ => Err(RuntimeError::ProtocolViolation(
                "invalid parser encoding checkpoint response".into(),
            )),
        }
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn reserve_script_load(
        &self,
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
    ) -> Result<ContextScriptLoad, RuntimeError> {
        let cleanup = script_cleanup_custody::Ticket::reserve(self.clone(), page_id)
            .map_err(RuntimeError::ContextService)?;
        cleanup
            .track_network_activity()
            .map_err(RuntimeError::ContextService)?;
        match self.request(ContextServiceRequest::ReserveScriptLoad { page_id, creator })? {
            ContextServiceResponse::ScriptLoadReserved { token } => {
                cleanup.attach(token);
                Ok(ContextScriptLoad {
                    client: self.clone(),
                    page_id,
                    token,
                    _cleanup: cleanup,
                })
            }
            _ => Err(RuntimeError::ProtocolViolation(
                "invalid script-load admission response".into(),
            )),
        }
    }
    #[cfg(any(test, feature = "v8-host"))]
    pub(crate) fn begin_resource_preload(
        &self,
        request: ContextServiceRequest,
    ) -> Result<Arc<PreparedResourcePreload>, RuntimeError> {
        let entry = self.3.admit(&request)?;
        if !entry.start() {
            return Ok(entry);
        }
        let (page, creator) = match &request {
            ContextServiceRequest::LoadScript {
                page_id, creator, ..
            } => (*page_id, creator.clone()),
            ContextServiceRequest::LoadPageResource {
                page_id, initiator, ..
            } => (*page_id, initiator.creator.clone()),
            _ => unreachable!("preload key validated"),
        };
        // A cleanup ticket must not own the store that owns that ticket.
        let transport = Self(
            self.0.clone(),
            self.1.clone(),
            self.2.clone(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            self.4.clone(),
        );
        let lease = match transport.reserve_script_load(page, creator) {
            Ok(lease) => lease,
            Err(error) => {
                entry.complete(Err(error));
                return Ok(entry);
            }
        };
        let token = lease.token;
        entry.attach(lease)?;
        let weak = Arc::downgrade(&entry);
        let status = entry.status.clone();
        if let Err(error) = crate::resource_io_executor::submit(move || {
            if weak.upgrade().is_none_or(|entry| entry.cancelled()) {
                return;
            }
            let response = transport.request_with_timing_metadata(
                ContextServiceRequest::PreloadResource {
                    page_id: page,
                    token,
                    request: Box::new(request),
                },
                Some(status),
                Some("link"),
            );
            if let Some(entry) = weak.upgrade() {
                entry.complete(response);
            }
        }) {
            entry.complete(Err(RuntimeError::ContextService(error)));
        }
        Ok(entry)
    }
    #[cfg(feature = "v8-host")]
    pub(crate) fn capture_resource_preloads(&self) -> ResourcePreloadSnapshot {
        self.3.snapshot()
    }
    pub(crate) fn claim_resource_preload(
        &self,
        request: &ContextServiceRequest,
        blocking: bool,
    ) -> Option<Arc<PreparedResourcePreload>> {
        self.3.claim(request, blocking)
    }

    pub fn new(client: IpcContextClient) -> Self {
        Self(
            Arc::new(IpcContextTransport::new(client).expect("context transport initialization")),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            None,
        )
    }

    pub(crate) fn for_route(&self, route: WorkerRoute) -> Self {
        Self(
            self.0.clone(),
            Some(route),
            self.2.clone(),
            self.3.clone(),
            #[cfg(feature = "v8-host")]
            None,
        )
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn direct_page_prompt_controller(
        &self,
        page: PageId,
    ) -> Result<Option<Arc<oryn_js::prompts::PromptController>>, RuntimeError> {
        let Some(direct) = self.4.as_ref() else {
            return Ok(None);
        };
        direct
            .context
            .upgrade()
            .ok_or(RuntimeError::Closed)?
            .page_prompt_controller(page)
            .map(Some)
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn direct(context: BrowserContext, source_page_id: PageId) -> Self {
        let (route, actor) = context
            .direct_actor_identity(source_page_id)
            .expect("native direct actor identity");
        let direct = Arc::new(DirectActorEndpoint {
            context: context.downgrade(),
            route,
            actor,
        });
        Self(
            ParentContextService::new_direct(context, source_page_id).into_shared(),
            None,
            Default::default(),
            Default::default(),
            #[cfg(feature = "v8-host")]
            Some(direct),
        )
    }

    pub fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.request_with_render_status(request, None)
    }
    pub(crate) fn request_with_render_status(
        &self,
        request: ContextServiceRequest,
        status: Option<crate::render_request_status::Status>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.request_with_timing_metadata(request, status, None)
    }
    fn request_with_timing_metadata(
        &self,
        request: ContextServiceRequest,
        status: Option<crate::render_request_status::Status>,
        initiator: Option<&str>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        // Payload-free request-API attempts on the calling actor thread, including
        // preload/cache waits. These are not counts of physical wire exchanges.
        // Nested requests retain exclusive timing; diagnostics never authorize IPC.
        let operation = match &request {
            ContextServiceRequest::NativePlatformQuery { request, .. } => match request {
                oryn_js::platform_control::Request::WindowState => "request.window_state",
                oryn_js::platform_control::Request::WindowPosition => "request.window_position",
                oryn_js::platform_control::Request::CspEvents { .. } => "request.csp_events",
                oryn_js::platform_control::Request::StorageEvents { .. } => {
                    "request.storage_events"
                }
                _ => "request.platform_other",
            },
            _ => "request.context_other",
        };
        let _profile = crate::native_timing::stage(operation);
        crate::native_timing::count(operation, 1);
        let stylesheet_evidence_request = matches!(&request,ContextServiceRequest::LoadPageResource{destination,..}if destination=="stylesheet")
            || matches!(&request,ContextServiceRequest::RunScriptLoad{request,..}if matches!(request.as_ref(),ContextServiceRequest::LoadPageResource{destination,..}if destination=="stylesheet"));
        if initiator != Some("link") && !stylesheet_evidence_request {
            let candidate=match &request{ContextServiceRequest::RunScriptLoad{page_id,token,render_blocking,request}=>self.claim_resource_preload(request,*render_blocking||matches!(request.as_ref(),ContextServiceRequest::LoadPageResource{initiator,..}if initiator.stylesheet.as_ref().is_some_and(|sheet|sheet.render_blocking))).map(|entry|(entry,Some((*page_id,*token)))),ContextServiceRequest::LoadPageResource{initiator,..}=>self.claim_resource_preload(&request,initiator.stylesheet.as_ref().is_some_and(|sheet|sheet.render_blocking)).map(|entry|(entry,None)),_=>None};
            if let Some((entry, unused_lease)) = candidate {
                let response = entry.wait();
                let consumer = match &request {
                    ContextServiceRequest::RunScriptLoad { request, .. } => request.as_ref(),
                    request => request,
                };
                let authorized = if response.is_err() {
                    Ok(entry.same_request(consumer))
                } else if let Some(proof) = entry.proof() {
                    match self.request(ContextServiceRequest::AuthorizePreloadConsumption {
                        proof,
                        request: Box::new(consumer.clone()),
                    }) {
                        Ok(ContextServiceResponse::PreloadConsumptionAuthorized { allowed }) => {
                            Ok(allowed)
                        }
                        Ok(_) => Err(RuntimeError::ProtocolViolation(
                            "invalid preload consumption authorization".into(),
                        )),
                        Err(error) => Err(error),
                    }
                } else {
                    Ok(entry.same_request(consumer)
                        && !matches!(consumer, ContextServiceRequest::LoadScript { .. }))
                };
                match authorized {
                    Ok(false) => self.3.finish_claim(&entry, false),
                    Ok(true) => {
                        self.3.finish_claim(&entry, true);
                        if let Some((page, token)) = unused_lease {
                            script_cleanup_custody::cancel_script(self, page, token);
                        }
                        return response;
                    }
                    Err(error) => {
                        self.3.finish_claim(&entry, false);
                        if let Some((page, token)) = unused_lease {
                            script_cleanup_custody::cancel_script(self, page, token);
                        }
                        return Err(error);
                    }
                }
            }
        }
        let mut timing = resource_timing_scope::observe(&request);
        if let (Some(initiator), Some(resource_timing_scope::Observation::Start(record))) =
            (initiator, &mut timing)
        {
            record.initiator = initiator.into();
        }
        let resource = match &request {
            ContextServiceRequest::PreloadResource {
                page_id,
                token,
                request,
            }
            | ContextServiceRequest::RunScriptLoad {
                page_id,
                token,
                request,
                ..
            } => match request.as_ref() {
                ContextServiceRequest::LoadScript { .. }
                | ContextServiceRequest::LoadTypedModule { .. } => Some((*page_id, *token, false)),
                ContextServiceRequest::LoadPageResource { .. } => Some((*page_id, *token, true)),
                _ => None,
            },
            _ => None,
        };
        let response = self.0.request_routed(self.1.as_ref(), request);
        if let Some(status) = status {
            let blocking = status.finish();
            if let Some(resource_timing_scope::Observation::Start(record)) = &mut timing {
                record.render_blocking = blocking;
            }
        }
        if let Some(timing) = timing {
            self.2.finish_timing(self.1.as_ref(), timing, &response);
        }
        if let Some((page, token, page_resource)) = resource {
            let completed_response = response.as_ref().ok().map(|response| match response {
                ContextServiceResponse::PreloadedResource { response, .. } => response.as_ref(),
                response => response,
            });
            let complete = if page_resource {
                matches!(
                    completed_response,
                    Some(
                        ContextServiceResponse::PageResource {
                            stream_id: None,
                            ..
                        } | ContextServiceResponse::PageBinaryResource { .. }
                            | ContextServiceResponse::PageResourceNoCommit { .. }
                            | ContextServiceResponse::EmbeddedResourceUnavailable
                    )
                )
            } else {
                matches!(
                    completed_response,
                    Some(
                        ContextServiceResponse::ScriptSource { .. }
                            | ContextServiceResponse::ScriptLoadFailed { .. }
                    )
                )
            };
            if complete {
                script_cleanup_custody::finish_script(self, page, token);
            } else if !(page_resource
                && matches!(
                    &response,
                    Ok(ContextServiceResponse::PageResource {
                        stream_id: Some(_),
                        ..
                    })
                ))
            {
                // An error or mismatched response cannot leave a retained
                // prepared-source handle owning an uncancelled request.
                script_cleanup_custody::cancel_script(self, page, token);
            }
        }
        response
    }
}

/// Reserved before a remote navigation is admitted. Local outbox failures and
/// abandoned publishers retain a parent stop request until acknowledgement.
#[cfg(any(test, feature = "v8-host"))]
pub(crate) struct ContextNavigationStop {
    ticket: script_cleanup_custody::Ticket,
}
#[cfg(any(test, feature = "v8-host"))]
impl ContextNavigationStop {
    #[cfg(feature = "v8-host")]
    pub(crate) fn cancel(&self) {
        self.ticket.cancel();
    }
    pub(crate) fn finish(&self) {
        self.ticket.finish_if_not_cancelled();
    }
}

/// Modal requests travel on multiplexed context IPC, independently of the
/// worker's command reader. Resolution uses the parent page's controller.
#[cfg(feature = "v8-host")]
pub(crate) struct NativePromptBackend {
    client: SharedContextClient,
    page_id: PageId,
}
#[cfg(feature = "v8-host")]
pub(crate) struct NativeMessageBackend {
    client: SharedContextClient,
    page_id: PageId,
}
#[cfg(feature = "v8-host")]
pub(crate) struct NativeWorkerBackend {
    client: SharedContextClient,
    page_id: PageId,
}
#[cfg(feature = "v8-host")]
impl NativeWorkerBackend {
    pub(crate) fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::worker_wire::WorkerTransport for NativeWorkerBackend {
    fn request(
        &self,
        request: oryn_js::worker_wire::AgentServiceRequest,
    ) -> Result<oryn_js::worker_wire::AgentServiceResponse, oryn_js::worker_wire::WorkerError> {
        if let oryn_js::worker_wire::AgentServiceRequest::SharedMemory { creator, request } =
            request
        {
            return match self.client.request(ContextServiceRequest::SharedMemory {
                page_id: self.page_id,
                creator,
                request,
            }) {
                Ok(ContextServiceResponse::SharedMemory(memory)) => Ok(
                    oryn_js::worker_wire::AgentServiceResponse::SharedMemory(memory),
                ),
                Ok(_) => Err(oryn_js::worker_wire::WorkerError::new(
                    "InvalidStateError",
                    "shared memory response mismatch",
                )),
                Err(error) => Err(oryn_js::worker_wire::WorkerError::new(
                    "InvalidStateError",
                    error.to_string(),
                )),
            };
        }
        match self.client.request(ContextServiceRequest::WorkerService {
            source_page_id: self.page_id,
            request,
        }) {
            Ok(ContextServiceResponse::WorkerService { result }) => result,
            Ok(_) => Err(oryn_js::worker_wire::WorkerError::new(
                "InvalidStateError",
                "native worker service response mismatch",
            )),
            Err(error) => Err(oryn_js::worker_wire::WorkerError::new(
                "InvalidStateError",
                error.to_string(),
            )),
        }
    }
}

#[cfg(feature = "v8-host")]
impl NativeMessageBackend {
    pub(crate) fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::message_wire::MessageTransport for NativeMessageBackend {
    fn request(
        &self,
        request: oryn_js::message_wire::MessageRequest,
    ) -> Result<oryn_js::message_wire::MessageResponse, String> {
        match self.client.request(ContextServiceRequest::MessageWire {
            source_page_id: self.page_id,
            request,
        }) {
            Ok(ContextServiceResponse::MessageWire { result }) => result,
            Ok(_) => Err("invalid native structured-message response".into()),
            Err(error) => Err(error.to_string()),
        }
    }
}
#[cfg(feature = "v8-host")]
impl NativePromptBackend {
    pub(crate) fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::prompts::PromptBackend for NativePromptBackend {
    fn request(
        &self,
        request: oryn_js::prompts::PromptRequest,
    ) -> Result<oryn_js::prompts::PromptResponse, oryn_js::prompts::PromptError> {
        use oryn_js::prompts::PromptError;
        match self.client.request(ContextServiceRequest::UserPrompt {
            page_id: self.page_id,
            request,
        }) {
            Ok(ContextServiceResponse::UserPrompt { response }) => response,
            _ => Err(PromptError::Retired),
        }
    }
    fn cancel(&self, token: oryn_js::prompts::PromptToken) {
        let _ = self
            .client
            .request(ContextServiceRequest::CancelUserPrompt {
                page_id: self.page_id,
                token,
            });
    }
}

pub(crate) fn finish_websocket_close(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    code: Option<u16>,
    reason: String,
    cancel: impl Fn() -> bool,
) -> (u16, String, bool) {
    let code = code.or_else(|| (!reason.is_empty()).then_some(1000));
    let frame = code.map(|code| CloseFrame {
        code: CloseCode::from(code),
        reason: reason.into(),
    });
    if socket.close(frame).is_err() {
        return (1006, String::new(), false);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cancel() && Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Close(frame)) => {
                let code = frame
                    .as_ref()
                    .map(|frame| u16::from(frame.code))
                    .unwrap_or(1005);
                let reason = frame
                    .map(|frame| frame.reason.to_string())
                    .unwrap_or_default();
                return (code, reason, true);
            }
            Ok(Message::Ping(value)) => {
                if socket.send(Message::Pong(value)).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    (1006, String::new(), false)
}

pub(crate) struct OwnedWebSocket {
    socket: Mutex<WebSocket<MaybeTlsStream<TcpStream>>>,
    interrupt: TcpStream,
}
impl OwnedWebSocket {
    fn new(socket: WebSocket<MaybeTlsStream<TcpStream>>) -> Result<Arc<Self>, RuntimeError> {
        let tcp = match socket.get_ref() {
            MaybeTlsStream::Plain(stream) => stream,
            MaybeTlsStream::Rustls(stream) => &stream.sock,
            _ => {
                return Err(RuntimeError::Network(
                    "unsupported WebSocket TLS stream".into(),
                ));
            }
        };
        let interrupt = tcp
            .try_clone()
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        Ok(Arc::new(Self {
            socket: Mutex::new(socket),
            interrupt,
        }))
    }
    pub(super) fn interrupt(&self) {
        let _ = self.interrupt.shutdown(std::net::Shutdown::Both);
    }
}
impl Drop for OwnedWebSocket {
    fn drop(&mut self) {
        self.interrupt();
    }
}

// A handshake is registered before DNS/connect and remains owned until a
// socket response is committed. Cancellation never waits on a transport mutex.
struct SocketHandshake {
    owner: crate::agent_registry::AgentOwner,
    cancelled: AtomicBool,
    interrupt: Mutex<Option<TcpStream>>,
}
impl SocketHandshake {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let stream = self
            .interrupt
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(stream) = stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}
struct SocketHandshakeLease {
    scope: Arc<SocketLifecycle>,
    id: u64,
    state: Arc<SocketHandshake>,
}
impl SocketHandshakeLease {
    fn begin(
        scope: Arc<SocketLifecycle>,
        id: u64,
        owner: crate::agent_registry::AgentOwner,
    ) -> Result<Self, RuntimeError> {
        if id == 0 {
            return Err(RuntimeError::ProtocolViolation(
                "WebSocket requires native request identity".into(),
            ));
        }
        let retired = scope.retired.lock().map_err(|_| {
            RuntimeError::ContextService("socket retirement registry poisoned".into())
        })?;
        if scope.sealed.load(Ordering::Acquire) || retired.contains(&owner) {
            return Err(RuntimeError::Closed);
        }
        let mut pending = scope.handshakes.lock().map_err(|_| {
            RuntimeError::ContextService("WebSocket handshake registry poisoned".into())
        })?;
        if pending.contains_key(&id) {
            return Err(RuntimeError::ProtocolViolation(
                "WebSocket request identity reused".into(),
            ));
        }
        if pending.len() >= 32 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending WebSocket handshakes".into(),
            });
        }
        let cancelled = scope
            .cancelled_http_requests
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("network cancellation registry poisoned".into())
            })?
            .remove(&id);
        let state = Arc::new(SocketHandshake {
            owner,
            cancelled: AtomicBool::new(cancelled),
            interrupt: Mutex::new(None),
        });
        pending.insert(id, state.clone());
        drop(pending);
        drop(retired);
        Ok(Self { scope, id, state })
    }
    fn check(&self) -> Result<(), RuntimeError> {
        if self.state.cancelled.load(Ordering::Acquire) {
            Err(RuntimeError::Network(
                "WebSocket establishment cancelled".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn attach(&self, stream: &TcpStream) -> Result<(), RuntimeError> {
        let interrupt = stream
            .try_clone()
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let mut slot = self
            .state
            .interrupt
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket interrupt poisoned".into()))?;
        if self.state.cancelled.load(Ordering::Acquire) {
            drop(slot);
            let _ = interrupt.shutdown(std::net::Shutdown::Both);
            return self.check();
        }
        *slot = Some(interrupt);
        Ok(())
    }
}
impl Drop for SocketHandshakeLease {
    fn drop(&mut self) {
        self.scope
            .handshakes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

/// Retained by the service and unacknowledged response custody. Context
/// retirement stores weak scopes, so no context/service reference cycle exists.
pub(crate) struct SocketLifecycle {
    page: PageId,
    handshakes: Mutex<BTreeMap<u64, Arc<SocketHandshake>>>,
    retired: Mutex<BTreeSet<crate::agent_registry::AgentOwner>>,
    sealed: AtomicBool,
    sockets: Arc<Mutex<BTreeMap<u64, Arc<OwnedWebSocket>>>>,
    owners: Arc<Mutex<BTreeMap<u64, oryn_js::worker_wire::Creator>>>,
    audits: Arc<Mutex<BTreeMap<u64, crate::resource_audit::WebSocketObservation>>>,
    streams: Arc<Mutex<BTreeMap<u64, Arc<HttpStreamEntry>>>>,
    http_cleanup: http_cleanup_custody::Scope,
    cancelled_http_requests: Arc<Mutex<BTreeSet<u64>>>,
}
impl SocketLifecycle {
    fn owner(
        &self,
        creator: &oryn_js::worker_wire::Creator,
    ) -> Result<crate::agent_registry::AgentOwner, RuntimeError> {
        match creator {
            oryn_js::worker_wire::Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } => Ok(crate::agent_registry::AgentOwner::Window {
                page: self.page,
                target: target.clone(),
                generation: *document_generation,
                epoch: *stream_epoch,
            }),
            _ => Err(RuntimeError::ProtocolViolation(
                "Window socket requires Window creator".into(),
            )),
        }
    }
    fn admit(
        &self,
        id: u64,
        creator: &oryn_js::worker_wire::Creator,
        socket: WebSocket<MaybeTlsStream<TcpStream>>,
        audit: Option<crate::resource_audit::WebSocketObservation>,
    ) -> Result<(), RuntimeError> {
        let owner = self.owner(creator)?;
        let socket = OwnedWebSocket::new(socket)?;
        let retired = self.retired.lock().map_err(|_| {
            RuntimeError::ContextService("socket retirement registry poisoned".into())
        })?;
        if self.sealed.load(Ordering::Acquire) || retired.contains(&owner) {
            return Err(RuntimeError::Closed);
        }
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("socket registry poisoned".into()))?;
        let mut owners = self
            .owners
            .lock()
            .map_err(|_| RuntimeError::ContextService("socket owner registry poisoned".into()))?;
        let mut audits = self
            .audits
            .lock()
            .map_err(|_| RuntimeError::ContextService("socket audit registry poisoned".into()))?;
        if sockets.len() >= 32 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "live WebSockets".into(),
            });
        }
        if sockets.contains_key(&id) {
            return Err(RuntimeError::ProtocolViolation(
                "socket identity reused".into(),
            ));
        }
        sockets.insert(id, socket);
        owners.insert(id, creator.clone());
        if let Some(audit) = audit {
            audits.insert(id, audit);
        }
        Ok(())
    }
    pub(crate) fn abort_request(&self, id: u64) -> Result<(), RuntimeError> {
        let handshakes = self.handshakes.lock().map_err(|_| {
            RuntimeError::ContextService("WebSocket handshake registry poisoned".into())
        })?;
        if let Some(handshake) = handshakes.get(&id).cloned() {
            drop(handshakes);
            handshake.cancel();
            return Ok(());
        }

        let active = self.http_cleanup.cancel(Some(id));
        if self.sealed.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut cancelled = self.cancelled_http_requests.lock().map_err(|_| {
            RuntimeError::ContextService("HTTP cancellation registry poisoned".into())
        })?;
        if !active {
            if !cancelled.contains(&id) && cancelled.len() >= 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "HTTP cancellation identities".into(),
                });
            }
            cancelled.insert(id);
        }
        drop(cancelled);
        drop(handshakes);
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?;
        let ids = streams
            .iter()
            .filter(|(_, entry)| entry.request_id == id)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let entries = ids
            .into_iter()
            .filter_map(|id| streams.remove(&id))
            .collect::<Vec<_>>();
        drop(streams);
        for entry in entries {
            if let Some(cleanup) = &entry.cleanup {
                cleanup.cancel();
            }
        }
        Ok(())
    }
    pub(crate) fn release_result(
        &self,
        creator: Option<&oryn_js::worker_wire::Creator>,
        stream_id: Option<u64>,
        socket_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        if stream_id.is_some() == socket_id.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "network release requires exactly one capability".into(),
            ));
        }
        if let Some(id) = stream_id {
            let mut streams = self.streams.lock().map_err(|_| {
                RuntimeError::ContextService("HTTP stream registry poisoned".into())
            })?;
            if streams.get(&id).is_some_and(|entry| {
                entry
                    .creator
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    != creator
            }) {
                return Err(RuntimeError::ProtocolViolation(
                    "foreign HTTP result release".into(),
                ));
            }
            let entry = streams.remove(&id);
            drop(streams);
            if let Some(entry) = entry {
                if entry.keepalive_drain.is_some() {
                    orphan_keepalive_response(entry);
                } else if let Some(cleanup) = &entry.cleanup {
                    cleanup.cancel();
                }
            }
        }
        if let Some(id) = socket_id {
            let mut sockets = self
                .sockets
                .lock()
                .map_err(|_| RuntimeError::ContextService("socket registry poisoned".into()))?;
            let mut owners = self.owners.lock().map_err(|_| {
                RuntimeError::ContextService("socket owner registry poisoned".into())
            })?;
            if sockets.contains_key(&id) && owners.get(&id) != creator {
                return Err(RuntimeError::ProtocolViolation(
                    "foreign socket result release".into(),
                ));
            }
            let socket = sockets.remove(&id);
            owners.remove(&id);
            drop(owners);
            drop(sockets);
            if let Some(socket) = socket {
                socket.interrupt();
            }
            if let Some(mut audit) = self
                .audits
                .lock()
                .map_err(|_| RuntimeError::ContextService("socket audit registry poisoned".into()))?
                .remove(&id)
            {
                audit.finish("result_not_received");
            }
        }
        Ok(())
    }
    fn admit_http(&self, id: u64, entry: Arc<HttpStreamEntry>) -> Result<(), RuntimeError> {
        let retired = self.retired.lock().map_err(|_| {
            RuntimeError::ContextService("network retirement registry poisoned".into())
        })?;
        let owner = entry
            .creator
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|creator| self.owner(creator))
            .transpose()?;
        if entry
            .cleanup
            .as_ref()
            .is_some_and(|cleanup| cleanup.cancelled())
        {
            return Err(RuntimeError::Closed);
        }
        if self.sealed.load(Ordering::Acquire)
            || owner.as_ref().is_some_and(|owner| retired.contains(owner))
        {
            if entry.keepalive_drain.is_some() {
                drop(retired);
                orphan_keepalive_response(entry);
                return Ok(());
            }
            return Err(RuntimeError::Closed);
        }
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?;
        if streams.len() >= 32 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending HTTP streams".into(),
            });
        }
        if streams.contains_key(&id) {
            return Err(RuntimeError::ProtocolViolation(
                "HTTP stream identity reused".into(),
            ));
        }
        streams.insert(id, entry);
        Ok(())
    }
    fn bind_frame_stream(
        &self,
        policy: &str,
        creator: oryn_js::worker_wire::Creator,
    ) -> Result<(), RuntimeError> {
        let owner = self.owner(&creator)?;
        let retired = self.retired.lock().map_err(|_| {
            RuntimeError::ContextService("socket retirement registry poisoned".into())
        })?;
        if self.sealed.load(Ordering::Acquire) || retired.contains(&owner) {
            return Err(RuntimeError::Closed);
        }
        let streams = self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?;
        for entry in streams
            .values()
            .filter(|entry| entry.prepared_frame_policy.as_deref() == Some(policy))
        {
            let mut current = entry.creator.lock().map_err(|_| {
                RuntimeError::ContextService("frame response creator poisoned".into())
            })?;
            if current.is_some() {
                return Err(RuntimeError::ProtocolViolation(
                    "frame response stream already bound".into(),
                ));
            }
            *current = Some(creator.clone());
        }
        Ok(())
    }
    pub(crate) fn retire(
        &self,
        owner: Option<&crate::agent_registry::AgentOwner>,
    ) -> Result<(), RuntimeError> {
        if owner.is_some_and(|owner|!matches!(owner,crate::agent_registry::AgentOwner::Window{page,..}if *page==self.page)){return Ok(());}
        let mut retired = self.retired.lock().map_err(|_| {
            RuntimeError::ContextService("socket retirement registry poisoned".into())
        })?;
        if let Some(owner) = owner {
            if retired.len() >= 4096 {
                self.sealed.store(true, Ordering::Release);
            } else {
                retired.insert(owner.clone());
            }
        } else {
            self.sealed.store(true, Ordering::Release);
        }
        let handshakes = self
            .handshakes
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("WebSocket handshake registry poisoned".into())
            })?
            .values()
            .filter(|entry| owner.is_none_or(|owner| owner == &entry.owner))
            .cloned()
            .collect::<Vec<_>>();
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("socket registry poisoned".into()))?;
        let mut owners = self
            .owners
            .lock()
            .map_err(|_| RuntimeError::ContextService("socket owner registry poisoned".into()))?;
        let mut audits = self
            .audits
            .lock()
            .map_err(|_| RuntimeError::ContextService("socket audit registry poisoned".into()))?;
        let ids = owners
            .iter()
            .filter(|(_, creator)| {
                owner.is_none_or(|owner| self.owner(creator).ok().as_ref() == Some(owner))
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut removed = Vec::new();
        let mut observations = Vec::new();
        for id in ids {
            owners.remove(&id);
            if let Some(socket) = sockets.remove(&id) {
                removed.push(socket);
            }
            if let Some(audit) = audits.remove(&id) {
                observations.push(audit);
            }
        }
        drop(audits);
        drop(owners);
        drop(sockets);
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?;
        let ids = streams
            .iter()
            .filter(|(_, entry)| {
                owner.is_none_or(|owner| {
                    entry
                        .creator
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .as_ref()
                        .and_then(|creator| self.owner(creator).ok())
                        .as_ref()
                        == Some(owner)
                })
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let discarded = ids
            .into_iter()
            .filter_map(|id| streams.remove(&id))
            .collect::<Vec<_>>();
        drop(streams);
        drop(retired);
        for handshake in handshakes {
            handshake.cancel();
        }
        for entry in discarded {
            if entry.keepalive_drain.is_some() {
                orphan_keepalive_response(entry);
            } else if let Some(cleanup) = &entry.cleanup {
                cleanup.cancel();
            }
        }
        // TCP close on drop, never a WebSocket close handshake or RPC.
        for socket in &removed {
            socket.interrupt();
        }
        drop(removed);
        for mut audit in observations {
            audit.finish("document_retired");
        }
        Ok(())
    }
    pub(crate) fn is_drained(&self) -> Result<bool, RuntimeError> {
        if !self
            .handshakes
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("WebSocket handshake registry poisoned".into())
            })?
            .is_empty()
        {
            return Ok(false);
        }
        if !self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket registry poisoned".into()))?
            .is_empty()
        {
            return Ok(false);
        }
        if !self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?
            .is_empty()
        {
            return Ok(false);
        }
        // Stream removal is not cancellation acknowledgement. Keep the retired
        // route until its original broker has released every admitted operation.
        self.http_cleanup.is_drained()
    }
    pub(crate) fn drain_cancelled_http(&self, budget: Duration) -> Result<bool, RuntimeError> {
        self.http_cleanup.drain_cancelled(budget)
    }
    pub(crate) fn page(&self) -> PageId {
        self.page
    }
}

#[cfg(feature = "v8-host")]
pub(crate) struct ContextParserEncodingPolicy {
    client: SharedContextClient,
    page_id: PageId,
    creator: oryn_js::worker_wire::Creator,
    token: String,
    _cleanup: script_cleanup_custody::Ticket,
}
#[cfg(feature = "v8-host")]
impl ContextParserEncodingPolicy {
    pub(crate) fn restore(&self) -> Result<(), RuntimeError> {
        match self
            .client
            .request(ContextServiceRequest::ParserEncodingPolicy {
                page_id: self.page_id,
                creator: self.creator.clone(),
                token: self.token.clone(),
                action: ParserEncodingPolicyAction::Restore,
            })? {
            ContextServiceResponse::DocumentSecurityInstalled => Ok(()),
            _ => Err(RuntimeError::ProtocolViolation(
                "invalid parser encoding restore response".into(),
            )),
        }
    }
}

/// Retired-route cleanup holds only the original resource registry and a weak
/// context reference. It cannot admit fresh navigation or mint capabilities;
/// exact previously selected blank receipts may finish their actor transfer.
#[derive(Clone)]
pub(crate) struct NavigationResourceCleanup {
    page: PageId,
    context: crate::runtime::WeakBrowserContext,
    streams: Arc<Mutex<BTreeMap<u64, Arc<HttpStreamEntry>>>>,
}
impl NavigationResourceCleanup {
    pub(crate) fn retire_reaped(&self, reaped_pages: &[PageId]) -> Result<(), RuntimeError> {
        // Stream cancellation/drain belongs to unbind, not grant reclamation.
        if self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?
            .values()
            .any(|entry| entry.page_id == self.page)
        {
            return Err(RuntimeError::ContextService(format!(
                "reaped page {} retains HTTP streams",
                self.page.0
            )));
        }
        if let Some(context) = self.context.upgrade() {
            context.retire_reaped_navigation_resources(self.page, reaped_pages)?;
        }
        Ok(())
    }
    pub(crate) fn acknowledge_blank_prerender(
        &self,
        activation: &BlankPrerenderActivation,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        if let Some(context) = self.context.upgrade() {
            context.acknowledge_selected_blank_prerender(self.page, activation)?;
        }
        Ok(ContextServiceResponse::PrerenderAcknowledged)
    }
    pub(crate) fn activate_blank_prerender(
        &self,
        activation: &BlankPrerenderActivation,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let Some(context) = self.context.upgrade() else {
            return Ok(ContextServiceResponse::PrerenderActivationObsolete);
        };
        match context.activate_selected_blank_prerender(self.page, activation) {
            Ok(crate::runtime::PrerenderActivationAck::Current) => {
                Ok(ContextServiceResponse::PrerenderActivated)
            }
            Ok(crate::runtime::PrerenderActivationAck::Obsolete)
            | Err(RuntimeError::NavigationCancelled) => {
                Ok(ContextServiceResponse::PrerenderActivationObsolete)
            }
            Err(error) => Err(error),
        }
    }
    pub(crate) fn publish_blank_prerender(
        &self,
        activation: &BlankPrerenderActivation,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let Some(context) = self.context.upgrade() else {
            return Ok(ContextServiceResponse::PrerenderActivationObsolete);
        };
        Ok(
            if context.publish_selected_blank_prerender(self.page, activation)? {
                ContextServiceResponse::PrerenderPublished
            } else {
                ContextServiceResponse::PrerenderActivationObsolete
            },
        )
    }
    pub(crate) fn release_parser_encoding(
        &self,
        creator: &oryn_js::worker_wire::Creator,
        token: &str,
    ) -> Result<(), RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "parser cleanup requires Window owner".into(),
            ));
        };
        if let Some(context) = self.context.upgrade() {
            context.parser_encoding_policy(
                crate::agent_registry::AgentOwner::Window {
                    page: self.page,
                    target: target.clone(),
                    generation: *document_generation,
                    epoch: *stream_epoch,
                },
                token,
                2,
            )?;
        }
        Ok(())
    }
    pub(crate) fn cancel_prerender(
        &self,
        token: &oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    ) -> Result<(), RuntimeError> {
        if let Some(context) = self.context.upgrade() {
            context.cancel_native_prerender(self.page, token, (generation, epoch))?;
        }
        Ok(())
    }
    pub(crate) fn cancel_blank_prerender(
        &self,
        destination: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        generation: u64,
        epoch: u64,
    ) -> Result<(), RuntimeError> {
        if let Some(context) = self.context.upgrade() {
            context.cancel_blank_prerender(self.page, destination, token, generation, epoch)?;
        }
        Ok(())
    }
    pub(crate) fn discard_local(&self, token: &str) -> Result<(), RuntimeError> {
        if let Some(context) = self.context.upgrade() {
            context.discard_prepared_local_navigation(self.page, token)?;
        }
        Ok(())
    }
    pub(crate) fn release_frame(&self, stream_id: u64, policy: &str) -> Result<(), RuntimeError> {
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?;
        if streams.get(&stream_id).is_some_and(|entry| {
            entry.page_id != self.page || entry.prepared_frame_policy.as_deref() != Some(policy)
        }) {
            return Err(RuntimeError::ProtocolViolation(
                "foreign frame response release".into(),
            ));
        }
        let entry = streams.remove(&stream_id);
        drop(streams);
        if let Some(entry) = entry
            && let Some(cleanup) = &entry.cleanup
        {
            cleanup.cancel();
        }
        if let Some(context) = self.context.upgrade() {
            context.discard_prepared_frame_policy(self.page, policy)?;
        }
        Ok(())
    }
    pub(crate) fn is_drained(&self) -> Result<bool, RuntimeError> {
        if self
            .streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?
            .values()
            .any(|entry| entry.page_id == self.page && entry.prepared_frame_policy.is_some())
        {
            return Ok(false);
        }
        self.context.upgrade().map_or(Ok(true), |context| {
            context.navigation_resource_grants_drained(self.page)
        })
    }
}

include!("support_creation_bind_import.rs");
include!("support_original_entry.rs");
include!("support_scope_export.rs");
pub struct ParentContextService {
    page_admitted: Mutex<bool>,
    weak_self: std::sync::OnceLock<std::sync::Weak<ParentContextService>>,
    context: crate::runtime::WeakBrowserContext,
    source_page_id: PageId,
    next_socket_id: AtomicU64,
    socket_lifecycle: Arc<SocketLifecycle>,
    sockets: Arc<Mutex<BTreeMap<u64, Arc<OwnedWebSocket>>>>,
    socket_owners: Arc<Mutex<BTreeMap<u64, oryn_js::worker_wire::Creator>>>,
    socket_audits: Arc<Mutex<BTreeMap<u64, crate::resource_audit::WebSocketObservation>>>,
    network: Option<SharedNetworkBroker>,
    next_http_stream_id: AtomicU64,
    ordinary_keepalive: Arc<ordinary_keepalive::Registry>,
    script_loads: crate::script_load_lease::ScriptLoadRegistry,
    script_policies: crate::script_policy_snapshot::ScriptPolicySnapshots,
    preload_authorizations: preload_authorization::Registry,
    preloads: crate::preload_cache::Cache,
    http_streams: Arc<Mutex<BTreeMap<u64, Arc<HttpStreamEntry>>>>,
    http_cleanup: http_cleanup_custody::Scope,
    cancelled_http_requests: Arc<Mutex<BTreeSet<u64>>>,
    authorized_storage_origins: Mutex<BTreeSet<String>>,
    shutting_down: Arc<AtomicBool>,
    trusted_in_process_runtime: bool,
}

pub(crate) fn explicit_websocket_connector(
    url: &Url,
    ca_pem: &[Vec<u8>],
) -> Result<Option<tungstenite::Connector>, RuntimeError> {
    use rustls::pki_types::pem::PemObject;
    if url.scheme() != "wss" || ca_pem.is_empty() {
        return Ok(None);
    }
    let mut roots = rustls::RootCertStore::empty();
    for pem in ca_pem {
        let certificates = rustls::pki_types::CertificateDer::pem_slice_iter(pem);
        for certificate in certificates {
            roots
                .add(
                    certificate.map_err(|_| {
                        RuntimeError::Network("invalid explicit WebSocket CA".into())
                    })?,
                )
                .map_err(|_| RuntimeError::Network("invalid explicit WebSocket CA".into()))?;
        }
    }
    if roots.is_empty() {
        return Err(RuntimeError::Network(
            "empty explicit WebSocket trust".into(),
        ));
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| RuntimeError::Network(error.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Some(tungstenite::Connector::Rustls(Arc::new(config))))
}

fn fixture_websocket_connector(
    url: &Url,
    all_addresses_loopback: bool,
    ca_path: Option<&std::ffi::OsStr>,
) -> Result<Option<tungstenite::Connector>, RuntimeError> {
    use rustls::pki_types::pem::PemObject;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "wss"
        || !all_addresses_loopback
        || !(host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1")
    {
        return Ok(None);
    }
    let Some(path) = ca_path else { return Ok(None) };
    let mut roots = rustls::RootCertStore::empty();
    let certificates = rustls::pki_types::CertificateDer::pem_file_iter(path)
        .map_err(|error| RuntimeError::Network(format!("read WPT WebSocket CA: {error}")))?;
    for certificate in certificates {
        roots
            .add(certificate.map_err(|error| {
                RuntimeError::Network(format!("parse WPT WebSocket CA: {error}"))
            })?)
            .map_err(|error| RuntimeError::Network(format!("invalid WPT WebSocket CA: {error}")))?;
    }
    if roots.is_empty() {
        return Err(RuntimeError::Network("empty WPT WebSocket CA".into()));
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| RuntimeError::Network(error.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Some(tungstenite::Connector::Rustls(Arc::new(config))))
}

struct HttpStreamState {
    preload: Option<crate::preload_cache::Candidate>,
    /// None only after exact terminal consumption; proof custody may remain.
    operation: Option<NetworkOperation>,
    current: bytes::Bytes,
    offset: usize,
    bytes_read: usize,
    content_length: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceInitiator {
    #[serde(default)]
    pub embedded_type: Option<String>,
    #[serde(default)]
    pub request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
    #[serde(default)]
    pub navigation: oryn_js::platform::NavigationRequestFlags,
    #[serde(default)]
    pub stylesheet: Option<StylesheetRequest>,
    pub creator: oryn_js::worker_wire::Creator,
    pub prepared_policy: Option<String>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StylesheetRequest {
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub render_blocking: bool,
    #[serde(default)]
    pub quirks: bool,
    pub nonce: Option<String>,
    pub integrity: Option<String>,
    pub cross_origin: Option<String>,
}
struct PageResourceOptions<'a> {
    response_lease: Option<crate::script_load_lease::ScriptLoadLease>,
    download: Option<&'a str>,
    script_authority: Option<oryn_js::navigation::ScriptNavigationAdmission>,
    initiator: &'a ResourceInitiator,
    form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    parent_encoding: Option<&'a str>,
    post: Option<oryn_js::navigation::PostResource>,
}

struct HttpStreamEntry {
    // Fixed inline proof shares this ORIGINAL 32-slot HTTP admission.
    support: Mutex<Option<crate::support_broker_occurrence::BrokerOccurrence>>,
    keepalive_drain: Option<crate::keepalive_drain::Ticket>,
    keepalive_context: Option<crate::runtime::WeakBrowserContext>,
    page_id: PageId,
    creator: Mutex<Option<oryn_js::worker_wire::Creator>>,
    prepared_frame_policy: Option<String>,
    frame_deadline: Option<std::time::Instant>,
    request_id: u64,
    state: Mutex<HttpStreamState>,
    cleanup: Option<http_cleanup_custody::Ticket>,
}

fn orphan_keepalive_response(entry: Arc<HttpStreamEntry>) {
    let Some(ticket) = entry.keepalive_drain.clone() else {
        return;
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    ticket.orphan(move || {
        if std::time::Instant::now() >= deadline
            || entry
                .cleanup
                .as_ref()
                .is_some_and(|cleanup| cleanup.cancelled())
            || entry.keepalive_context.as_ref().is_some_and(|context| {
                context
                    .upgrade()
                    .is_none_or(|context| !context.keepalive_context_active())
            })
        {
            if let Some(cleanup) = &entry.cleanup {
                cleanup.cancel();
            }
            return true;
        }
        let Ok(mut state) = entry.state.try_lock() else {
            return false;
        };
        let remaining = state.current.len().saturating_sub(state.offset);
        state.bytes_read = state.bytes_read.saturating_add(remaining);
        state.current = bytes::Bytes::new();
        state.offset = 0;
        if state.bytes_read > 16 * 1024 * 1024 {
            if let Some(cleanup) = &entry.cleanup {
                cleanup.cancel();
            }
            return true;
        }
        let Some(operation) = state.operation.as_mut() else { return true; };
        operation.request_body_chunk();
        match operation.try_event() {
            Ok(NetworkEvent::Chunk { request_id, bytes })
                if request_id == operation.request_id =>
            {
                state.current = bytes;
                false
            }
            Ok(NetworkEvent::Complete { request_id, .. })
                if request_id == operation.request_id =>
            {
                if let Some(cleanup) = &entry.cleanup {
                    if state
                        .content_length
                        .is_none_or(|length| length == state.bytes_read)
                    {
                        cleanup.finish_keepalive();
                    } else {
                        cleanup.cancel();
                    }
                }
                true
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => false,
            _ => {
                if let Some(cleanup) = &entry.cleanup {
                    cleanup.cancel();
                }
                true
            }
        }
    });
}

impl Drop for ParentContextService {
    fn drop(&mut self) {
        self.cancel_all_network_operations();
    }
}

fn inherited_frame_encoding(
    parent: &oryn_js::worker_wire::AgentOrigin,
    response_origin: &str,
    response_opaque: bool,
    label: Option<&str>,
) -> Option<String> {
    if parent.opaque || response_opaque || parent.serialized != response_origin {
        return None;
    }
    oryn_net::document_fallback_encoding(label).map(str::to_owned)
}

fn collect_frame_download_body(
    operation: &mut NetworkOperation,
    cleanup: &http_cleanup_custody::Ticket,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, RuntimeError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut body = Vec::new();
    loop {
        if cancelled.load(Ordering::Acquire) || cleanup.cancelled() {
            return Err(RuntimeError::Closed);
        }
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| RuntimeError::Network("frame response deadline".into()))?;
        operation.request_body_chunk();
        match receive_network_event(operation, remaining)? {
            Some(NetworkEvent::Chunk { request_id, bytes })
                if request_id == operation.request_id =>
            {
                if body
                    .len()
                    .checked_add(bytes.len())
                    .is_none_or(|bytes| bytes > 16 * 1024 * 1024)
                    || body.try_reserve(bytes.len()).is_err()
                {
                    return Err(RuntimeError::Network("frame response body limit".into()));
                }
                body.extend_from_slice(&bytes);
            }
            Some(NetworkEvent::Complete {
                encoded_body_size: _,
                request_id,
            }) if request_id == operation.request_id => return Ok(body),
            Some(NetworkEvent::Failed { request_id, error })
                if request_id == operation.request_id =>
            {
                return Err(RuntimeError::from(error));
            }
            _ => {
                return Err(RuntimeError::ProtocolViolation(
                    "frame response body out of order".into(),
                ));
            }
        }
    }
}

fn receive_network_event(
    events: &mut NetworkOperation,
    timeout: Duration,
) -> Result<Option<NetworkEvent>, RuntimeError> {
    receive_network_event_with_cancel(events, timeout, &AtomicBool::new(false))
}
fn receive_network_event_with_cancel(
    events: &mut NetworkOperation,
    timeout: Duration,
    cancellation: &AtomicBool,
) -> Result<Option<NetworkEvent>, RuntimeError> {
    receive_network_event_with_check(events, timeout, cancellation, None)
}
fn receive_network_event_with_check(
    events: &mut NetworkOperation,
    timeout: Duration,
    cancellation: &AtomicBool,
    external: Option<&dyn Fn() -> bool>,
) -> Result<Option<NetworkEvent>, RuntimeError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if cancellation.load(Ordering::Acquire) || external.is_some_and(|check| check()) {
            return Err(RuntimeError::Network("resource owner cancelled".into()));
        }
        match events.try_event() {
            Ok(event) => return Ok(Some(event)),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if std::time::Instant::now() >= deadline {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "network operation wall time".into(),
                    });
                }
                std::thread::park_timeout(Duration::from_millis(1));
            }
        }
    }
}

impl ParentContextService {
    pub(crate) fn navigation_resource_cleanup(&self) -> NavigationResourceCleanup {
        NavigationResourceCleanup {
            page: self.source_page_id,
            context: self.context.clone(),
            streams: self.http_streams.clone(),
        }
    }

    pub(crate) fn network_cleanup_registry(&self) -> Arc<SocketLifecycle> {
        self.socket_lifecycle.clone()
    }
    pub(crate) fn script_cleanup_registry(&self) -> crate::script_load_lease::ScriptLoadRegistry {
        self.script_loads.clone()
    }
    pub(crate) fn navigation_cleanup_control(
        &self,
    ) -> Result<Option<Arc<crate::navigation_control::NavigationControl>>, RuntimeError> {
        match self.context_owner() {
            Ok(context) => context.page_navigation_cleanup_control(self.source_page_id),
            Err(RuntimeError::Closed) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub(crate) fn into_shared(self) -> Arc<Self> {
        let service = Arc::new(self);
        let _ = service.weak_self.set(Arc::downgrade(&service));
        service
    }
    fn context_owner(&self) -> Result<BrowserContext, RuntimeError> {
        self.context.upgrade().ok_or(RuntimeError::Closed)
    }

    pub fn new(context: BrowserContext, source_page_id: PageId) -> Self {
        let network = context.shared_network_broker();
        Self::new_with_network(context, source_page_id, network)
    }

    pub fn new_with_network(
        context: BrowserContext,
        source_page_id: PageId,
        network: Option<SharedNetworkBroker>,
    ) -> Self {
        let service = Self::new_staged_with_network(context, source_page_id, network);
        if service.admit_registered_page().is_err() {
            service.shutting_down.store(true, Ordering::Release);
            service
                .socket_lifecycle
                .sealed
                .store(true, Ordering::Release);
        }
        service
    }

    // Native worker route construction precedes page registry publication.
    // Staging admits no requests; only the exact bound route's post-registration
    // hook can activate its lifecycle. Ordinary constructors remain fail-closed.
    pub(crate) fn new_staged_with_network(
        context: BrowserContext,
        source_page_id: PageId,
        network: Option<SharedNetworkBroker>,
    ) -> Self {
        let authorized_storage_origins = context
            .page_origin(source_page_id)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let sockets = Arc::new(Mutex::new(BTreeMap::new()));
        let socket_owners = Arc::new(Mutex::new(BTreeMap::new()));
        let socket_audits = Arc::new(Mutex::new(BTreeMap::new()));
        let http_streams = Arc::new(Mutex::new(BTreeMap::new()));
        let http_cleanup = http_cleanup_custody::Scope::default();
        let cancelled_http_requests = Arc::new(Mutex::new(BTreeSet::new()));
        let socket_lifecycle = Arc::new(SocketLifecycle {
            page: source_page_id,
            handshakes: Mutex::new(BTreeMap::new()),
            retired: Mutex::new(BTreeSet::new()),
            sealed: AtomicBool::new(false),
            sockets: sockets.clone(),
            owners: socket_owners.clone(),
            audits: socket_audits.clone(),
            streams: http_streams.clone(),
            http_cleanup: http_cleanup.clone(),
            cancelled_http_requests: cancelled_http_requests.clone(),
        });

        Self {
            page_admitted: Mutex::new(false),
            weak_self: Default::default(),
            context: context.downgrade(),
            source_page_id,
            next_socket_id: AtomicU64::new(1),
            sockets,
            socket_owners,
            socket_audits,
            socket_lifecycle: socket_lifecycle.clone(),
            network: network.map(SharedNetworkBroker::new_request_scope),
            next_http_stream_id: AtomicU64::new(1),
            ordinary_keepalive: Default::default(),
            script_loads: Default::default(),
            script_policies: Default::default(),
            preload_authorizations: Default::default(),
            preloads: Default::default(),
            http_streams,
            http_cleanup,
            cancelled_http_requests,
            authorized_storage_origins: Mutex::new(authorized_storage_origins),
            shutting_down: Arc::new(AtomicBool::new(false)),
            trusted_in_process_runtime: false,
        }
    }

    pub(crate) fn admit_registered_page(&self) -> Result<(), RuntimeError> {
        let mut admitted = self
            .page_admitted
            .lock()
            .map_err(|_| RuntimeError::ContextService("page service admission poisoned".into()))?;
        if self.shutting_down.load(Ordering::Acquire)
            || self.socket_lifecycle.sealed.load(Ordering::Acquire)
        {
            return Err(RuntimeError::Closed);
        }
        if *admitted {
            return Ok(());
        }
        self.context_owner()?
            .register_socket_lifecycle(&self.socket_lifecycle)?;
        *admitted = true;
        Ok(())
    }

    pub(crate) fn ensure_page_admitted(&self) -> Result<(), RuntimeError> {
        if self.shutting_down.load(Ordering::Acquire)
            || self.socket_lifecycle.sealed.load(Ordering::Acquire)
            || !*self.page_admitted.lock().map_err(|_| {
                RuntimeError::ContextService("page service admission poisoned".into())
            })?
        {
            return Err(RuntimeError::Closed);
        }
        Ok(())
    }

    pub(crate) fn cancel_all_network_operations(&self) {
        // Serialize retirement with activation; a late registration must never
        // reopen an already revoked worker route.
        {
            let _admitted = self
                .page_admitted
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.shutting_down.store(true, Ordering::Release);
        }
        self.preloads.clear();
        let _ = self.socket_lifecycle.retire(None);
        self.http_cleanup.retire_preserving_keepalive();
        self.ordinary_keepalive.clear();
        self.script_loads.retire();
        self.script_policies.retire();
        self.preload_authorizations.retire();
        if let Ok(context) = self.context_owner() {
            if let Ok(controller) = context.page_prompt_controller(self.source_page_id) {
                controller.retire();
            }
            if let Ok(controller) = context.page_navigation_control(self.source_page_id) {
                let _ = controller.retire();
            }
            let _ = context.retire_queued_navigation_receipts(self.source_page_id);
        }
        let sockets = self
            .sockets
            .lock()
            .map(|mut sockets| std::mem::take(&mut *sockets))
            .unwrap_or_default();
        for socket in sockets.values() {
            socket.interrupt();
        }
        drop(sockets);
        if let Ok(mut owners) = self.socket_owners.lock() {
            owners.clear();
        }
        if let Ok(mut audits) = self.socket_audits.lock() {
            audits.clear();
        }
        let streams = self
            .http_streams
            .lock()
            .map(|mut streams| std::mem::take(&mut *streams))
            .unwrap_or_default();
        drop(streams);
    }

    pub(crate) fn drain_http_cleanup(&self, budget: Duration) -> Result<(), RuntimeError> {
        let started = std::time::Instant::now();
        self.http_cleanup.drain(budget)?;
        if let Some(network) = &self.network {
            network
                .drain_owned_operations(budget.saturating_sub(started.elapsed()))
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
        }
        Ok(())
    }

    fn next_http_identity(&self) -> Result<u64, RuntimeError> {
        self.next_http_stream_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| RuntimeError::WorkerLimitExceeded {
                resource: "HTTP identifiers exhausted".into(),
            })
    }

    #[cfg(feature = "v8-host")]
    fn new_direct(context: BrowserContext, source_page_id: PageId) -> Self {
        let network = context.shared_network_broker();
        let mut service = Self::new_with_network(context, source_page_id, network);
        service.trusted_in_process_runtime = true;
        service
    }

    pub(crate) fn handle(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let details = match &request {
            ContextServiceRequest::BindFrameWorkerPolicy {
                page_id,
                document_generation,
                ..
            } => Some((
                *page_id,
                None,
                Some(*document_generation),
                crate::ipc_progress::Phase::FramePolicyBegin,
                crate::ipc_progress::Phase::FramePolicyReturned,
            )),
            ContextServiceRequest::HttpStreamRead {
                page_id, stream_id, ..
            } => Some((
                *page_id,
                Some(*stream_id),
                None,
                crate::ipc_progress::Phase::ParentBodyBegin,
                crate::ipc_progress::Phase::ParentBodyReturned,
            )),
            _ => None,
        };
        let progress = details.and_then(|(page, _, _, _, _)| crate::ipc_progress::observed(page.0));
        if let Some((_, stream, generation, begin, _)) = details {
            crate::ipc_progress::record(progress, begin, stream, generation);
        }
        let result = self.handle_inner(request);
        if let Some((_, stream, generation, _, end)) = details {
            crate::ipc_progress::result(progress, end, stream, generation, Some(result.is_ok()));
        }
        result
    }

    fn handle_inner(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        if !*self
            .page_admitted
            .lock()
            .map_err(|_| RuntimeError::ContextService("page service admission poisoned".into()))?
        {
            return Err(RuntimeError::ProtocolViolation(
                "page service has not been admitted".into(),
            ));
        }
        if let ContextServiceRequest::NavigationEvent {
            page_id,
            id,
            event: crate::navigation_control::NavigationEvent::Stopped(reason),
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            let result = match self.navigation_cleanup_control()? {
                Some(control) => control.active(*id).and_then(|job| job.stop(reason.clone())),
                None => Err(crate::navigation_control::NavigationStop::Retired),
            };
            return Ok(ContextServiceResponse::NavigationControl { result });
        }
        if let ContextServiceRequest::CancelScriptLoad { page_id, token } = &request {
            self.validate_socket_owner(*page_id)?;
            return Ok(if self.script_loads.cancel_and_observe(*token)? {
                ContextServiceResponse::ScriptLoadCancelled
            } else {
                ContextServiceResponse::ScriptLoadCancellationPending
            });
        }
        if let ContextServiceRequest::HttpRequestAbort {
            page_id,
            request_id,
        } = &request
        {
            return self.abort_http_request(*page_id, *request_id);
        }
        if let ContextServiceRequest::ParserEncodingPolicy {
            page_id,
            creator,
            token,
            action: ParserEncodingPolicyAction::Release,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            self.navigation_resource_cleanup()
                .release_parser_encoding(creator, token)?;
            return Ok(ContextServiceResponse::DocumentSecurityInstalled);
        }
        // Accepted blank activation receipts own completion independently of
        // the initiating document's current generation or shutdown state.
        if let ContextServiceRequest::AcknowledgeBlankPrerender {
            page_id,
            activation,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            return self
                .navigation_resource_cleanup()
                .acknowledge_blank_prerender(activation);
        }
        if let ContextServiceRequest::ActivateBlankPrerender {
            page_id,
            activation,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            return match self
                .context_owner()?
                .activate_selected_blank_prerender(*page_id, activation)
            {
                Ok(crate::runtime::PrerenderActivationAck::Current) => {
                    Ok(ContextServiceResponse::PrerenderActivated)
                }
                Ok(crate::runtime::PrerenderActivationAck::Obsolete)
                | Err(RuntimeError::NavigationCancelled) => {
                    Ok(ContextServiceResponse::PrerenderActivationObsolete)
                }
                Err(error) => Err(error),
            };
        }
        if let ContextServiceRequest::PublishBlankPrerender {
            page_id,
            activation,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            return Ok(
                if self
                    .context_owner()?
                    .publish_selected_blank_prerender(*page_id, activation)?
                {
                    ContextServiceResponse::PrerenderPublished
                } else {
                    ContextServiceResponse::PrerenderActivationObsolete
                },
            );
        }
        if let ContextServiceRequest::CancelBlankPrerender {
            page_id,
            destination,
            token,
            generation,
            epoch,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            match self.context_owner() {
                Ok(context) => context.cancel_blank_prerender(
                    *page_id,
                    *destination,
                    token,
                    *generation,
                    *epoch,
                )?,
                Err(RuntimeError::Closed) => {}
                Err(error) => return Err(error),
            };
            return Ok(ContextServiceResponse::PrerenderCancelled);
        }
        if let ContextServiceRequest::CancelPrerender {
            page_id,
            token,
            generation,
            epoch,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            self.navigation_resource_cleanup()
                .cancel_prerender(token, *generation, *epoch)?;
            return Ok(ContextServiceResponse::PrerenderCancelled);
        }
        if let ContextServiceRequest::DiscardPreparedLocalNavigation { page_id, token } = &request {
            self.validate_socket_owner(*page_id)?;
            self.navigation_resource_cleanup().discard_local(token)?;
            return Ok(ContextServiceResponse::LocalNavigationDiscarded);
        }
        if let ContextServiceRequest::ReleaseFrameResponseStream {
            page_id,
            stream_id,
            policy,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            self.navigation_resource_cleanup()
                .release_frame(*stream_id, policy)?;
            return Ok(ContextServiceResponse::NetworkResultReleased);
        }
        if let ContextServiceRequest::ReleaseNetworkResult {
            page_id,
            creator,
            stream_id,
            socket_id,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            self.socket_lifecycle
                .release_result(creator.as_ref(), *stream_id, *socket_id)?;
            return Ok(ContextServiceResponse::NetworkResultReleased);
        }
        if let ContextServiceRequest::ReleaseDeferredContainer { page_id, frame } = &request {
            self.validate_socket_owner(*page_id)?;
            match self.context_owner() {
                Ok(context) => context.remove_deferred_container(*page_id, frame),
                Err(RuntimeError::Closed) => {}
                Err(error) => return Err(error),
            };
            return Ok(ContextServiceResponse::DeferredFetchResult { result: Ok(()) });
        }
        if let ContextServiceRequest::AcknowledgeDeferredFetch { page_id, token } = &request {
            self.validate_socket_owner(*page_id)?;
            let result = match self.context_owner() {
                Ok(context) => context.acknowledge_deferred_fetch(*page_id, token),
                Err(RuntimeError::Closed) => Ok(()),
                Err(error) => Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                    error.to_string(),
                )),
            };
            return Ok(ContextServiceResponse::DeferredFetchResult { result });
        }
        if let ContextServiceRequest::QueryDeferredFetch { page_id, token }
        | ContextServiceRequest::ActivateDeferredFetch { page_id, token }
        | ContextServiceRequest::CancelDeferredFetch { page_id, token, .. } = &request
        {
            self.validate_socket_owner(*page_id)?;
            let result = match self.context_owner() {
                Ok(context) => match &request {
                    ContextServiceRequest::QueryDeferredFetch { .. } => {
                        context.deferred_fetch_state(*page_id, token)
                    }
                    ContextServiceRequest::ActivateDeferredFetch { .. } => {
                        context.activate_deferred_fetch(*page_id, token)
                    }
                    ContextServiceRequest::CancelDeferredFetch { creator, .. } => {
                        context.cancel_deferred_fetch(*page_id, creator, token)
                    }
                    _ => unreachable!(),
                },
                Err(RuntimeError::Closed) => Ok(oryn_js::fetch_later::DeferredFetchState::Canceled),
                Err(error) => Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                    error.to_string(),
                )),
            };
            return Ok(ContextServiceResponse::DeferredFetchState { result });
        }
        if let ContextServiceRequest::FinishHyperlinkAudit {
            page_id,
            token,
            commit,
        } = &request
        {
            self.validate_socket_owner(*page_id)?;
            match self.context_owner() {
                Ok(context) => context.finish_hyperlink_audit(*page_id, token, *commit)?,
                Err(RuntimeError::Closed) => {}
                Err(error) => return Err(error),
            };
            return Ok(ContextServiceResponse::HyperlinkAuditFinished);
        }
        if let ContextServiceRequest::ReleaseOrdinaryKeepalive { page_id, token } = &request {
            self.validate_socket_owner(*page_id)?;
            self.ordinary_keepalive.release(token);
            return Ok(ContextServiceResponse::OrdinaryKeepaliveReleased);
        }
        if let ContextServiceRequest::RollbackAuxiliaryRoot {
            page_id,
            expected_generation,
        } = &request
        {
            if *page_id != self.source_page_id {
                return Err(RuntimeError::ContextService(
                    "foreign auxiliary rollback endpoint".into(),
                ));
            }
            match self.context_owner() {
                Ok(context) => {
                    context.rollback_initial_auxiliary(*page_id, *expected_generation)?
                }
                Err(RuntimeError::Closed) => {}
                Err(error) => return Err(error),
            }
            return Ok(ContextServiceResponse::PageClosed);
        }
        // Keep the context alive for this bounded request, never for the
        // lifetime of the service retained by one of its own page actors.
        let context_owner = self.context_owner()?;
        if self.shutting_down.load(Ordering::Acquire)
            && !matches!(
                &request,
                ContextServiceRequest::ClaimNavigationExecution { .. }
                    | ContextServiceRequest::SettleNavigationExecution { .. }
                    | ContextServiceRequest::AcceptNavigationExecution { .. }
            )
        {
            return Err(RuntimeError::WorkerCrashed(
                "context service is shutting down".into(),
            ));
        }
        match request {
            ContextServiceRequest::WebFeatureProfile { page_id } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::WebFeatureProfile {
                    profile: context_owner.web_feature_profile(),
                })
            }
            ContextServiceRequest::ReserveDeferredContainer {
                page_id,
                creator,
                frame,
                source,
                allow,
            } => {
                self.validate_socket_owner(page_id)?;
                let result = (|| {
                    let oryn_js::worker_wire::Creator::Window {
                        target,
                        document_generation,
                        stream_epoch,
                    } = creator
                    else {
                        return Err("deferred container requires Window".to_owned());
                    };
                    let (owner, _, _) = context_owner
                        .window_worker_creator(page_id, &target, document_generation, stream_epoch)
                        .map_err(|error| error.to_string())?;
                    context_owner.reserve_deferred_frame(&owner, page_id, frame, &source, &allow)
                })()
                .map_err(oryn_js::fetch_later::DeferredFetchError::Transport);
                Ok(ContextServiceResponse::DeferredFetchResult { result })
            }
            ContextServiceRequest::PrepareDeferredFetch {
                page_id,
                creator,
                token,
                request,
                activate_after,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::DeferredFetchResult {
                    result: context_owner.prepare_deferred_fetch(
                        page_id,
                        creator,
                        token,
                        request,
                        activate_after,
                    ),
                })
            }
            ContextServiceRequest::AcknowledgeDeferredFetch { .. }
            | ContextServiceRequest::QueryDeferredFetch { .. }
            | ContextServiceRequest::ReleaseDeferredContainer { .. }
            | ContextServiceRequest::ActivateDeferredFetch { .. }
            | ContextServiceRequest::CancelDeferredFetch { .. } => {
                unreachable!("deferred Fetch completion handled before admission")
            }
            ContextServiceRequest::PrepareHyperlinkAudit {
                page_id,
                creator,
                token,
                destination,
                urls,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.prepare_hyperlink_audit(
                    page_id,
                    &creator,
                    token,
                    &destination,
                    urls,
                )?;
                Ok(ContextServiceResponse::HyperlinkAuditPrepared)
            }
            ContextServiceRequest::FinishHyperlinkAudit { .. } => {
                unreachable!("audit completion handled before admission")
            }
            ContextServiceRequest::GetDocumentDomain { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                let identity = context_owner
                    .document_domain_identity(page_id, &creator)
                    .map_err(RuntimeError::ContextService)?;
                Ok(ContextServiceResponse::DocumentDomainState {
                    alias_key: context_owner
                        .document_domain_alias_key(page_id, &creator)
                        .map_err(RuntimeError::ContextService)?,
                    relaxed_domain: identity.relaxed_domain().map(str::to_owned),
                    access_token_key: identity.security_token_key(),
                    origin_keyed: context_owner
                        .document_origin_agent_cluster(page_id, &creator)
                        .map_err(RuntimeError::ContextService)?,
                })
            }
            ContextServiceRequest::PrepareDocumentDomain {
                page_id,
                creator,
                candidate,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::DocumentDomainPrepared {
                    result: context_owner.prepare_document_domain(page_id, &creator, &candidate),
                })
            }
            ContextServiceRequest::CommitDocumentDomain {
                page_id,
                creator,
                candidate,
                expected_domain,
                expected_affected,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::DocumentDomainPrepared {
                    result: context_owner.commit_document_domain(
                        page_id,
                        &creator,
                        &candidate,
                        expected_domain.as_deref(),
                        &expected_affected,
                    ),
                })
            }
            ContextServiceRequest::NativePlatformQuery {
                page_id,
                creator,
                request,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::NativePlatformResult {
                    result: context_owner.native_platform_query(page_id, creator, request),
                })
            }
            ContextServiceRequest::SharedMemory {
                page_id,
                creator,
                request,
            } => {
                self.validate_socket_owner(page_id)?;
                let oryn_js::worker_wire::Creator::Window {
                    target,
                    document_generation,
                    stream_epoch,
                } = creator
                else {
                    return Err(RuntimeError::ProtocolViolation(
                        "Window memory route requires native Window creator".into(),
                    ));
                };
                let owner = if request.requires_current_document() {
                    context_owner
                        .window_worker_creator(page_id, &target, document_generation, stream_epoch)?
                        .0
                } else {
                    // Socket ownership is checked above. No page-global or
                    // author origin can grant access to an existing capability.
                    crate::agent_registry::AgentOwner::Window {
                        page: page_id,
                        target,
                        generation: document_generation,
                        epoch: stream_epoch,
                    }
                };
                context_owner
                    .shared_memory_request(&owner, request)
                    .map(ContextServiceResponse::SharedMemory)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))
            }
            ContextServiceRequest::InheritFrameWorkerPolicy {
                document_url,
                page_id,
                frame,
                document_generation,
                stream_epoch,
                parent,
                parent_generation,
                parent_epoch,
                container_allow,
                container_source,
                container_sandbox,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.inherit_frame_worker_policy_at_url(
                    crate::runtime::worker_service::InheritedFramePolicyInputs {
                        page: page_id,
                        frame: frame.clone(),
                        generation: document_generation,
                        epoch: stream_epoch,
                        parent: &parent,
                        parent_generation,
                        parent_epoch,
                        document_url: &document_url,
                    },
                )?;
                context_owner.register_isolation_container(
                    crate::runtime::worker_service::IsolationContainerInputs {
                        page: page_id,
                        frame: frame.clone(),
                        generation: document_generation,
                        epoch: stream_epoch,
                        allow: container_allow,
                        source: container_source,
                        sandbox: container_sandbox,
                    },
                )?;
                let bound_policy = context_owner.window_worker_policy(
                    page_id,
                    &oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                    document_generation,
                    stream_epoch,
                )?;
                let hide_nonces = bound_policy.hide_nonces;
                let automatic_features_allowed =
                    crate::csp::CspPolicySet::from_agent_policy(&bound_policy)
                        .sandbox()
                        .is_none_or(|sandbox| sandbox.allow_scripts);
                let (_, _, origin) = context_owner.window_worker_creator(
                    page_id,
                    &oryn_js::message_wire::WindowTarget::Frame(frame),
                    document_generation,
                    stream_epoch,
                )?;
                Ok(ContextServiceResponse::FrameWorkerPolicyBound {
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                    hide_nonces,
                    automatic_features_allowed,
                })
            }
            ContextServiceRequest::BindFrameWorkerPolicy {
                page_id,
                frame,
                document_generation,
                stream_epoch,
                url,
                token,
                parent,
                parent_generation,
                parent_epoch,
                container_allow,
                container_source,
                container_sandbox,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.bind_frame_worker_policy_with_sandbox(
                    crate::runtime::worker_service::FramePolicyInputs {
                        page: page_id,
                        frame: frame.clone(),
                        generation: document_generation,
                        epoch: stream_epoch,
                        url: &url,
                        token: &token,
                        opaque_sandbox: container_sandbox.as_ref().is_some_and(|value| {
                            !value
                                .split_ascii_whitespace()
                                .any(|token| token.eq_ignore_ascii_case("allow-same-origin"))
                        }),
                    },
                )?;
                context_owner.register_activation_parent(
                    crate::runtime::worker_service::ActivationParentInputs {
                        page: page_id,
                        frame: frame.clone(),
                        generation: document_generation,
                        epoch: stream_epoch,
                        parent,
                        parent_generation,
                        parent_epoch,
                    },
                )?;

                context_owner.register_isolation_container(
                    crate::runtime::worker_service::IsolationContainerInputs {
                        page: page_id,
                        frame: frame.clone(),
                        generation: document_generation,
                        epoch: stream_epoch,
                        allow: container_allow,
                        source: container_source,
                        sandbox: container_sandbox,
                    },
                )?;
                self.socket_lifecycle.bind_frame_stream(
                    &token,
                    oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                        document_generation,
                        stream_epoch,
                    },
                )?;
                self.bind_support_occurrence(page_id, &token,
                    &oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                        document_generation, stream_epoch,
                    })?;
                let bound_policy = context_owner.window_worker_policy(
                    page_id,
                    &oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                    document_generation,
                    stream_epoch,
                )?;
                let hide_nonces = bound_policy.hide_nonces;
                let automatic_features_allowed =
                    crate::csp::CspPolicySet::from_agent_policy(&bound_policy)
                        .sandbox()
                        .is_none_or(|sandbox| sandbox.allow_scripts);
                let (_, _, origin) = context_owner.window_worker_creator(
                    page_id,
                    &oryn_js::message_wire::WindowTarget::Frame(frame),
                    document_generation,
                    stream_epoch,
                )?;
                Ok(ContextServiceResponse::FrameWorkerPolicyBound {
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                    hide_nonces,
                    automatic_features_allowed,
                })
            }
            ContextServiceRequest::UpdateRealmDocumentUrl {
                page_id,
                creator,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let owner = self.script_document_owner(page_id, &creator)?;
                context_owner.update_realm_document_url(&owner, &url)?;
                Ok(ContextServiceResponse::RealmDocumentUrlUpdated)
            }
            ContextServiceRequest::ActivateBlankPrerender { .. }
            | ContextServiceRequest::PublishBlankPrerender { .. }
            | ContextServiceRequest::AcknowledgeBlankPrerender { .. } => {
                unreachable!("blank activation completion handled before admission")
            }
            ContextServiceRequest::ActivatePrerender {
                page_id,
                token,
                generation,
                epoch,
                predecessor,
            } => {
                self.validate_socket_owner(page_id)?;
                match context_owner.activate_native_prerender(
                    page_id,
                    &token,
                    (generation, epoch),
                    &predecessor,
                ) {
                    Ok(crate::runtime::PrerenderActivationAck::Current) => {
                        Ok(ContextServiceResponse::PrerenderActivated)
                    }
                    Ok(crate::runtime::PrerenderActivationAck::Obsolete)
                    | Err(RuntimeError::NavigationCancelled) => {
                        Ok(ContextServiceResponse::PrerenderActivationObsolete)
                    }
                    Err(error) => Err(error),
                }
            }
            ContextServiceRequest::RegisterPrerenderFrame {
                page_id,
                parent,
                frame,
                generation,
                epoch,
                origin,
                inherit_origin,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.register_native_prerender_frame(
                    crate::runtime::NativePrerenderFrameInputs {
                        page: page_id,
                        parent: &parent,
                        frame,
                        generation,
                        epoch,
                        origin,
                        inherit_origin,
                    },
                )?;
                Ok(ContextServiceResponse::PrerenderFrameRegistered)
            }
            ContextServiceRequest::MarkPrerenderReady {
                page_id,
                token,
                generation,
                epoch,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.mark_native_prerender_ready(page_id, &token, (generation, epoch))?;
                Ok(ContextServiceResponse::PrerenderReady)
            }
            ContextServiceRequest::MarkPrerenderLoaded {
                page_id,
                token,
                generation,
                epoch,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.mark_native_prerender_loaded(page_id, &token, (generation, epoch))?;
                Ok(ContextServiceResponse::PrerenderLoaded)
            }
            ContextServiceRequest::ReadPrerenderAuthority {
                page_id,
                token,
                generation,
                epoch,
            } => {
                self.validate_socket_owner(page_id)?;
                let scope =
                    context_owner.native_prerender_scope(page_id, &token, (generation, epoch))?;
                let policy = scope.policy.ok_or_else(|| {
                    RuntimeError::ProtocolViolation(
                        "prerender response policy is uncommitted".into(),
                    )
                })?;
                Ok(ContextServiceResponse::PrerenderAuthority {
                    initiator_page: scope
                        .blank_source
                        .as_ref()
                        .map_or(page_id, |(source, _)| *source),
                    url: scope.url,
                    policy,
                    origin: scope.origin,
                })
            }
            ContextServiceRequest::ReserveBlankPrerender {
                page_id,
                creator,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let (destination, admission, scope) =
                    context_owner.reserve_blank_prerender(page_id, &creator, &url)?;
                Ok(ContextServiceResponse::BlankPrerenderReserved {
                    page_id: destination,
                    admission,
                    token: scope.token,
                    generation: scope.lifetime.0,
                    epoch: scope.lifetime.1,
                })
            }
            ContextServiceRequest::ReservePrerender {
                page_id,
                creator,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let scope = context_owner.reserve_native_prerender(page_id, &creator, &url)?;
                Ok(ContextServiceResponse::PrerenderReserved {
                    token: scope.token,
                    generation: scope.lifetime.0,
                    epoch: scope.lifetime.1,
                })
            }
            ContextServiceRequest::CommitPrerenderResponse {
                response_policy,
                page_id,
                token,
                generation,
                epoch,
                url,
                headers,
            } => {
                if response_policy.is_some() || !self.trusted_in_process_runtime {
                    return Err(RuntimeError::ProtocolViolation(
                        "prerender response policy requires its owning worker route grant".into(),
                    ));
                }
                self.validate_socket_owner(page_id)?;
                let scope =
                    context_owner.native_prerender_scope(page_id, &token, (generation, epoch))?;
                let policy = context_owner
                    .commit_prerender_response_policy(page_id, &scope, &url, &headers)?;
                let committed =
                    context_owner.native_prerender_scope(page_id, &token, (generation, epoch))?;
                Ok(ContextServiceResponse::PrerenderResponseCommitted {
                    policy,
                    origin: committed.origin,
                })
            }
            ContextServiceRequest::CancelBlankPrerender { .. }
            | ContextServiceRequest::CancelPrerender { .. } => {
                unreachable!("prerender cleanup handled before live admission")
            }
            ContextServiceRequest::ParkInitialDocument { page_id } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::InitialDocumentParked {
                    token: context_owner.park_initial_document(page_id)?,
                })
            }
            ContextServiceRequest::RestoreInitialDocument { page_id, token } => {
                self.validate_socket_owner(page_id)?;
                let current = context_owner.finish_initial_document(page_id, token, true)?;
                Ok(if current {
                    ContextServiceResponse::InitialDocumentRestored
                } else {
                    ContextServiceResponse::InitialDocumentRestorationObsolete
                })
            }
            ContextServiceRequest::DiscardInitialDocument { page_id, token } => {
                self.validate_socket_owner(page_id)?;
                context_owner.finish_initial_document(page_id, token, false)?;
                Ok(ContextServiceResponse::InitialDocumentDiscarded)
            }
            ContextServiceRequest::PrepareJavascriptNavigation {
                page_id,
                admission,
                javascript_url,
                url,
                html,
            } => {
                self.validate_socket_owner(page_id)?;
                let (token, body, media_type, headers, origin, base_url) = context_owner
                    .prepare_javascript_navigation(
                        page_id,
                        admission,
                        &javascript_url,
                        &url,
                        html,
                    )?;
                Ok(ContextServiceResponse::BlankNavigationPrepared {
                    token,
                    body_base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        body,
                    ),
                    media_type,
                    headers,
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                    base_url,
                })
            }
            ContextServiceRequest::PrepareBlankNavigation {
                page_id,
                creator,
                url,
                base_url,
            } => {
                self.validate_socket_owner(page_id)?;
                let creator = match creator {
                    Some(creator) => creator,
                    None => context_owner.native_initial_document_creator(page_id)?,
                };
                let (token, body, media_type, headers, origin, base_url) = context_owner
                    .prepare_blank_navigation(page_id, creator, &url, base_url.as_deref())?;
                Ok(ContextServiceResponse::BlankNavigationPrepared {
                    token,
                    body_base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        body,
                    ),
                    media_type,
                    headers,
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                    base_url,
                })
            }
            ContextServiceRequest::PrepareServiceLocalNavigation {
                page_id,
                admission,
                url,
                blob,
            } => {
                self.validate_socket_owner(page_id)?;
                let (token, body, media_type, headers, origin) = context_owner
                    .prepare_service_local_navigation(page_id, admission, &url, blob)?;
                let body_base64 =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body);
                let serialized = origin.serialized;
                let opaque_key = origin.opaque.then_some(origin.authority);
                Ok(if blob {
                    ContextServiceResponse::BlobNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        serialized,
                        opaque_key,
                    }
                } else {
                    ContextServiceResponse::LocalDataNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        serialized,
                        opaque_key,
                    }
                })
            }
            ContextServiceRequest::PrepareInitialLocalNavigation { page_id, url, blob } => {
                self.validate_socket_owner(page_id)?;
                let creator = context_owner.native_initial_document_creator(page_id)?;
                let (token, body, media_type, headers, origin) =
                    context_owner.prepare_local_navigation(page_id, creator, &url, blob)?;
                let body_base64 =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body);
                let serialized = origin.serialized;
                let opaque_key = origin.opaque.then_some(origin.authority);
                Ok(if blob {
                    ContextServiceResponse::BlobNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        serialized,
                        opaque_key,
                    }
                } else {
                    ContextServiceResponse::LocalDataNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        serialized,
                        opaque_key,
                    }
                })
            }
            ContextServiceRequest::PrepareBlobNavigation {
                page_id,
                creator,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let (token, body, media_type, headers, origin) =
                    context_owner.prepare_local_navigation(page_id, creator, &url, true)?;
                Ok(ContextServiceResponse::BlobNavigationPrepared {
                    token,
                    body_base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        body,
                    ),
                    media_type,
                    headers,
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                })
            }
            ContextServiceRequest::PrepareLocalDataNavigation {
                page_id,
                creator,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let (token, body, media_type, headers, origin) =
                    context_owner.prepare_local_navigation(page_id, creator, &url, false)?;
                Ok(ContextServiceResponse::LocalDataNavigationPrepared {
                    token,
                    body_base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        body,
                    ),
                    media_type,
                    headers,
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                })
            }
            ContextServiceRequest::CommitPreparedLocalNavigation { page_id, token } => {
                self.validate_socket_owner(page_id)?;
                let (origin, policy) =
                    context_owner.commit_prepared_local_navigation(page_id, &token)?;
                Ok(ContextServiceResponse::LocalNavigationCommitted {
                    serialized: origin.serialized,
                    opaque_key: origin.opaque.then_some(origin.authority),
                    policy,
                })
            }
            ContextServiceRequest::DiscardPreparedLocalNavigation { .. } => {
                unreachable!("local grant cleanup handled before live context admission")
            }
            ContextServiceRequest::StickyUserActivation { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                let result = match creator {
                    oryn_js::worker_wire::Creator::Window {
                        target,
                        document_generation,
                        stream_epoch,
                    } => context_owner
                        .sticky_native_activation(&crate::agent_registry::AgentOwner::Window {
                            page: page_id,
                            target,
                            generation: document_generation,
                            epoch: stream_epoch,
                        })
                        .map_err(|e| e.to_string()),
                    _ => Err("worker has no window activation".into()),
                };
                Ok(ContextServiceResponse::TransientUserActivation { result })
            }
            ContextServiceRequest::TransientUserActivation {
                page_id,
                creator,
                consume,
            } => {
                self.validate_socket_owner(page_id)?;
                let result = match creator {
                    oryn_js::worker_wire::Creator::Window {
                        target,
                        document_generation,
                        stream_epoch,
                    } => context_owner
                        .transient_native_activation(
                            &crate::agent_registry::AgentOwner::Window {
                                page: page_id,
                                target,
                                generation: document_generation,
                                epoch: stream_epoch,
                            },
                            consume,
                        )
                        .map_err(|error| error.to_string()),
                    _ => Err("worker cannot consume window activation".into()),
                };
                Ok(ContextServiceResponse::TransientUserActivation { result })
            }
            ContextServiceRequest::NotifyUserActivation { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                let result = match creator {
                    oryn_js::worker_wire::Creator::Window {
                        target,
                        document_generation,
                        stream_epoch,
                    } => context_owner
                        .record_native_activation(crate::agent_registry::AgentOwner::Window {
                            page: page_id,
                            target,
                            generation: document_generation,
                            epoch: stream_epoch,
                        })
                        .map_err(|error| error.to_string()),
                    _ => Err("worker cannot publish window activation".into()),
                };
                Ok(ContextServiceResponse::UserActivationRecorded { result })
            }
            ContextServiceRequest::WorkerService {
                source_page_id,
                request,
            } => {
                self.validate_socket_owner(source_page_id)?;
                let network = self.network.clone().ok_or_else(|| {
                    RuntimeError::ContextService("worker network broker unavailable".into())
                })?;
                Ok(ContextServiceResponse::WorkerService {
                    result: crate::agent_service::window_request(
                        &context_owner,
                        source_page_id,
                        request,
                        network,
                        &self.shutting_down,
                    ),
                })
            }
            ContextServiceRequest::MessageWire {
                source_page_id,
                request,
            } => {
                self.validate_socket_owner(source_page_id)?;
                // Window IPC never infers the sender document from a page ID.
                // Pure MessageHub/native registry callers retain their separate
                // internal API; executable realm callers carry InDocument.
                if request.requires_window_document_scope() {
                    return Ok(ContextServiceResponse::MessageWire {
                        result: Err(
                            "Window message operation requires native document scope".into()
                        ),
                    });
                }
                Ok(ContextServiceResponse::MessageWire {
                    result: context_owner.handle_message_wire(source_page_id, request),
                })
            }
            ContextServiceRequest::CheckNativeTargetNavigation {
                page_id,
                creator,
                target_page_id,
                target,
            } => {
                self.validate_socket_owner(page_id)?;
                if !context_owner.native_target_navigation_allowed(
                    page_id,
                    &creator,
                    target_page_id,
                    &target,
                )? {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "native sandbox target navigation denied".into(),
                    });
                }
                Ok(ContextServiceResponse::NavigationAuthorized)
            }
            ContextServiceRequest::UpdateFrameContainerPolicy {
                page_id,
                creator,
                parent,
                allow,
                source,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.update_live_frame_container_policy(
                    page_id, &creator, &parent, allow, source,
                )?;
                Ok(ContextServiceResponse::NavigationAuthorized)
            }
            ContextServiceRequest::CheckFormNavigationPolicy {
                page_id,
                authority,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let (_, decision) = context_owner
                    .form_policy_for_navigation_report(&authority, page_id, &url, true, None)?;
                if !decision.allowed {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "form navigation violates original source policy".into(),
                    });
                }
                Ok(ContextServiceResponse::FormNavigationPolicyChecked {
                    source: context_owner.form_fetch_source(&authority, page_id)?,
                })
            }
            ContextServiceRequest::AuthorizePageDownload {
                page_id,
                creator,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.authorize_page_download(page_id, &creator, &url)?;
                Ok(ContextServiceResponse::PageDownloadAuthorized)
            }
            ContextServiceRequest::QuarantinePageDownload {
                page_id,
                creator,
                url,
                suggested_filename,
                media_type,
                body_base64,
            } => {
                self.validate_socket_owner(page_id)?;
                const MAX_BODY: usize = 16 * 1024 * 1024;
                if body_base64.len() > MAX_BODY.div_ceil(3) * 4
                    || url.len() > 65536
                    || suggested_filename.len() > 65536
                    || media_type.len() > 4096
                {
                    return Err(RuntimeError::Download(
                        "page download transport capacity".into(),
                    ));
                }
                let body =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                        .map_err(|_| {
                            RuntimeError::Download("invalid download body encoding".into())
                        })?;
                if body.len() > MAX_BODY {
                    return Err(RuntimeError::Download("page download body capacity".into()));
                }
                let download = context_owner.quarantine_page_download(
                    page_id,
                    creator,
                    url,
                    suggested_filename,
                    media_type,
                    body,
                )?;
                Ok(ContextServiceResponse::DownloadQuarantined { download })
            }
            ContextServiceRequest::InstallDocumentSourceMetadata {
                page_id,
                creator,
                url,
                referrer,
                meta_csp,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::DocumentSource {
                    source: context_owner.install_document_source_metadata(
                        page_id,
                        creator,
                        &url,
                        &referrer,
                        meta_csp.as_deref(),
                    )?,
                })
            }
            ContextServiceRequest::CaptureDocumentSource {
                page_id,
                creator,
                url,
                referrer,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::DocumentSource {
                    source: context_owner
                        .capture_document_source(page_id, creator, &url, &referrer)?,
                })
            }
            ContextServiceRequest::DeriveInertDocumentSource {
                page_id,
                source,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::DocumentSource {
                    source: context_owner.derive_inert_document_source(page_id, &source, &url)?,
                })
            }
            ContextServiceRequest::DocumentFormAuthority { page_id, source } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::FormNavigationAuthority {
                    authority: context_owner.document_form_authority(page_id, &source)?,
                })
            }
            ContextServiceRequest::CheckDocumentTargetNavigation {
                page_id,
                target_page_id,
                source,
                target,
            } => {
                self.validate_socket_owner(page_id)?;
                if context_owner.document_target_navigation_allowed(
                    target_page_id.unwrap_or(page_id),
                    &source,
                    &target,
                )? {
                    Ok(ContextServiceResponse::NavigationAuthorized)
                } else {
                    Ok(ContextServiceResponse::PageDenied {
                        reason: "document sandbox navigation denied".into(),
                    })
                }
            }
            ContextServiceRequest::RetargetFormNavigationAuthority {
                page_id,
                target_page_id,
                authority,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::FormNavigationAuthority {
                    authority: context_owner.retarget_form_authority(
                        &authority,
                        page_id,
                        target_page_id,
                    )?,
                })
            }
            ContextServiceRequest::IssueFormNavigationAuthority {
                page_id,
                creator,
                referrer_policy,
                source_url,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::FormNavigationAuthority {
                    authority: context_owner.issue_form_authority_with_metadata(
                        page_id,
                        creator,
                        Some(&referrer_policy),
                        Some(&source_url),
                    )?,
                })
            }
            ContextServiceRequest::IssueScriptNavigationAuthority {
                page_id,
                target_page_id,
                creator,
                target,
                url,
                referrer_policy,
                source_url,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ScriptNavigationAuthority {
                    authority: context_owner.issue_script_navigation_authority(
                        crate::runtime::ScriptNavigationInputs {
                            source: page_id,
                            source_creator: creator,
                            destination: target_page_id.unwrap_or(page_id),
                            target_creator: target,
                            destination_url: &url,
                            source_url: &source_url,
                            referrer_policy: &referrer_policy,
                        },
                    )?,
                })
            }
            ContextServiceRequest::IssueDocumentScriptNavigationAuthority {
                page_id,
                source,
                target,
                url,
                referrer_policy,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ScriptNavigationAuthority {
                    authority: context_owner.issue_document_script_navigation_authority(
                        page_id,
                        &source,
                        target,
                        &url,
                        referrer_policy.as_deref(),
                    )?,
                })
            }
            ContextServiceRequest::ServiceNavigationToScript {
                page_id,
                admission,
                url,
            } => {
                self.validate_socket_owner(page_id)?;
                let admission =
                    context_owner.service_navigation_to_script(page_id, admission, &url)?;
                Ok(ContextServiceResponse::ScriptNavigationAuthority {
                    authority: admission.authority,
                })
            }
            ContextServiceRequest::AcquireNavigationNetworkSource {
                page_id,
                nonce: _,
                target,
                url,
                authority,
            } => {
                self.validate_socket_owner(page_id)?;
                // Direct actors use this context's broker and need no remote route grant.
                Ok(ContextServiceResponse::NavigationNetworkSource {
                    source: authority.source(&context_owner, page_id, &target, &url)?,
                    bound: false,
                })
            }
            ContextServiceRequest::DiscardNavigationNetworkSource { page_id, .. } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::NavigationNetworkSourceDiscarded)
            }
            ContextServiceRequest::ReadServiceNavigationSource {
                page_id,
                target,
                url,
                authority,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ScriptNavigationSource {
                    source: context_owner
                        .service_navigation_source(&authority, page_id, &target, &url)?,
                })
            }
            ContextServiceRequest::ReadScriptNavigationSource {
                page_id,
                target,
                url,
                authority,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ScriptNavigationSource {
                    source: context_owner
                        .script_navigation_source(&authority, page_id, &target, &url)?,
                })
            }
            ContextServiceRequest::NativeWindowTargetName {
                page_id,
                creator,
                value,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::WindowTargetName {
                    value: context_owner.native_root_target_name(page_id, &creator, value)?,
                })
            }
            ContextServiceRequest::NativeWindowOpener { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::WindowOpener {
                    opener: self
                        .context_owner()?
                        .native_browsing_context_opener(page_id, &creator)?,
                })
            }
            ContextServiceRequest::DiscardNativeFrameOpener { page_id, frame } => {
                self.validate_socket_owner(page_id)?;
                self.context_owner()?
                    .discard_native_frame_opener(page_id, &frame)?;
                Ok(ContextServiceResponse::OpenerDisowned)
            }
            ContextServiceRequest::DisownNativeOpener { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                self.context_owner()?
                    .disown_native_opener(page_id, &creator)?;
                Ok(ContextServiceResponse::OpenerDisowned)
            }
            ContextServiceRequest::NavigateRelatedWindow {
                source_page_id,
                creator,
                target_page_id,
                target,
                expected,
                url,
                replace,
            } => {
                self.validate_socket_owner(source_page_id)?;
                context_owner.queue_related_window_navigation(
                    crate::runtime::RelatedWindowNavigationInputs {
                        source: source_page_id,
                        creator: &creator,
                        target_page: target_page_id,
                        target: &target,
                        expected,
                        url: &url,
                        replace,
                    },
                )?;
                Ok(ContextServiceResponse::NavigationAuthorized)
            }
            ContextServiceRequest::SettleNavigationExecution {
                page_id,
                receipt,
                outcome,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(
                    match context_owner.settle_queued_navigation(page_id, &receipt, outcome)? {
                        crate::queued_navigation::SettlementAck::Accepted => {
                            ContextServiceResponse::NavigationExecutionSettled
                        }
                        crate::queued_navigation::SettlementAck::Retired => {
                            ContextServiceResponse::NavigationExecutionRetired
                        }
                    },
                )
            }
            ContextServiceRequest::AcceptNavigationExecution { page_id, receipt } => {
                self.validate_socket_owner(page_id)?;
                Ok(
                    if context_owner
                        .accept_queued_navigation(page_id, &receipt)?
                        .is_some()
                    {
                        ContextServiceResponse::NavigationExecutionAccepted
                    } else {
                        ContextServiceResponse::NavigationExecutionRetired
                    },
                )
            }
            ContextServiceRequest::ClaimNavigationExecution { page_id, receipt } => {
                self.validate_socket_owner(page_id)?;
                if self.shutting_down.load(Ordering::Acquire) {
                    return Ok(ContextServiceResponse::NavigationExecutionClaimRejected);
                }
                Ok(
                    if context_owner.claim_queued_navigation(page_id, &receipt)? {
                        ContextServiceResponse::NavigationExecutionClaimed
                    } else {
                        ContextServiceResponse::NavigationExecutionClaimRejected
                    },
                )
            }
            ContextServiceRequest::PublishNativeWindowInventory {
                page_id,
                creator,
                inventory,
            } => {
                self.validate_socket_owner(page_id)?;
                context_owner.publish_native_window_inventory(page_id, &creator, &inventory)?;
                Ok(ContextServiceResponse::NativeWindowInventoryPublished)
            }
            ContextServiceRequest::ReadRelatedWindow {
                source_page_id,
                creator,
                target_page_id,
                target,
                expected,
                local_inventory,
            } => {
                self.validate_socket_owner(source_page_id)?;
                Ok(ContextServiceResponse::RelatedWindow {
                    snapshot: context_owner.read_related_window_with_inventory(
                        source_page_id,
                        &creator,
                        target_page_id,
                        &target,
                        expected,
                        &local_inventory,
                    )?,
                })
            }
            ContextServiceRequest::ReadNativeWindowOrigin {
                source_page_id,
                creator,
                target_page_id,
                target,
                expected,
            } => {
                self.validate_socket_owner(source_page_id)?;
                let (serialized, opaque) = self.context_owner()?.read_native_window_origin(
                    source_page_id,
                    &creator,
                    target_page_id,
                    &target,
                    expected,
                )?;
                Ok(ContextServiceResponse::SecurityOrigin { serialized, opaque })
            }
            ContextServiceRequest::NativeWindowGroup {
                source_page_id,
                target_page_id,
                expected,
            } => {
                self.validate_socket_owner(source_page_id)?;
                let (token, opener, live) =
                    context_owner.native_window_group(source_page_id, target_page_id, expected)?;
                Ok(ContextServiceResponse::NativeWindowGroup {
                    token,
                    opener,
                    opener_target: context_owner.native_opener_target(source_page_id)?,
                    live,
                })
            }
            ContextServiceRequest::AutomationWindowStatus {
                source_page_id,
                target_page_id,
            } => {
                self.validate_socket_owner(source_page_id)?;
                // Inspect both identities in one registry snapshot. A retained
                // worker route must not authorize references after its source closes.
                let pages = context_owner.pages();
                let is_live = |id| {
                    pages.iter().any(|page| {
                        page.page.page_id == id
                            && matches!(
                                page.status,
                                oryn_common::v3::PageStatus::Open
                                    | oryn_common::v3::PageStatus::AttentionRequired
                            )
                    })
                };
                if !is_live(source_page_id) {
                    return Err(RuntimeError::PageNotFound(source_page_id));
                }
                let live = is_live(target_page_id);
                Ok(ContextServiceResponse::AutomationWindowStatus { live })
            }
            ContextServiceRequest::DragFeedback { page_id, feedback } => {
                self.validate_socket_owner(page_id)?;
                context_owner.publish_drag_feedback(page_id, feedback)?;
                Ok(ContextServiceResponse::DragFeedbackPublished)
            }
            ContextServiceRequest::NavigationCheckpoint { page_id, id } => {
                self.validate_socket_owner(page_id)?;
                let control = context_owner.page_navigation_control(page_id)?;
                Ok(ContextServiceResponse::NavigationControl {
                    result: control.active(id).and_then(|job| job.checkpoint()),
                })
            }
            ContextServiceRequest::NavigationEvent { page_id, id, event } => {
                self.validate_socket_owner(page_id)?;
                let control = context_owner.page_navigation_control(page_id)?;
                Ok(ContextServiceResponse::NavigationControl {
                    result: control.active(id).and_then(|job| job.publish(event)),
                })
            }
            ContextServiceRequest::ClipboardAuthorizeScript {
                page_id,
                creator,
                write,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ClipboardScriptAuthorized {
                    result: context_owner
                        .authorize_clipboard_for_creator(page_id, &creator, write)
                        .map_err(clipboard_service_error),
                })
            }
            ContextServiceRequest::ClipboardRead { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ClipboardRead {
                    result: context_owner
                        .read_clipboard_for_creator(page_id, &creator)
                        .map_err(clipboard_service_error),
                })
            }
            ContextServiceRequest::ClipboardWrite {
                page_id,
                creator,
                data,
            } => {
                self.validate_socket_owner(page_id)?;
                let result = data.validate().and_then(|()| {
                    context_owner
                        .write_clipboard_for_creator(page_id, &creator, data)
                        .map_err(clipboard_service_error)
                });
                Ok(ContextServiceResponse::ClipboardWrite { result })
            }
            ContextServiceRequest::UserPrompt { page_id, request } => {
                self.validate_socket_owner(page_id)?;
                let controller = context_owner.page_prompt_controller(page_id)?;
                // No group/route mutex is held while waiting for human/driver input.
                Ok(ContextServiceResponse::UserPrompt {
                    response: controller.wait_for_request(request),
                })
            }
            ContextServiceRequest::CancelUserPrompt { page_id, token } => {
                self.validate_socket_owner(page_id)?;
                let controller = context_owner.page_prompt_controller(page_id)?;
                // Exact token comparison prevents a late cancellation dismissing
                // a subsequent prompt in this page or another document/realm.
                let _ = controller.cancel(token);
                Ok(ContextServiceResponse::UserPromptCancelled)
            }
            ContextServiceRequest::GetReferrerPolicy { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                let owner = self.script_document_owner(page_id, &creator)?;
                let (_, _, policy) = context_owner
                    .native_worker_authority(&owner)
                    .map_err(|failure| RuntimeError::ContextService(failure.to_string()))?;
                // Native policy includes the exact frame/prerender response and
                // meta state; a later active root never supplies this answer.
                let value =
                    crate::referrer::ReferrerPolicy::default().after_response(&policy.headers);
                Ok(ContextServiceResponse::ReferrerPolicy {
                    value: value.as_str().into(),
                })
            }
            ContextServiceRequest::PrepareOrdinaryKeepalive { page_id, request } => {
                Ok(ContextServiceResponse::OrdinaryKeepalivePrepared {
                    token: self.prepare_ordinary_keepalive(page_id, request)?,
                })
            }
            ContextServiceRequest::StartOrdinaryKeepalive { page_id, token } => {
                self.validate_socket_owner(page_id)?;
                self.ordinary_keepalive.take(&token)?.wait()
            }
            ContextServiceRequest::ReleaseOrdinaryKeepalive { .. } => {
                unreachable!("keepalive release handled before admission")
            }
            ContextServiceRequest::HttpStreamStart { page_id, request } => {
                self.start_http_stream(page_id, request, crate::csp::CspDestination::Connect)
            }
            ContextServiceRequest::XsltTransform {
                page_id,
                creator,
                request_id,
                request,
            } => {
                #[cfg(feature = "v8-host")]
                {
                    self.validate_socket_owner(page_id)?;
                    let oryn_js::worker_wire::Creator::Window {
                        target,
                        document_generation,
                        stream_epoch,
                    } = creator
                    else {
                        return Err(RuntimeError::ProtocolViolation(
                            "XSLT helper requires Window creator".into(),
                        ));
                    };
                    let (owner, _, _) = self.context_owner()?.window_worker_creator(
                        page_id,
                        &target,
                        document_generation,
                        stream_epoch,
                    )?;
                    // Reuse the existing bounded, retirement-aware cancellable
                    // establishment owner. It holds no socket for this role.
                    let operation = SocketHandshakeLease::begin(
                        self.socket_lifecycle.clone(),
                        request_id,
                        owner,
                    )?;
                    operation.check()?;
                    let result = crate::xslt_process::run(request, || operation.check().is_err())
                        .map_err(RuntimeError::ContextService)?;
                    operation.check()?;
                    Ok(ContextServiceResponse::XsltTransformed { transform: result })
                }
                #[cfg(not(feature = "v8-host"))]
                {
                    let _ = (page_id, creator, request_id, request);
                    Err(RuntimeError::ProtocolViolation(
                        "XSLT unavailable without native V8 runtime".into(),
                    ))
                }
            }
            ContextServiceRequest::XsltStreamStart { page_id, request } => {
                if request.client.is_none()
                    || request.method != "GET"
                    || !request.body_base64.is_empty()
                    || request.fetch.as_ref().is_none_or(|fetch| {
                        fetch.mode != "same-origin" || fetch.redirect != "follow" || fetch.keepalive
                    })
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "XSLT requires a document-owned same-origin GET".into(),
                    ));
                }
                self.start_http_stream_with_destination(HttpStreamDestinationInputs {
                    page_id,
                    request,
                    destination: crate::csp::CspDestination::Script,
                    cors_enabled: true,
                    media_destination: None,
                    xslt: true,
                    preload: false,
                    template_nonce: None,
                    frozen: None,
                })
            }
            ContextServiceRequest::PrefetchStreamStart { page_id, request } => {
                if request.client.is_none()
                    || request.method != "GET"
                    || !request.body_base64.is_empty()
                    || request.fetch.as_ref().is_some_and(|fetch| {
                        fetch.mode != "no-cors" || fetch.redirect != "follow" || fetch.keepalive
                    })
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "prefetch requires a document-owned no-CORS GET".into(),
                    ));
                }
                self.start_http_stream_with_destination(HttpStreamDestinationInputs {
                    page_id,
                    request,
                    destination: crate::csp::CspDestination::Prefetch,
                    cors_enabled: false,
                    media_destination: None,
                    xslt: false,
                    preload: false,
                    template_nonce: None,
                    frozen: None,
                })
            }
            ContextServiceRequest::TemplateStreamStart {
                page_id,
                request,
                nonce,
            } => {
                if request.client.is_none()
                    || request.method != "GET"
                    || !request.body_base64.is_empty()
                    || !request
                        .fetch
                        .as_ref()
                        .is_some_and(|fetch| fetch.mode == "cors" && fetch.redirect == "follow")
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "external template requires a document-owned CORS GET".into(),
                    ));
                }
                self.start_http_stream_with_destination(HttpStreamDestinationInputs {
                    page_id,
                    request,
                    destination: crate::csp::CspDestination::Script,
                    cors_enabled: true,
                    media_destination: None,
                    xslt: false,
                    preload: false,
                    template_nonce: Some(nonce.as_deref()),
                    frozen: None,
                })
            }
            ContextServiceRequest::ImagePreloadStreamStart {
                page_id,
                request,
                cors_enabled,
            } => {
                if request.method != "GET" || !request.body_base64.is_empty() {
                    return Err(RuntimeError::ProtocolViolation(
                        "invalid image preload request".into(),
                    ));
                }
                self.start_http_stream_with_destination(HttpStreamDestinationInputs {
                    page_id,
                    request,
                    destination: crate::csp::CspDestination::Image,
                    cors_enabled,
                    media_destination: None,
                    xslt: false,
                    preload: true,
                    template_nonce: None,
                    frozen: None,
                })
            }
            ContextServiceRequest::MediaStreamStart {
                page_id,
                request,
                cors_enabled,
                destination,
            } => {
                if request.method != "GET"
                    || !request.body_base64.is_empty()
                    || !matches!(destination.as_str(), "audio" | "video" | "track")
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "invalid native media request".into(),
                    ));
                }
                if destination == "track"
                    && !request
                        .fetch
                        .as_ref()
                        .is_some_and(|fetch| matches!(fetch.mode.as_str(), "same-origin" | "cors"))
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "text track requires native CORS mode".into(),
                    ));
                }
                self.start_http_stream_with_destination(HttpStreamDestinationInputs {
                    page_id,
                    request,
                    destination: crate::csp::CspDestination::Media,
                    cors_enabled,
                    media_destination: Some(&destination),
                    xslt: false,
                    preload: false,
                    template_nonce: None,
                    frozen: None,
                })
            }
            ContextServiceRequest::ImageStreamStart {
                page_id,
                request,
                cors_enabled,
            } => {
                if request.method != "GET" || !request.body_base64.is_empty() {
                    return Err(RuntimeError::ProtocolViolation(
                        "image stream requires a bodyless GET".into(),
                    ));
                }
                self.start_http_stream_with_cors(
                    page_id,
                    request,
                    crate::csp::CspDestination::Image,
                    cors_enabled,
                )
            }
            ContextServiceRequest::HttpStreamRead {
                page_id,
                creator,
                stream_id,
                max_bytes,
            } => {
                self.validate_http_stream_creator(page_id, stream_id, creator.as_ref(), true)?;
                self.read_http_stream(page_id, stream_id, max_bytes)
            }
            ContextServiceRequest::HttpStreamAbort {
                page_id,
                stream_id,
                creator,
            } => {
                self.validate_http_stream_creator(page_id, stream_id, creator.as_ref(), false)?;
                self.abort_http_stream(page_id, stream_id)
            }
            ContextServiceRequest::HttpRequestAbort {
                page_id,
                request_id,
            } => self.abort_http_request(page_id, request_id),
            ContextServiceRequest::Storage { creator, request } => {
                let creator = creator.ok_or_else(|| {
                    RuntimeError::ProtocolViolation(
                        "storage requires native document authority".into(),
                    )
                })?;
                let owner = self.script_document_owner(self.source_page_id, &creator)?;
                let (partition, native_origin, source_policy) = context_owner
                    .native_worker_authority(&owner)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                self.validate_storage_scope(&request, &native_origin)?;
                self.script_document_owner(self.source_page_id, &creator)?;
                let response = match request {
                    StorageRequest::IndexedDb { command, .. } => {
                        crate::storage::validate_script_idb_command(&command)
                            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                        context_owner.execute_window_indexed_db(
                            &owner,
                            &partition,
                            &native_origin,
                            command,
                        )?
                    }
                    request => context_owner.execute_storage_with_events(
                        self.source_page_id,
                        creator.clone(),
                        source_policy.url.clone(),
                        request,
                    )?,
                };
                self.script_document_owner(self.source_page_id, &creator)?;
                Ok(ContextServiceResponse::Storage {
                    response: Box::new(response),
                })
            }
            ContextServiceRequest::OpenPage {
                hyperlink_audit,
                local_inventory,
                link_navigation,
                user_initiated,
                defer_navigation,
                local_related_navigation,
                selected_target,
                document_source,
                opener_page_id,
                noopener,
                target_name,
                form_authority,
                creator,
                url,
                post,
                referrer_policy,
            } => {
                if opener_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot open a popup for another page".into(),
                    ));
                }
                if hyperlink_audit
                    .as_ref()
                    .is_some_and(|token| token.page_id != opener_page_id.0)
                    || (!link_navigation && hyperlink_audit.is_some())
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "invalid link auditing owner".into(),
                    ));
                }
                let mut hyperlink_audit = crate::runtime::hyperlink_audit::Pending::new(
                    context_owner.clone(),
                    hyperlink_audit,
                );
                let document_snapshot = document_source
                    .as_ref()
                    .map(|source| context_owner.document_popup_source(opener_page_id, source))
                    .transpose()?;
                if link_navigation
                    && (post.is_some() || form_authority.is_some() || document_source.is_none())
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "invalid selected link source or resource".into(),
                    ));
                }
                let creator = document_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.0.clone())
                    .or(creator)
                    .ok_or_else(|| {
                        RuntimeError::ProtocolViolation("popup native creator missing".into())
                    })?;
                let oryn_js::worker_wire::Creator::Window {
                    target,
                    document_generation,
                    stream_epoch,
                } = &creator
                else {
                    return Err(RuntimeError::ProtocolViolation(
                        "popup creator must be a Window".into(),
                    ));
                };
                let native_origin = if document_snapshot.is_none() {
                    Some(
                        context_owner
                            .window_worker_creator(
                                opener_page_id,
                                target,
                                *document_generation,
                                *stream_epoch,
                            )?
                            .2,
                    )
                } else {
                    None
                };
                if target_name.len() > 65_536 {
                    return Err(RuntimeError::ProtocolViolation(
                        "target name exceeds native bound".into(),
                    ));
                }
                let target_name =
                    oryn_js::navigation::effective_navigation_target(&target_name, noopener)
                        .to_owned();
                let existing = if let Some(receipt) = &selected_target {
                    Some(context_owner.resolve_navigation_target_with_local(
                        opener_page_id,
                        receipt,
                        &local_inventory,
                    )?)
                } else if !target_name.is_empty() && !target_name.eq_ignore_ascii_case("_blank") {
                    if let Some(source) = &document_source {
                        context_owner.named_familiar_document_navigation_target(
                            opener_page_id,
                            source,
                            &target_name,
                            &local_inventory,
                        )?
                    } else {
                        context_owner.named_familiar_navigation_target_with_local(
                            opener_page_id,
                            &creator,
                            &target_name,
                            &local_inventory,
                        )?
                    }
                } else {
                    None
                };
                let source_policy = if let Some(snapshot) = &document_snapshot {
                    snapshot.2.clone()
                } else {
                    crate::csp::CspPolicySet::from_agent_policy(
                        &context_owner.window_worker_policy(
                            opener_page_id,
                            target,
                            *document_generation,
                            *stream_epoch,
                        )?,
                    )
                };
                if existing.is_none() && !self.context_owner()?.popups_allowed() {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "popup policy denied page creation".into(),
                    });
                }
                if existing.is_none()
                    && source_policy
                        .sandbox()
                        .is_some_and(|sandbox| !sandbox.allow_popups)
                {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "Content-Security-Policy sandbox denied popup creation".into(),
                    });
                }
                let opener_origin = document_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.1.clone())
                    .unwrap_or_else(|| native_origin.unwrap().serialized);
                let target_origin = url.as_deref().and_then(|value| {
                    context_owner
                        .resolve_navigation_url(self.source_page_id, value)
                        .ok()
                        .map(|url| url.origin().ascii_serialization())
                });
                if existing.is_none()
                    && let Err(error) = self.context_owner()?.require_policy(
                        PolicyRequest {
                            action: ConsequentialAction::Popup,
                            source_origin: opener_origin.clone(),
                            target_origin,
                            detail: url.clone(),
                        },
                        None,
                    )
                {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: error.to_string(),
                    });
                }
                if let Some(url) = &url {
                    self.context_owner()?
                        .validate_page_url(self.source_page_id, url)?;
                }
                if let Some(resource) = &post {
                    if url
                        .as_ref()
                        .and_then(|url| {
                            context_owner
                                .resolve_navigation_url(opener_page_id, url)
                                .ok()
                        })
                        .is_none_or(|url| !resource.valid_for(&url))
                    {
                        return Err(RuntimeError::ContextService(
                            "invalid popup POST resource".into(),
                        ));
                    }
                    match self.handle(ContextServiceRequest::AuthorizeFormSubmission {
                        report_source: None,
                        authority: form_authority.clone(),
                        page_id: opener_page_id,
                        url: url.clone().unwrap(),
                        method: "POST".into(),
                    })? {
                        ContextServiceResponse::FormSubmissionAuthorized => {}
                        response @ ContextServiceResponse::PageDenied { .. } => {
                            return Ok(response);
                        }
                        _ => {
                            return Err(RuntimeError::ContextService(
                                "invalid form authorization response".into(),
                            ));
                        }
                    }
                }
                if let Some((existing, frame)) = existing {
                    let selected_target = Some(match selected_target {
                        Some(receipt) => receipt,
                        None => {
                            if let Some(source) = &document_source {
                                context_owner.capture_document_navigation_target(
                                    opener_page_id,
                                    source,
                                    existing,
                                    frame.clone(),
                                )?
                            } else {
                                context_owner.capture_navigation_target(
                                    opener_page_id,
                                    &creator,
                                    existing,
                                    frame.clone(),
                                )?
                            }
                        }
                    });
                    let target = match &frame {
                        Some(frame) => oryn_js::worker_wire::Creator::Window {
                            target: oryn_js::message_wire::WindowTarget::Frame(
                                frame.frame_token.clone().ok_or_else(|| {
                                    RuntimeError::ProtocolViolation(
                                        "selected frame identity missing".into(),
                                    )
                                })?,
                            ),
                            document_generation: frame.document_generation,
                            stream_epoch: frame.document_authority_epoch,
                        },
                        None => context_owner.root_navigation_creator(existing)?,
                    };
                    let allowed = if let Some(source) = &document_source {
                        context_owner
                            .document_target_navigation_allowed(existing, source, &target)?
                    } else {
                        context_owner.native_target_navigation_allowed(
                            opener_page_id,
                            &creator,
                            existing,
                            &target,
                        )?
                    };
                    if !allowed {
                        return Ok(ContextServiceResponse::PageDenied {
                            reason: "native sandbox target navigation denied".into(),
                        });
                    }
                    // This producer is Window.open: forms and link selection carry
                    // document/form provenance or defer_navigation instead.
                    if !noopener
                        && !defer_navigation
                        && !link_navigation
                        && document_source.is_none()
                        && form_authority.is_none()
                        && post.is_none()
                        && frame.is_some()
                        && !["_self", "_parent", "_top"]
                            .iter()
                            .any(|keyword| target_name.eq_ignore_ascii_case(keyword))
                    {
                        context_owner.set_named_frame_opener(
                            opener_page_id,
                            &creator,
                            existing,
                            &target,
                        )?;
                    }
                    let selected_frame = frame.clone();
                    if !defer_navigation && let Some(url) = url {
                        if link_navigation {
                            let source = document_source.clone().ok_or_else(|| {
                                RuntimeError::ProtocolViolation(
                                    "selected link document source missing".into(),
                                )
                            })?;
                            context_owner.queue_document_navigation(
                                crate::runtime::DocumentNavigationInputs {
                                    page_id: existing,
                                    url,
                                    source,
                                    referrer_policy,
                                    user_initiated,
                                    selected: frame,
                                    hyperlink_audit: hyperlink_audit.take(),
                                },
                            )?;
                        } else if document_source.is_none()
                            && form_authority.is_none()
                            && post.is_none()
                        {
                            let oryn_js::worker_wire::Creator::Window { target, .. } = &target
                            else {
                                return Err(RuntimeError::ProtocolViolation(
                                    "Window.open destination is not a Window".into(),
                                ));
                            };
                            context_owner.queue_open_window_navigation(
                                opener_page_id,
                                &creator,
                                existing,
                                target,
                                &url,
                                referrer_policy.as_deref() == Some("no-referrer"),
                            )?;
                        } else {
                            let authority = form_authority
                                .as_ref()
                                .map(|authority| {
                                    context_owner.retarget_form_authority(
                                        authority,
                                        opener_page_id,
                                        existing,
                                    )
                                })
                                .transpose()?;
                            context_owner.queue_target_resource_navigation(
                                existing,
                                url,
                                post,
                                referrer_policy,
                                authority,
                                frame,
                            )?;
                        }
                    }
                    return Ok(ContextServiceResponse::PageOpened {
                        blank_prerender: None,
                        selected_frame,
                        selected_target,
                        related_root: None,
                        related_navigation_url: None,
                        page_id: existing,
                        created: false,
                        opener_page_id: None,
                    });
                }
                // This claim transfers the already-running candidate to the source
                // actor. It must not enqueue an ordinary navigation or create a page.
                // Forms reach this point only after the GET query is finalized; POST
                // resources never activate a prerender.
                if !defer_navigation
                    && !link_navigation
                    && noopener
                    && target_name.eq_ignore_ascii_case("_blank")
                    && post.is_none()
                    && let Some(url) = url.as_deref()
                    && let Some(receipt) =
                        context_owner.claim_blank_prerender(opener_page_id, &creator, None, url)?
                {
                    return Ok(ContextServiceResponse::PageOpened {
                        page_id: receipt.page_id,
                        blank_prerender: Some(receipt),
                        selected_frame: None,
                        selected_target: None,
                        related_root: None,
                        related_navigation_url: None,
                        created: false,
                        opener_page_id: None,
                    });
                }
                // Clone sessionStorage before the popup's first document can
                // execute, matching the browser's initial-copy semantics.
                let initial_url = url;
                let (noopener, force_blank) = if let Some(source) = &document_source {
                    context_owner.document_popup_relationship_policy(
                        opener_page_id,
                        source,
                        initial_url.as_deref(),
                        noopener,
                    )?
                } else {
                    context_owner.native_popup_relationship_policy(
                        opener_page_id,
                        &creator,
                        initial_url.as_deref(),
                        noopener,
                    )?
                };
                let target_name = if force_blank || target_name.eq_ignore_ascii_case("_blank") {
                    String::new()
                } else {
                    target_name
                };
                let page = if let Some(source) = &document_source {
                    context_owner.new_named_popup_from_document(
                        opener_page_id,
                        source,
                        noopener,
                        target_name,
                    )?
                } else {
                    self.context_owner()?.new_named_popup_from_creator(
                        opener_page_id,
                        &creator,
                        noopener,
                        target_name,
                    )?
                };
                if defer_navigation {
                    context_owner.native_initial_document_creator(page.page_id())?;
                }
                let opener_scope = StorageScope {
                    origin: opener_origin.clone(),
                    top_level_page_id: opener_page_id,
                };
                let popup_scope = StorageScope {
                    origin: opener_origin,
                    top_level_page_id: page.page_id(),
                };
                if !noopener
                    && let StorageResponse::Entries { entries } = self
                        .context_owner()?
                        .context_store()
                        .execute(StorageRequest::Entries {
                            area: StorageArea::Session,
                            scope: opener_scope,
                        })
                        .map_err(|error| RuntimeError::ContextService(error.to_string()))?
                {
                    for (key, value) in entries {
                        self.context_owner()?
                            .context_store()
                            .execute(StorageRequest::Set {
                                area: StorageArea::Session,
                                scope: popup_scope.clone(),
                                key,
                                value,
                            })
                            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                    }
                }
                let selected_target = Some(if let Some(source) = &document_source {
                    context_owner.capture_document_navigation_target(
                        opener_page_id,
                        source,
                        page.page_id(),
                        None,
                    )?
                } else {
                    context_owner.capture_navigation_target(
                        opener_page_id,
                        &creator,
                        page.page_id(),
                        None,
                    )?
                });
                // Window.open has already exposed the new auxiliary's initial
                // about:blank Document. Opening that exact URL must not enqueue
                // a second document that later discards synchronous author edits.
                // Existing targets and form/link navigations retain their normal
                // navigation path.
                let retain_initial_blank = !link_navigation
                    && document_source.is_none()
                    && form_authority.is_none()
                    && post.is_none()
                    && initial_url.as_deref() == Some("about:blank");
                let related_root = context_owner.related_root_admission(page.page_id());
                // The source command still owns the shared worker dispatcher. A
                // parent goto here would wait for that command to finish before
                // its auxiliary could load. Only the native Window.open producer
                // accepts this transfer into the newly admitted related actor.
                let related_navigation_url = if local_related_navigation
                    && related_root.is_some()
                    && !noopener
                    && !defer_navigation
                    && !retain_initial_blank
                    && !link_navigation
                    && document_source.is_none()
                    && form_authority.is_none()
                    && post.is_none()
                    && referrer_policy.is_none()
                {
                    initial_url.clone()
                } else {
                    None
                };
                if let Some(url) = initial_url.filter(|_| {
                    !defer_navigation && !retain_initial_blank && related_navigation_url.is_none()
                }) {
                    if link_navigation {
                        let source = document_source.clone().ok_or_else(|| {
                            RuntimeError::ProtocolViolation(
                                "selected link document source missing".into(),
                            )
                        })?;
                        context_owner.queue_document_navigation(
                            crate::runtime::DocumentNavigationInputs {
                                page_id: page.page_id(),
                                url,
                                source,
                                referrer_policy,
                                user_initiated,
                                selected: None,
                                hyperlink_audit: hyperlink_audit.take(),
                            },
                        )?;
                    } else {
                        self.context_owner()?.queue_resource_navigation(
                            page.page_id(),
                            url,
                            post,
                            referrer_policy,
                            form_authority
                                .as_ref()
                                .map(|authority| {
                                    context_owner.retarget_form_authority(
                                        authority,
                                        opener_page_id,
                                        page.page_id(),
                                    )
                                })
                                .transpose()?,
                        )?;
                    }
                }
                Ok(ContextServiceResponse::PageOpened {
                    blank_prerender: None,
                    selected_frame: None,
                    selected_target,
                    related_root,
                    related_navigation_url,
                    page_id: page.page_id(),
                    created: true,
                    opener_page_id: (!noopener).then_some(opener_page_id),
                })
            }
            ContextServiceRequest::ActivatePage {
                source_page_id,
                target_page_id,
                creator,
                expected,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot impersonate another page".into(),
                    ));
                }
                self.context_owner()?.activate_page_from_creator(
                    source_page_id,
                    &creator,
                    target_page_id,
                    expected,
                )?;
                Ok(ContextServiceResponse::PageActivated {
                    page_id: target_page_id,
                })
            }
            ContextServiceRequest::CommitPage { page_id, url } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot commit another page".into(),
                    ));
                }
                self.context_owner()?
                    .validate_page_commit_url(page_id, &url)?;
                self.context_owner()?.commit_page_url(page_id, url)?;
                Ok(ContextServiceResponse::PageCommitted)
            }
            ContextServiceRequest::CommitPreparedLoadHtml {
                page_id,
                token,
                document_url,
                headers,
                body_sha256,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "worker cannot commit another page's HTML".into(),
                    ));
                }
                self.context_owner()?.commit_prepared_load_html(
                    page_id,
                    &token,
                    &document_url,
                    &headers,
                    &body_sha256,
                )?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::CommitNavigationResponsePolicy { .. }
            | ContextServiceRequest::DiscardNavigationResponsePolicy { .. } => {
                // Only the authenticated worker router owns these grants.
                // Direct service callers cannot turn their response headers
                // into parent policy, including in unit-test builds.
                Err(RuntimeError::ProtocolViolation(
                    "navigation response policy requires its owning worker route".into(),
                ))
            }
            ContextServiceRequest::InstallDocumentSecurity {
                page_id,
                document_url,
                headers,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot install policy for another page".into(),
                    ));
                }
                if self.trusted_in_process_runtime {
                    self.context_owner()?.install_document_security(
                        page_id,
                        document_url,
                        &headers,
                    )?;
                } else {
                    return Err(RuntimeError::ProtocolViolation(
                        "worker document policy requires a native response or prepared-document grant".into(),
                    ));
                }
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::ReportRealmCspRequest {
                page_id,
                creator,
                request,
                redirected,
                original_url,
            } => {
                if page_id != self.source_page_id
                    || request.url.len() > 65536
                    || original_url.as_ref().is_some_and(|url| url.len() > 65536)
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "invalid realm CSP report request".into(),
                    ));
                }
                let owner = self.script_document_owner(page_id, &creator)?;
                let (_, _, policy) = context_owner
                    .native_worker_authority(&owner)
                    .map_err(|failure| RuntimeError::ContextService(failure.to_string()))?;
                let policy = crate::csp::CspPolicySet::from_agent_policy(&policy);
                let decision = policy.authorize_redirect_from(
                    &request,
                    redirected,
                    original_url.as_deref().unwrap_or(&request.url),
                );
                context_owner.handle_csp_violations_for_creator(
                    page_id,
                    decision.violations,
                    Some(&policy),
                    Some(creator.clone()),
                )?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::ParserEncodingPolicy {
                page_id,
                creator,
                token,
                action,
            } => {
                self.validate_socket_owner(page_id)?;
                let oryn_js::worker_wire::Creator::Window {
                    target,
                    document_generation,
                    stream_epoch,
                } = creator
                else {
                    return Err(RuntimeError::ProtocolViolation(
                        "parser encoding requires Window owner".into(),
                    ));
                };
                context_owner.parser_encoding_policy(
                    crate::agent_registry::AgentOwner::Window {
                        page: page_id,
                        target,
                        generation: document_generation,
                        epoch: stream_epoch,
                    },
                    &token,
                    match action {
                        ParserEncodingPolicyAction::Prepare => 0,
                        ParserEncodingPolicyAction::Restore => 1,
                        ParserEncodingPolicyAction::Release => 2,
                    },
                )?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::InstallRealmMetadata {
                page_id,
                creator,
                meta_csp,
                meta_referrer,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign realm metadata page".into(),
                    ));
                }
                let owner = self.script_document_owner(page_id, &creator)?;
                context_owner.install_realm_metadata(
                    &owner,
                    meta_csp.as_deref(),
                    meta_referrer.as_deref(),
                )?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            // Kept as wire tombstones so old/forged clients fail explicitly.
            // Only exact native document metadata may alter realm policy.
            ContextServiceRequest::InstallMetaSecurity { .. }
            | ContextServiceRequest::InstallReferrerPolicy { .. } => {
                Err(RuntimeError::ProtocolViolation(
                    "page-only metadata publication is retired; exact document authority required"
                        .into(),
                ))
            }
            ContextServiceRequest::ReportCspViolations {
                page_id,
                violations,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot report violations for another page".into(),
                    ));
                }
                if violations.len() > 32 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "CSP violation batch".into(),
                    });
                }
                self.context_owner()?
                    .handle_csp_violations(page_id, violations)?;
                Ok(ContextServiceResponse::CspViolationsAccepted)
            }
            ContextServiceRequest::GetDocumentCookie { page_id, creator } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot read another page's cookies".into(),
                    ));
                }
                Ok(ContextServiceResponse::DocumentCookie {
                    value: self.context_owner()?.document_cookie_for_creator(
                        page_id,
                        creator.as_ref().ok_or_else(|| {
                            RuntimeError::ProtocolViolation(
                                "cookie read requires native document authority".into(),
                            )
                        })?,
                        None,
                    )?,
                })
            }
            ContextServiceRequest::SetDocumentCookie {
                page_id,
                creator,
                value,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot write another page's cookies".into(),
                    ));
                }
                self.context_owner()?.document_cookie_for_creator(
                    page_id,
                    creator.as_ref().ok_or_else(|| {
                        RuntimeError::ProtocolViolation(
                            "cookie write requires native document authority".into(),
                        )
                    })?,
                    Some(value),
                )?;
                Ok(ContextServiceResponse::DocumentCookieSet)
            }
            ContextServiceRequest::GetNativeDocumentAuthority {
                page_id,
                document_url,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "foreign native document authority route".into(),
                    ));
                }
                let (serialized, opaque_key, secure_context, document_generation, initial_document) =
                    self.context_owner()?
                        .native_document_authority(page_id, &document_url)?;
                Ok(ContextServiceResponse::NativeDocumentAuthority {
                    serialized,
                    opaque_key,
                    secure_context,
                    document_generation,
                    initial_document,
                })
            }
            ContextServiceRequest::GetSecurityOrigin { page_id } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot read another page's security origin".into(),
                    ));
                }
                Ok(ContextServiceResponse::SecurityOrigin {
                    serialized: self.context_owner()?.page_origin(page_id)?,
                    opaque: self.context_owner()?.is_opaque_origin(page_id)?,
                })
            }
            ContextServiceRequest::MaterializeUpload { page_id, label } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot materialize an upload for another page".into(),
                    ));
                }
                let file = self
                    .context_owner()?
                    .materialize_upload_label(page_id, &label)?;
                use base64::Engine as _;
                Ok(ContextServiceResponse::UploadMaterialized {
                    filename: file.filename,
                    media_type: file.media_type,
                    body_base64: base64::engine::general_purpose::STANDARD.encode(file.bytes),
                    last_modified: 0,
                })
            }
            ContextServiceRequest::NavigatePage { page_id, url } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot navigate another page".into(),
                    ));
                }
                self.context_owner()?.validate_page_url(page_id, &url)?;
                let source_origin = self.context_owner()?.page_origin(page_id)?;
                let resolved = self
                    .context_owner()?
                    .resolve_navigation_url(page_id, &url)?;
                let target_origin = resolved.origin().ascii_serialization();
                if source_origin != target_origin
                    && source_origin != "null"
                    && let Err(error) = self.context_owner()?.require_policy(
                        PolicyRequest {
                            action: ConsequentialAction::CrossOriginNavigation,
                            source_origin,
                            target_origin: Some(target_origin),
                            detail: Some(resolved.to_string()),
                        },
                        None,
                    )
                {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: error.to_string(),
                    });
                }
                self.context_owner()?
                    .queue_script_navigation(page_id, url)?;
                Ok(ContextServiceResponse::NavigationQueued)
            }
            ContextServiceRequest::CaptureCommandNavigationSource {
                page_id,
                initial_only,
            } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::CommandNavigationSource {
                    creator: if initial_only {
                        context_owner.native_initial_document_creator(page_id)?
                    } else {
                        context_owner.capture_command_navigation_source(page_id)?
                    },
                })
            }
            ContextServiceRequest::AuthorizeNavigation {
                page_id,
                source,
                url,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot authorize navigation for another page".into(),
                    ));
                }
                if let NavigationPolicySource::Prerender {
                    token,
                    generation,
                    epoch,
                } = &source
                {
                    context_owner.authorize_prerender_navigation(
                        page_id,
                        token,
                        (*generation, *epoch),
                        &url,
                    )?;
                    return Ok(ContextServiceResponse::NavigationAuthorized);
                }
                self.context_owner()?.validate_page_url(page_id, &url)?;
                let creator = match source {
                    NavigationPolicySource::Document(creator) => creator,
                    NavigationPolicySource::InitialAboutBlank => {
                        context_owner.native_initial_document_creator(page_id)?
                    }
                    NavigationPolicySource::Prerender { .. } => {
                        unreachable!("prerender scope handled before document lookup")
                    }
                };
                let owner = self.script_document_owner(page_id, &creator)?;
                let (_, origin, policy) = context_owner
                    .native_worker_authority(&owner)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let source_origin = origin.serialized;
                let resolved = Url::parse(&policy.url)
                    .and_then(|base| base.join(&url))
                    .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
                let target_origin = resolved.origin().ascii_serialization();
                if source_origin != target_origin && source_origin != "null" {
                    let request = PolicyRequest {
                        action: ConsequentialAction::CrossOriginNavigation,
                        source_origin,
                        target_origin: Some(target_origin),
                        detail: Some(resolved.to_string()),
                    };
                    let authorization = if self
                        .context_owner()?
                        .navigation_chain_is_preconfirmed(page_id)
                    {
                        self.context_owner()?.require_preconfirmed_policy(request)
                    } else {
                        self.context_owner()?.require_policy(request, None)
                    };
                    if let Err(error) = authorization {
                        return Ok(ContextServiceResponse::PageDenied {
                            reason: error.to_string(),
                        });
                    }
                }
                Ok(ContextServiceResponse::NavigationAuthorized)
            }
            ContextServiceRequest::AuthorizeFormSubmission {
                report_source,
                page_id,
                authority,
                url,
                method,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot authorize form submission for another page".into(),
                    ));
                }
                self.context_owner()?.validate_page_url(page_id, &url)?;
                let resolved = self
                    .context_owner()?
                    .resolve_navigation_url(page_id, &url)?;
                let source_origin = if let Some(authority) = &authority {
                    let (origin, decision) = context_owner.form_policy_for_navigation_report(
                        authority,
                        page_id,
                        resolved.as_str(),
                        report_source.is_some(),
                        report_source.as_deref(),
                    )?;
                    if !decision.allowed {
                        return Ok(ContextServiceResponse::PageDenied {
                            reason: "form submission blocked by original source policy".into(),
                        });
                    }
                    origin
                } else {
                    if context_owner
                        .csp_sandbox(page_id)?
                        .is_some_and(|sandbox| !sandbox.allow_forms)
                    {
                        return Ok(ContextServiceResponse::PageDenied {
                            reason: "Content-Security-Policy sandbox denied form submission".into(),
                        });
                    }
                    let decision = context_owner.authorize_csp(
                        page_id,
                        &crate::csp::CspRequest {
                            destination: crate::csp::CspDestination::Form,
                            url: resolved.to_string(),
                            nonce: None,
                            inline_source: None,
                            parser_inserted: false,
                            initiated_by_trusted_script: false,
                        },
                    )?;
                    context_owner.handle_csp_violations(page_id, decision.violations)?;
                    if !decision.allowed {
                        return Ok(ContextServiceResponse::PageDenied {
                            reason:
                                "form submission blocked by Content-Security-Policy form-action"
                                    .into(),
                        });
                    }
                    context_owner.page_origin(page_id)?
                };
                let target_origin = resolved.origin().ascii_serialization();
                let policy_request = PolicyRequest {
                    action: ConsequentialAction::FormSubmission,
                    source_origin,
                    target_origin: Some(target_origin),
                    detail: Some(format!("{} {}", method.to_uppercase(), resolved)),
                };
                let authorization = if self.context_owner()?.consume_form_submission_grant(page_id)
                {
                    self.context_owner()?
                        .require_preconfirmed_policy(policy_request)
                } else {
                    self.context_owner()?.require_policy(policy_request, None)
                };
                if let Err(error) = authorization {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: error.to_string(),
                    });
                }
                Ok(ContextServiceResponse::FormSubmissionAuthorized)
            }
            ContextServiceRequest::ReadScriptCloseState {
                source_page_id,
                creator,
                target_page_id,
                expected,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign closing-state endpoint".into(),
                    ));
                }
                Ok(ContextServiceResponse::ScriptCloseState {
                    closing: self.context_owner()?.script_close_state(
                        source_page_id,
                        &creator,
                        target_page_id,
                        expected,
                    )?,
                })
            }
            ContextServiceRequest::BeginScriptCloseHistory {
                page_id,
                creator,
                revision,
                count,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign history projection endpoint".into(),
                    ));
                }
                self.context_owner()?.change_script_close_history(
                    page_id,
                    &creator,
                    crate::runtime::script_close_history_projection::Change::Begin,
                    revision,
                    count,
                )?;
                Ok(ContextServiceResponse::ScriptCloseHistoryPublished)
            }
            ContextServiceRequest::PublishScriptCloseHistory {
                page_id,
                creator,
                revision,
                count,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign history projection endpoint".into(),
                    ));
                }
                self.context_owner()?.change_script_close_history(
                    page_id,
                    &creator,
                    crate::runtime::script_close_history_projection::Change::Publish,
                    revision,
                    count,
                )?;
                Ok(ContextServiceResponse::ScriptCloseHistoryPublished)
            }
            ContextServiceRequest::AbortScriptCloseHistory {
                page_id,
                creator,
                revision,
                count,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign history projection endpoint".into(),
                    ));
                }
                self.context_owner()?.change_script_close_history(
                    page_id,
                    &creator,
                    crate::runtime::script_close_history_projection::Change::Abort,
                    revision,
                    count,
                )?;
                Ok(ContextServiceResponse::ScriptCloseHistoryPublished)
            }
            ContextServiceRequest::RollbackAuxiliaryRoot { .. } => {
                unreachable!("rollback handled before live admission")
            }
            ContextServiceRequest::BeginActionCloseScope { page_id, action_id } => {
                self.validate_socket_owner(page_id)?;
                self.context_owner()?
                    .begin_action_close_scope(page_id, action_id)?;
                Ok(ContextServiceResponse::ActionCloseScopeUpdated)
            }
            ContextServiceRequest::EndActionCloseScope {
                page_id,
                action_id,
                retain,
            } => {
                self.validate_socket_owner(page_id)?;
                self.context_owner()?
                    .end_action_close_scope(page_id, action_id, retain)?;
                Ok(ContextServiceResponse::ActionCloseScopeUpdated)
            }
            ContextServiceRequest::ClosePage {
                source_page_id,
                target_page_id,
                creator,
                expected,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "foreign script-close endpoint".into(),
                    ));
                }
                if !self.context_owner()?.request_script_close(
                    source_page_id,
                    &creator,
                    target_page_id,
                    expected,
                )? {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "target is not script-closable by this incumbent Window".into(),
                    });
                }
                Ok(ContextServiceResponse::PageClosed)
            }
            ContextServiceRequest::PollEvents {
                page_id,
                creator,
                after_event_id,
                max_events,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot poll another page's events".into(),
                    ));
                }
                Ok(ContextServiceResponse::Events {
                    events: self.context_owner()?.poll_context_events(
                        page_id,
                        creator.as_ref().ok_or_else(|| {
                            RuntimeError::ProtocolViolation(
                                "event polling requires native creator".into(),
                            )
                        })?,
                        after_event_id,
                        max_events.min(32),
                    )?,
                })
            }
            ContextServiceRequest::LoadWorkerScript { .. } => Err(RuntimeError::ProtocolViolation(
                "page-only worker loading is retired; native worker admission required".into(),
            )),
            ContextServiceRequest::CaptureScriptPolicy { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                let owner = self.script_document_owner(page_id, &creator)?;
                let context = self.context_owner()?;
                let (_, _, policy) = context
                    .native_worker_authority(&owner)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                // Cached documents retain their preparation snapshots but cannot
                // use them until the live Creator admission check succeeds.
                self.script_policies
                    .retain_live(|owner| context.retained_worker_authority(owner).is_ok())?;
                self.script_document_owner(page_id, &creator)?;
                Ok(ContextServiceResponse::CapturedScriptPolicy {
                    authority: self.script_policies.capture(owner, policy)?,
                })
            }
            ContextServiceRequest::CaptureScriptSourcePolicy {
                page_id,
                creator,
                source,
            } => {
                self.validate_socket_owner(page_id)?;
                let owner = self.script_document_owner(page_id, &creator)?;
                let context = self.context_owner()?;
                let fetch = context.live_document_fetch_source(page_id, &creator, &source)?;
                let (_, _, policy) = context
                    .native_worker_authority(&owner)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                // The sealed source supplies URL only. Current native authority
                // still supplies CSP/isolation; preserving policy-self identity
                // prevents history rewrites from changing CSP 'self'.
                let document_self_url = policy.document_self_url.clone();
                let mut policy = policy.with_document_url(fetch.url);
                policy.document_self_url = document_self_url;
                self.script_policies
                    .retain_live(|owner| context.retained_worker_authority(owner).is_ok())?;
                self.script_document_owner(page_id, &creator)?;
                Ok(ContextServiceResponse::CapturedScriptPolicy {
                    authority: self.script_policies.capture(owner, policy)?,
                })
            }
            ContextServiceRequest::ReserveScriptLoad { page_id, creator } => {
                self.validate_socket_owner(page_id)?;
                let owner = self.script_document_owner(page_id, &creator)?;
                Ok(ContextServiceResponse::ScriptLoadReserved {
                    token: self.script_loads.reserve_for(owner)?,
                })
            }
            ContextServiceRequest::SelectSupportFrameResponse { page_id, stream_id, creator } => {
                self.select_support_frame_response(page_id, stream_id, &creator)
            }
            ContextServiceRequest::ReleaseFrameResponseStream { .. }
            | ContextServiceRequest::ReleaseNetworkResult { .. } => {
                unreachable!("network cleanup handled before live context admission")
            }
            ContextServiceRequest::CancelScriptLoad { .. } => {
                unreachable!("cleanup is handled before live context admission")
            }
            ContextServiceRequest::PreloadResource {
                page_id,
                token,
                request,
            } => self.load_preload_resource(page_id, token, *request),
            ContextServiceRequest::AuthorizePreloadConsumption { proof, request } => {
                self.authorize_preload_consumption(&proof, &request)
            }
            ContextServiceRequest::ValidateStylesheetResponse {
                page_id,
                creator,
                id,
                url,
                body_sha256,
            } => {
                self.validate_socket_owner(page_id)?;
                let owner = crate::stylesheet_response_proof::issued_owner(page_id, &creator)?;
                let origin_clean = self.script_loads.validate_stylesheet_response(
                    &owner,
                    id,
                    &url,
                    &body_sha256,
                )?;
                let origin_clean = if self
                    .context_owner()?
                    .native_worker_authority(&owner)
                    .is_ok()
                {
                    origin_clean
                } else {
                    None
                };
                Ok(ContextServiceResponse::StylesheetResponseValidated { origin_clean })
            }
            ContextServiceRequest::ReleaseStylesheetResponse {
                page_id,
                creator,
                id,
            } => {
                self.validate_socket_owner(page_id)?;
                let owner = crate::stylesheet_response_proof::issued_owner(page_id, &creator)?;
                self.script_loads.release_stylesheet_response(&owner, id)?;
                Ok(ContextServiceResponse::StylesheetResponseReleased)
            }
            ContextServiceRequest::RunScriptLoad {
                page_id,
                token,
                request,
                ..
            } => {
                self.validate_socket_owner(page_id)?;
                let creator = match request.as_ref() {
                    ContextServiceRequest::LoadScript { creator, .. }
                    | ContextServiceRequest::LoadTypedModule { creator, .. } => creator,
                    ContextServiceRequest::LoadPageResource { initiator, .. } => &initiator.creator,
                    _ => {
                        return Err(RuntimeError::ProtocolViolation(
                            "script lease request type".into(),
                        ));
                    }
                };
                let owner = self.script_document_owner(page_id, creator)?;
                let lease = self.script_loads.claim_for(token, &owner)?;
                let lease_cancel = lease.cancel.clone();
                match *request {
                    ContextServiceRequest::LoadScript {
                        fallback_encoding,
                        page_id: inner,
                        creator,
                        url,
                        module,
                        nonce,
                        integrity,
                        cross_origin,
                        initiator_url,
                        referrer_policy,
                        referrer_source,
                        script_policy,
                        parser_inserted,
                    } if inner == page_id => self.load_page_script_with_cancel(PageScriptInputs {
                        page_id,
                        requested_url: &url,
                        module,
                        nonce: nonce.as_deref(),
                        initiator_url: initiator_url.as_deref(),
                        referrer_policy: referrer_policy.as_deref(),
                        integrity: integrity.as_deref(),
                        cross_origin: cross_origin.as_deref(),
                        referrer_source: referrer_source.as_deref(),
                        kind: None,
                        cancellation: Some(&lease_cancel),
                        creator: Some(&creator),
                        script_policy: script_policy.as_deref(),
                        parser_inserted,
                        fallback_encoding: fallback_encoding.as_deref(),
                    }),
                    ContextServiceRequest::LoadTypedModule {
                        page_id: inner,
                        creator,
                        url,
                        kind,
                        nonce,
                        integrity,
                        cross_origin,
                        initiator_url,
                        referrer_policy,
                        referrer_source,
                        script_policy,
                        parser_inserted,
                    } if inner == page_id => self.load_page_script_with_cancel(PageScriptInputs {
                        page_id,
                        requested_url: &url,
                        module: true,
                        nonce: nonce.as_deref(),
                        initiator_url: initiator_url.as_deref(),
                        referrer_policy: referrer_policy.as_deref(),
                        integrity: integrity.as_deref(),
                        cross_origin: cross_origin.as_deref(),
                        referrer_source: referrer_source.as_deref(),
                        kind: Some(kind),
                        cancellation: Some(&lease_cancel),
                        creator: Some(&creator),
                        script_policy: script_policy.as_deref(),
                        parser_inserted,
                        fallback_encoding: None,
                    }),
                    ContextServiceRequest::LoadPageResource {
                        page_id: inner,
                        url,
                        destination,
                        download,
                        script_authority,
                        initiator,
                        form_authority,
                        initiator_url,
                        referrer_policy,
                        parent_encoding,
                        post,
                    } if inner == page_id => self.load_page_resource_with_cancel(
                        page_id,
                        &url,
                        &destination,
                        PageResourceOptions {
                            response_lease: Some(lease),
                            download: download.as_deref(),
                            script_authority,
                            initiator: &initiator,
                            form_authority,
                            initiator_url: initiator_url.as_deref(),
                            referrer_policy: referrer_policy.as_deref(),
                            parent_encoding: parent_encoding.as_deref(),
                            post,
                        },
                        &lease_cancel,
                    ),
                    _ => Err(RuntimeError::ProtocolViolation(
                        "script lease accepts only its owning page script request".into(),
                    )),
                }
            }
            ContextServiceRequest::LoadScript {
                fallback_encoding,
                page_id,
                creator,
                url,
                module,
                nonce,
                integrity,
                cross_origin,
                initiator_url,
                referrer_policy,
                referrer_source,
                script_policy,
                parser_inserted,
            } => self.load_page_script_with_cancel(PageScriptInputs {
                page_id,
                requested_url: &url,
                module,
                nonce: nonce.as_deref(),
                initiator_url: initiator_url.as_deref(),
                referrer_policy: referrer_policy.as_deref(),
                integrity: integrity.as_deref(),
                cross_origin: cross_origin.as_deref(),
                referrer_source: referrer_source.as_deref(),
                kind: None,
                cancellation: None,
                creator: Some(&creator),
                script_policy: script_policy.as_deref(),
                parser_inserted,
                fallback_encoding: fallback_encoding.as_deref(),
            }),
            ContextServiceRequest::LoadTypedModule {
                nonce,
                page_id,
                creator,
                url,
                kind,
                integrity,
                cross_origin,
                initiator_url,
                referrer_policy,
                referrer_source,
                script_policy,
                parser_inserted,
            } => self.load_page_script_with_cancel(PageScriptInputs {
                page_id,
                requested_url: &url,
                module: true,
                nonce: nonce.as_deref(),
                initiator_url: initiator_url.as_deref(),
                referrer_policy: referrer_policy.as_deref(),
                integrity: integrity.as_deref(),
                cross_origin: cross_origin.as_deref(),
                referrer_source: referrer_source.as_deref(),
                kind: Some(kind),
                cancellation: None,
                creator: Some(&creator),
                script_policy: script_policy.as_deref(),
                parser_inserted,
                fallback_encoding: None,
            }),
            ContextServiceRequest::LoadPageResource {
                download,
                script_authority,
                page_id,
                initiator,
                form_authority,
                url,
                destination,
                parent_encoding,
                initiator_url,
                referrer_policy,
                post,
            } => self.load_page_resource(
                page_id,
                &url,
                &destination,
                PageResourceOptions {
                    response_lease: None,
                    download: download.as_deref(),
                    script_authority,
                    initiator: &initiator,
                    form_authority,
                    initiator_url: initiator_url.as_deref(),
                    referrer_policy: referrer_policy.as_deref(),
                    parent_encoding: parent_encoding.as_deref(),
                    post,
                },
            ),
            ContextServiceRequest::AuthorizeTrustedTypes {
                page_id,
                creator,
                check,
            } => self.authorize_trusted_types(page_id, &creator, &check),
            ContextServiceRequest::AuthorizeInlineStyle {
                page_id,
                creator,
                source,
                nonce,
                attribute,
            } => {
                self.authorize_inline_style(page_id, &creator, &source, nonce.as_deref(), attribute)
            }
            ContextServiceRequest::AuthorizeInlineScript {
                page_id,
                creator,
                parser_inserted,
                script_policy,
                speculation_rules,
                source,
                nonce,
            } => self.authorize_inline_script_kind(InlineScriptAuthorization {
                page_id,
                creator: creator.as_ref(),
                source: &source,
                nonce: nonce.as_deref(),
                parser_inserted,
                script_policy: script_policy.as_deref(),
                speculation_rules,
            }),
            ContextServiceRequest::WebSocketOpen {
                request_id,
                page_id,
                creator,
                url,
                protocols,
            } => self.open_websocket(page_id, &url, &protocols, creator.as_ref(), request_id),
            ContextServiceRequest::WebSocketSend {
                page_id,
                creator,
                socket_id,
                frame,
            } => {
                self.validate_websocket_creator(page_id, socket_id, creator.as_ref(), true)?;
                let response = self.send_websocket(page_id, socket_id, frame)?;
                self.validate_websocket_creator(page_id, socket_id, creator.as_ref(), true)?;
                Ok(response)
            }
            ContextServiceRequest::WebSocketPoll {
                page_id,
                creator,
                socket_id,
                max_frames,
            } => {
                self.validate_websocket_creator(page_id, socket_id, creator.as_ref(), true)?;
                let response = self.poll_websocket(page_id, socket_id, max_frames)?;
                self.validate_websocket_creator(page_id, socket_id, creator.as_ref(), true)?;
                Ok(response)
            }
            ContextServiceRequest::WebSocketClose {
                page_id,
                creator,
                socket_id,
                code,
                reason,
            } => {
                self.validate_websocket_creator(page_id, socket_id, creator.as_ref(), false)?;
                self.close_websocket(page_id, socket_id, code, reason)
            }
        }
    }

    fn validate_socket_owner(&self, page_id: PageId) -> Result<(), RuntimeError> {
        if page_id != self.source_page_id {
            return Err(RuntimeError::ContextService(
                "worker cannot operate another page's WebSocket".into(),
            ));
        }
        Ok(())
    }

    fn start_http_stream(
        &self,
        page_id: PageId,
        request: HttpStreamRequest,
        destination: crate::csp::CspDestination,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.start_http_stream_with_cors(page_id, request, destination, true)
    }

    fn start_http_stream_with_cors(
        &self,
        page_id: PageId,
        request: HttpStreamRequest,
        destination: crate::csp::CspDestination,
        cors_enabled: bool,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.start_http_stream_with_destination(HttpStreamDestinationInputs {
            page_id,
            request,
            destination,
            cors_enabled,
            media_destination: None,
            xslt: false,
            preload: false,
            template_nonce: None,
            frozen: None,
        })
    }
    fn start_http_stream_with_destination(
        &self,
        params: HttpStreamDestinationInputs<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let HttpStreamDestinationInputs {
            page_id,
            request,
            destination,
            cors_enabled,
            media_destination,
            xslt,
            preload,
            template_nonce,
            frozen,
        } = params;

        self.validate_socket_owner(page_id)?;
        if request.upgrade_mixed_image && destination != crate::csp::CspDestination::Image {
            return Err(RuntimeError::ProtocolViolation(
                "image upgrade category on a non-image request".into(),
            ));
        }
        if let Some(fetch) = &request.fetch {
            fetch
                .validate()
                .map_err(|error| RuntimeError::ProtocolViolation(error.into()))?;
        }
        let cors_enabled = request.fetch.as_ref().map_or(cors_enabled, |fetch| {
            matches!(fetch.mode.as_str(), "cors" | "same-origin")
        });
        let keepalive = request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive);
        if keepalive && destination != crate::csp::CspDestination::Connect {
            return Err(RuntimeError::ProtocolViolation(
                "keepalive on non-Fetch destination".into(),
            ));
        }
        let keepalive_drain = keepalive
            .then(crate::keepalive_drain::Ticket::reserve)
            .transpose()
            .map_err(RuntimeError::ContextService)?;
        let mut keepalive_lease: Option<Arc<crate::keepalive::Lease>> = None;
        let context_closed = || {
            keepalive
                && self
                    .context
                    .upgrade()
                    .is_none_or(|context| !context.keepalive_context_active())
        };
        let timing_started = Instant::now();
        let mut timing_allowed = true;
        let mut response_start_micros = 0u64;
        let mut transport_timing = None;
        let mut timing_hops = Vec::new();
        let mut response_type = None;
        let mut service_response_url = None;
        let mut service_range_requested = false;
        let mut service_redirected = false;
        let mut service_cors = false;
        let deadline = Instant::now() + Duration::from_secs(30);

        let service_owner = if let Some(frozen) = &frozen {
            Some(frozen.owner.clone())
        } else {
            match request.client.as_ref() {
                Some(oryn_js::worker_wire::Creator::Window {
                    target,
                    document_generation,
                    stream_epoch,
                }) => Some(
                    self.context_owner()?
                        .window_worker_creator(
                            page_id,
                            target,
                            *document_generation,
                            *stream_epoch,
                        )?
                        .0,
                ),
                Some(oryn_js::worker_wire::Creator::Agent(_)) => {
                    return Err(RuntimeError::ProtocolViolation(
                        "Window fetch route cannot impersonate an agent".into(),
                    ));
                }
                None => None,
            }
        };
        if self
            .cancelled_http_requests
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP cancellation lock poisoned".into()))?
            .remove(&request.request_id)
        {
            return Err(RuntimeError::Network("HTTP request cancelled".into()));
        }
        let mut body = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            request.body_base64,
        )
        .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if body.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "HTTP request body".into(),
            });
        }
        let network = self
            .network
            .clone()
            .ok_or_else(|| {
                RuntimeError::ContextService("HTTP streaming broker is unavailable".into())
            })?
            .for_page(page_id.0);
        let page_url = match &frozen {
            Some(authority) => authority.top_url.clone(),
            None => self.context_owner()?.page_url(page_id)?,
        };
        let native_authority = match &frozen {
            Some(authority) => Some(authority.native.clone()),
            None => service_owner
                .as_ref()
                .map(|owner| {
                    self.context_owner()?
                        .native_worker_authority(owner)
                        .map_err(|error| RuntimeError::ContextService(error.to_string()))
                })
                .transpose()?,
        };
        let source_url = native_authority
            .as_ref()
            .map(|(_, _, policy)| policy.url.as_str())
            .unwrap_or(&page_url);
        let page_base =
            Url::parse(&page_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let source = Url::parse(source_url)
            .or_else(|_| page_base.join(source_url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let source_origin = native_authority
            .as_ref()
            .map(|(_, origin, _)| origin.serialized.clone())
            .unwrap_or_else(|| source.origin().ascii_serialization());
        let source_opaque = native_authority
            .as_ref()
            .is_some_and(|(_, origin, _)| origin.opaque);
        let native_csp = native_authority
            .as_ref()
            .map(|(_, _, policy)| crate::csp::CspPolicySet::from_agent_policy(policy));
        let embedder = match native_authority.as_ref() {
            Some((_, _, policy)) => {
                crate::agent_isolation::DocumentIsolationPolicy::from_headers(&policy.headers)
            }
            None => self.context_owner()?.page_isolation_policy(page_id)?,
        };
        let coep_reporting = match native_authority.as_ref() {
            Some((_, _, policy)) => {
                crate::coep_reporting::Policy::new(&policy.url, &policy.headers)
            }
            None => self.context_owner()?.document_reporting_policy(page_id)?,
        };
        let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
            request
                .fetch
                .as_ref()
                .map(|fetch| fetch.mode.as_str())
                .unwrap_or(if cors_enabled { "cors" } else { "no-cors" }),
            match destination {
                crate::csp::CspDestination::Image => "image",
                crate::csp::CspDestination::Script if xslt => "xslt",
                crate::csp::CspDestination::Object => "object",
                crate::csp::CspDestination::Media => media_destination.unwrap_or("video"),
                _ => "",
            },
            source_opaque,
        );
        let mut request_origin = source_origin.clone();
        let mut opaque_redirect_origin = false;
        let mut cors_response = false;
        let mut target = Url::parse(&request.url)
            .or_else(|_| source.join(&request.url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let original_request_url = target.to_string();
        let preload_key = if destination == crate::csp::CspDestination::Image {
            request
                .client
                .as_ref()
                .map(|creator| crate::preload_cache::Key {
                    page: page_id,
                    creator: creator.clone(),
                    url: {
                        let mut url = target.clone();
                        url.set_fragment(None);
                        url.to_string()
                    },
                    cors: cors_enabled,
                    isolation: embedder,
                    credentials: request.credentials_mode.clone().unwrap_or_else(|| {
                        if request.credentials {
                            "include".into()
                        } else {
                            "omit".into()
                        }
                    }),
                    integrity: request
                        .fetch
                        .as_ref()
                        .map(|f| f.integrity.clone())
                        .unwrap_or_default(),
                    range: request
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
                        .map(|(_, value)| value.clone()),
                })
        } else {
            None
        };
        let mut cached_preload = if preload {
            None
        } else {
            preload_key.as_ref().and_then(|key| self.preloads.take(key))
        };

        let cached_preload_timing = cached_preload
            .as_ref()
            .map(|response| response.timing_allowed);
        let mut preload_hops = Vec::new();
        let mut base_headers = sanitize_worker_headers(request.headers);
        let mut method = request.method;
        if request
            .fetch
            .as_ref()
            .is_some_and(|fetch| fetch.mode == "no-cors")
        {
            if !matches!(method.as_str(), "GET" | "HEAD" | "POST") {
                return Err(RuntimeError::ProtocolViolation(
                    "no-cors method is not safelisted".into(),
                ));
            }
            base_headers.retain(|(name, value)| {
                oryn_common::http_headers::no_cors_request_header(name, value)
                    || (method.eq_ignore_ascii_case("GET")
                        && request
                            .fetch
                            .as_ref()
                            .is_some_and(|fetch| fetch.privileged_range)
                        && name.eq_ignore_ascii_case("range")
                        && oryn_common::http_headers::safelisted_request_header(name, value))
            });
        }
        let credentials_mode = request.credentials_mode.clone();
        let public_request_id = request.request_id;
        let mut referrer_policy = request
            .referrer_policy
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map_or(
                match &frozen {
                    Some(authority) => authority.referrer,
                    None => match native_authority.as_ref() {
                        Some((_, _, policy)) => crate::referrer::ReferrerPolicy::default()
                            .after_response(&policy.headers),
                        None => self.context_owner()?.page_referrer_policy(page_id)?,
                    },
                },
                crate::referrer::ReferrerPolicy::parse_list,
            );
        let mut referrer_source = request
            .initiator_url
            .as_deref()
            .and_then(|value| Url::parse(value).ok())
            .filter(|value| !source_opaque && value.origin().ascii_serialization() == source_origin)
            .or_else(|| Some(source.clone()));
        let mut redirects = 0_u8;
        let mut request_upload = None;
        let (
            mut operation,
            response_status,
            response_headers,
            mut content_length,
            service_handled,
            cleanup,
        ) = loop {
            if context_closed() {
                return Err(RuntimeError::Closed);
            }
            if Instant::now() >= deadline {
                return Err(RuntimeError::Network("HTTP fetch deadline exceeded".into()));
            }
            if let Some(upgraded) = match &native_csp {
                Some(policy) => policy.upgraded_url(&target),
                None => self.context_owner()?.upgrade_csp_url(page_id, &target)?,
            } {
                target = upgraded;
            }
            upgrade_mixed_image(
                &source_origin,
                &mut target,
                request.upgrade_mixed_image
                    || destination == crate::csp::CspDestination::Media
                        && media_destination != Some("track"),
            );
            fetch_metadata.observe(Some(&source_origin), &target);
            preload_hops.push(target.to_string());
            let csp_request = crate::csp::CspRequest {
                destination,
                url: target.to_string(),
                nonce: template_nonce.flatten().map(str::to_owned),
                inline_source: None,
                parser_inserted: template_nonce.is_some(),
                initiated_by_trusted_script: false,
            };
            let csp = match &native_csp {
                Some(policy) => policy.authorize_redirect_from(
                    &csp_request,
                    redirects != 0,
                    &original_request_url,
                ),
                None => self.context_owner()?.authorize_csp_redirect_from(
                    page_id,
                    &csp_request,
                    redirects != 0,
                    &original_request_url,
                )?,
            };
            if !csp.violations.is_empty() {
                if let Some(policy) = native_csp.as_ref() {
                    if let Some(frozen) = &frozen {
                        self.context_owner()?
                            .report_worker_csp(
                                &frozen.owner,
                                policy,
                                csp.violations.clone(),
                                &page_url,
                                true,
                            )
                            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                    } else {
                        self.context_owner()?.handle_csp_violations_for_creator(
                            page_id,
                            csp.violations.clone(),
                            Some(policy),
                            request.client.clone(),
                        )?;
                    }
                } else {
                    self.context_owner()?
                        .handle_csp_violations(page_id, csp.violations.clone())?;
                }
            }
            if !csp.allowed {
                return Err(RuntimeError::ContextService(format!(
                    "request blocked by Content-Security-Policy {destination:?}"
                )));
            }
            if let Some(cached) = &cached_preload {
                for (index, url) in cached.hops.iter().enumerate().skip(1) {
                    let url = Url::parse(url)
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    enforce_mixed_content(source.as_str(), &url)?;
                    let csp_request = crate::csp::CspRequest {
                        destination,
                        url: url.to_string(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: false,
                        initiated_by_trusted_script: false,
                    };
                    let result = match &native_csp {
                        Some(policy) => policy.authorize_redirect_from(
                            &csp_request,
                            index != 0,
                            &original_request_url,
                        ),
                        None => self.context_owner()?.authorize_csp_redirect_from(
                            page_id,
                            &csp_request,
                            index != 0,
                            &original_request_url,
                        )?,
                    };
                    if !result.violations.is_empty() {
                        if let Some(policy) = &native_csp {
                            self.context_owner()?.handle_csp_violations_for_creator(
                                page_id,
                                result.violations.clone(),
                                Some(policy),
                                request.client.clone(),
                            )?;
                        } else {
                            self.context_owner()?
                                .handle_csp_violations(page_id, result.violations.clone())?;
                        }
                    }
                    if !result.allowed {
                        return Err(RuntimeError::Network(
                            "cached preload redirect blocked by current CSP".into(),
                        ));
                    }
                }
            }
            if matches!(target.scheme(), "data" | "blob") {
                if redirects != 0 {
                    return Err(RuntimeError::Network(
                        "HTTP redirect to local resource denied".into(),
                    ));
                }
                // Fetch data URLs produce a local basic response. Keep them
                // behind the same native ownership, CSP, stream quota and
                // cancellation checks as HTTP without sending them to a socket
                // broker or requiring a CORS preflight for an opaque origin.
                if method != "GET" {
                    return Err(RuntimeError::Network("local resource requires GET".into()));
                }
                let (status, local_headers, bytes) = if target.scheme() == "blob" {
                    let owner = service_owner.as_ref().ok_or_else(|| {
                        RuntimeError::ProtocolViolation(
                            "blob fetch requires authenticated document owner".into(),
                        )
                    })?;
                    let mut lookup = target.clone();
                    lookup.set_fragment(None);
                    let blob = match &frozen {
                        Some(authority) => authority
                            .blob
                            .as_ref()
                            .filter(|(url, _)| url == lookup.as_str())
                            .map(|(_, blob)| blob.as_ref().clone())
                            .ok_or_else(|| {
                                RuntimeError::Network(
                                    "keepalive blob identity was not captured".into(),
                                )
                            })?,
                        None => self
                            .context_owner()?
                            .resolve_native_blob(owner, lookup.as_str())
                            .map_err(|error| RuntimeError::Network(error.to_string()))?,
                    };
                    if blob.bytes.len() > 16 * 1024 * 1024 {
                        return Err(RuntimeError::WorkerLimitExceeded {
                            resource: "decoded blob URL response body".into(),
                        });
                    }
                    crate::local_resource_response::blob(
                        blob.bytes,
                        blob.media_type,
                        base_headers
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("range"))
                            .map(|(_, value)| value.as_str()),
                    )
                    .map_err(RuntimeError::Network)?
                } else {
                    let (content_type, bytes) = crate::network::decode_data_url(&target)
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    (
                        200,
                        vec![
                            (
                                "content-type".into(),
                                content_type
                                    .unwrap_or_else(|| "text/plain;charset=US-ASCII".into()),
                            ),
                            ("content-length".into(), bytes.len().to_string()),
                        ],
                        bytes,
                    )
                };
                if bytes.len() > 16 * 1024 * 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "decoded data URL response body".into(),
                    });
                }
                let length = bytes.len();
                let (sender, events) = tokio::sync::mpsc::channel(2);
                sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: public_request_id,
                        bytes: bytes.into(),
                    })
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                sender
                    .try_send(NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: public_request_id,
                    })
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                break (
                    NetworkOperation {
                        body_demand_pending: Default::default(),
                        prefetched: Default::default(),
                        request_id: public_request_id,
                        events,
                        demand: None,
                    },
                    status,
                    local_headers,
                    Some(length),
                    false,
                    None,
                );
            }
            enforce_mixed_content(source.as_str(), &target)?;
            let cross_origin = source_opaque
                || opaque_redirect_origin
                || target.origin().ascii_serialization() != source_origin;
            if request
                .fetch
                .as_ref()
                .is_some_and(|fetch| fetch.mode == "same-origin")
                && cross_origin
            {
                return Err(RuntimeError::Network(
                    "same-origin fetch crossed an origin boundary".into(),
                ));
            }
            cors_response |= cross_origin;
            let credentials = match credentials_mode.as_deref() {
                Some("include") => true,
                Some("omit") => false,
                Some("same-origin") => !cross_origin,
                _ => request.credentials,
            };
            let credentials = credentials && !embedder.omit_credentials(cors_enabled, cross_origin);
            let mut headers = base_headers.clone();
            if destination == crate::csp::CspDestination::Prefetch {
                headers.push(("sec-purpose".into(), "prefetch".into()));
            }
            if let Some(origin) = crate::referrer::request_origin_header(
                &request_origin,
                &source,
                &target,
                &method,
                cors_enabled && cors_response,
                referrer_policy,
            ) {
                headers.push(("origin".into(), origin));
            }
            let referrer = referrer_source
                .as_ref()
                .and_then(|source| referrer_policy.header(source, &target));
            referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
            if let Some(referrer) = referrer {
                headers.push(("referer".into(), referrer));
            }
            let operation_request_id = self.next_http_identity()?;
            let cleanup = if keepalive {
                http_cleanup_custody::Ticket::reserve_keepalive(
                    &self.http_cleanup,
                    public_request_id,
                    network.clone(),
                )?
            } else {
                http_cleanup_custody::Ticket::reserve(
                    &self.http_cleanup,
                    public_request_id,
                    network.clone(),
                )?
            };
            let service_response = if let Some(cached) = cached_preload.take() {
                let mut response = cached.response;
                let bytes = std::mem::take(&mut response.body);
                let operation = crate::network::buffered_response_operation(
                    operation_request_id,
                    response.status,
                    response.headers.clone(),
                    bytes,
                )
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
                Some((response, operation))
            } else if let Some(service_owner) = service_owner.as_ref() {
                let shutdown = self.shutting_down.clone();
                let keepalive_context = self.context.clone();
                let cancelled_requests = self.cancelled_http_requests.clone();
                let cancellation_request = request.request_id;
                let cleanup_cancelled = cleanup.cancellation_probe();
                let cancelled: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
                    cleanup_cancelled()
                        || (!keepalive && shutdown.load(Ordering::Acquire))
                        || (keepalive
                            && keepalive_context
                                .upgrade()
                                .is_none_or(|context| !context.keepalive_context_active()))
                        || cancelled_requests
                            .lock()
                            .map(|requests| requests.contains(&cancellation_request))
                            .unwrap_or(true)
                });
                let service_request = oryn_js::worker_wire::AgentFetchRequest {
                    keepalive: request.fetch.as_ref().is_some_and(|fetch| fetch.keepalive),
                    privileged_range: request
                        .fetch
                        .as_ref()
                        .is_some_and(|fetch| fetch.privileged_range),
                    cache: request
                        .fetch
                        .as_ref()
                        .map_or_else(Default::default, |fetch| fetch.cache),
                    navigation: Default::default(),
                    integrity: request
                        .fetch
                        .as_ref()
                        .map(|fetch| fetch.integrity.clone())
                        .unwrap_or_default(),
                    navigation_preload: false,
                    streaming: true,
                    id: 0,
                    client_id: String::new(),
                    url: target.to_string(),
                    method: method.clone(),
                    headers: headers.clone(),
                    body: body.clone(),
                    destination: match destination {
                        crate::csp::CspDestination::Image => "image".into(),
                        crate::csp::CspDestination::Script if xslt => "xslt".into(),
                        crate::csp::CspDestination::Object => "object".into(),
                        crate::csp::CspDestination::Media => {
                            media_destination.unwrap_or("video").into()
                        }
                        _ => String::new(),
                    },
                    mode: request
                        .fetch
                        .as_ref()
                        .map(|fetch| fetch.mode.clone())
                        .unwrap_or_else(|| {
                            if cors_enabled {
                                "cors".into()
                            } else {
                                "no-cors".into()
                            }
                        }),
                    credentials: if embedder.omit_credentials(cors_enabled, cross_origin) {
                        "omit".into()
                    } else {
                        credentials_mode.clone().unwrap_or_else(|| {
                            if request.credentials {
                                "include".into()
                            } else {
                                "omit".into()
                            }
                        })
                    },
                    redirect: request
                        .fetch
                        .as_ref()
                        .map(|fetch| fetch.redirect.clone())
                        .unwrap_or_else(|| "follow".into()),
                };
                match &frozen {
                    Some(authority) => match &authority.controller {
                        Some(controller) => controller.fetch_operation(
                            service_request,
                            operation_request_id,
                            Arc::new(AtomicBool::new(false)),
                            Some(cancelled),
                        ),
                        None => Ok(None),
                    },
                    None => match self.context_owner()?.existing_service_registry() {
                        Some(services) => services.fetch_operation_with_check(
                            service_owner,
                            service_request,
                            operation_request_id,
                            Arc::new(AtomicBool::new(false)),
                            Some(cancelled),
                        ),
                        None => Ok(None),
                    },
                }
                .map_err(|error| RuntimeError::Network(error.to_string()))?
            } else {
                None
            };
            let service_handled = service_response.is_some();
            let service_timing = service_response
                .as_ref()
                .and_then(|(response, _)| response.timing.clone());
            service_range_requested = service_response
                .as_ref()
                .is_some_and(|(response, _)| response.range_requested);
            service_redirected = service_response
                .as_ref()
                .is_some_and(|(response, _)| response.redirected);
            service_cors = service_response
                .as_ref()
                .is_some_and(|(response, _)| response.response_type == "cors");
            let service_readable = service_response.as_ref().is_some_and(|(response, _)| {
                matches!(
                    response.response_type.as_str(),
                    "basic" | "cors" | "default"
                )
            });
            service_response_url = service_response
                .as_ref()
                .filter(|(response, _)| !response.response_url.is_empty())
                .map(|(response, _)| response.response_url.clone());
            if !service_handled
                && cors_enabled
                && cross_origin
                && (request.use_cors_preflight || cors_preflight_required(&method, &headers))
            {
                let requested_headers = cors_non_safelisted_headers(&headers);
                let mut preflight_headers = vec![
                    ("origin".into(), request_origin.clone()),
                    ("access-control-request-method".into(), method.clone()),
                ];
                if !requested_headers.is_empty() {
                    preflight_headers.push((
                        "access-control-request-headers".into(),
                        requested_headers.join(", "),
                    ));
                }
                let preflight = collect_operation_bounded_with_cancel(
                    &network,
                    NetworkRequest {
                        request_id: self.next_http_identity()?,
                        method: "OPTIONS".into(),
                        url: target.to_string(),
                        headers: preflight_headers,
                        body: Vec::new(),
                        credentials: false,
                        security: crate::cookies::CookieRequestContext {
                            fetch_metadata: fetch_metadata.clone(),
                            dns_timing: None,
                            credentials: crate::cookies::CookieCredentials::Omit,
                            kind: crate::cookies::CookieRequestKind::Fetch,
                            initiator_url: Some(source_origin.clone()),
                            top_level_url: Some(page_url.clone()),
                            ..Default::default()
                        },
                        resolved_addrs: Vec::new(),
                    },
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(10)),
                    64 * 1024,
                    cleanup.cancellation(),
                )
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
                enforce_cors_preflight(
                    &request_origin,
                    credentials,
                    &method,
                    &requested_headers,
                    &preflight,
                )?;
            }
            let mut operation = if let Some((response, operation)) = service_response {
                cleanup.attach(operation.request_id);
                response_type = match response.response_type.as_str() {
                    "opaque" if !cors_enabled => Some(oryn_js::platform::FetchResponseType::Opaque),
                    "opaqueredirect"
                        if request
                            .fetch
                            .as_ref()
                            .is_some_and(|fetch| fetch.redirect == "manual") =>
                    {
                        Some(oryn_js::platform::FetchResponseType::OpaqueRedirect)
                    }
                    "error" | "opaque" | "opaqueredirect" => {
                        return Err(RuntimeError::Network(
                            "unusable service-worker fetch response".into(),
                        ));
                    }
                    _ if response.status == 0 => {
                        return Err(RuntimeError::Network(
                            "unusable service-worker fetch response".into(),
                        ));
                    }
                    "basic" | "cors" | "default" => None,
                    _ => {
                        return Err(RuntimeError::Network(
                            "invalid native service response type".into(),
                        ));
                    }
                };
                operation
            } else {
                if context_closed() {
                    return Err(RuntimeError::Closed);
                }
                if keepalive && keepalive_lease.is_none() {
                    let owner = service_owner.as_ref().ok_or_else(|| {
                        RuntimeError::ProtocolViolation("keepalive requires exact client".into())
                    })?;
                    keepalive_lease = Some(Arc::new(
                        self.context_owner()?
                            .reserve_keepalive(owner, body.len())
                            .map_err(RuntimeError::Network)?,
                    ));
                }
                if let Some(lease) = &keepalive_lease {
                    cleanup.retain_keepalive(lease.clone());
                }
                cleanup.attach(operation_request_id);
                let operation = network
                    .start_operation(NetworkRequest {
                        request_id: operation_request_id,
                        method: method.clone(),
                        url: target.to_string(),
                        headers,
                        body: body.clone(),
                        credentials,
                        security: crate::cookies::CookieRequestContext {
                            fetch_metadata: fetch_metadata.clone(),
                            dns_timing: None,
                            fetch_cache: request
                                .fetch
                                .as_ref()
                                .map_or_else(Default::default, |fetch| fetch.cache),
                            credentials: if credentials {
                                crate::cookies::CookieCredentials::Include
                            } else {
                                crate::cookies::CookieCredentials::Omit
                            },
                            kind: if destination == crate::csp::CspDestination::Image {
                                crate::cookies::CookieRequestKind::Image
                            } else if destination == crate::csp::CspDestination::Media {
                                crate::cookies::CookieRequestKind::Media
                            } else {
                                crate::cookies::CookieRequestKind::Fetch
                            },
                            initiator_url: Some(source_origin.clone()),
                            top_level_url: Some(page_url.clone()),
                            ..Default::default()
                        },
                        resolved_addrs: Vec::new(),
                    })
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                if operation.request_id != operation_request_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "HTTP broker changed operation identity".into(),
                    ));
                }
                operation
            };
            if self
                .cancelled_http_requests
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("HTTP cancellation lock poisoned".into())
                })?
                .remove(&public_request_id)
            {
                cleanup.cancel();
                return Err(RuntimeError::Network("HTTP request cancelled".into()));
            }
            let header_event = match receive_network_event_with_check(
                &mut operation,
                deadline.saturating_duration_since(Instant::now()),
                cleanup.cancellation(),
                Some(&context_closed),
            ) {
                Ok(event) => event,
                Err(error) => {
                    cleanup.cancel();
                    return Err(error);
                }
            };
            if self
                .cancelled_http_requests
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("HTTP cancellation lock poisoned".into())
                })?
                .remove(&public_request_id)
            {
                cleanup.cancel();
                return Err(RuntimeError::Network("HTTP request cancelled".into()));
            }
            let (response_status, response_headers, content_length, upload, hop_timing) =
                match header_event {
                    Some(NetworkEvent::Headers {
                        response_policy: _,
                        timing,
                        request_id,
                        status,
                        headers,
                        content_length,
                        upload,
                    }) if request_id == operation.request_id => (
                        status,
                        headers,
                        content_length,
                        upload,
                        timing.map(|timing| *timing),
                    ),
                    Some(NetworkEvent::Failed { request_id, error })
                        if request_id == operation.request_id =>
                    {
                        return Err(RuntimeError::from(error));
                    }
                    Some(_) => {
                        cleanup.cancel();
                        return Err(RuntimeError::ProtocolViolation(
                            "HTTP stream received a stale, unmatched, or out-of-order event".into(),
                        ));
                    }
                    None => {
                        cleanup.cancel();
                        return Err(RuntimeError::Network(
                            "HTTP stream ended before response headers".into(),
                        ));
                    }
                };
            response_start_micros =
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            transport_timing = hop_timing.map(|timing| {
                let offset = response_start_micros.saturating_sub(timing.response_start);
                timing.shifted(offset)
            });
            if redirects == 0 {
                request_upload = upload;
            }
            let response_policy_url = service_response_url
                .as_deref()
                .and_then(|value| Url::parse(value).ok())
                .unwrap_or_else(|| target.clone());
            if let Some(metadata) = service_timing.as_ref() {
                timing_allowed &=
                    crate::network::timing_metadata_allows(metadata, &source_origin, source_opaque);
                timing_hops.extend(metadata.hops.iter().cloned());
            } else {
                timing_allowed &= crate::network::timing_allow_origin(
                    &response_headers,
                    &source_origin,
                    !source_opaque
                        && response_policy_url.origin().ascii_serialization() == source_origin,
                ) && (!service_redirected || cached_preload_timing.is_some())
                    && cached_preload_timing.unwrap_or(true);
                timing_hops.push(crate::network::timing_hop(
                    &response_policy_url,
                    &response_headers,
                ));
            }
            if timing_hops.len() > 21 {
                timing_hops.clear();
                timing_allowed = false;
            }

            let _ = self.context_owner()?.deliver_coep_reports(
                &page_url,
                coep_reporting.response(crate::coep_reporting::ResponsePolicyInputs {
                    origin: &source_origin,
                    opaque: source_opaque,
                    target: &response_policy_url,
                    cors: cors_enabled || service_readable,
                    credentials,
                    headers: &response_headers,
                    destination: match destination {
                        crate::csp::CspDestination::Image => "image",
                        crate::csp::CspDestination::Script if xslt => "xslt",
                        crate::csp::CspDestination::Object => "object",
                        crate::csp::CspDestination::Media => media_destination.unwrap_or("video"),
                        _ => "",
                    },
                }),
            );
            if let Err(error) = embedder.authorize_response(
                &source_origin,
                source_opaque,
                &response_policy_url,
                cors_enabled || service_readable,
                credentials,
                &response_headers,
            ) {
                cleanup.cancel();
                return Err(RuntimeError::Network(error));
            }
            if !service_handled
                && cors_enabled
                && cross_origin
                && !cors_headers_allowed(&request_origin, credentials, &response_headers)
            {
                cleanup.cancel();
                return Err(RuntimeError::ContextService(format!(
                    "cross-origin response denied by CORS: initiator={source_origin}, response={}",
                    target.origin().ascii_serialization()
                )));
            }
            if oryn_net::fetch::is_redirect_status(response_status)
                && let Some(location) = response_headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
            {
                if let Some(fetch) = &request.fetch {
                    match fetch.redirect.as_str() {
                        "error" => {
                            cleanup.cancel();
                            return Err(RuntimeError::Network(
                                "fetch redirect mode is error".into(),
                            ));
                        }
                        "manual" => {
                            response_type =
                                Some(oryn_js::platform::FetchResponseType::OpaqueRedirect);
                            break (
                                operation,
                                response_status,
                                response_headers,
                                content_length,
                                service_handled,
                                Some(cleanup),
                            );
                        }
                        _ => {}
                    }
                }
                // This response body will not be exposed, including on invalid Location.
                cleanup.cancel();
                if redirects >= 20 {
                    return Err(RuntimeError::Network("HTTP redirect limit exceeded".into()));
                }
                let next = target
                    .join(location)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                oryn_net::fetch::rewrite_redirect_request(
                    response_status,
                    &target,
                    &next,
                    &mut method,
                    &mut base_headers,
                    &mut body,
                );
                if let Some(lease) = &keepalive_lease {
                    lease.shrink_to(body.len());
                }
                referrer_policy = referrer_policy.after_response(&response_headers);
                if target.origin() != next.origin() && target.origin() != source.origin() {
                    request_origin = "null".into();
                    opaque_redirect_origin = true;
                }
                target = next;
                redirects += 1;
                continue;
            }
            break (
                operation,
                response_status,
                response_headers,
                content_length,
                service_handled,
                Some(cleanup),
            );
        };
        if destination != crate::csp::CspDestination::Media
            && content_length.is_some_and(|length| length > 16 * 1024 * 1024)
        {
            if let Some(cleanup) = &cleanup {
                cleanup.cancel();
            }
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "decoded HTTP response body".into(),
            });
        }
        if let Some(fetch) = &request.fetch
            && !fetch.integrity.is_empty()
        {
            let local_cancel = AtomicBool::new(false);
            let cancelled = cleanup
                .as_ref()
                .map(|ticket| ticket.cancellation())
                .unwrap_or(if keepalive {
                    &local_cancel
                } else {
                    &self.shutting_down
                });
            let bytes = crate::network::collect_started_body_with_metadata(
                &mut operation,
                deadline,
                16 * 1024 * 1024,
                cancelled,
                Some(&context_closed),
            )
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
            crate::integrity::verify(
                &fetch.integrity,
                &bytes.bytes,
                response_type.is_none()
                    && (service_handled
                        || cors_enabled
                        || !cors_response
                        || request
                            .fetch
                            .as_ref()
                            .is_some_and(|fetch| fetch.mode == "navigate")),
            )
            .map_err(|error| {
                RuntimeError::Network(format!("fetch integrity check failed: {error:?}"))
            })?;
            if let Some(cleanup) = &cleanup {
                cleanup.finish_keepalive();
            }
            drop(keepalive_lease.take());
            content_length = Some(bytes.bytes.len());
            operation = crate::network::buffered_body_operation(operation.request_id, bytes)
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
        }
        if (response_type.is_some()
            || (!service_handled
                && request
                    .fetch
                    .as_ref()
                    .is_some_and(|fetch| fetch.mode == "no-cors")
                && cors_response))
            && response_type.is_none()
        {
            response_type = Some(oryn_js::platform::FetchResponseType::Opaque);
        }
        let preload_candidate = if preload && (200..300).contains(&response_status) {
            preload_key.map(|key| crate::preload_cache::Candidate {
                key,
                response: crate::preload_cache::Response {
                    timing_allowed,
                    hops: preload_hops,
                    response: oryn_js::worker_wire::AgentFetchResponse {
                        timing: None,
                        redirected: service_redirected || redirects != 0,
                        range_requested: service_range_requested
                            || base_headers
                                .iter()
                                .any(|(name, _)| name.eq_ignore_ascii_case("range")),
                        status: response_status,
                        status_text: String::new(),
                        headers: sanitize_parent_response_headers(response_headers.clone()),
                        body: Vec::new(),
                        response_url: service_response_url
                            .clone()
                            .unwrap_or_else(|| target.to_string()),
                        response_type: if response_type.is_some()
                            || (!cors_enabled && cors_response)
                        {
                            "opaque"
                        } else if service_cors || (cors_enabled && cors_response) {
                            "cors"
                        } else {
                            "basic"
                        }
                        .into(),
                    },
                },
            })
        } else {
            None
        };
        let retained_encoded_body_size = transport_timing
            .as_ref()
            .and_then(|timing| timing.encoded_body_size);
        let timing = Some(oryn_js::platform::ResourceTimingMetadata {
            server_timing: oryn_js::server_timing::parse(&response_headers),
            response_end_micros: None,
            navigation_phases: None,
            producer_clock_micros: crate::network::timing_clock_origin(
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ),
            decoded_body_size: None,
            origin: source_origin.clone(),
            hops: timing_hops,
            transport: transport_timing,
            allow_details: timing_allowed,
            response_status,
            response_start_micros,
            encoded_body_size: retained_encoded_body_size.or_else(|| {
                response_headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.parse().ok())
            }),
            content_type: oryn_common::mime_type::MimeType::from_headers(&response_headers)
                .map(|mime| mime.essence().to_owned())
                .unwrap_or_default(),
        });
        let total_bytes = content_length;
        let stream_id = self.next_http_identity()?;
        if !keepalive && let Some(creator) = request.client.as_ref() {
            self.script_document_owner(page_id, creator)?;
        }
        if !keepalive && self.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::Closed);
        }
        self.socket_lifecycle.admit_http(
            stream_id,
            Arc::new(HttpStreamEntry {
                support: Mutex::new(None),
                keepalive_context: keepalive.then(|| self.context.clone()),
                keepalive_drain,
                page_id,
                creator: Mutex::new(request.client.clone()),
                prepared_frame_policy: None,
                frame_deadline: None,
                request_id: public_request_id,
                cleanup,
                state: Mutex::new(HttpStreamState {
                    preload: preload_candidate,
                    operation: Some(operation),
                    current: bytes::Bytes::new(),
                    offset: 0,
                    bytes_read: 0,
                    content_length: total_bytes,
                }),
            }),
        )?;
        target.set_fragment(None);
        let internal_response =
            response_type.map(|_| oryn_js::platform::InternalResponseMetadata {
                status: response_status,
                headers: sanitize_parent_response_headers(response_headers.clone()),
                url: service_response_url
                    .clone()
                    .unwrap_or_else(|| target.to_string()),
                redirected: service_redirected || redirects != 0,
                range_requested: service_range_requested
                    || base_headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("range")),
            });
        let response_headers = if !service_handled
            && cors_enabled
            && cors_response
            && destination == crate::csp::CspDestination::Connect
        {
            let include_credentials = credentials_mode
                .as_deref()
                .map_or(request.credentials, |mode| mode == "include");
            cors_exposed_response_headers(response_headers, include_credentials)
        } else {
            response_headers
        };
        target.set_fragment(None);
        Ok(ContextServiceResponse::HttpStreamStarted {
            timing,
            internal_response,
            response_type,
            upload: request_upload,
            // Image consumers use this bit to mark an opaque no-CORS image as
            // not origin-clean; authorized CORS images remain origin-clean.
            cors_filtered: response_type.is_some()
                || (service_cors && destination == crate::csp::CspDestination::Connect)
                || !service_handled
                    && cors_response
                    && ((destination == crate::csp::CspDestination::Connect && cors_enabled)
                        || (destination != crate::csp::CspDestination::Connect && !cors_enabled)),
            url: if response_type.is_some() {
                String::new()
            } else {
                service_response_url.unwrap_or_else(|| target.to_string())
            },
            redirected: response_type.is_none() && (service_redirected || redirects != 0),
            stream_id,
            request_id: public_request_id,
            status: if response_type.is_some() {
                0
            } else {
                response_status
            },
            headers: if response_type.is_some() {
                Vec::new()
            } else {
                sanitize_parent_response_headers(response_headers)
            },
            total_bytes,
        })
    }

    fn abort_http_request(
        &self,
        page_id: PageId,
        request_id: u64,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        self.socket_lifecycle.abort_request(request_id)?;
        Ok(ContextServiceResponse::HttpRequestAborted)
    }

    fn validate_http_stream_creator(
        &self,
        page: PageId,
        id: u64,
        creator: Option<&oryn_js::worker_wire::Creator>,
        read: bool,
    ) -> Result<(), RuntimeError> {
        self.validate_socket_owner(page)?;
        let entry = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream registry poisoned".into()))?
            .get(&id)
            .cloned();
        if let Some(entry) = entry {
            if entry
                .creator
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                != creator
            {
                return Err(RuntimeError::ProtocolViolation(
                    "HTTP stream belongs to another document".into(),
                ));
            }
            if read && entry.prepared_frame_policy.is_some() && creator.is_none() {
                return Err(RuntimeError::ProtocolViolation(
                    "uncommitted frame response stream".into(),
                ));
            }
            if read && let Some(creator) = creator {
                self.script_document_owner(page, creator)?;
            }
        }
        Ok(())
    }

    fn read_http_stream(
        &self,
        page_id: PageId,
        stream_id: u64,
        max_bytes: usize,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let progress = crate::ipc_progress::observed(page_id.0);
        self.validate_socket_owner(page_id)?;
        let entry = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
            .get(&stream_id)
            .cloned()
            .ok_or_else(|| RuntimeError::ContextService("unknown HTTP stream".into()))?;
        if entry.page_id != page_id {
            return Err(RuntimeError::ContextService(
                "worker cannot read another page's HTTP stream".into(),
            ));
        }
        let result = (|| {
            // Only this stream is serialized. The registry lock is not held
            // while its transport waits, so unrelated streams and aborts can
            // make progress concurrently.
            let mut stream = entry
                .state
                .lock()
                .map_err(|_| RuntimeError::ContextService("HTTP stream state poisoned".into()))?;
            crate::ipc_progress::record(
                progress,
                crate::ipc_progress::Phase::ParentBodyLockAcquired,
                Some(stream_id),
                None,
            );
            let requested = max_bytes.clamp(1, 64 * 1024);
            let mut terminal = false;
            let mut terminal_encoded_bytes = None;
            let mut chunk = Vec::with_capacity(requested);
            while chunk.len() < requested && !terminal {
                while stream.offset >= stream.current.len() {
                    stream.current = bytes::Bytes::new();
                    stream.offset = 0;
                    crate::ipc_progress::record(
                        progress,
                        crate::ipc_progress::Phase::ParentBodyGrant,
                        Some(stream_id),
                        None,
                    );
                    let operation = stream.operation.as_mut().ok_or_else(||
                        RuntimeError::ProtocolViolation("HTTP stream read after terminal".into()))?;
                    operation.request_body_chunk();
                    let timeout = entry
                        .frame_deadline
                        .map(|deadline| {
                            deadline
                                .checked_duration_since(std::time::Instant::now())
                                .ok_or_else(|| {
                                    RuntimeError::Network("frame response deadline".into())
                                })
                        })
                        .transpose()?
                        .unwrap_or(Duration::from_secs(30));
                    match receive_network_event(operation, timeout)? {
                        Some(NetworkEvent::Chunk { request_id, bytes })
                            if request_id == operation.request_id =>
                        {
                            crate::ipc_progress::record(
                                progress,
                                crate::ipc_progress::Phase::ParentChunkReceived,
                                Some(stream_id),
                                None,
                            );
                            stream.current = bytes
                        }
                        Some(NetworkEvent::Complete {
                            encoded_body_size,
                            request_id,
                        }) if request_id == operation.request_id => {
                            terminal = true;
                            terminal_encoded_bytes = encoded_body_size;
                            break;
                        }
                        Some(NetworkEvent::Failed { request_id, error })
                            if request_id == operation.request_id =>
                        {
                            return Err(RuntimeError::from(error));
                        }
                        Some(_) => {
                            return Err(RuntimeError::ProtocolViolation(
                                "HTTP stream emitted a stale, unmatched, or duplicate event".into(),
                            ));
                        }
                        None => {
                            return Err(RuntimeError::Network(
                                "HTTP stream closed without completion".into(),
                            ));
                        }
                    }
                }
                if terminal {
                    break;
                }
                let available = stream.current.len().saturating_sub(stream.offset);
                let read = (requested - chunk.len()).min(available);
                chunk.extend_from_slice(&stream.current.slice(stream.offset..stream.offset + read));
                stream.offset += read;
                if entry.prepared_frame_policy.is_some() && !chunk.is_empty() {
                    break;
                }
            }
            stream.bytes_read = stream.bytes_read.saturating_add(chunk.len());
            if let Some(preload) = &mut stream.preload {
                preload.response.response.body.extend_from_slice(&chunk);
            }
            if stream.bytes_read > 16 * 1024 * 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "decoded HTTP response body".into(),
                });
            }
            if stream.content_length.is_some_and(|length| {
                stream.bytes_read > length || (terminal && stream.bytes_read != length)
            }) {
                return Err(RuntimeError::Network(
                    "HTTP response length does not match its terminal body".into(),
                ));
            }
            if terminal {
                stream.current = bytes::Bytes::new();
                stream.offset = 0;
                // The original cleanup/proof entry remains, but its completed
                // network receiver and demand handle no longer retain IO.
                stream.operation.take();
            }
            Ok((chunk, terminal, terminal_encoded_bytes))
        })();
        let (chunk, done, encoded_body_size) = match result {
            Ok(result) => result,
            Err(error) => {
                self.http_streams
                    .lock()
                    .ok()
                    .and_then(|mut streams| streams.remove(&stream_id));
                if let Some(cleanup) = &entry.cleanup {
                    cleanup.cancel();
                }
                return Err(error);
            }
        };
        let retain_support = match self.publish_support_body(page_id, &entry, &chunk, done) {
            Ok(selected) => selected,
            Err(error) => {
                self.http_streams.lock().ok().and_then(|mut streams| streams.remove(&stream_id));
                if let Some(cleanup) = &entry.cleanup { cleanup.cancel(); }
                return Err(error);
            }
        };
        if done {
            if let Some(cleanup) = &entry.cleanup {
                cleanup.finish_keepalive();
            }
            if !retain_support {
                self.http_streams
                    .lock()
                    .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
                    .remove(&stream_id);
            }
        }
        let creator = entry
            .creator
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(creator) = creator.as_ref() {
            self.script_document_owner(page_id, creator)?;
        }
        if done
            && !self.shutting_down.load(Ordering::Acquire)
            && !entry
                .cleanup
                .as_ref()
                .is_some_and(|cleanup| cleanup.cancelled())
            && let Some(candidate) = entry
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .preload
                .take()
        {
            self.preloads.insert(candidate.key, candidate.response);
        }
        Ok(ContextServiceResponse::HttpStreamChunk {
            encoded_body_size,
            stream_id,
            body_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, chunk),
            done,
        })
    }

    fn abort_http_stream(
        &self,
        page_id: PageId,
        stream_id: u64,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let entry = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
            .get(&stream_id)
            .cloned();
        // EOF, request cancellation and an explicit body abort can race. Once
        // this endpoint's authority has been validated, absent streams need no
        // further cleanup. A live stream owned by another page is still denied.
        let Some(entry) = entry else {
            return Ok(ContextServiceResponse::HttpStreamAborted);
        };
        if entry.page_id != page_id {
            return Err(RuntimeError::ContextService(
                "worker cannot abort another page's HTTP stream".into(),
            ));
        }
        self.http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
            .remove(&stream_id);
        if let Some(cleanup) = &entry.cleanup {
            cleanup.cancel();
        }
        Ok(ContextServiceResponse::HttpStreamAborted)
    }

    fn authorize_trusted_types(
        &self,
        page_id: PageId,
        creator: &oryn_js::worker_wire::Creator,
        check: &oryn_js::trusted_types::PolicyCheck,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let bytes = check.payload_bytes();
        if bytes > MAX_PAGE_SCRIPT_BYTES {
            return Err(RuntimeError::ContextService(
                "Trusted Types check exceeds source capacity".into(),
            ));
        }
        let context = self.context_owner()?;
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "Trusted Types check requires native Window creator".into(),
            ));
        };
        let policy = crate::csp::CspPolicySet::from_agent_policy(&context.window_worker_policy(
            page_id,
            target,
            *document_generation,
            *stream_epoch,
        )?);
        let decision = policy.check_trusted_types(check);
        if !decision.violations.is_empty() {
            context.handle_csp_violations_for_creator(
                page_id,
                decision.violations,
                Some(&policy),
                Some(creator.clone()),
            )?;
        }
        context.window_worker_creator(page_id, target, *document_generation, *stream_epoch)?;
        Ok(ContextServiceResponse::TrustedTypesAuthorization {
            allowed: decision.allowed,
        })
    }

    fn authorize_inline_style(
        &self,
        page_id: PageId,
        creator: &oryn_js::worker_wire::Creator,
        source: &str,
        nonce: Option<&str>,
        attribute: bool,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        if source.len() > MAX_PAGE_SCRIPT_BYTES || nonce.is_some_and(|nonce| nonce.len() > 4096) {
            return Err(RuntimeError::ContextService(
                "inline style authorization exceeds source capacity".into(),
            ));
        }
        let context = self.context_owner()?;
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "inline style requires current native Window creator".into(),
            ));
        };
        // Policy is selected from exact native realm identity, never a URL or
        // caller-supplied policy. This includes inherited local document policy.
        let policy = crate::csp::CspPolicySet::from_agent_policy(&context.window_worker_policy(
            page_id,
            target,
            *document_generation,
            *stream_epoch,
        )?);
        let request = crate::csp::CspRequest {
            destination: if attribute {
                crate::csp::CspDestination::StyleAttribute
            } else {
                crate::csp::CspDestination::Style
            },
            url: "inline".into(),
            nonce: nonce.map(str::to_owned),
            inline_source: Some(source.into()),
            parser_inserted: false,
            initiated_by_trusted_script: false,
        };
        let decision = policy.authorize(&request);
        if !decision.violations.is_empty() {
            context.handle_csp_violations_for_creator(
                page_id,
                decision.violations,
                Some(&policy),
                Some(creator.clone()),
            )?;
        }
        context.window_worker_creator(page_id, target, *document_generation, *stream_epoch)?;
        Ok(ContextServiceResponse::InlineStyleAuthorization {
            allowed: decision.allowed,
        })
    }

    #[cfg(test)]
    fn authorize_inline_script(
        &self,
        page_id: PageId,
        creator: Option<&oryn_js::worker_wire::Creator>,
        source: &str,
        nonce: Option<&str>,
        parser_inserted: bool,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.authorize_inline_script_with_policy(
            page_id,
            creator,
            source,
            nonce,
            parser_inserted,
            None,
        )
    }
    #[cfg(test)]
    fn authorize_inline_script_with_policy(
        &self,
        page_id: PageId,
        creator: Option<&oryn_js::worker_wire::Creator>,
        source: &str,
        nonce: Option<&str>,
        parser_inserted: bool,
        script_policy: Option<&str>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.authorize_inline_script_kind(InlineScriptAuthorization {
            page_id,
            creator,
            source,
            nonce,
            parser_inserted,
            script_policy,
            speculation_rules: false,
        })
    }
    fn authorize_inline_script_kind(
        &self,
        params: InlineScriptAuthorization<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let InlineScriptAuthorization {
            page_id,
            creator,
            source,
            nonce,
            parser_inserted,
            script_policy,
            speculation_rules,
        } = params;

        self.validate_socket_owner(page_id)?;
        let context = self.context_owner()?;
        let Some(
            creator @ oryn_js::worker_wire::Creator::Window {
                target,
                document_generation,
                stream_epoch,
            },
        ) = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "inline document script requires current native Window creator".into(),
            ));
        };
        let current =
            context.window_worker_policy(page_id, target, *document_generation, *stream_epoch)?;
        let captured = script_policy
            .map(|token| {
                self.script_document_owner(page_id, creator)
                    .and_then(|owner| self.script_policies.get(token, &owner))
            })
            .transpose()?;
        let policy =
            crate::csp::CspPolicySet::from_agent_policy(captured.as_deref().unwrap_or(&current));
        let request = crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Script,
            url: "inline".into(),
            nonce: nonce.map(str::to_owned),
            inline_source: Some(source.to_owned()),
            parser_inserted,
            initiated_by_trusted_script: false,
        };
        let decision = if speculation_rules {
            policy.authorize_speculation_rules(&request)
        } else {
            policy.authorize(&request)
        };
        if !decision.violations.is_empty() {
            context.handle_csp_violations_for_creator(
                page_id,
                decision.violations.clone(),
                Some(&policy),
                Some(creator.clone()),
            )?;
        }
        context.window_worker_creator(page_id, target, *document_generation, *stream_epoch)?;
        if !decision.allowed {
            return Ok(ContextServiceResponse::ScriptLoadFailed {
                failure: crate::script_load_failure::ScriptLoadFailure::Fetch(
                    "inline script blocked by Content-Security-Policy".into(),
                ),
            });
        }
        Ok(ContextServiceResponse::ScriptAuthorized)
    }

    #[cfg(all(test, feature = "in-process-probe"))]
    fn load_page_script(
        &self,
        page_id: PageId,
        requested_url: &str,
        module: bool,
        nonce: Option<&str>,
        initiator_url: Option<&str>,
        referrer_policy: Option<&str>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.load_page_script_with_integrity(IntegrityScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity: None,
        })
    }

    #[cfg(all(test, feature = "in-process-probe"))]
    fn load_page_script_with_integrity(
        &self,
        params: IntegrityScriptInputs<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let IntegrityScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
        } = params;

        self.load_page_script_with_options(ScriptOptionsInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin: None,
        })
    }

    #[cfg(all(test, feature = "in-process-probe"))]
    fn load_page_script_with_options(
        &self,
        params: ScriptOptionsInputs<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let ScriptOptionsInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
        } = params;

        self.load_page_script_with_referrer(ScriptReferrerInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source: None,
        })
    }

    #[cfg(all(test, feature = "in-process-probe"))]
    fn load_page_script_with_referrer(
        &self,
        params: ScriptReferrerInputs<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let ScriptReferrerInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
        } = params;

        self.load_page_script_for_type(TypedScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind: None,
        })
    }

    #[cfg(all(test, feature = "in-process-probe"))]
    fn load_page_script_for_type(
        &self,
        params: TypedScriptInputs<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let TypedScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
        } = params;

        self.load_page_script_classified(PageScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
            cancellation: None,
            creator: None,
            script_policy: None,
            parser_inserted: false,
            fallback_encoding: None,
        })
        .map_err(crate::script_load_failure::ScriptLoadFailure::runtime)
    }

    fn script_document_owner(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
    ) -> Result<crate::agent_registry::AgentOwner, RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "window script route cannot impersonate agent".into(),
            ));
        };
        self.context_owner()?
            .window_worker_creator(page, target, *document_generation, *stream_epoch)
            .map(|(owner, _, _)| owner)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))
    }

    fn load_page_script_with_cancel(
        &self,
        params: PageScriptInputs<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let PageScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
            cancellation,
            creator,
            script_policy,
            parser_inserted,
            fallback_encoding,
        } = params;

        match self.load_page_script_classified(PageScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
            cancellation,
            creator,
            script_policy,
            parser_inserted,
            fallback_encoding,
        }) {
            Ok(response) => Ok(response),
            Err(crate::script_load_failure::ScriptLoadFailure::Infrastructure(message)) => {
                Err(RuntimeError::ContextService(message))
            }
            Err(failure) => Ok(ContextServiceResponse::ScriptLoadFailed { failure }),
        }
    }

    fn load_page_script_classified(
        &self,
        params: PageScriptInputs<'_>,
    ) -> Result<ContextServiceResponse, crate::script_load_failure::ScriptLoadFailure> {
        let PageScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
            cancellation,
            creator,
            script_policy,
            parser_inserted,
            fallback_encoding,
        } = params;

        self.load_page_script_classified_traced(TracedPageScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
            cancellation,
            creator,
            script_policy,
            parser_inserted,
            fallback_encoding,
            trace: None,
            hash_evidence: None,
        })
    }
    fn load_page_script_classified_traced(
        &self,
        params: TracedPageScriptInputs<'_>,
    ) -> Result<ContextServiceResponse, crate::script_load_failure::ScriptLoadFailure> {
        let TracedPageScriptInputs {
            page_id,
            requested_url,
            module,
            nonce,
            initiator_url,
            referrer_policy,
            integrity,
            cross_origin,
            referrer_source,
            kind,
            cancellation,
            creator,
            script_policy,
            parser_inserted,
            fallback_encoding,
            mut trace,
            mut hash_evidence,
        } = params;

        use crate::script_load_failure::ScriptLoadFailure;
        self.validate_socket_owner(page_id)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let cancellation = cancellation.unwrap_or(&self.shutting_down);
        if cancellation.load(Ordering::Acquire) {
            return Err(ScriptLoadFailure::Cancelled);
        }
        let network = self.network.clone().ok_or_else(|| {
            RuntimeError::ContextService("script resource broker is unavailable".into())
        })?;
        let page_url = self.context_owner()?.page_url(page_id)?;
        let native_authority = creator
            .map(|creator| {
                self.script_document_owner(page_id, creator)
                    .and_then(|owner| {
                        self.context_owner()?
                            .native_worker_authority(&owner)
                            .map_err(|error| RuntimeError::ContextService(error.to_string()))
                    })
            })
            .transpose()?;
        let source_url = native_authority
            .as_ref()
            .map(|(_, _, policy)| policy.url.as_str())
            .unwrap_or_else(|| initiator_url.unwrap_or(&page_url));
        let native_origin = native_authority
            .as_ref()
            .map(|(_, origin, _)| origin.clone());
        let embedder = match native_authority.as_ref() {
            Some((_, _, policy)) => {
                crate::agent_isolation::DocumentIsolationPolicy::from_headers(&policy.headers)
            }
            None => self.context_owner()?.page_isolation_policy(page_id)?,
        };
        let coep_reporting = match native_authority.as_ref() {
            Some((_, _, policy)) => {
                crate::coep_reporting::Policy::new(&policy.url, &policy.headers)
            }
            None => self.context_owner()?.document_reporting_policy(page_id)?,
        };
        let captured_policy = script_policy
            .map(|token| {
                let creator = creator.ok_or_else(|| {
                    RuntimeError::ProtocolViolation(
                        "captured script policy requires native creator".into(),
                    )
                })?;
                let owner = self.script_document_owner(page_id, creator)?;
                self.script_policies.get(token, &owner)
            })
            .transpose()?;
        let native_csp = captured_policy
            .as_deref()
            .map(crate::csp::CspPolicySet::from_agent_policy)
            .or_else(|| {
                native_authority
                    .as_ref()
                    .map(|(_, _, policy)| crate::csp::CspPolicySet::from_agent_policy(policy))
            });

        let source =
            Url::parse(source_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let mut referrer_source = Some(
            referrer_source
                .map(Url::parse)
                .transpose()
                .map_err(|error| RuntimeError::Network(error.to_string()))?
                .or(captured_policy
                    .as_deref()
                    .map(|policy| Url::parse(&policy.url))
                    .transpose()
                    .map_err(|error| RuntimeError::Network(error.to_string()))?)
                .unwrap_or_else(|| source.clone()),
        );
        // Captured Document URL is referrer provenance only. `source` remains
        // the existing resolution base for requested_url and request security.
        let source_origin = native_origin
            .as_ref()
            .map(|origin| origin.serialized.clone())
            .unwrap_or_else(|| source.origin().ascii_serialization());
        let source_opaque = native_origin.as_ref().is_some_and(|origin| origin.opaque);
        let mut request_origin = source_origin.clone();
        let mut opaque_redirect_origin = false;
        let cors = module || cross_origin.is_some();
        let module_type = kind.unwrap_or_default();
        let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
            if cors { "cors" } else { "no-cors" },
            module_type.fetch_destination(),
            source_opaque,
        );
        let include_credentials =
            cross_origin.is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"));
        let credentials_mode = if include_credentials || !cors {
            crate::cookies::CookieCredentials::Include
        } else {
            crate::cookies::CookieCredentials::SameOrigin
        };
        let mut integrity_eligible = true;
        let mut policy = referrer_policy
            .filter(|value| !value.trim().is_empty())
            .map_or(
                native_authority
                    .as_ref()
                    .map(|(_, _, policy)| {
                        crate::referrer::ReferrerPolicy::default().after_response(&policy.headers)
                    })
                    .unwrap_or(self.context_owner()?.page_referrer_policy(page_id)?),
                crate::referrer::ReferrerPolicy::parse_list,
            );
        let script_policy = policy;
        let timing_started = Instant::now();
        let mut timing_hops = Vec::new();
        let mut url = source
            .join(requested_url)
            .or_else(|_| Url::parse(requested_url))
            .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
        let hash_request_url = url.to_string();
        for redirect in 0..=10 {
            if let Some(creator) = creator {
                self.script_document_owner(page_id, creator)?;
            }
            if let Some(upgraded) = match &native_csp {
                Some(policy) => policy.upgraded_url(&url),
                None => self.context_owner()?.upgrade_csp_url(page_id, &url)?,
            } {
                url = upgraded;
            }
            if let Some(trace) = trace.as_mut() {
                trace.push(url.to_string());
            }
            enforce_mixed_content(&source_origin, &url)
                .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
            if !cors
                && !matches!(url.scheme(), "data" | "blob")
                && (source_opaque || url.origin().ascii_serialization() != source_origin)
            {
                integrity_eligible = false;
            }
            let csp_request = crate::csp::CspRequest {
                destination: module_type.csp_destination(),
                url: url.to_string(),
                nonce: nonce.map(str::to_owned),
                inline_source: None,
                parser_inserted,
                initiated_by_trusted_script: !parser_inserted,
            };
            let decision = match &native_csp {
                Some(policy) => {
                    policy.authorize_redirect_from(&csp_request, redirect != 0, &hash_request_url)
                }
                None => self.context_owner()?.authorize_csp_redirect_from(
                    page_id,
                    &csp_request,
                    redirect != 0,
                    &hash_request_url,
                )?,
            };
            if !decision.violations.is_empty() {
                self.context_owner()?.handle_csp_violations_for_creator(
                    page_id,
                    decision.violations.clone(),
                    native_csp.as_ref(),
                    creator.cloned(),
                )?;
            }
            if !decision.allowed {
                return Err(ScriptLoadFailure::Fetch(
                    "dynamic script blocked by Content-Security-Policy script-src".into(),
                ));
            }
            if matches!(url.scheme(), "data" | "blob") {
                let (media_type, body) = if url.scheme() == "blob" {
                    let creator = creator.ok_or_else(|| {
                        ScriptLoadFailure::Fetch("Blob script requires native creator".into())
                    })?;
                    let owner = self.script_document_owner(page_id, creator)?;
                    let blob = self
                        .context_owner()?
                        .resolve_native_blob(&owner, url.as_str())
                        .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
                    (Some(blob.media_type), blob.bytes)
                } else {
                    crate::network::decode_data_url(&url)
                        .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?
                };
                if body.len() > MAX_PAGE_SCRIPT_BYTES {
                    return Err(ScriptLoadFailure::Capacity(
                        "decoded dynamic script data URL".into(),
                    ));
                }
                crate::integrity::verify(integrity.unwrap_or(""), &body, integrity_eligible)
                    .map_err(|error| {
                        ScriptLoadFailure::Integrity(format!(
                            "script integrity verification failed: {error:?}"
                        ))
                    })?;
                let source = decode_page_module(
                    &body,
                    media_type.as_deref(),
                    module,
                    kind,
                    fallback_encoding,
                )
                .map_err(ScriptLoadFailure::decoded)?;
                if let Some(creator) = creator {
                    self.script_document_owner(page_id, creator)?;
                }
                if cancellation.load(Ordering::Acquire) {
                    return Err(ScriptLoadFailure::Cancelled);
                }
                if let Some(slot) = hash_evidence.as_deref_mut() {
                    *slot = Some(crate::csp::ScriptHashEvidence::from_bytes(
                        &body,
                        cors || integrity_eligible,
                    ));
                }
                if module_type == crate::ModuleType::JavaScript
                    && !matches!(creator, Some(oryn_js::worker_wire::Creator::Agent(_)))
                {
                    self.context_owner()?.report_script_hash(
                        page_id,
                        &page_url,
                        native_csp.as_ref(),
                        &hash_request_url,
                        &body,
                        cors || integrity_eligible,
                    );
                }
                let timing = Some(crate::network::local_resource_timing_metadata(
                    &source_origin,
                    &body,
                    media_type.as_deref(),
                    timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                ));
                return Ok(ContextServiceResponse::ScriptSource {
                    timing,
                    url: url.to_string(),
                    source,
                    module,
                    muted_errors: !cors && !integrity_eligible,
                    referrer_policy: script_policy.as_str().to_owned(),
                });
            }
            let mut headers = vec![(
                "accept".into(),
                "text/javascript,application/javascript,*/*;q=0.1".into(),
            )];
            let referrer = referrer_source
                .as_ref()
                .and_then(|source| policy.header(source, &url));
            // A later redirect policy may further reduce this value, but must
            // not recover path/query data already removed by an earlier hop.
            referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
            if let Some(referrer) = referrer {
                headers.push(("referer".into(), referrer));
            }
            let cross_origin_module = cors
                && (source_opaque
                    || opaque_redirect_origin
                    || url.origin().ascii_serialization() != source_origin);
            if let Some(origin) = crate::referrer::request_origin_header(
                &request_origin,
                &source,
                &url,
                "GET",
                cross_origin_module,
                policy,
            ) {
                headers.push(("origin".into(), origin));
            }
            fetch_metadata.observe(Some(&source_origin), &url);
            let omit = embedder.omit_credentials(
                cors,
                source_opaque || url.origin().ascii_serialization() != source_origin,
            );
            let response = collect_operation_bounded_with_cancel(
                &network,
                NetworkRequest {
                    request_id: self.next_http_identity()?,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials: (include_credentials || !cross_origin_module) && !omit,
                    security: crate::cookies::CookieRequestContext {
                        fetch_metadata: fetch_metadata.clone(),
                        dns_timing: None,
                        credentials: if omit {
                            crate::cookies::CookieCredentials::Omit
                        } else {
                            credentials_mode
                        },
                        kind: crate::cookies::CookieRequestKind::Script,
                        initiator_url: Some(source_url.into()),
                        top_level_url: Some(page_url.clone()),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                deadline
                    .checked_duration_since(std::time::Instant::now())
                    .ok_or_else(|| {
                        ScriptLoadFailure::Fetch("script fetch deadline exceeded".into())
                    })?,
                MAX_PAGE_SCRIPT_BYTES,
                cancellation,
            )
            .map_err(ScriptLoadFailure::network)?;
            if let Some(creator) = creator {
                self.script_document_owner(page_id, creator)?;
            }
            if cross_origin_module {
                let mut origins = response
                    .headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"));
                let allowed = origins.next().is_some_and(|(_, value)| {
                    let value = value.trim_matches([' ', '\t', '\r', '\n']);
                    (!include_credentials && value == "*") || value == request_origin
                }) && origins.next().is_none();
                let credentials_allowed = !include_credentials || {
                    let mut values = response.headers.iter().filter(|(name, _)| {
                        name.eq_ignore_ascii_case("access-control-allow-credentials")
                    });
                    values
                        .next()
                        .is_some_and(|(_, value)| value.trim_matches([' ', '\t']) == "true")
                        && values.next().is_none()
                };
                if !allowed || !credentials_allowed {
                    return Err(ScriptLoadFailure::Fetch(
                        "cross-origin script failed CORS".into(),
                    ));
                }
            }
            timing_hops.push(crate::network::timing_hop(&url, &response.headers));
            let _ = self.context_owner()?.deliver_coep_reports(
                &page_url,
                coep_reporting.response(crate::coep_reporting::ResponsePolicyInputs {
                    origin: &source_origin,
                    opaque: source_opaque,
                    target: &url,
                    cors,
                    credentials: (include_credentials || !cross_origin_module) && !omit,
                    headers: &response.headers,
                    destination: "script",
                }),
            );
            embedder
                .authorize_response(
                    &source_origin,
                    source_opaque,
                    &url,
                    cors,
                    (include_credentials || !cross_origin_module) && !omit,
                    &response.headers,
                )
                .map_err(ScriptLoadFailure::Fetch)?;
            if oryn_net::fetch::is_redirect_status(response.status) {
                if redirect == 10 {
                    return Err(ScriptLoadFailure::Fetch(
                        "dynamic script redirect limit exceeded".into(),
                    ));
                }
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        ScriptLoadFailure::Fetch("script redirect omitted Location".into())
                    })?;
                policy = policy.after_response(&response.headers);
                let next = url
                    .join(location)
                    .map_err(|error| ScriptLoadFailure::Fetch(error.to_string()))?;
                if cors && url.origin() != next.origin() && url.origin() != source.origin() {
                    request_origin = "null".into();
                    opaque_redirect_origin = true;
                }
                url = next;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(ScriptLoadFailure::Fetch(format!(
                    "script response returned HTTP {}",
                    response.status
                )));
            }
            if response.body.len() > MAX_PAGE_SCRIPT_BYTES {
                return Err(ScriptLoadFailure::Capacity(
                    "dynamic script resource".into(),
                ));
            }
            let extracted_type = oryn_common::mime_type::MimeType::from_headers(&response.headers)
                .map(|mime| mime.serialize());
            let content_type = extracted_type.as_deref();
            if !kind.map_or_else(
                || oryn_net::script_response_mime_allowed(&response.headers, module),
                |kind| kind.mime_allowed(&response.headers),
            ) {
                return Err(ScriptLoadFailure::Fetch(
                    "script response blocked by MIME type or nosniff".into(),
                ));
            }
            crate::integrity::verify(
                integrity.unwrap_or(""),
                &response.body,
                cors || integrity_eligible,
            )
            .map_err(|error| {
                ScriptLoadFailure::Integrity(format!(
                    "script integrity verification failed: {error:?}"
                ))
            })?;
            let source = decode_page_module(
                &response.body,
                content_type,
                module,
                kind,
                fallback_encoding,
            )
            .map_err(ScriptLoadFailure::decoded)?;
            if cancellation.load(Ordering::Acquire) {
                return Err(ScriptLoadFailure::Cancelled);
            }
            if let Some(creator) = creator {
                self.script_document_owner(page_id, creator)?;
                self.record_support_import(page_id, creator, &url, &response.body)?;
            }
            if let Some(slot) = hash_evidence.as_deref_mut() {
                *slot = Some(crate::csp::ScriptHashEvidence::from_bytes(
                    &response.body,
                    cors || integrity_eligible,
                ));
            }
            if module_type == crate::ModuleType::JavaScript
                && !matches!(creator, Some(oryn_js::worker_wire::Creator::Agent(_)))
            {
                self.context_owner()?.report_script_hash(
                    page_id,
                    &page_url,
                    native_csp.as_ref(),
                    &hash_request_url,
                    &response.body,
                    cors || integrity_eligible,
                );
            }
            let timing = Some(crate::network::resource_timing_metadata(
                &source_origin,
                source_opaque,
                timing_hops,
                &response,
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ));
            return Ok(ContextServiceResponse::ScriptSource {
                timing,
                url: url.to_string(),
                source,
                module,
                muted_errors: !cors && !integrity_eligible,
                referrer_policy: script_policy
                    .after_response(&response.headers)
                    .as_str()
                    .to_owned(),
            });
        }
        Err(ScriptLoadFailure::Fetch(
            "dynamic script request did not produce a response".into(),
        ))
    }

    fn load_page_resource(
        &self,
        page_id: PageId,
        requested_url: &str,
        destination: &str,
        options: PageResourceOptions<'_>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.load_page_resource_with_cancel(
            page_id,
            requested_url,
            destination,
            options,
            &self.shutting_down,
        )
    }
    fn load_page_resource_with_cancel(
        &self,
        page_id: PageId,
        requested_url: &str,
        destination: &str,
        options: PageResourceOptions<'_>,
        cancellation: &AtomicBool,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.load_page_resource_traced(
            page_id,
            requested_url,
            destination,
            options,
            cancellation,
            None,
        )
    }
    fn load_page_resource_traced(
        &self,
        page_id: PageId,
        requested_url: &str,
        destination: &str,
        options: PageResourceOptions<'_>,
        cancellation: &AtomicBool,
        mut trace: Option<&mut Vec<String>>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let document_resource = matches!(destination, "frame" | "object" | "embed");
        let embedded_representation = matches!(destination, "object" | "embed");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let PageResourceOptions {
            mut response_lease,
            download,
            script_authority,
            initiator,
            form_authority,
            initiator_url,
            referrer_policy,
            parent_encoding,
            mut post,
        } = options;
        self.validate_socket_owner(page_id)?;
        let owner = self.script_document_owner(page_id, &initiator.creator)?;
        if destination == "stylesheet" && response_lease.is_none() {
            let token = self.script_loads.reserve_for(owner.clone())?;
            match self.script_loads.claim_for(token, &owner) {
                Ok(lease) => response_lease = Some(lease),
                Err(error) => {
                    let _ = self.script_loads.cancel(token);
                    return Err(error);
                }
            }
        }
        let context = self.context_owner()?;
        let (_, native_origin, native_policy) = context
            .native_worker_authority(&owner)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let resource_policy = match &initiator.prepared_policy {
            Some(token) => {
                if document_resource {
                    return Err(RuntimeError::ProtocolViolation(
                        "prepared child policy cannot authorize embedding navigation".into(),
                    ));
                }
                context.prepared_frame_resource_policy(page_id, token)?
            }
            None => native_policy,
        };
        let resource_policy = if destination == "stylesheet" && initiator.prepared_policy.is_none()
        {
            match initiator
                .stylesheet
                .as_ref()
                .and_then(|sheet| sheet.policy.as_deref())
            {
                Some(token) => (*self.script_policies.get(token, &owner)?).clone(),
                None => resource_policy,
            }
        } else {
            resource_policy
        };
        let native_csp = crate::csp::CspPolicySet::from_agent_policy(&resource_policy);
        if !embedded_representation && initiator.embedded_type.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "embedded type on unrelated resource".into(),
            ));
        }
        if destination != "stylesheet" && initiator.stylesheet.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "stylesheet options on unrelated resource".into(),
            ));
        }
        let stylesheet = initiator.stylesheet.as_ref();
        let cors = destination == "font"
            || stylesheet.is_some_and(|options| options.cross_origin.is_some());
        let use_credentials = stylesheet
            .and_then(|options| options.cross_origin.as_deref())
            .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"));
        let embedder =
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(&resource_policy.headers);
        let coep_reporting =
            crate::coep_reporting::Policy::new(&resource_policy.url, &resource_policy.headers);
        let mut integrity_eligible = true;

        let script_source = if let Some(admission) = &script_authority {
            if !document_resource {
                return Err(RuntimeError::ProtocolViolation(
                    "script navigation authority cannot authorize a non-frame resource".into(),
                ));
            }
            Some(context.script_navigation_source(
                &admission.authority,
                page_id,
                &admission.target,
                requested_url,
            )?)
        } else {
            None
        };
        let service_local_policy = script_authority
            .as_ref()
            .map(|admission| {
                context.script_navigation_local_policy(admission, page_id, requested_url)
            })
            .transpose()?
            .flatten();
        let form_navigation = form_authority.is_some() || post.is_some();
        if let Some(authority) = &form_authority {
            if !document_resource {
                return Err(RuntimeError::ProtocolViolation(
                    "form authority cannot authorize a non-frame resource".into(),
                ));
            }
            let resolved = self
                .context_owner()?
                .resolve_page_url(page_id, requested_url)?;
            let (_, decision) = self.context_owner()?.form_policy_for_navigation(
                authority,
                page_id,
                resolved.as_str(),
            )?;
            if !decision.allowed {
                return Err(RuntimeError::ContextService(
                    "form navigation violates original source policy".into(),
                ));
            }
        }

        if let Some(resource) = &post {
            if !document_resource
                || self
                    .context_owner()?
                    .resolve_page_url(page_id, requested_url)
                    .map_or(true, |url| !resource.valid_for(&url))
            {
                return Err(RuntimeError::ContextService(
                    "invalid frame POST resource".into(),
                ));
            }
            match self.handle(ContextServiceRequest::AuthorizeFormSubmission {
                report_source: None,
                authority: form_authority.clone(),
                page_id,
                url: requested_url.into(),
                method: "POST".into(),
            })? {
                ContextServiceResponse::FormSubmissionAuthorized => {}
                ContextServiceResponse::PageDenied { reason } => {
                    return Err(RuntimeError::ContextService(reason));
                }
                _ => {
                    return Err(RuntimeError::ContextService(
                        "invalid form authorization response".into(),
                    ));
                }
            }
        }
        let (csp_destination, cookie_kind, accept, text_kind) = match destination {
            "stylesheet" => (
                crate::csp::CspDestination::Style,
                crate::cookies::CookieRequestKind::Style,
                "text/css,*/*;q=0.1",
                oryn_net::TextResourceKind::Css,
            ),
            "frame" => (
                crate::csp::CspDestination::Frame,
                crate::cookies::CookieRequestKind::Frame,
                "text/html,application/xhtml+xml;q=0.9,*/*;q=0.1",
                oryn_net::TextResourceKind::Html,
            ),
            "object" | "embed" => (
                crate::csp::CspDestination::Object,
                crate::cookies::CookieRequestKind::Object,
                "*/*",
                oryn_net::TextResourceKind::Html,
            ),
            "image" => (
                crate::csp::CspDestination::Image,
                crate::cookies::CookieRequestKind::Image,
                "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8",
                oryn_net::TextResourceKind::Fetch,
            ),
            "font" => (
                crate::csp::CspDestination::Font,
                crate::cookies::CookieRequestKind::Font,
                "font/woff2,font/woff,application/font-woff,*/*;q=0.1",
                oryn_net::TextResourceKind::Fetch,
            ),
            _ => {
                return Err(RuntimeError::ContextService(format!(
                    "unsupported parser resource destination {destination:?}"
                )));
            }
        };
        let page_url = self.context_owner()?.page_url(page_id)?;
        let form_source = form_authority
            .as_ref()
            .map(|authority| self.context_owner()?.form_fetch_source(authority, page_id))
            .transpose()?;
        let navigation_source = script_source.or(form_source);
        let source_url = navigation_source
            .as_ref()
            .map(|source| source.url.as_str())
            .unwrap_or_else(|| initiator_url.unwrap_or(&resource_policy.url));
        let source =
            Url::parse(source_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let replay = initiator.request_state.as_ref();
        if let Some(state) = replay {
            state
                .retained_bytes()
                .map_err(|error| RuntimeError::ContextService(error.into()))?;
            if !document_resource
                || !(initiator.navigation.reload || initiator.navigation.history)
                || script_authority.is_some()
            {
                return Err(RuntimeError::ContextService(
                    "request provenance requires frame history navigation".into(),
                ));
            }
        }
        let request_initiator_url = replay.map_or_else(
            || Some(source.to_string()),
            |state| state.initiator_url.clone(),
        );
        let request_opaque = replay.map_or_else(
            || {
                navigation_source
                    .as_ref()
                    .map_or(native_origin.opaque, |source| source.opaque_origin)
            },
            |state| state.opaque_initiator,
        );
        let mut referrer_source = Some(source.clone());
        let referrer_policy = if script_authority.is_some() {
            None
        } else {
            referrer_policy
        };
        let mut policy = referrer_policy
            .filter(|value| !value.trim().is_empty())
            .map_or(
                navigation_source
                    .as_ref()
                    .map(|source| {
                        crate::referrer::ReferrerPolicy::parse_list(&source.referrer_policy)
                    })
                    .unwrap_or_else(|| {
                        crate::referrer::ReferrerPolicy::default()
                            .after_response(&resource_policy.headers)
                    }),
                crate::referrer::ReferrerPolicy::parse_list,
            );
        if let Some(state) = replay {
            referrer_source = state
                .referrer
                .as_deref()
                .map(Url::parse)
                .transpose()
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
            policy = crate::referrer::ReferrerPolicy::parse_list(&state.referrer_policy);
        }
        let timing_started = Instant::now();
        let mut timing_hops = Vec::new();
        let mut navigation_phases = crate::network::NavigationPhaseClock::new(timing_started);
        let mut url = source
            .join(requested_url)
            .or_else(|_| Url::parse(requested_url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if destination == "frame" {
            // Protocol handlers affect navigations only. Resolve the registered
            // target before scheme, CSP, form and network admission; resources
            // retain their original URL and cannot invoke external handlers.
            url = context.resolve_navigation_url(page_id, url.as_str())?;
            if let Some(authority) = &form_authority {
                let (_, decision) = context.form_policy_for_navigation_report(
                    authority,
                    page_id,
                    url.as_str(),
                    true,
                    None,
                )?;
                if !decision.allowed {
                    return Err(RuntimeError::ContextService(
                        "resolved form navigation violates original source policy".into(),
                    ));
                }
            }
        }
        let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
            if document_resource {
                "navigate"
            } else if cors {
                "cors"
            } else {
                "no-cors"
            },
            match destination {
                "frame" => "iframe",
                "stylesheet" => "style",
                other => other,
            },
            native_origin.opaque,
        );
        if let Some(source) = &navigation_source {
            fetch_metadata.user_activation = source.user_activation;
        }
        fetch_metadata.opaque_initiator = request_opaque;
        let request_origin = request_initiator_url
            .as_deref()
            .and_then(|url| Url::parse(url).ok())
            .map(|url| url.origin().ascii_serialization());
        let original_form_url = url.to_string();
        let initial_site = crate::cookies::SchemefulSite::from_url(&url);
        let mut redirect_chain_cross_site = false;
        let mut font_request_origin = native_origin.serialized.clone();
        let navigation_origin_source = request_initiator_url
            .as_deref()
            .and_then(|value| Url::parse(value).ok())
            .unwrap_or_else(|| source.clone());
        let mut navigation_request_origin = if request_opaque {
            "null".into()
        } else {
            request_origin.clone().unwrap_or_else(|| "null".into())
        };
        for redirect in 0..=10 {
            navigation_phases.begin_hop();
            if cancellation.load(Ordering::Acquire) || self.shutting_down.load(Ordering::Acquire) {
                if destination == "stylesheet" {
                    return Err(RuntimeError::StylesheetFailure(
                        crate::script_load_failure::ScriptLoadFailure::Cancelled,
                    ));
                }
                return Err(RuntimeError::Network("resource owner cancelled".into()));
            }
            if std::time::Instant::now() >= deadline {
                return Err(RuntimeError::Network("resource deadline exceeded".into()));
            }
            self.script_document_owner(page_id, &initiator.creator)?;
            if let Some(admission) = &script_authority {
                self.script_document_owner(page_id, &admission.target)?;
            }
            if let Some(upgraded) = native_csp.upgraded_url(&url) {
                url = upgraded;
            }
            if let Some(trace) = trace.as_mut() {
                trace.push(url.to_string());
            }
            fetch_metadata.observe(request_origin.as_deref(), &url);
            enforce_mixed_content(&native_origin.serialized, &url).map_err(|error| {
                if destination == "stylesheet" {
                    RuntimeError::StylesheetFailure(
                        crate::script_load_failure::ScriptLoadFailure::Fetch(error.to_string()),
                    )
                } else {
                    error
                }
            })?;
            if post
                .as_ref()
                .is_some_and(|resource| !resource.valid_for(&url))
            {
                return Err(RuntimeError::ContextService(
                    "invalid redirected frame POST resource".into(),
                ));
            }
            if form_navigation && redirect > 0 {
                match self.handle(ContextServiceRequest::AuthorizeFormSubmission {
                    report_source: Some(original_form_url.clone()),
                    authority: form_authority.clone(),
                    page_id,
                    url: url.to_string(),
                    method: if post.is_some() { "POST" } else { "GET" }.into(),
                })? {
                    ContextServiceResponse::FormSubmissionAuthorized => {}
                    ContextServiceResponse::PageDenied { reason } => {
                        return Err(RuntimeError::ContextService(reason));
                    }
                    _ => {
                        return Err(RuntimeError::ContextService(
                            "invalid redirected form authorization response".into(),
                        ));
                    }
                }
            }
            let decision = native_csp.authorize_redirect_from(
                &crate::csp::CspRequest {
                    destination: csp_destination,
                    url: url.to_string(),
                    nonce: stylesheet.and_then(|options| options.nonce.clone()),
                    inline_source: None,
                    parser_inserted: true,
                    initiated_by_trusted_script: false,
                },
                redirect != 0,
                &original_form_url,
            );
            if !decision.violations.is_empty() {
                self.context_owner()?.handle_csp_violations_for_creator(
                    page_id,
                    decision.violations.clone(),
                    Some(&native_csp),
                    Some(initiator.creator.clone()),
                )?;
            }
            if !decision.allowed {
                if destination == "stylesheet" {
                    return Err(RuntimeError::StylesheetFailure(
                        crate::script_load_failure::ScriptLoadFailure::Fetch(
                            "stylesheet blocked by Content-Security-Policy".into(),
                        ),
                    ));
                }
                return Err(RuntimeError::ContextService(format!(
                    "{destination} blocked by Content-Security-Policy"
                )));
            }
            if !cors
                && !matches!(url.scheme(), "data" | "blob")
                && (native_origin.opaque
                    || url.origin().ascii_serialization() != native_origin.serialized)
            {
                integrity_eligible = false;
            }
            if matches!(url.scheme(), "data" | "blob") {
                let embedding_origin = native_origin.clone();
                let (media_type, body, local_policy, native_origin) = if url.scheme() == "blob" {
                    let mut lookup = url.clone();
                    lookup.set_fragment(None);
                    let blob = context
                        .resolve_native_blob(&owner, lookup.as_str())
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    (Some(blob.media_type), blob.bytes, blob.policy, blob.origin)
                } else {
                    let (media, body) = crate::network::decode_data_url(&url)
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    let policy = match service_local_policy.as_ref() {
                        Some(policy) => policy.clone(),
                        None => {
                            context
                                .native_worker_authority(&owner)
                                .map_err(|error| RuntimeError::ContextService(error.to_string()))?
                                .2
                        }
                    };
                    let origin = oryn_js::origin::Origin::opaque();
                    (
                        media,
                        body,
                        policy,
                        oryn_js::worker_wire::AgentOrigin {
                            serialized: origin.serialize(),
                            opaque: true,
                            authority: origin.security_token_key(),
                        },
                    )
                };
                if destination == "stylesheet" {
                    let headers = media_type
                        .as_ref()
                        .map(|mime| vec![("content-type".into(), mime.clone())])
                        .unwrap_or_default();
                    if !oryn_net::stylesheet_response_allowed(
                        &headers,
                        stylesheet.is_some_and(|options| options.quirks),
                        true,
                    ) {
                        return Err(RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch(
                            "stylesheet response blocked by MIME type".into(),
                        )));
                    }
                }
                if let Some(options) = stylesheet {
                    crate::integrity::verify(
                        options.integrity.as_deref().unwrap_or(""),
                        &body,
                        true,
                    )
                    .map_err(|error| {
                        RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Integrity(format!("stylesheet integrity failed: {error:?}")))
                    })?;
                }
                if body.len() > 16 * 1024 * 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: format!("decoded {destination} data URL"),
                    });
                }
                let computed_type = if document_resource {
                    let sniff_type = crate::embedded_resource_type::sniff_input(
                        destination,
                        media_type.as_deref(),
                    );
                    let computed =
                        oryn_net::mime_sniff::document_type(sniff_type.as_deref(), false, &body);
                    Some(if embedded_representation {
                        match crate::embedded_resource_type::select(
                            destination,
                            media_type.as_deref(),
                            initiator.embedded_type.as_deref(),
                            &computed,
                            false,
                        ) {
                            Ok(value) => value,
                            Err(_) => {
                                return Ok(ContextServiceResponse::EmbeddedResourceUnavailable);
                            }
                        }
                    } else {
                        computed
                    })
                } else {
                    media_type.clone()
                };
                let inherited_encoding = if document_resource {
                    inherited_frame_encoding(
                        &embedding_origin,
                        &native_origin.serialized,
                        native_origin.opaque,
                        parent_encoding,
                    )
                } else {
                    None
                };
                if !embedded_representation
                    && document_resource
                    && (download.is_some()
                        || computed_type
                            .as_deref()
                            .is_some_and(crate::download::computed_type_is_download))
                {
                    let filename = crate::download::response_download_filename(
                        &[],
                        &url,
                        download.or(Some("")),
                    )
                    .unwrap_or_else(|| "download".into());
                    self.handle(ContextServiceRequest::QuarantinePageDownload {
                        page_id,
                        creator: initiator.creator.clone(),
                        url: url.to_string(),
                        suggested_filename: filename,
                        media_type: media_type.clone().unwrap_or_default(),
                        body_base64: base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &body,
                        ),
                    })?;
                    return Ok(ContextServiceResponse::PageResourceNoCommit {
                        reason: "download".into(),
                    });
                }
                let timing = Some(crate::network::local_resource_timing_metadata(
                    &source.origin().ascii_serialization(),
                    &body,
                    media_type.as_deref(),
                    timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                ));
                if matches!(destination, "font" | "image") {
                    return Ok(ContextServiceResponse::PageBinaryResource {
                        timing,
                        url: url.to_string(),
                        body_base64: base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &body,
                        ),
                        content_type: media_type,
                        destination: destination.into(),
                    });
                }
                let media_type = computed_type;
                let (source, encoding) = oryn_net::decode_resource_text_with_fallback(
                    &body,
                    media_type.as_deref(),
                    page_resource_text_kind(text_kind, media_type.as_deref()),
                    if destination == "stylesheet" {
                        parent_encoding
                    } else {
                        inherited_encoding.as_deref()
                    },
                );
                let stylesheet_response = if destination == "stylesheet" {
                    Some(
                        response_lease
                            .take()
                            .ok_or_else(|| {
                                RuntimeError::ProtocolViolation(
                                    "stylesheet response admission absent".into(),
                                )
                            })?
                            .publish_stylesheet(
                                url.as_str(),
                                &source,
                                integrity_eligible
                                    && (url.scheme() == "data"
                                        || native_origin.authority == embedding_origin.authority),
                            )?,
                    )
                } else {
                    None
                };
                return Ok(ContextServiceResponse::PageResource {
                    stylesheet_response,
                    inherited_encoding,
                    response_headers: Vec::new(),
                    raw_body_base64: (document_resource).then(|| {
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body)
                    }),
                    request_state: (document_resource).then(|| {
                        oryn_common::navigation_request::NavigationRequestState {
                            initiator_url: request_initiator_url.clone(),
                            opaque_initiator: request_opaque,
                            referrer: referrer_source
                                .as_ref()
                                .and_then(|source| policy.header(source, &url)),
                            referrer_policy: policy.as_str().into(),
                        }
                    }),
                    timing: timing.map(Box::new),
                    content_language: None,
                    stream_id: None,
                    effective_post: post.clone(),
                    native_origin: if document_resource {
                        Some(native_origin.clone())
                    } else {
                        None
                    },
                    worker_policy_token: if document_resource
                        && (!embedded_representation
                            || embedded_document_mime(media_type.as_deref()))
                    {
                        Some(context.prepare_local_frame_worker_policy_for_creator(
                            page_id,
                            url.as_str(),
                            &local_policy,
                            &source,
                            native_origin,
                            Some(&initiator.creator),
                        )?)
                    } else {
                        None
                    },
                    url: url.to_string(),
                    source,
                    encoding: encoding.into(),
                    destination: destination.into(),
                    content_type: media_type,
                    modals_allowed: crate::csp::CspPolicySet::from_agent_policy(&local_policy)
                        .sandbox()
                        .is_none_or(|sandbox| sandbox.allow_modals),
                    document_referrer: referrer_source
                        .as_ref()
                        .and_then(|source| policy.header(source, &url))
                        .unwrap_or_default(),
                    referrer_policy: crate::referrer::ReferrerPolicy::default()
                        .after_response(&local_policy.headers)
                        .as_str()
                        .into(),
                    refresh_url: None,
                });
            }
            let network = self.network.clone().ok_or_else(|| {
                RuntimeError::ContextService("page resource broker is unavailable".into())
            })?;
            let network = if document_resource {
                SharedNetworkBroker::new(crate::service_network::ServiceNavigationBroker {
                    context: context.downgrade(),
                    page: page_id,
                    inner: network,
                })
            } else {
                network
            };
            let mut request_headers = vec![("accept".into(), accept.into())];
            if let Some(resource) = &post {
                request_headers.push(("content-type".into(), resource.content_type.clone()));
            }
            let request_referrer = referrer_source
                .as_ref()
                .and_then(|source| policy.header(source, &url));
            referrer_source = request_referrer
                .as_deref()
                .and_then(|value| Url::parse(value).ok());
            if let Some(referrer) = &request_referrer {
                request_headers.push(("referer".into(), referrer.clone()));
            }
            if cors && (!url.username().is_empty() || url.password().is_some()) {
                return Err(RuntimeError::Network(
                    "CORS resource URL contains credentials".into(),
                ));
            }
            let resource_cross = native_origin.opaque
                || url.origin().ascii_serialization() != native_origin.serialized;
            let font_cross = cors
                && (font_request_origin == "null"
                    || url.origin().ascii_serialization() != font_request_origin);
            let header_origin = if document_resource {
                &navigation_request_origin
            } else {
                &font_request_origin
            };
            let header_source = if document_resource {
                &navigation_origin_source
            } else {
                &source
            };
            if let Some(value) = crate::referrer::request_origin_header(
                header_origin,
                header_source,
                &url,
                if post.is_some() { "POST" } else { "GET" },
                font_cross,
                policy,
            ) {
                request_headers.push(("origin".into(), value));
            }
            let request =
                NetworkRequest {
                    request_id: self.next_http_identity()?,
                    method: if post.is_some() { "POST" } else { "GET" }.into(),
                    url: url.to_string(),
                    headers: request_headers,
                    body: post
                        .as_ref()
                        .map_or_else(Vec::new, |resource| resource.bytes.clone()),
                    credentials: (!cors || use_credentials || !font_cross)
                        && !embedder.omit_credentials(cors || document_resource, resource_cross),
                    security: crate::cookies::CookieRequestContext {
                        fetch_metadata: fetch_metadata.clone(),
                        dns_timing: None,
                        navigation: if document_resource {
                            initiator.navigation
                        } else {
                            Default::default()
                        },
                        kind: cookie_kind,
                        credentials: if embedder
                            .omit_credentials(cors || document_resource, resource_cross)
                        {
                            crate::cookies::CookieCredentials::Omit
                        } else if cors && !use_credentials {
                            crate::cookies::CookieCredentials::SameOrigin
                        } else {
                            crate::cookies::CookieCredentials::Include
                        },
                        redirect_chain_cross_site,
                        initiator_url: if request_opaque {
                            Some("about:blank".into())
                        } else {
                            request_initiator_url.clone()
                        },
                        top_level_url: Some(navigation_source.as_ref().map_or_else(
                            || page_url.clone(),
                            |source| source.top_level_url.clone(),
                        )),
                        ancestor_urls: match &navigation_source {
                            Some(source) => source.ancestor_urls.clone(),
                            None => {
                                let mut ancestors = context.window_ancestor_urls(&owner)?;
                                ancestors.push(if native_origin.opaque {
                                    "about:blank".into()
                                } else {
                                    source_url.to_owned()
                                });
                                ancestors
                            }
                        },
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                };
            let frame_ancestors = request.security.ancestor_urls.clone();
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| {
                    RuntimeError::Network("resource deadline exceeded before transport".into())
                })?;
            let mut frame_stream = None;
            let mut response = if document_resource {
                let cleanup = http_cleanup_custody::Ticket::reserve(
                    &self.http_cleanup,
                    request.request_id,
                    network.clone(),
                )?;
                let request_id = request.request_id;
                cleanup.attach(request_id);
                let mut operation = network
                    .start_operation(request)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                if operation.request_id != request_id {
                    return Err(RuntimeError::ProtocolViolation(
                        "frame transport changed request identity".into(),
                    ));
                }
                let event =
                    receive_network_event_with_cancel(&mut operation, remaining, cancellation)?;
                match event {
                    Some(NetworkEvent::Headers {
                        timing,
                        request_id,
                        status,
                        headers,
                        content_length,
                        ..
                    }) if request_id == operation.request_id => {
                        if content_length.is_some_and(|bytes| bytes > 16 * 1024 * 1024) {
                            return Err(RuntimeError::Network("frame response body limit".into()));
                        }
                        frame_stream = Some((operation, cleanup, content_length));
                        NetworkResponse {
                            response_policy: None,
                            timing: timing.map(|timing| *timing),
                            request_id,
                            status,
                            headers,
                            body: Vec::new(),
                        }
                    }
                    Some(NetworkEvent::Failed { request_id, error })
                        if request_id == operation.request_id =>
                    {
                        return Err(RuntimeError::from(error));
                    }
                    _ => {
                        return Err(RuntimeError::ProtocolViolation(
                            "frame response headers out of order".into(),
                        ));
                    }
                }
            } else {
                collect_operation_bounded_with_cancel(
                    &network,
                    request,
                    remaining,
                    16 * 1024 * 1024,
                    cancellation,
                )
                .map_err(|error| {
                    if destination == "stylesheet" {
                        RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::network(error))
                    } else {
                        RuntimeError::Network(error.to_string())
                    }
                })?
            };
            timing_hops.push(crate::network::timing_hop(&url, &response.headers));
            if !document_resource {
                let _ = context.deliver_coep_reports(
                    &page_url,
                    coep_reporting.response(crate::coep_reporting::ResponsePolicyInputs {
                        origin: &native_origin.serialized,
                        opaque: native_origin.opaque,
                        target: &url,
                        cors,
                        credentials: (!cors || use_credentials || !font_cross)
                            && !embedder
                                .omit_credentials(cors || document_resource, resource_cross),
                        headers: &response.headers,
                        destination: if destination == "stylesheet" {
                            "style"
                        } else {
                            destination
                        },
                    }),
                );
            }
            if !document_resource {
                embedder
                    .authorize_response(
                        &native_origin.serialized,
                        native_origin.opaque,
                        &url,
                        cors,
                        (!cors || use_credentials || !font_cross)
                            && !embedder
                                .omit_credentials(cors || document_resource, resource_cross),
                        &response.headers,
                    )
                    .map_err(|message| if destination == "stylesheet" {
                        RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch(message))
                    } else { RuntimeError::Network(message) })?;
            }
            if font_cross {
                enforce_cors_response(&font_request_origin, use_credentials, &response)?;
            }
            if cancellation.load(Ordering::Acquire) {
                if destination == "stylesheet" {
                    return Err(RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Cancelled));
                }
                return Err(RuntimeError::Network(
                    "resource cancelled before response publication".into(),
                ));
            }
            self.script_document_owner(page_id, &initiator.creator)?;
            if let Some(admission) = &script_authority {
                self.script_document_owner(page_id, &admission.target)?;
            }
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                navigation_phases.redirected();
                oryn_js::navigation::PostResource::redirect(&mut post, response.status);
                if redirect == 10 {
                    if destination == "stylesheet" {
                        return Err(RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch("stylesheet redirect limit exceeded".into())));
                    }
                    return Err(RuntimeError::Network(format!(
                        "{destination} redirect limit exceeded"
                    )));
                }
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        if destination == "stylesheet" {
                            RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch("stylesheet redirect omitted Location".into()))
                        } else { RuntimeError::Network(format!("{destination} redirect omitted Location")) }
                    })?;
                policy = policy.after_response(&response.headers);
                let next = url
                    .join(location)
                    .map_err(|error| if destination == "stylesheet" {
                        RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch(error.to_string()))
                    } else { RuntimeError::Network(error.to_string()) })?;
                if cors && (!next.username().is_empty() || next.password().is_some()) {
                    return Err(RuntimeError::Network(
                        "font redirect contains credentials".into(),
                    ));
                }
                if cors
                    && url.origin() != next.origin()
                    && url.origin().ascii_serialization() != native_origin.serialized
                {
                    font_request_origin = "null".into();
                }
                if url.origin() != next.origin()
                    && url.origin() != navigation_origin_source.origin()
                {
                    navigation_request_origin = "null".into();
                }
                url = next;
                redirect_chain_cross_site |=
                    crate::cookies::SchemefulSite::from_url(&url) != initial_site;
                continue;
            }
            if destination == "object" && !(200..300).contains(&response.status) {
                return Err(RuntimeError::Network(format!(
                    "embedded resource returned HTTP {}",
                    response.status
                )));
            }
            let supplied_type = oryn_common::mime_type::MimeType::from_headers(&response.headers)
                .map(|mime| mime.serialize());
            let sniff_type =
                crate::embedded_resource_type::sniff_input(destination, supplied_type.as_deref());
            let computed_type = if document_resource && !matches!(response.status, 204 | 205) {
                let no_sniff = crate::network::response_no_sniff(&response.headers);
                Some(if let Some((operation, cleanup, _)) = &mut frame_stream {
                    crate::network::sniff_document_operation(
                        operation,
                        sniff_type.as_deref(),
                        no_sniff,
                        || {
                            if cancellation.load(Ordering::Acquire) || cleanup.cancelled() {
                                return Err("frame owner cancelled during MIME sniff".into());
                            }
                            if std::time::Instant::now() >= deadline {
                                return Err("frame deadline during MIME sniff".into());
                            }
                            Ok(())
                        },
                    )
                    .map_err(RuntimeError::Network)?
                } else {
                    oryn_net::mime_sniff::document_type(
                        sniff_type.as_deref(),
                        no_sniff,
                        &response.body,
                    )
                })
            } else {
                supplied_type.clone()
            };
            let computed_type = if embedded_representation {
                Some(
                    match crate::embedded_resource_type::select(
                        destination,
                        supplied_type.as_deref(),
                        initiator.embedded_type.as_deref(),
                        computed_type
                            .as_deref()
                            .unwrap_or("application/octet-stream"),
                        crate::network::response_no_sniff(&response.headers),
                    ) {
                        Ok(value) => value,
                        Err(_) => return Ok(ContextServiceResponse::EmbeddedResourceUnavailable),
                    },
                )
            } else {
                computed_type
            };
            if document_resource {
                if matches!(response.status, 204 | 205) {
                    return Ok(ContextServiceResponse::PageResourceNoCommit {
                        reason: format!("HTTP {}", response.status),
                    });
                }
                let attribute = download.filter(|_| source.origin() == url.origin());
                if let Some(filename) = (!embedded_representation)
                    .then(|| {
                        crate::download::response_download_filename(
                            &response.headers,
                            &url,
                            attribute.or_else(|| {
                                computed_type
                                    .as_deref()
                                    .filter(|mime| crate::download::computed_type_is_download(mime))
                                    .map(|_| "")
                            }),
                        )
                    })
                    .flatten()
                {
                    if let Some((mut operation, cleanup, _)) = frame_stream.take() {
                        response.body =
                            collect_frame_download_body(&mut operation, &cleanup, cancellation)?;
                    }
                    let media_type =
                        oryn_common::mime_type::MimeType::from_headers(&response.headers)
                            .map(|mime| mime.serialize())
                            .unwrap_or_default();
                    self.handle(ContextServiceRequest::QuarantinePageDownload {
                        page_id,
                        creator: initiator.creator.clone(),
                        url: url.to_string(),
                        suggested_filename: filename,
                        media_type,
                        body_base64: base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &response.body,
                        ),
                    })?;
                    return Ok(ContextServiceResponse::PageResourceNoCommit {
                        reason: "download".into(),
                    });
                }
            }
            if document_resource
                && (!embedded_representation || embedded_document_mime(computed_type.as_deref()))
            {
                crate::frame_embedding::authorize(
                    url.as_str(),
                    &response.headers,
                    &frame_ancestors,
                )
                .map_err(RuntimeError::Network)?;
                let credentials = !embedder.omit_credentials(document_resource, resource_cross);
                let _ = context.deliver_coep_reports(
                    &page_url,
                    coep_reporting.frame(
                        &native_origin.serialized,
                        native_origin.opaque,
                        &url,
                        credentials,
                        &response.headers,
                    ),
                );
                embedder
                    .authorize_response(
                        &native_origin.serialized,
                        native_origin.opaque,
                        &url,
                        false,
                        credentials,
                        &response.headers,
                    )
                    .map_err(RuntimeError::Network)?;
                embedder
                    .authorize_frame_response(&response.headers)
                    .map_err(RuntimeError::Network)?;
            } else if embedded_representation {
                let credentials = !embedder.omit_credentials(document_resource, resource_cross);
                let _ = context.deliver_coep_reports(
                    &page_url,
                    coep_reporting.response(crate::coep_reporting::ResponsePolicyInputs {
                        origin: &native_origin.serialized,
                        opaque: native_origin.opaque,
                        target: &url,
                        cors: false,
                        credentials,
                        headers: &response.headers,
                        destination,
                    }),
                );
                embedder
                    .authorize_response(
                        &native_origin.serialized,
                        native_origin.opaque,
                        &url,
                        false,
                        credentials,
                        &response.headers,
                    )
                    .map_err(RuntimeError::Network)?;
            }
            let extracted_type = computed_type;
            let content_type = extracted_type.as_deref();
            let essence = content_type
                .unwrap_or("text/html")
                .split(';')
                .next()
                .unwrap_or("text/html")
                .trim()
                .to_ascii_lowercase();
            if frame_stream.is_some()
                && !(essence == "text/html"
                    || oryn_dom::xml::is_text_mime(&essence)
                    || matches!(essence.as_str(), "application/xml" | "text/xml")
                    || essence.ends_with("+xml"))
                && let Some((mut operation, cleanup, _)) = frame_stream.take()
            {
                response.body =
                    collect_frame_download_body(&mut operation, &cleanup, cancellation)?;
            }
            if destination == "stylesheet" {
                if let Some(options) = stylesheet {
                    crate::integrity::verify(
                        options.integrity.as_deref().unwrap_or(""),
                        &response.body,
                        integrity_eligible,
                    )
                    .map_err(|error| {
                        RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Integrity(format!("stylesheet integrity failed: {error:?}")))
                    })?;
                }
                if !(200..300).contains(&response.status) {
                    return Err(RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch(format!(
                        "stylesheet response returned HTTP {}",
                        response.status
                    ))));
                }
                if !oryn_net::stylesheet_response_allowed(
                    &response.headers,
                    stylesheet.is_some_and(|options| options.quirks),
                    cors || integrity_eligible,
                ) {
                    return Err(RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch(
                        "stylesheet response blocked by MIME type or nosniff".into(),
                    )));
                }
            }
            let mut timing_metadata = crate::network::resource_timing_metadata(
                &native_origin.serialized,
                native_origin.opaque,
                timing_hops.clone(),
                &response,
                timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            );
            timing_metadata.navigation_phases = Some(navigation_phases.snapshot());
            let timing = Some(timing_metadata);
            if matches!(destination, "font" | "image") {
                if !(200..300).contains(&response.status) {
                    return Err(RuntimeError::Network(format!(
                        "{destination} response returned HTTP {}",
                        response.status
                    )));
                }
                return Ok(ContextServiceResponse::PageBinaryResource {
                    timing,
                    url: url.to_string(),
                    body_base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &response.body,
                    ),
                    content_type: content_type.map(str::to_owned),
                    destination: destination.into(),
                });
            }
            let inherited_encoding = if document_resource {
                inherited_frame_encoding(
                    &native_origin,
                    &url.origin().ascii_serialization(),
                    matches!(url.origin(), url::Origin::Opaque(_)),
                    parent_encoding,
                )
            } else {
                None
            };
            let (source, encoding) = if document_resource
                && oryn_dom::xml::media_document_tag(content_type.unwrap_or("")).is_some()
            {
                (String::new(), "UTF-8")
            } else {
                oryn_net::decode_resource_text_with_fallback(
                    &response.body,
                    content_type,
                    page_resource_text_kind(text_kind, content_type),
                    if destination == "stylesheet" {
                        parent_encoding
                    } else {
                        inherited_encoding.as_deref()
                    },
                )
            };
            let document_referrer = request_referrer.unwrap_or_default();
            let response_policy =
                crate::referrer::ReferrerPolicy::default().after_response(&response.headers);
            // The native Document consumes the original Refresh header with
            // its delay and charset; legacy pre-resolved immediate redirects
            // carry no authority and are no longer produced.
            let refresh_url = None;
            if document_resource {
                self.authorized_storage_origins
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService(
                            "authorized frame-origin registry is poisoned".into(),
                        )
                    })?
                    .insert(url.origin().ascii_serialization());
            }
            let worker_policy_token = if document_resource
                && (!embedded_representation || embedded_document_mime(content_type))
            {
                Some(
                    self.context_owner()?
                        .prepare_frame_worker_policy_for_creator(
                            page_id,
                            url.as_str(),
                            &response.headers,
                            &source,
                            Some(&initiator.creator),
                        )?,
                )
            } else {
                None
            };
            let stream_id = if let Some((operation, cleanup, content_length)) = frame_stream {
                let admitted = (|| {
                    let stream_id = self.next_http_identity()?;
                    let request_id = operation.request_id;
                    let support = self.prepare_support_occurrence(page_id, request_id,
                        stream_id, &initiator.creator, &source, url.as_str())?;
                    self.socket_lifecycle.admit_http(
                        stream_id,
                        Arc::new(HttpStreamEntry {
                            support: Mutex::new(support),
                            keepalive_context: None,
                            keepalive_drain: None,
                            page_id,
                            creator: Mutex::new(None),
                            prepared_frame_policy: worker_policy_token.clone(),
                            frame_deadline: Some(
                                std::time::Instant::now() + Duration::from_secs(30),
                            ),
                            request_id,
                            cleanup: Some(cleanup),
                            state: Mutex::new(HttpStreamState {
                                preload: None,
                                operation: Some(operation),
                                current: bytes::Bytes::new(),
                                offset: 0,
                                bytes_read: 0,
                                content_length,
                            }),
                        }),
                    )?;
                    Ok::<u64, RuntimeError>(stream_id)
                })();
                match admitted {
                    Ok(id) => Some(id),
                    Err(error) => {
                        if let Some(token) = &worker_policy_token {
                            context.discard_prepared_frame_policy(page_id, token)?;
                        }
                        return Err(error);
                    }
                }
            } else {
                None
            };
            let stylesheet_response = if destination == "stylesheet" {
                Some(
                    response_lease
                        .take()
                        .ok_or_else(|| {
                            RuntimeError::ProtocolViolation(
                                "stylesheet response admission absent".into(),
                            )
                        })?
                        .publish_stylesheet(
                            url.as_str(),
                            &source,
                            crate::stylesheet_response_proof::http_origin_clean(
                                cors,
                                integrity_eligible,
                            ),
                        )?,
                )
            } else {
                None
            };
            return Ok(ContextServiceResponse::PageResource {
                stylesheet_response,
                inherited_encoding,
                response_headers: response.headers.clone(),
                raw_body_base64: (document_resource && stream_id.is_none()).then(|| {
                    base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &response.body,
                    )
                }),
                request_state: (document_resource).then(|| {
                    oryn_common::navigation_request::NavigationRequestState {
                        initiator_url: request_initiator_url.clone(),
                        opaque_initiator: request_opaque,
                        referrer: (!document_referrer.is_empty())
                            .then(|| document_referrer.clone()),
                        referrer_policy: policy.as_str().into(),
                    }
                }),
                timing: timing.map(Box::new),
                content_language: oryn_dom::language::content_language_header(
                    response
                        .headers
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                        .map(|(_, value)| value.as_str()),
                ),
                stream_id,
                native_origin: None,
                effective_post: post.clone(),
                worker_policy_token,
                url: url.to_string(),
                source,
                encoding: encoding.into(),
                destination: destination.into(),
                content_type: content_type.map(str::to_owned),
                modals_allowed: crate::csp::CspPolicySet::from_headers(
                    url.as_str(),
                    &response.headers,
                )
                .sandbox()
                .is_none_or(|sandbox| sandbox.allow_modals),
                document_referrer,
                referrer_policy: response_policy.as_str().into(),
                refresh_url,
            });
        }
        Err(RuntimeError::Network(format!(
            "{destination} request did not produce a response"
        )))
    }

    fn open_websocket(
        &self,
        page_id: PageId,
        url: &str,
        protocols: &[String],
        creator: Option<&oryn_js::worker_wire::Creator>,
        request_id: u64,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let socket_id = self
            .next_socket_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| RuntimeError::WorkerLimitExceeded {
                resource: "WebSocket identifiers exhausted".into(),
            })?;
        let creator = creator.ok_or_else(|| {
            RuntimeError::ProtocolViolation("WebSocket requires native document creator".into())
        })?;
        let document_owner = self.script_document_owner(page_id, creator)?;
        let (_, native_origin, native_policy) = self
            .context_owner()?
            .native_worker_authority(&document_owner)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let policy = crate::csp::CspPolicySet::from_agent_policy(&native_policy);
        let mut observation = self.network.as_ref().and_then(|network| {
            network
                .resource_audit()
                .begin_websocket(page_id.0, socket_id, url)
        });
        let mut url = Url::parse(url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(upgraded) = policy.upgraded_url(&url) {
            url = upgraded;
        }
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(RuntimeError::ContextService(
                "WebSocket URL must use ws or wss".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        if url.fragment().is_some()
            || protocols.iter().any(|value| {
                value.is_empty()
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                    || !seen.insert(value)
            })
        {
            return Err(RuntimeError::ProtocolViolation(
                "invalid WebSocket URL or protocol token".into(),
            ));
        }
        let decision = policy.authorize(&crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Connect,
            url: url.to_string(),
            nonce: None,
            inline_source: None,
            parser_inserted: false,
            initiated_by_trusted_script: false,
        });
        if !decision.violations.is_empty() {
            self.context_owner()?
                .handle_csp_violations(page_id, decision.violations.clone())?;
        }
        if !decision.allowed {
            return Err(RuntimeError::ContextService(
                "WebSocket blocked by Content-Security-Policy connect-src".into(),
            ));
        }
        enforce_mixed_content(&native_policy.url, &url)?;
        let owner = self.context_owner()?;
        let handshake = SocketHandshakeLease::begin(
            self.socket_lifecycle.clone(),
            request_id,
            document_owner.clone(),
        )?;
        handshake.check()?;
        let addresses = owner.resolve_network_endpoint(&url)?;
        handshake.check()?;
        let connector = match owner.fixture_ca_pem() {
            Some(ca) => explicit_websocket_connector(&url, ca)?,
            None => fixture_websocket_connector(
                &url,
                !addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()),
                std::env::var_os("ORYN_WPT_CA_CERT").as_deref(),
            )?,
        };
        let timeout = self.context_owner()?.request_timeout();
        let mut stream = None;
        let mut last_error = None;
        let deadline = Instant::now() + timeout;
        for address in addresses {
            loop {
                handshake.check()?;
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match TcpStream::connect_timeout(&address, left.min(Duration::from_millis(100))) {
                    Ok(candidate) => {
                        stream = Some(candidate);
                        break;
                    }
                    Err(error) => {
                        let retry = matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        );
                        last_error = Some(error);
                        if !retry {
                            break;
                        }
                    }
                }
            }
            if stream.is_some() || Instant::now() >= deadline {
                break;
            }
        }
        let stream = stream.ok_or_else(|| {
            RuntimeError::Network(
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "WebSocket resolver returned no address".into()),
            )
        })?;
        handshake.attach(&stream)?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|_| stream.set_write_timeout(Some(timeout)))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        for (name, value) in crate::fetch_metadata::websocket_headers(
            &url,
            &native_origin.serialized,
            native_origin.opaque,
        ) {
            request.headers_mut().insert(
                name,
                value.parse().map_err(|error| {
                    RuntimeError::Network(format!("invalid native Fetch Metadata: {error}"))
                })?,
            );
        }
        let origin = native_origin.serialized;
        request.headers_mut().insert(
            "origin",
            origin
                .parse()
                .map_err(|error| RuntimeError::Network(format!("invalid Origin: {error}")))?,
        );
        if let Some(cookie) = self.context_owner()?.cookie_header_for_creator(
            page_id,
            creator,
            &url,
            "GET",
            crate::cookies::CookieRequestKind::WebSocket,
        )? {
            request.headers_mut().insert(
                "cookie",
                cookie
                    .parse()
                    .map_err(|error| RuntimeError::Network(format!("invalid Cookie: {error}")))?,
            );
        }
        if !protocols.is_empty() {
            request.headers_mut().insert(
                "sec-websocket-protocol",
                protocols.join(", ").parse().map_err(|error| {
                    RuntimeError::Network(format!("invalid WebSocket protocol: {error}"))
                })?,
            );
        }
        let (mut socket, response) =
            tungstenite::client_tls_with_config(request, stream, None, connector)
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
        handshake.check()?;
        // Polling is short and non-blocking at the broker boundary; TLS setup
        // gets the existing request deadline, not the subsequent 10 ms poll slice.
        let tcp = match socket.get_mut() {
            MaybeTlsStream::Plain(stream) => stream,
            MaybeTlsStream::Rustls(stream) => &mut stream.sock,
            _ => {
                return Err(RuntimeError::Network(
                    "unsupported WebSocket TLS stream".into(),
                ));
            }
        };
        tcp.set_read_timeout(Some(Duration::from_millis(10)))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let protocol = validate_websocket_negotiation(response.headers(), protocols)
            .map_err(RuntimeError::Network)?;
        if let Some(audit) = observation.as_mut() {
            audit.opened(response.status().as_u16());
        }
        self.script_document_owner(page_id, creator)?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::Closed);
        }
        self.socket_lifecycle
            .admit(socket_id, creator, socket, observation)?;
        Ok(ContextServiceResponse::WebSocketOpened {
            socket_id,
            protocol,
        })
    }

    fn validate_websocket_creator(
        &self,
        page: PageId,
        id: u64,
        creator: Option<&oryn_js::worker_wire::Creator>,
        require_live: bool,
    ) -> Result<(), RuntimeError> {
        self.validate_socket_owner(page)?;
        let creator = creator.ok_or_else(|| {
            RuntimeError::ProtocolViolation("WebSocket requires native creator".into())
        })?;
        let matches = self
            .socket_owners
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket owner registry poisoned".into()))?
            .get(&id)
            == Some(creator);
        if !matches {
            return Err(RuntimeError::ProtocolViolation(
                "WebSocket belongs to another document lifetime".into(),
            ));
        }
        if require_live {
            self.script_document_owner(page, creator)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn retain_websocket(
        &self,
        id: u64,
        socket: WebSocket<MaybeTlsStream<TcpStream>>,
        audit: Option<crate::resource_audit::WebSocketObservation>,
    ) -> Result<(), RuntimeError> {
        let socket = OwnedWebSocket::new(socket)?;
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::Closed);
        }
        if sockets.len() >= 32 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "live WebSockets".into(),
            });
        }
        if sockets.contains_key(&id) {
            return Err(RuntimeError::ProtocolViolation(
                "WebSocket identity reused".into(),
            ));
        }
        if let Some(audit) = audit {
            self.socket_audits
                .lock()
                .map_err(|_| RuntimeError::ContextService("WebSocket audit lock poisoned".into()))?
                .insert(id, audit);
        }
        sockets.insert(id, socket);
        Ok(())
    }

    fn send_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        frame: WebSocketFrame,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let entry = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket registry poisoned".into()))?
            .get(&socket_id)
            .cloned()
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        // Only the individual socket lock is held during transport IO.
        let mut socket = entry
            .socket
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket transport poisoned".into()))?;
        let message = match frame {
            WebSocketFrame::Text { value } => Message::Text(value.into()),
            WebSocketFrame::Binary { base64 } => Message::Binary(
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, base64)
                    .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?
                    .into(),
            ),
            WebSocketFrame::Close { code, reason } => Message::Close(Some(CloseFrame {
                code: code.map(CloseCode::from).unwrap_or(CloseCode::Normal),
                reason: reason.into(),
            })),
        };
        let bytes = if message.is_text() || message.is_binary() {
            message.len()
        } else {
            0
        };
        let result = socket.send(message);
        // A failed send can leave a readable socket. Keep its observation so
        // subsequent received payloads cannot silently escape the audit.
        if result.is_err()
            && let Ok(mut audits) = self.socket_audits.lock()
            && let Some(audit) = audits.get_mut(&socket_id)
        {
            audit.finish("failed");
        }
        result.map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Ok(mut audits) = self.socket_audits.lock()
            && let Some(audit) = audits.get_mut(&socket_id)
        {
            audit.bytes(bytes, 0);
        }
        Ok(ContextServiceResponse::WebSocketSent)
    }

    fn poll_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        max_frames: usize,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let entry = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket registry poisoned".into()))?
            .get(&socket_id)
            .cloned()
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        // Only the individual socket lock is held during transport IO.
        let mut socket = entry
            .socket
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket transport poisoned".into()))?;
        let mut frames = Vec::new();
        while frames.len() < max_frames.min(32) {
            match socket.read() {
                Ok(Message::Text(value)) => {
                    if let Ok(mut audits) = self.socket_audits.lock()
                        && let Some(audit) = audits.get_mut(&socket_id)
                    {
                        audit.bytes(0, value.len());
                    }
                    frames.push(WebSocketFrame::Text {
                        value: value.to_string(),
                    })
                }
                Ok(Message::Binary(value)) => {
                    if let Ok(mut audits) = self.socket_audits.lock()
                        && let Some(audit) = audits.get_mut(&socket_id)
                    {
                        audit.bytes(0, value.len());
                    }
                    frames.push(WebSocketFrame::Binary {
                        base64: base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            value,
                        ),
                    })
                }
                Ok(Message::Close(frame)) => {
                    socket
                        .flush()
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    if let Ok(mut audits) = self.socket_audits.lock()
                        && let Some(audit) = audits.get_mut(&socket_id)
                    {
                        audit.finish("closed");
                    }
                    frames.push(WebSocketFrame::Close {
                        code: frame.as_ref().map(|frame| u16::from(frame.code)),
                        reason: frame
                            .map(|frame| frame.reason.to_string())
                            .unwrap_or_default(),
                    });
                    break;
                }
                Ok(Message::Ping(value)) => {
                    socket
                        .send(Message::Pong(value))
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                }
                Ok(Message::Pong(_) | Message::Frame(_)) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    return Err(RuntimeError::Network(
                        "WebSocket terminated without closing handshake".into(),
                    ));
                }
                Err(
                    tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ),
                ) => {
                    return Err(RuntimeError::Network(
                        "WebSocket terminated without closing handshake".into(),
                    ));
                }
                Err(error) => return Err(RuntimeError::Network(error.to_string())),
            }
        }
        Ok(ContextServiceResponse::WebSocketEvents { frames })
    }

    fn close_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let entry = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?
            .get(&socket_id)
            .cloned()
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        let mut socket = entry
            .socket
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket transport poisoned".into()))?;
        let (code, reason, was_clean) = finish_websocket_close(&mut socket, code, reason, || {
            self.shutting_down.load(Ordering::Acquire)
        });
        drop(socket);
        self.sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket registry poisoned".into()))?
            .remove(&socket_id);
        self.socket_owners
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket owner registry poisoned".into()))?
            .remove(&socket_id);
        if let Ok(mut audits) = self.socket_audits.lock()
            && let Some(mut audit) = audits.remove(&socket_id)
        {
            audit.finish(if was_clean { "closed" } else { "failed" });
        }
        Ok(ContextServiceResponse::WebSocketClosed {
            code,
            reason,
            was_clean,
        })
    }

    fn validate_storage_scope(
        &self,
        request: &StorageRequest,
        native_origin: &oryn_js::worker_wire::AgentOrigin,
    ) -> Result<(), RuntimeError> {
        if native_origin.opaque {
            return Err(RuntimeError::ContextService(
                "SecurityError: opaque document cannot access storage".into(),
            ));
        }
        let origin_is_authorized =
            |origin: &str| -> Result<bool, RuntimeError> { Ok(origin == native_origin.serialized) };

        let (scope, area) = match request {
            StorageRequest::Get { area, scope, .. }
            | StorageRequest::Set { area, scope, .. }
            | StorageRequest::Remove { area, scope, .. }
            | StorageRequest::Clear { area, scope }
            | StorageRequest::Entries { area, scope } => (Some(scope), Some(area)),
            StorageRequest::IndexedDb { origin, .. } => {
                if origin == "null" {
                    return Err(RuntimeError::ContextService(
                        "SecurityError: opaque origins cannot access IndexedDB".into(),
                    ));
                }
                if !origin_is_authorized(origin)? {
                    return Err(RuntimeError::ContextService(
                        "IndexedDB origin was not authorized for this page worker".into(),
                    ));
                }
                (None, None)
            }
        };
        if let (Some(scope), Some(area)) = (scope, area) {
            if scope.top_level_page_id != self.source_page_id
                || !origin_is_authorized(&scope.origin)?
            {
                return Err(RuntimeError::ContextService(
                    "storage scope did not match the worker page".into(),
                ));
            }
            if *area == StorageArea::Session && scope.top_level_page_id != self.source_page_id {
                return Err(RuntimeError::ContextService(
                    "session storage is scoped to the top-level page".into(),
                ));
            }
        }
        Ok(())
    }
}

impl ContextTransport for ParentContextService {
    fn response_scope(
        &self,
        route: Option<&WorkerRoute>,
        request: Option<&ContextServiceRequest>,
    ) -> Result<Option<ResponseScope>, RuntimeError> {
        if route.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "parent response scope has foreign route".into(),
            ));
        }
        if matches!(
            request,
            Some(
                ContextServiceRequest::ReleaseStylesheetResponse { .. }
                    | ContextServiceRequest::ValidateStylesheetResponse { .. }
                    | ContextServiceRequest::CancelBlankPrerender { .. }
                    | ContextServiceRequest::ReleaseOrdinaryKeepalive { .. }
                    | ContextServiceRequest::ParserEncodingPolicy {
                        action: ParserEncodingPolicyAction::Release,
                        ..
                    }
                    | ContextServiceRequest::CancelScriptLoad { .. }
                    | ContextServiceRequest::ReleaseFrameResponseStream { .. }
                    | ContextServiceRequest::ReleaseNetworkResult { .. }
                    | ContextServiceRequest::HttpRequestAbort { .. }
            )
        ) {
            return Ok(None);
        }
        let context = self.context_owner()?;
        let cleanup = matches!(
            request,
            Some(
                ContextServiceRequest::CancelScriptLoad { .. }
                    | ContextServiceRequest::SharedMemory {
                        request: oryn_js::shared_memory::SharedMemoryRequest::CancelAdmission { .. }
                            | oryn_js::shared_memory::SharedMemoryRequest::Release { .. },
                        ..
                    }
                    | ContextServiceRequest::WorkerService {
                        request: oryn_js::worker_wire::AgentServiceRequest::Release { .. }
                            | oryn_js::worker_wire::AgentServiceRequest::Terminate { .. }
                            | oryn_js::worker_wire::AgentServiceRequest::PlatformCancel { .. }
                            | oryn_js::worker_wire::AgentServiceRequest::PlatformRetire,
                        ..
                    }
            )
        );
        if !cleanup {
            context.admit_context_response()?;
        }
        Ok(Some(ResponseScope {
            keepalive: Some(self.ordinary_keepalive.clone()),
            context: context.downgrade(),
            source: ResponseSource::Window(self.source_page_id),
            scripts: Some(self.script_loads.clone()),
            sockets: Some(response_custody::NetworkResponseCustody {
                _lifecycle: Some(self.socket_lifecycle.clone()),
                sockets: self.sockets.clone(),
                owners: self.socket_owners.clone(),
                audits: self.socket_audits.clone(),
                streams: self.http_streams.clone(),
            }),
        }))
    }
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.handle(request)
    }
}

pub fn serve_context_service(
    stream: UnixStream,
    service: ParentContextService,
) -> Result<(), RuntimeError> {
    let service = service.into_shared();
    let result = serve_context_transport(stream, service.clone());
    service.cancel_all_network_operations();
    let cleanup = service.drain_http_cleanup(Duration::from_secs(2));
    result.and(cleanup)
}

pub(crate) fn serve_context_transport(
    stream: UnixStream,
    service: Arc<dyn ContextTransport>,
) -> Result<(), RuntimeError> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
    let mut reader = BufReader::new(reader_stream);
    let writer = Arc::new(Mutex::new(stream));
    let mut last_message_id = 0_u64;
    let dispatch_limit = ContextDispatchLimit::default();
    let publications = Arc::new(Mutex::new(response_custody::Publications::default()));
    let _publication_owner = response_custody::PublicationOwner(publications.clone());
    let diagnostics = crate::ipc_progress::context_phases::Diagnostics::new();
    loop {
        let mut line = Vec::new();
        let read = read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if read == 0 {
            break;
        }
        if let Ok(receipt) = serde_json::from_slice::<ResponseReceipt>(&line) {
            if let Err(error) = response_custody::Publications::receive(&publications, &receipt) {
                let _ = reader.get_ref().shutdown(Shutdown::Both);
                return Err(error);
            }
            crate::ipc_progress::context_phases::receipt(
                &diagnostics,
                receipt.route.as_ref(),
                receipt.received_id,
            );
            continue;
        }
        let decode_started = Instant::now();
        let request = serde_json::from_slice::<RequestEnvelope>(&line);
        let diagnostic = request
            .as_ref()
            .ok()
            .and_then(|envelope| envelope.diagnostic);
        let (message_id, route, request, error) = match request {
            Ok(envelope)
                if envelope.protocol_version == CONTEXT_SERVICE_PROTOCOL_VERSION
                    && envelope.message_id > last_message_id =>
            {
                last_message_id = envelope.message_id;
                (
                    envelope.message_id,
                    envelope.route,
                    Some(envelope.request),
                    None,
                )
            }
            Ok(envelope) => (
                envelope.message_id,
                envelope.route,
                None,
                Some("duplicate message id or unsupported context protocol".into()),
            ),
            Err(error) => {
                // A malformed envelope has no authenticated correlation/route.
                // An invented response ID strands the real pending operation.
                // Closing wakes every client waiter and reclaims publications.
                let _ = reader.get_ref().shutdown(Shutdown::Both);
                return Err(RuntimeError::ProtocolViolation(format!(
                    "malformed context request: {error}"
                )));
            }
        };
        let mut phases = crate::ipc_progress::context_phases::Scope::start(
            &diagnostics,
            diagnostic,
            route.as_ref(),
            message_id,
            request
                .as_ref()
                .map(ContextServiceRequest::diagnostic_operation)
                .unwrap_or("InvalidEnvelope"),
            decode_started,
        );
        let service = service.clone();
        let writer = writer.clone();
        let publications = publications.clone();
        let permit = match dispatch_limit.admit() {
            Ok(permit) => permit,
            Err(error) => {
                // Do not queue another task (or a potentially blocking error
                // response) for an over-budget peer. Close the channel so all
                // pending client operations fail, without taking the writer lock.
                let _ = reader.get_ref().shutdown(Shutdown::Both);
                return Err(error);
            }
        };
        std::thread::Builder::new()
            .name("oryn-context-dispatch".into())
            .spawn(move || -> Result<(), RuntimeError> {
                phases.advance(); // admission and thread start -> handler
                let scope_result = service.response_scope(route.as_ref(), request.as_ref());
                let scope_error = scope_result.as_ref().err().map(ToString::to_string);
                let scope = scope_result.ok().flatten();
                let request_scope = request
                    .as_ref()
                    .map(response_custody::ResponseRequest::capture)
                    .unwrap_or_default();
                let (mut response, error) = if let Some(error) = scope_error {
                    (None, Some(error))
                } else if let Some(request) = request {
                    match service.request_routed(route.as_ref(), request) {
                        Ok(response) => (Some(response), None),
                        Err(error) => (None, Some(error.to_string())),
                    }
                } else {
                    (None, error)
                };
                let mut custody =
                    response_custody::ResponseGuard::new(scope, request_scope, response.as_ref());
                let attachment = response
                    .as_mut()
                    .and_then(shared_memory_response)
                    .and_then(|memory| memory.native_descriptor.clone());
                phases.advance(); // handler -> response writer wait
                let writer_owner = writer;
                let mut writer = writer_owner.lock().map_err(|_| {
                    RuntimeError::ContextService("context writer lock poisoned".into())
                })?;
                phases.advance(); // writer acquired -> encoding/custody
                // Serialize under the writer lock: at most one bounded encoded
                // response buffer is retained by this channel, not one per dispatch.
                let encoded = encode_context_response_status(
                    ResponseEnvelope {
                        protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                        route: route.clone(),
                        correlation_id: message_id,
                        response,
                        error,
                    },
                    MAX_CONTEXT_FRAME_BYTES,
                );
                let (encoded, preserved) = match encoded {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        let _ = writer.shutdown(Shutdown::Both);
                        return Err(error);
                    }
                };
                if !preserved {
                    // Oversized replacement never transfers the original grants.
                    drop(writer);
                    drop(custody);
                    custody = response_custody::ResponseGuard::new(None, Default::default(), None);
                    writer = writer_owner.lock().map_err(|_| {
                        RuntimeError::ContextService("context writer lock poisoned".into())
                    })?;
                }
                let inserted = publications
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(message_id, route, custody, permit);
                if let Err(unpublished) = inserted {
                    let _ = writer.shutdown(Shutdown::Both);
                    drop(writer);
                    drop(unpublished);
                    return Err(RuntimeError::Closed);
                }
                phases.advance(); // encoding/custody -> write
                phases.await_receipt();
                let written = writer
                    .write_all(&encoded)
                    .and_then(|_| writer.flush())
                    .and_then(|_| match attachment.filter(|_| preserved) {
                        Some(fd) => crate::shared_memory_fd::send(&writer, message_id, fd.as_ref()),
                        None => Ok(()),
                    })
                    .map_err(|error| RuntimeError::ContextService(error.to_string()));
                if written.is_err() {
                    let _ = writer.shutdown(Shutdown::Both);
                }
                drop(writer);
                if written.is_err() {
                    response_custody::Publications::discard(&publications, message_id);
                }
                phases.finished(written.is_ok());
                written
            })
            .map_err(|error| {
                let _ = reader.get_ref().shutdown(Shutdown::Both);
                RuntimeError::ContextService(format!("context dispatch thread: {error}"))
            })?;
    }
    Ok(())
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<usize> {
    let mut limited = reader.take(limit.saturating_add(1) as u64);
    let read = limited.read_until(b'\n', output)?;
    if output.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "context-service frame exceeded limit",
        ));
    }
    Ok(read)
}

/// Only native image-element `src` demand is marked upgradable. CSS, object,
/// input-image and imageset requests retain the blockable category. IP literals
/// never gain an HTTPS rewrite; trustworthy loopback URLs need no rewrite.
pub(crate) fn upgrade_mixed_image(source: &str, target: &mut Url, eligible: bool) {
    if eligible
        && target.scheme() == "http"
        && matches!(target.host(), Some(url::Host::Domain(_)))
        && enforce_mixed_content(source, target).is_err()
    {
        let _ = target.set_scheme("https");
    }
}

/// Active subresource mixed-content gate, after CSP URL upgrades at each hop.
pub(crate) fn enforce_mixed_content(source_origin: &str, target: &Url) -> Result<(), RuntimeError> {
    crate::mixed_content::enforce(source_origin, target).map_err(RuntimeError::ContextService)
}

fn cors_origin_allowed(source_origin: &str, credentials: bool, response: &NetworkResponse) -> bool {
    cors_headers_allowed(source_origin, credentials, &response.headers)
}

pub(crate) fn enforce_cors_preflight(
    source_origin: &str,
    credentials: bool,
    method: &str,
    requested_headers: &[String],
    response: &NetworkResponse,
) -> Result<(), RuntimeError> {
    if !oryn_net::cors::cors_preflight_allowed(
        source_origin,
        credentials,
        method,
        requested_headers,
        response.status,
        &response.headers,
    ) {
        return Err(RuntimeError::ContextService(
            "cross-origin request denied by CORS preflight".into(),
        ));
    }
    Ok(())
}

fn page_resource_text_kind(
    default: oryn_net::TextResourceKind,
    content_type: Option<&str>,
) -> oryn_net::TextResourceKind {
    let essence = content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if default == oryn_net::TextResourceKind::Html
        && (matches!(essence.as_str(), "text/xml" | "application/xml") || essence.ends_with("+xml"))
    {
        oryn_net::TextResourceKind::Xml
    } else if default == oryn_net::TextResourceKind::Html && oryn_dom::xml::is_text_mime(&essence) {
        oryn_net::TextResourceKind::PlainText
    } else {
        default
    }
}

fn enforce_cors_response(
    source_origin: &str,
    credentials: bool,
    response: &NetworkResponse,
) -> Result<(), RuntimeError> {
    if cors_origin_allowed(source_origin, credentials, response) {
        Ok(())
    } else {
        Err(RuntimeError::ContextService(
            "cross-origin response denied by CORS".into(),
        ))
    }
}

#[cfg(test)]
mod fixture_tls_tests {
    #[test]
    fn live_websocket_limit_rejects_without_replacing_and_cleanup_releases_capacity() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let service = super::ParentContextService::new(context, page.page_id());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peers = Vec::new();
        let mut socket = || {
            let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (peer, _) = listener.accept().unwrap();
            peers.push(peer);
            tungstenite::WebSocket::from_raw_socket(
                tungstenite::stream::MaybeTlsStream::Plain(client),
                tungstenite::protocol::Role::Client,
                None,
            )
        };
        for id in 1..=32 {
            service.retain_websocket(id, socket(), None).unwrap();
        }
        assert!(matches!(
            service.retain_websocket(33, socket(), None),
            Err(crate::RuntimeError::WorkerLimitExceeded { .. })
        ));
        assert_eq!(service.sockets.lock().unwrap().len(), 32);
        service.sockets.lock().unwrap().remove(&1);
        service.retain_websocket(34, socket(), None).unwrap();
        assert_eq!(service.sockets.lock().unwrap().len(), 32);
        service.cancel_all_network_operations();
        assert!(service.sockets.lock().unwrap().is_empty());
    }
    #[test]
    fn http_identity_exhaustion_does_not_wrap_or_reuse_prior_values() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let service = super::ParentContextService::new(context, page.page_id());
        service
            .next_http_stream_id
            .store(u64::MAX - 1, std::sync::atomic::Ordering::Release);
        assert_eq!(service.next_http_identity().unwrap(), u64::MAX - 1);
        assert!(service.next_http_identity().is_err());
        assert!(service.next_http_identity().is_err());
        assert_eq!(
            service
                .next_http_stream_id
                .load(std::sync::atomic::Ordering::Acquire),
            u64::MAX
        );
    }
    #[test]
    fn exhausted_websocket_identity_rejects_before_url_or_network_work_without_wrapping() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let service = super::ParentContextService::new(context, page.page_id());
        service
            .next_socket_id
            .store(u64::MAX, std::sync::atomic::Ordering::Release);
        for _ in 0..2 {
            let result = service.open_websocket(page.page_id(), "not a URL", &[], None, 0);
            assert!(
                matches!(result,Err(crate::RuntimeError::WorkerLimitExceeded{resource}) if resource=="WebSocket identifiers exhausted")
            );
            assert_eq!(
                service
                    .next_socket_id
                    .load(std::sync::atomic::Ordering::Acquire),
                u64::MAX
            );
            assert!(service.sockets.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn explicit_websocket_trust_rejects_invalid_ca_without_hostname_rewriting() {
        let url = url::Url::parse("wss://www.web-platform.test:8889/socket").unwrap();
        assert!(
            super::explicit_websocket_connector(&url, &[b"not a certificate".to_vec()]).is_err()
        );
        assert!(
            super::explicit_websocket_connector(&url, &[])
                .unwrap()
                .is_none()
        );
        assert_eq!(url.host_str(), Some("www.web-platform.test"));
        let plain = url::Url::parse("ws://www.web-platform.test:8888/socket").unwrap();
        assert!(
            super::explicit_websocket_connector(&plain, &[])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fixture_ca_is_never_selected_for_non_loopback_websockets() {
        let missing = std::ffi::OsStr::new("/nonexistent-wpt-ca.pem");
        for (address, loopback) in [
            ("wss://public.example/", true),
            ("wss://localhost/", false),
            ("ws://localhost/", true),
        ] {
            assert!(
                super::fixture_websocket_connector(
                    &url::Url::parse(address).unwrap(),
                    loopback,
                    Some(missing)
                )
                .unwrap()
                .is_none()
            );
        }
        assert!(
            super::fixture_websocket_connector(
                &url::Url::parse("wss://localhost/").unwrap(),
                true,
                Some(missing)
            )
            .is_err()
        );
    }
}

#[cfg(all(test, feature = "in-process-probe"))]
mod tests {
    use super::*;
    use crate::{
        Browser, ContextOptions, PolicyRule,
        network::{NetworkError, NetworkStreamResponse, SharedNetworkBroker},
    };

    #[tokio::test(flavor = "current_thread")]
    async fn automation_window_status_rejects_closed_source_and_revokes_closed_target_immediately()
    {
        let context = Browser::in_process_probe().new_context();
        let source = context.new_page();
        let target = context.new_page();
        let service = ParentContextService::new(context.clone(), source.page_id());
        let status = || ContextServiceRequest::AutomationWindowStatus {
            source_page_id: source.page_id(),
            target_page_id: target.page_id(),
        };
        assert!(matches!(
            service.handle(status()).unwrap(),
            ContextServiceResponse::AutomationWindowStatus { live: true }
        ));
        let retained = context.begin_script_close(target.page_id()).unwrap();
        // Admission is revoked before teardown starts, not after an async close task runs.
        assert!(matches!(
            service.handle(status()).unwrap(),
            ContextServiceResponse::AutomationWindowStatus { live: false }
        ));
        retained.close_ordered().await.unwrap();
        let retained_source = context.begin_script_close(source.page_id()).unwrap();
        assert!(
            matches!(service.handle(status()), Err(RuntimeError::PageNotFound(id)) if id == source.page_id())
        );
        retained_source.close_ordered().await.unwrap();
    }

    #[test]
    fn superseded_page_id_only_message_and_origin_routes_are_not_deserializable() {
        for request in [
            serde_json::json!({"PostMessage":{"source_page_id":1,"target_page_id":2,"target_origin":"*","data":"forged"}}),
            serde_json::json!({"GetRelatedPageOrigin":{"source_page_id":1,"target_page_id":2}}),
        ] {
            assert!(serde_json::from_value::<ContextServiceRequest>(request).is_err());
        }
    }

    #[test]
    fn drag_feedback_requires_page_route_and_current_page_generation() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let other = context.new_page();
        let service = ParentContextService::new(context, page.page_id());
        let feedback = oryn_js::drag_image::FeedbackBatch {
            page_generation: u64::MAX,
            updates: Vec::new(),
        };
        assert!(
            service
                .handle(ContextServiceRequest::DragFeedback {
                    page_id: other.page_id(),
                    feedback: feedback.clone()
                })
                .is_err()
        );
        assert!(
            service
                .handle(ContextServiceRequest::DragFeedback {
                    page_id: page.page_id(),
                    feedback
                })
                .is_err()
        );
        assert!(page.drag_feedback().unwrap().is_empty());
    }

    #[test]
    fn navigation_events_require_page_route_and_active_operation_identity() {
        use crate::navigation_control::*;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let other = context.new_page();
        let service = ParentContextService::new(context.clone(), page.page_id());
        let control = page.navigation_control().unwrap();
        let job = control
            .begin(NavigationRequest {
                command: NavigationCommand::Refresh,
                strategy: PageLoadStrategy::Normal,
                timeout_ms: None,
            })
            .unwrap();
        assert!(
            service
                .handle(ContextServiceRequest::NavigationEvent {
                    page_id: other.page_id(),
                    id: job.id,
                    event: NavigationEvent::Started
                })
                .is_err()
        );
        assert!(matches!(
            service
                .handle(ContextServiceRequest::NavigationEvent {
                    page_id: page.page_id(),
                    id: NavigationId(job.id.0 + 1),
                    event: NavigationEvent::Started
                })
                .unwrap(),
            ContextServiceResponse::NavigationControl {
                result: Err(NavigationStop::Superseded)
            }
        ));
        assert!(matches!(
            service
                .handle(ContextServiceRequest::NavigationEvent {
                    page_id: page.page_id(),
                    id: job.id,
                    event: NavigationEvent::Started
                })
                .unwrap(),
            ContextServiceResponse::NavigationControl { result: Ok(()) }
        ));
        service.cancel_all_network_operations();
        assert_eq!(job.checkpoint(), Err(NavigationStop::Retired));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn navigation_stop_cleanup_survives_closed_admission_without_foreign_authority() {
        use crate::navigation_control::*;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let other = context.new_page();
        let service = ParentContextService::new(context.clone(), page.page_id());
        let control = page.navigation_control().unwrap();
        let job = control
            .begin(NavigationRequest {
                command: NavigationCommand::Refresh,
                strategy: PageLoadStrategy::None,
                timeout_ms: None,
            })
            .unwrap();
        job.started().unwrap();
        let stop = |page_id, id| ContextServiceRequest::NavigationEvent {
            page_id,
            id,
            event: NavigationEvent::Stopped(NavigationStop::Cancelled),
        };
        assert!(matches!(
            service
                .handle(stop(page.page_id(), NavigationId(job.id.0 + 1)))
                .unwrap(),
            ContextServiceResponse::NavigationControl {
                result: Err(NavigationStop::Superseded)
            }
        ));
        assert!(job.progress().unwrap().stopped.is_none());
        let closing = context.begin_script_close(page.page_id()).unwrap();
        assert!(service.handle(stop(other.page_id(), job.id)).is_err());
        assert!(matches!(
            service.handle(stop(page.page_id(), job.id)).unwrap(),
            ContextServiceResponse::NavigationControl {
                result: Ok(()) | Err(NavigationStop::Retired)
            }
        ));
        assert!(job.progress().unwrap().stopped.is_some());
        service.shutting_down.store(true, Ordering::Release);
        assert!(matches!(
            service.handle(stop(page.page_id(), job.id)).unwrap(),
            ContextServiceResponse::NavigationControl {
                result: Ok(()) | Err(NavigationStop::Retired)
            }
        ));
        assert!(
            service
                .handle(ContextServiceRequest::NavigationEvent {
                    page_id: page.page_id(),
                    id: job.id,
                    event: NavigationEvent::Started
                })
                .is_err()
        );
        closing.close_ordered().await.unwrap();
        assert!(matches!(
            service.handle(stop(page.page_id(), job.id)).unwrap(),
            ContextServiceResponse::NavigationControl {
                result: Err(NavigationStop::Retired)
            }
        ));
        assert!(service.handle(stop(other.page_id(), job.id)).is_err());
        other.close().await.unwrap();
    }

    #[test]
    fn data_navigation_preparation_rejects_foreign_and_non_data_inputs() {
        let context = Browser::in_process_probe().new_context();
        let source = context.new_page();
        let other = context.new_page();
        let service = ParentContextService::new(context.clone(), source.page_id());
        let original_origin = context.page_origin(source.page_id()).unwrap();
        for (page_id, url) in [
            (other.page_id(), "data:text/html,foreign"),
            (source.page_id(), "https://example.test/"),
            (source.page_id(), "data:text/html;base64,%%%"),
        ] {
            assert!(
                service
                    .handle(ContextServiceRequest::PrepareInitialLocalNavigation {
                        page_id,
                        url: url.into(),
                        blob: false
                    })
                    .is_err()
            );
            assert_eq!(
                context.page_origin(source.page_id()).unwrap(),
                original_origin
            );
        }
        let ContextServiceResponse::LocalDataNavigationPrepared{token,..}=service.handle(ContextServiceRequest::PrepareInitialLocalNavigation{
            page_id:source.page_id(),url:"data:text/plain,%3Cmeta%20http-equiv=Content-Security-Policy%20content=default-src%20none%3E".into(),blob:false,
        }).unwrap()else{panic!()};
        assert_eq!(
            context.page_origin(source.page_id()).unwrap(),
            original_origin
        );
        let (_, policy) = context
            .commit_prepared_local_navigation(source.page_id(), &token)
            .unwrap();
        assert!(policy.is_empty(), "text/plain meta text cannot add CSP");
    }

    #[test]
    fn initial_local_navigation_materializes_native_owner_and_retires_it_on_commit() {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page().page_id();
        let other = context.new_page().page_id();
        let service = ParentContextService::new(context.clone(), page);
        let creator = context.native_initial_document_creator(page).unwrap();
        assert_eq!(
            creator,
            context.native_initial_document_creator(page).unwrap()
        );
        let Creator::Window {
            document_generation,
            stream_epoch,
            ..
        } = creator
        else {
            panic!()
        };
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Root, document_generation, stream_epoch)
                .is_ok()
        );
        assert!(
            service
                .handle(ContextServiceRequest::PrepareInitialLocalNavigation {
                    page_id: other,
                    url: "data:text/plain,foreign".into(),
                    blob: false
                })
                .is_err()
        );
        let ContextServiceResponse::LocalDataNavigationPrepared { token, .. } = service
            .handle(ContextServiceRequest::PrepareInitialLocalNavigation {
                page_id: page,
                url: "data:text/plain,first".into(),
                blob: false,
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(context.page_origin(page).unwrap(), "null");
        context
            .commit_prepared_local_navigation(page, &token)
            .unwrap();
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Root, document_generation, stream_epoch)
                .is_err()
        );
        assert!(context.native_initial_document_creator(page).is_err());
        assert!(
            !context
                .native_document_authority(page, "data:text/plain,first")
                .unwrap()
                .4
        );
    }

    #[test]
    fn initial_document_rollback_restores_native_policy_after_local_and_http_commit_failure() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page().page_id();
        let other = context.new_page().page_id();
        for local in [true, false] {
            let creator = context.native_initial_document_creator(page).unwrap();
            let original = context.page_origin(page).unwrap();
            let pending = if local {
                Some(
                    context
                        .prepare_local_navigation(
                            page,
                            creator,
                            "data:text/plain,replacement",
                            false,
                        )
                        .unwrap()
                        .0,
                )
            } else {
                None
            };
            let token = context.park_initial_document(page).unwrap();
            if let Some(grant) = pending {
                context
                    .commit_prepared_local_navigation(page, &grant)
                    .unwrap();
            } else {
                context
                    .install_document_security(
                        page,
                        "https://replacement.test/".into(),
                        &[(
                            "content-security-policy".into(),
                            "default-src 'none'".into(),
                        )],
                    )
                    .unwrap();
            }
            assert!(
                context
                    .finish_initial_document(other, token.clone(), true)
                    .is_err()
            );
            context
                .finish_initial_document(page, token.clone(), true)
                .unwrap();
            assert_eq!(context.page_origin(page).unwrap(), original);
            assert!(context.native_initial_document_creator(page).is_ok());
            assert!(context.finish_initial_document(page, token, true).unwrap());
        }
        let token = context.park_initial_document(page).unwrap();
        context
            .finish_initial_document(page, token.clone(), false)
            .unwrap();
        context
            .finish_initial_document(page, token.clone(), false)
            .unwrap();
        assert!(context.finish_initial_document(page, token, true).is_err());
    }

    #[test]
    fn initial_auxiliary_policy_keeps_creator_self_and_real_blank_url() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let context = Browser::in_process_probe().new_context();
        let opener = context.new_page().page_id();
        context
            .install_document_security(
                opener,
                "https://source.test/".into(),
                &[("content-security-policy".into(), "img-src 'self'".into())],
            )
            .unwrap();
        context
            .handle_message_wire(
                opener,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let popup = context
            .new_popup_from_creator(
                opener,
                &Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap()
            .page_id();
        let creator = context.native_initial_document_creator(popup).unwrap();
        let Creator::Window {
            document_generation,
            stream_epoch,
            ..
        } = creator.clone()
        else {
            panic!()
        };
        let snapshot = context
            .window_worker_policy(
                popup,
                &WindowTarget::Root,
                document_generation,
                stream_epoch,
            )
            .unwrap();
        assert_eq!(snapshot.url, "about:blank");
        assert_eq!(
            snapshot.policy_self_url.as_deref(),
            Some("https://source.test/")
        );
        assert!(
            context
                .native_document_authority(popup, "about:blank")
                .unwrap()
                .4
        );
        let (_, _, _, headers, origin) = context
            .prepare_local_navigation(popup, creator, "data:text/html,child", false)
            .unwrap();
        assert!(origin.opaque);
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "content-security-policy"
                    && value.contains("img-src 'self'"))
        );
    }

    #[test]
    fn local_navigation_grants_preserve_outgoing_origin_and_reject_replay_or_retired_source() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let other = context.new_page();
        context
            .install_document_security(
                page.page_id(),
                "https://source.test/".into(),
                &[
                    ("content-security-policy".into(), "img-src 'self'".into()),
                    ("referrer-policy".into(), "no-referrer".into()),
                ],
            )
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let (token, body, _, headers, origin) = context
            .prepare_local_navigation(
                page.page_id(),
                creator.clone(),
                "data:text/html,hello",
                false,
            )
            .unwrap();
        assert_eq!(body, b"hello");
        assert!(origin.opaque);
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://source.test"
        );
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "referrer-policy" && value == "no-referrer")
        );
        assert!(
            context
                .commit_prepared_local_navigation(other.page_id(), &token)
                .is_err()
        );
        let committed = context
            .commit_prepared_local_navigation(page.page_id(), &token)
            .unwrap();
        assert_eq!(committed.0.authority, origin.authority);
        assert!(
            context
                .commit_prepared_local_navigation(page.page_id(), &token)
                .is_err()
        );
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        context
            .install_document_security(page.page_id(), "https://source.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let (token, _, _, _, _) = context
            .prepare_local_navigation(page.page_id(), creator, "data:text/plain,stale", false)
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .commit_prepared_local_navigation(page.page_id(), &token)
                .is_err()
        );
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://source.test"
        );
    }

    #[test]
    fn local_navigation_commit_requires_retained_parked_source_not_evicted_snapshot() {
        use oryn_js::{
            message_wire::{MessageRequest, MessageResponse, WindowTarget},
            worker_wire::Creator,
        };
        for evicted in [false, true] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page().page_id();
            context
                .install_document_security(page, "https://source.test/".into(), &[])
                .unwrap();
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let (token, _, _, _, _) = context
                .prepare_local_navigation(
                    page,
                    Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                    "data:text/plain,destination",
                    false,
                )
                .unwrap();
            let MessageResponse::DocumentParked(parked) = context
                .handle_message_wire(
                    page,
                    MessageRequest::ParkDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            if evicted {
                context
                    .handle_message_wire(page, MessageRequest::DiscardDocument { token: parked })
                    .unwrap();
            }
            assert_eq!(
                context
                    .commit_prepared_local_navigation(page, &token)
                    .is_ok(),
                !evicted
            );
        }
    }

    #[test]
    fn blob_navigation_grant_retains_native_opaque_origin_without_mutating_source() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        context
            .install_document_security(page.page_id(), "data:text/html,source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let (owner, _, origin) = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap();
        let url = context
            .register_native_blob(owner, b"destination".to_vec(), "text/html".into())
            .unwrap();
        let (token, bytes, _, _, granted) = context
            .prepare_local_navigation(
                page.page_id(),
                Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                &url,
                true,
            )
            .unwrap();
        assert_eq!(bytes, b"destination");
        assert_eq!(origin.authority, granted.authority);
        context
            .discard_prepared_local_navigation(page.page_id(), &token)
            .unwrap();
        assert!(
            context
                .commit_prepared_local_navigation(page.page_id(), &token)
                .is_err()
        );
        assert_eq!(
            context
                .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
                .unwrap()
                .2
                .authority,
            origin.authority
        );
    }

    #[test]
    fn page_download_route_uses_exact_native_source_and_rejects_retired_or_foreign_callers() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let mut options = crate::ContextOptions::default();
        options.action_policy.rules.insert(
            crate::security::ConsequentialAction::Download,
            crate::security::PolicyRule::Allow,
        );
        options.action_policy.max_download_bytes = 4;
        let context = Browser::in_process_probe().new_context_with_options(options);
        let source = context.new_page();
        let other = context.new_page();
        for page in [source.page_id(), other.page_id()] {
            context
                .install_document_security(page, "https://source.test/".into(), &[])
                .unwrap();
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
        }
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), source.page_id());
        let request = |page, body: &[u8]| ContextServiceRequest::QuarantinePageDownload {
            page_id: page,
            creator: creator.clone(),
            url: "https://files.test/asset".into(),
            suggested_filename: "../result.txt".into(),
            media_type: "text/plain".into(),
            body_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body),
        };
        assert!(matches!(
            service
                .handle(ContextServiceRequest::AuthorizePageDownload {
                    page_id: source.page_id(),
                    creator: creator.clone(),
                    url: "data:text/plain,allowed".into()
                })
                .unwrap(),
            ContextServiceResponse::PageDownloadAuthorized
        ));
        assert!(
            service
                .handle(ContextServiceRequest::AuthorizePageDownload {
                    page_id: other.page_id(),
                    creator: creator.clone(),
                    url: "https://files.test/x".into()
                })
                .is_err()
        );
        assert!(service.handle(request(other.page_id(), b"x")).is_err());
        assert!(service.handle(request(source.page_id(), b"large")).is_err());
        assert!(context.downloads().is_empty());
        let ContextServiceResponse::DownloadQuarantined { download } =
            service.handle(request(source.page_id(), b"data")).unwrap()
        else {
            panic!()
        };
        assert_eq!(download.source_origin, "https://source.test");
        assert_eq!(download.suggested_filename, "result.txt");
        assert_eq!(context.release_download(download.id).unwrap(), b"data");
        context
            .handle_message_wire(
                source.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(service.handle(request(source.page_id(), b"x")).is_err());
        assert!(context.downloads().is_empty());
    }

    #[test]
    fn page_download_rejects_native_initiator_sandbox_even_when_response_url_is_foreign() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let mut options = crate::ContextOptions::default();
        options.action_policy.rules.insert(
            crate::security::ConsequentialAction::Download,
            crate::security::PolicyRule::Allow,
        );
        let context = Browser::in_process_probe().new_context_with_options(options);
        let source = context.new_page();
        context
            .install_document_security(
                source.page_id(),
                "https://source.test/".into(),
                &[(
                    "content-security-policy".into(),
                    "sandbox allow-scripts allow-same-origin".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                source.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .authorize_page_download(
                    source.page_id(),
                    &Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    },
                    "data:text/plain,blocked"
                )
                .is_err()
        );
        assert!(
            context
                .quarantine_page_download(
                    source.page_id(),
                    Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    },
                    "https://other.test/file".into(),
                    "file".into(),
                    "text/plain".into(),
                    b"x".to_vec()
                )
                .is_err()
        );
        assert!(context.downloads().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn structured_message_route_rejects_impersonation_and_retired_source() {
        use oryn_js::message_wire::{MessageRequest, MessageResponse};
        let context = Browser::in_process_probe().new_context();
        let source = context.new_page();
        let other = context.new_page();
        let service = ParentContextService::new(context.clone(), source.page_id());
        assert!(
            service
                .handle(ContextServiceRequest::MessageWire {
                    source_page_id: other.page_id(),
                    request: MessageRequest::CreateChannel,
                })
                .is_err()
        );
        assert!(matches!(
            service
                .handle(ContextServiceRequest::MessageWire {
                    source_page_id: source.page_id(),
                    request: MessageRequest::CreateChannel
                })
                .unwrap(),
            ContextServiceResponse::MessageWire { result: Err(_) }
        ));
        context
            .install_document_security(source.page_id(), "https://a.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                source.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let ports = match service
            .handle(ContextServiceRequest::MessageWire {
                source_page_id: source.page_id(),
                request: MessageRequest::InDocument {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                    request: Box::new(MessageRequest::CreateChannel),
                },
            })
            .unwrap()
        {
            ContextServiceResponse::MessageWire {
                result: Ok(MessageResponse::Channel(ports)),
            } => ports,
            _ => panic!("expected native channel"),
        };
        let retained = context.begin_script_close(source.page_id()).unwrap();
        assert!(matches!(
            service
                .handle(ContextServiceRequest::MessageWire {
                    source_page_id: source.page_id(),
                    request: MessageRequest::PollPort {
                        port: ports[0].clone(),
                        max: 1
                    },
                })
                .unwrap(),
            ContextServiceResponse::MessageWire { result: Err(_) }
        ));
        retained.close_ordered().await.unwrap();
        other.close().await.unwrap();
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn parent_prompt_resolution_bypasses_page_actor_and_rejects_foreign_page() {
        use oryn_js::prompts::*;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let other = context.new_page();
        let controller = page.prompt_controller().unwrap();
        let service = ParentContextService::new(context.clone(), page.page_id()).into_shared();
        let token = PromptToken {
            id: 1,
            realm: oryn_js::realm::RealmId(1),
            document_generation: 1,
        };
        let request = PromptRequest {
            token,
            kind: PromptKind::Prompt,
            text: "Name".into(),
            default_value: "initial".into(),
        };
        assert!(
            service
                .handle(ContextServiceRequest::UserPrompt {
                    page_id: other.page_id(),
                    request: request.clone()
                })
                .is_err()
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let dispatch = service.clone();
        let page_id = page.page_id();
        let thread = std::thread::spawn(move || {
            tx.send(dispatch.handle(ContextServiceRequest::UserPrompt { page_id, request }))
                .unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !controller.is_waiting() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        if !controller.is_waiting() {
            controller.retire();
            panic!("prompt was not published");
        }
        assert!(
            other
                .prompt_controller()
                .unwrap()
                .inspect()
                .unwrap()
                .is_none()
        );
        let stale = PromptToken { id: 2, ..token };
        service
            .handle(ContextServiceRequest::CancelUserPrompt {
                page_id,
                token: stale,
            })
            .unwrap();
        assert!(controller.is_waiting());
        controller.set_input(token, "replacement".into()).unwrap();
        controller.accept(token).unwrap();
        let response = rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        assert!(
            matches!(response,ContextServiceResponse::UserPrompt {response:Ok(PromptResponse::Accepted(text))} if text=="replacement")
        );
        thread.join().unwrap();
        assert!(controller.inspect().unwrap().is_none());
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn retiring_context_route_wakes_pending_prompt_dispatch() {
        use oryn_js::prompts::*;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let controller = page.prompt_controller().unwrap();
        let service = ParentContextService::new(context, page.page_id()).into_shared();
        let dispatch = service.clone();
        let page_id = page.page_id();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            tx.send(dispatch.handle(ContextServiceRequest::UserPrompt {
                page_id,
                request: PromptRequest {
                    token: PromptToken {
                        id: 1,
                        realm: oryn_js::realm::RealmId(1),
                        document_generation: 1,
                    },
                    kind: PromptKind::Alert,
                    text: "waiting".into(),
                    default_value: String::new(),
                },
            }))
            .unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !controller.is_waiting() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        service.cancel_all_network_operations();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(),
            ContextServiceResponse::UserPrompt {
                response: Err(PromptError::Retired)
            }
        ));
        thread.join().unwrap();
    }

    #[test]
    fn native_image_upgrade_is_category_bound_and_preserves_custom_ports() {
        struct Image(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Image {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
        }
        for (url, eligible, expected) in [
            (
                "http://images.test:8443/image",
                true,
                Some("https://images.test:8443/image"),
            ),
            (
                "http://images.test/image",
                true,
                Some("https://images.test/image"),
            ),
            ("http://images.test/image", false, None),
            ("http://192.0.2.1/image", true, None),
            (
                "http://localhost/image",
                true,
                Some("http://localhost/image"),
            ),
        ] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://source.test/".into())
                .unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(Image(seen.clone()))),
            );
            let request = HttpStreamRequest {
                upgrade_mixed_image: eligible,
                fetch: None,
                client: None,
                use_cors_preflight: false,
                request_id: 700,
                method: "GET".into(),
                url: url.into(),
                headers: Vec::new(),
                body_base64: String::new(),
                credentials: false,
                credentials_mode: Some("omit".into()),
                initiator_url: None,
                referrer_policy: None,
            };
            let result = service.start_http_stream_with_cors(
                page_id,
                request,
                crate::csp::CspDestination::Image,
                false,
            );
            assert_eq!(result.is_ok(), expected.is_some(), "{url}/{eligible}");
            if let Some(expected) = expected {
                assert_eq!(seen.lock().unwrap()[0].url, expected);
            } else {
                assert!(seen.lock().unwrap().is_empty());
            }
            if let Ok(ContextServiceResponse::HttpStreamStarted { stream_id, .. }) = result {
                service.abort_http_stream(page_id, stream_id).unwrap();
            }
        }
    }

    #[test]
    fn fetch_redirect_admission_ignores_source_path_only_after_first_hop() {
        struct Redirect(Arc<Mutex<usize>>);
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                *self.0.lock().unwrap() += 1;
                let first = request.url.ends_with("/allowed");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if first { 302 } else { 200 },
                    headers: if first {
                        vec![("location".into(), "/other".into())]
                    } else {
                        Vec::new()
                    },
                    body: Vec::new(),
                })
            }
        }
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        context
            .install_document_security(
                page_id,
                "https://example.test/".into(),
                &[(
                    "Content-Security-Policy".into(),
                    "connect-src https://example.test/allowed".into(),
                )],
            )
            .unwrap();
        let seen = Arc::new(Mutex::new(0));
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Redirect(seen.clone()))),
        );
        let request = |url: &str| HttpStreamRequest {
            upgrade_mixed_image: false,
            fetch: Some(oryn_js::platform::FetchOptions {
                keepalive: false,
                privileged_range: false,
                cache: Default::default(),
                mode: "cors".into(),
                redirect: "follow".into(),
                integrity: String::new(),
            }),
            client: None,
            use_cors_preflight: false,
            request_id: 600,
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body_base64: String::new(),
            credentials: false,
            credentials_mode: Some("omit".into()),
            initiator_url: None,
            referrer_policy: None,
        };
        let result = service
            .start_http_stream(
                page_id,
                request("https://example.test/allowed"),
                crate::csp::CspDestination::Connect,
            )
            .unwrap();
        let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = result else {
            panic!("redirected response")
        };
        service.abort_http_stream(page_id, stream_id).unwrap();
        assert_eq!(*seen.lock().unwrap(), 2);
        assert!(
            service
                .start_http_stream(
                    page_id,
                    request("https://example.test/other"),
                    crate::csp::CspDestination::Connect
                )
                .is_err()
        );
        assert_eq!(*seen.lock().unwrap(), 2);
    }

    #[test]
    fn fetch_modes_redirect_filters_and_integrity_gate_response_publication() {
        use oryn_js::platform::{FetchOptions, FetchResponseType};
        struct Replies(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Replies {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let redirect = request.url.ends_with("/start");
                let mut headers = vec![
                    ("access-control-allow-origin".into(), "*".into()),
                    ("content-type".into(), "text/plain".into()),
                ];
                if redirect {
                    headers.push(("location".into(), "https://other.test/end".into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers,
                    body: b"hello".to_vec(),
                })
            }
        }
        let good = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
        for (mode, redirect, integrity, url, success, hops, filter) in [
            (
                "cors",
                "follow",
                good,
                "https://example.test/start",
                true,
                2,
                None,
            ),
            (
                "cors",
                "follow",
                "sha256-AA==",
                "https://example.test/start",
                false,
                2,
                None,
            ),
            (
                "cors",
                "error",
                "",
                "https://example.test/start",
                false,
                1,
                None,
            ),
            (
                "cors",
                "manual",
                "",
                "https://example.test/start",
                true,
                1,
                Some(FetchResponseType::OpaqueRedirect),
            ),
            (
                "cors",
                "manual",
                good,
                "https://example.test/start",
                false,
                1,
                None,
            ),
            (
                "same-origin",
                "follow",
                "",
                "https://example.test/start",
                false,
                1,
                None,
            ),
            (
                "same-origin",
                "follow",
                "",
                "https://other.test/end",
                false,
                0,
                None,
            ),
            (
                "no-cors",
                "follow",
                "",
                "https://other.test/end",
                true,
                1,
                Some(FetchResponseType::Opaque),
            ),
            (
                "no-cors",
                "follow",
                good,
                "https://other.test/end",
                false,
                1,
                None,
            ),
            (
                "no-cors",
                "follow",
                good,
                "https://example.test/end",
                true,
                1,
                None,
            ),
            (
                "cors",
                "follow",
                good,
                "data:text/plain,hello",
                true,
                0,
                None,
            ),
            (
                "cors",
                "follow",
                "sha256-AA==",
                "data:text/plain,hello",
                false,
                0,
                None,
            ),
        ] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://example.test/private".into())
                .unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(Replies(seen.clone()))),
            );
            let result = service.start_http_stream(
                page_id,
                HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: Some(FetchOptions {
                        keepalive: false,
                        privileged_range: false,
                        cache: Default::default(),
                        mode: mode.into(),
                        redirect: redirect.into(),
                        integrity: integrity.into(),
                    }),
                    client: None,
                    use_cors_preflight: false,
                    request_id: 500,
                    method: "GET".into(),
                    url: url.into(),
                    headers: Vec::new(),
                    body_base64: String::new(),
                    credentials: false,
                    credentials_mode: Some("omit".into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
                crate::csp::CspDestination::Connect,
            );
            assert_eq!(
                result.is_ok(),
                success,
                "{mode}/{redirect}/{url}/{integrity}: {result:?}"
            );
            assert_eq!(seen.lock().unwrap().len(), hops);
            if let Ok(ContextServiceResponse::HttpStreamStarted {
                internal_response,
                stream_id,
                response_type,
                status,
                url,
                headers,
                ..
            }) = result
            {
                assert_eq!(response_type, filter);
                if filter.is_some() {
                    assert_eq!(status, 0);
                    assert!(url.is_empty());
                    assert!(headers.is_empty());
                    let internal = internal_response.unwrap();
                    assert_eq!(
                        internal.status,
                        if redirect == "manual" { 302 } else { 200 }
                    );
                    let ContextServiceResponse::HttpStreamChunk { body_base64, .. } =
                        service.read_http_stream(page_id, stream_id, 4096).unwrap()
                    else {
                        panic!("private response body")
                    };
                    assert_eq!(
                        base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            body_base64
                        )
                        .unwrap(),
                        b"hello"
                    );
                    if redirect == "manual" {
                        assert!(internal.headers.iter().any(|(name, _)| name == "location"));
                    }
                }
                service.abort_http_stream(page_id, stream_id).unwrap();
            }
        }
    }

    #[test]
    fn streamed_fetch_redirects_rewrite_requests_and_preserve_terminal_responses() {
        struct Redirects {
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
            status: u16,
            location: bool,
        }
        impl NetworkBroker for Redirects {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                let first = request.url.ends_with("/start");
                let mut headers = vec![];
                if first && self.location {
                    headers.push(("location".into(), "/end".into()));
                    headers.push(("referrer-policy".into(), "no-referrer".into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if first { self.status } else { 200 },
                    headers,
                    body: b"response".to_vec(),
                })
            }
        }
        for (status, method, location, expected) in [
            (301, "POST", true, "GET"),
            (302, "POST", true, "GET"),
            (303, "HEAD", true, "HEAD"),
            (307, "POST", true, "POST"),
            (308, "PUT", true, "PUT"),
            (302, "POST", false, "POST"),
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://example.test/private?q=1".into())
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(Redirects {
                    requests: requests.clone(),
                    status,
                    location,
                })),
            );
            let result = service
                .start_http_stream(
                    page_id,
                    HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: None,
                        client: None,
                        use_cors_preflight: false,
                        request_id: 500,
                        method: method.into(),
                        url: "https://example.test/start".into(),
                        headers: vec![("Content-Type".into(), "text/plain".into())],
                        body_base64: if method == "HEAD" {
                            String::new()
                        } else {
                            "YWJj".into()
                        },
                        credentials: false,
                        credentials_mode: Some("omit".into()),
                        initiator_url: None,
                        referrer_policy: None,
                    },
                    crate::csp::CspDestination::Connect,
                )
                .unwrap();
            let ContextServiceResponse::HttpStreamStarted {
                stream_id,
                status: actual_status,
                url,
                redirected,
                cors_filtered,
                ..
            } = result
            else {
                panic!("stream response")
            };
            assert_eq!(actual_status, if location { 200 } else { status });
            assert_eq!(redirected, location);
            assert!(!cors_filtered);
            assert_eq!(
                url,
                if location {
                    "https://example.test/end"
                } else {
                    "https://example.test/start"
                }
            );
            let seen = requests.lock().unwrap();
            assert_eq!(seen.len(), if location { 2 } else { 1 });
            let final_request = seen.last().unwrap();
            assert_eq!(final_request.method, expected);
            if location {
                assert!(
                    !final_request
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
                );
                assert_eq!(
                    final_request
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("content-type")),
                    expected != "GET"
                );
            }
            assert_eq!(
                final_request.body.as_slice(),
                if expected == "GET" || expected == "HEAD" {
                    &b""[..]
                } else {
                    &b"abc"[..]
                }
            );
            drop(seen);
            service
                .handle(ContextServiceRequest::HttpStreamAbort {
                    creator: None,
                    page_id,
                    stream_id,
                })
                .unwrap();
        }
    }

    #[test]
    fn streamed_fetch_rejects_cross_origin_redirect_before_following_location() {
        struct DeniedRedirect(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for DeniedRedirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 302,
                    headers: vec![("location".into(), "https://example.test/end".into())],
                    body: vec![],
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(DeniedRedirect(requests.clone()))),
        );
        let error = service
            .start_http_stream(
                page_id,
                HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: None,
                    client: None,
                    use_cors_preflight: false,
                    request_id: 501,
                    method: "GET".into(),
                    url: "https://other.test/start".into(),
                    headers: vec![],
                    body_base64: String::new(),
                    credentials: false,
                    credentials_mode: Some("omit".into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
                crate::csp::CspDestination::Connect,
            )
            .unwrap_err();
        assert!(error.to_string().contains("CORS"));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn frame_fallback_uses_embedding_origin_not_navigation_initiator() {
        struct Body;
        impl NetworkBroker for Body {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/html".into())],
                    body: vec![0xc0],
                })
            }
        }
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        context
            .install_document_security(page.page_id(), "https://example.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let initiator = ResourceInitiator {
            embedded_type: None,
            request_state: None,
            navigation: Default::default(),
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            stylesheet: None,
            prepared_policy: None,
        };
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Body)),
        );
        for (url, expected) in [
            ("https://example.test/frame", Some("windows-1251")),
            ("https://other.test/frame", None),
        ] {
            let response = service
                .load_page_resource(
                    page.page_id(),
                    url,
                    "frame",
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: Some("https://unrelated-initiator.test/"),
                        referrer_policy: None,
                        parent_encoding: Some("windows-1251"),
                        post: None,
                    },
                )
                .unwrap();
            let ContextServiceResponse::PageResource {
                inherited_encoding,
                encoding,
                ..
            } = response
            else {
                panic!("frame response")
            };
            assert_eq!(inherited_encoding.as_deref(), expected);
            assert_eq!(encoding, expected.unwrap_or("windows-1252"));
        }
    }
    #[test]
    fn opaque_and_non_ascii_compatible_frame_fallbacks_are_not_inherited() {
        let mut parent = oryn_js::worker_wire::AgentOrigin {
            serialized: "https://example.test".into(),
            opaque: false,
            authority: "strict-parent".into(),
        };
        assert_eq!(
            inherited_frame_encoding(&parent, "https://example.test", false, Some("windows-1251"))
                .as_deref(),
            Some("windows-1251")
        );
        for label in ["UTF-16LE", "UTF-16BE", "replacement", "not-an-encoding"] {
            assert_eq!(
                inherited_frame_encoding(&parent, "https://example.test", false, Some(label)),
                None
            );
        }
        assert_eq!(
            inherited_frame_encoding(
                &parent,
                "https://example.test:444",
                false,
                Some("windows-1251")
            ),
            None
        );
        assert_eq!(
            inherited_frame_encoding(&parent, "https://example.test", true, Some("windows-1251")),
            None
        );
        parent.opaque = true;
        assert_eq!(
            inherited_frame_encoding(&parent, "https://example.test", false, Some("windows-1251")),
            None
        );
    }
    #[test]
    fn frame_response_modal_permission_comes_from_enforcing_headers_only() {
        struct Headers(Vec<(String, String)>);
        impl NetworkBroker for Headers {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {response_policy: None,timing:None,request_id:request.request_id,status:200,headers:self.0.clone(),
                    body:b"<meta http-equiv=Content-Security-Policy content='sandbox allow-modals'><p>frame</p>".to_vec()})
            }
        }
        for (headers, allowed) in [
            (vec![], true),
            (
                vec![(
                    "content-security-policy".into(),
                    "sandbox allow-scripts".into(),
                )],
                false,
            ),
            (
                vec![(
                    "content-security-policy".into(),
                    "sandbox allow-modals".into(),
                )],
                true,
            ),
            (
                vec![(
                    "content-security-policy-report-only".into(),
                    "sandbox".into(),
                )],
                true,
            ),
            (
                vec![
                    (
                        "content-security-policy".into(),
                        "sandbox allow-modals".into(),
                    ),
                    (
                        "content-security-policy".into(),
                        "sandbox allow-scripts".into(),
                    ),
                ],
                false,
            ),
        ] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            context
                .install_document_security(page.page_id(), "https://example.test/".into(), &[])
                .unwrap();
            context
                .handle_message_wire(
                    page.page_id(),
                    oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let initiator = ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: oryn_js::worker_wire::Creator::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                stylesheet: None,
                prepared_policy: None,
            };
            let service = ParentContextService::new_with_network(
                context,
                page.page_id(),
                Some(SharedNetworkBroker::new(Headers(headers))),
            );
            let response = service
                .load_page_resource(
                    page.page_id(),
                    "https://example.test/frame",
                    "frame",
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: None,
                        referrer_policy: None,
                        parent_encoding: None,
                        post: None,
                    },
                )
                .unwrap();
            let ContextServiceResponse::PageResource { modals_allowed, .. } = response else {
                panic!("frame response")
            };
            assert_eq!(modals_allowed, allowed);
        }
    }

    #[test]
    fn redirected_resource_reports_the_reduced_referrer_actually_sent() {
        struct Resources(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Resources {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let first = request.url.ends_with("/start");
                let mut headers = vec![("content-type".into(), "text/css".into())];
                if first {
                    headers.push(("location".into(), "/end".into()));
                    headers.push(("referrer-policy".into(), "unsafe-url".into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if first { 302 } else { 200 },
                    headers,
                    body: b"body{}".to_vec(),
                })
            }
        }
        for destination in ["stylesheet", "frame"] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::new().new_context();
            let page = context.new_page();
            context
                .install_document_security(
                    page.page_id(),
                    "https://example.test/private?q=1".into(),
                    &[],
                )
                .unwrap();
            context
                .handle_message_wire(
                    page.page_id(),
                    oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let initiator = ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: oryn_js::worker_wire::Creator::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                stylesheet: None,
                prepared_policy: None,
            };
            let service = ParentContextService::new_with_network(
                context,
                page.page_id(),
                Some(SharedNetworkBroker::new(Resources(requests.clone()))),
            );
            let result = service
                .load_page_resource(
                    page.page_id(),
                    "https://other.test/start",
                    destination,
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: None,
                        referrer_policy: None,
                        parent_encoding: None,
                        post: None,
                    },
                )
                .unwrap();
            let ContextServiceResponse::PageResource {
                document_referrer,
                request_state,
                ..
            } = result
            else {
                panic!("resource response")
            };
            assert_eq!(document_referrer, "https://example.test/");
            if destination == "frame" {
                let state = request_state.unwrap();
                state.retained_bytes().unwrap();
                assert_eq!(
                    state.initiator_url.as_deref(),
                    Some("https://example.test/private?q=1")
                );
                assert_eq!(state.referrer.as_deref(), Some("https://example.test/"));
                assert_eq!(state.referrer_policy, "unsafe-url");
            } else {
                assert!(request_state.is_none());
            }
            let seen = requests.lock().unwrap();
            assert_eq!(seen.len(), 2);
            for request in seen.iter() {
                assert!(
                    request
                        .headers
                        .contains(&("referer".into(), document_referrer.clone()))
                );
            }
        }
    }

    #[test]
    fn frame_history_request_provenance_survives_current_policy_changes() {
        struct Capture(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Capture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/html".into())],
                    body: b"<body>history".to_vec(),
                })
            }
        }
        for reload in [false, true] {
            for suppressed in [false, true] {
                let requests = Arc::new(Mutex::new(Vec::new()));
                let context = Browser::new().new_context();
                let page = context.new_page();
                context
                    .install_document_security(
                        page.page_id(),
                        "https://current.test/private".into(),
                        &[("referrer-policy".into(), "unsafe-url".into())],
                    )
                    .unwrap();
                context
                    .handle_message_wire(
                        page.page_id(),
                        oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                            document_generation: 1,
                            stream_epoch: 0,
                        },
                    )
                    .unwrap();
                let state = oryn_common::navigation_request::NavigationRequestState {
                    initiator_url: Some("https://original.test/private".into()),
                    opaque_initiator: false,
                    referrer: (!suppressed).then(|| "https://original.test/".into()),
                    referrer_policy: if suppressed { "no-referrer" } else { "origin" }.into(),
                };
                let mut initiator = ResourceInitiator {
                    embedded_type: None,
                    request_state: Some(state.clone()),
                    navigation: oryn_js::platform::NavigationRequestFlags {
                        reload,
                        history: !reload,
                    },
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                    stylesheet: None,
                    prepared_policy: None,
                };
                let service = ParentContextService::new_with_network(
                    context,
                    page.page_id(),
                    Some(SharedNetworkBroker::new(Capture(requests.clone()))),
                );
                let response = service
                    .load_page_resource(
                        page.page_id(),
                        "https://current.test/child",
                        "frame",
                        PageResourceOptions {
                            response_lease: None,
                            download: None,
                            script_authority: None,
                            initiator: &initiator,
                            form_authority: None,
                            initiator_url: None,
                            referrer_policy: Some("unsafe-url"),
                            parent_encoding: None,
                            post: Some(oryn_js::navigation::PostResource {
                                bytes: b"x=1".to_vec(),
                                content_type: "application/x-www-form-urlencoded".into(),
                            }),
                        },
                    )
                    .unwrap();
                let ContextServiceResponse::PageResource {
                    request_state,
                    document_referrer,
                    ..
                } = response
                else {
                    panic!("frame response")
                };
                assert_eq!(request_state, Some(state.clone()));
                assert_eq!(
                    document_referrer,
                    state.referrer.clone().unwrap_or_default()
                );
                {
                    let seen = requests.lock().unwrap();
                    let request = seen.last().unwrap();
                    assert_eq!(request.security.initiator_url, state.initiator_url);
                    assert_eq!(request.method, "POST");
                    assert_eq!(
                        request
                            .headers
                            .iter()
                            .find(|(name, _)| name == "origin")
                            .map(|(_, value)| value.as_str()),
                        Some(if suppressed {
                            "null"
                        } else {
                            "https://original.test"
                        })
                    );
                    assert_eq!(
                        request
                            .headers
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("referer"))
                            .map(|(_, value)| value.clone()),
                        state.referrer
                    );
                    let mut request = request.clone();
                    crate::fetch_metadata::apply(&mut request).unwrap();
                    assert!(
                        request
                            .headers
                            .contains(&("sec-fetch-site".into(), "cross-site".into()))
                    );
                }
                initiator.navigation = Default::default();
                let before = requests.lock().unwrap().len();
                assert!(
                    service
                        .load_page_resource(
                            page.page_id(),
                            "https://current.test/new",
                            "frame",
                            PageResourceOptions {
                                response_lease: None,
                                download: None,
                                script_authority: None,
                                initiator: &initiator,
                                form_authority: None,
                                initiator_url: None,
                                referrer_policy: None,
                                parent_encoding: None,
                                post: None
                            }
                        )
                        .is_err()
                );
                assert_eq!(requests.lock().unwrap().len(), before);
            }
        }
    }

    #[test]
    fn script_redirect_applies_policy_without_restoring_reduced_referrer_data() {
        struct Redirect {
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
            policy: &'static str,
        }
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                let first = request.url.ends_with("/start.js");
                let mut headers = vec![
                    ("content-type".into(), "text/javascript".into()),
                    ("access-control-allow-origin".into(), "*".into()),
                ];
                if first {
                    headers.push(("location".into(), "/end.js".into()));
                    headers.push(("referrer-policy".into(), self.policy.into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if first { 302 } else { 200 },
                    headers,
                    body: b"export{}".to_vec(),
                })
            }
        }
        for (url, policy, initial, first, second) in [
            (
                "/start.js",
                "no-referrer",
                None,
                Some("https://example.test/private?q=1"),
                None,
            ),
            (
                "https://cdn.example/start.js",
                "unsafe-url",
                None,
                Some("https://example.test/"),
                Some("https://example.test/"),
            ),
            ("/start.js", "unsafe-url", Some("no-referrer"), None, None),
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::new().new_context();
            let page = context.new_page();
            // This test helper resolves relative requests from the native page URL.
            context
                .commit_page_url(page.page_id(), "https://example.test/private?q=1".into())
                .unwrap();
            context
                .install_document_security(
                    page.page_id(),
                    "https://example.test/private?q=1".into(),
                    &[],
                )
                .unwrap();
            context
                .handle_message_wire(
                    page.page_id(),
                    oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                page.page_id(),
                Some(SharedNetworkBroker::new(Redirect {
                    requests: requests.clone(),
                    policy,
                })),
            );
            service
                .load_page_script(page.page_id(), url, true, None, None, initial)
                .unwrap();
            let seen = requests.lock().unwrap();
            assert_eq!(seen.len(), 2);
            for (request, expected) in seen.iter().zip([first, second]) {
                let actual = request
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("referer"))
                    .map(|(_, value)| value.as_str());
                assert_eq!(actual, expected, "{url} with response policy {policy}");
            }
        }
    }

    #[test]
    fn script_loading_does_not_follow_non_redirect_3xx_location_headers() {
        struct StatusResponse {
            status: u16,
            requests: Arc<Mutex<usize>>,
        }
        impl NetworkBroker for StatusResponse {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                *self.requests.lock().unwrap() += 1;
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: self.status,
                    headers: vec![
                        ("location".into(), "/other.js".into()),
                        ("content-type".into(), "text/javascript".into()),
                    ],
                    body: Vec::new(),
                })
            }
        }
        for status in [300, 304, 305, 306, 399] {
            let requests = Arc::new(Mutex::new(0));
            let context = Browser::new().new_context();
            let page = context.new_page();
            context
                .commit_page_url(page.page_id(), "https://example.test/".into())
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                page.page_id(),
                Some(SharedNetworkBroker::new(StatusResponse {
                    status,
                    requests: requests.clone(),
                })),
            );
            assert!(
                service
                    .load_page_script(page.page_id(), "/script.js", true, None, None, None)
                    .is_err()
            );
            assert_eq!(
                *requests.lock().unwrap(),
                1,
                "HTTP {status} must not follow Location"
            );
        }
    }

    #[test]
    fn module_redirect_chain_uses_an_opaque_origin_after_cross_origin_hops() {
        struct Redirects(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Redirects {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let (status, location, origin) = if request.url == "https://first.test/start.js" {
                    (
                        302,
                        Some("https://second.test/next.js"),
                        "https://example.test",
                    )
                } else if request.url == "https://second.test/next.js" {
                    (302, Some("https://example.test/final.js"), "null")
                } else {
                    (200, None, "null")
                };
                let mut headers = vec![
                    ("content-type".into(), "text/javascript".into()),
                    ("access-control-allow-origin".into(), origin.into()),
                ];
                if let Some(location) = location {
                    headers.push(("location".into(), location.into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status,
                    headers,
                    body: b"export{}".to_vec(),
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Redirects(requests.clone()))),
        );
        service
            .load_page_script(
                page.page_id(),
                "https://first.test/start.js",
                true,
                None,
                None,
                None,
            )
            .unwrap();
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 3);
        for (request, origin) in seen.iter().zip(["https://example.test", "null", "null"]) {
            assert!(request.headers.contains(&("origin".into(), origin.into())));
            assert!(
                !request.credentials,
                "opaque redirect must not regain same-origin credentials"
            );
        }
    }

    #[test]
    fn script_integrity_checks_raw_bytes_before_decoding_and_cross_origin_eligibility() {
        use base64::Engine;
        use sha2::Digest;
        struct Script;
        impl NetworkBroker for Script {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        (
                            "content-type".into(),
                            "text/javascript; charset=windows-1252".into(),
                        ),
                        ("access-control-allow-origin".into(), "*".into()),
                    ],
                    body: b"//\xe9\n".to_vec(),
                })
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Script)),
        );
        let raw = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(b"//\xe9\n"))
        );
        let decoded = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD
                .encode(sha2::Sha256::digest("//é\n".as_bytes()))
        );
        assert!(
            service
                .load_page_script_with_integrity(IntegrityScriptInputs {
                    page_id: page.page_id(),
                    requested_url: "/script.js",
                    module: false,
                    nonce: None,
                    initiator_url: None,
                    referrer_policy: None,
                    integrity: Some(&raw)
                })
                .is_ok()
        );
        assert!(
            service
                .load_page_script_with_integrity(IntegrityScriptInputs {
                    page_id: page.page_id(),
                    requested_url: "/script.js",
                    module: false,
                    nonce: None,
                    initiator_url: None,
                    referrer_policy: None,
                    integrity: Some(&decoded)
                })
                .is_err()
        );
        // Classic no-CORS is ineligible even when the server sends ACAO; module
        // fetch has actually performed CORS validation and may use SRI.
        assert!(
            service
                .load_page_script_with_integrity(IntegrityScriptInputs {
                    page_id: page.page_id(),
                    requested_url: "https://cdn.test/script.js",
                    module: false,
                    nonce: None,
                    initiator_url: None,
                    referrer_policy: None,
                    integrity: Some(&raw)
                })
                .is_err()
        );
        assert!(
            service
                .load_page_script_with_integrity(IntegrityScriptInputs {
                    page_id: page.page_id(),
                    requested_url: "https://cdn.test/script.js",
                    module: true,
                    nonce: None,
                    initiator_url: None,
                    referrer_policy: None,
                    integrity: Some(&raw)
                })
                .is_ok()
        );
        let data_hash = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(b"export{}"))
        );
        assert!(
            service
                .load_page_script_with_integrity(IntegrityScriptInputs {
                    page_id: page.page_id(),
                    requested_url: "data:text/javascript,export%7B%7D",
                    module: true,
                    nonce: None,
                    initiator_url: None,
                    referrer_policy: None,
                    integrity: Some(&data_hash)
                })
                .is_ok()
        );
    }

    #[test]
    fn classic_redirect_home_reports_opaque_response_taint_to_worker() {
        struct Scripts;
        impl NetworkBroker for Scripts {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let redirect = request.url.starts_with("https://remote.test/");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "https://example.test/final.js".into())]
                    } else {
                        vec![("content-type".into(), "text/javascript".into())]
                    },
                    body: b"throw 42".to_vec(),
                })
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Scripts)),
        );
        assert!(matches!(
            service
                .load_page_script(
                    page.page_id(),
                    "https://remote.test/start.js",
                    false,
                    None,
                    None,
                    None
                )
                .unwrap(),
            ContextServiceResponse::ScriptSource {
                muted_errors: true,
                ..
            }
        ));
        assert!(matches!(
            service
                .load_page_script(
                    page.page_id(),
                    "https://example.test/direct.js",
                    false,
                    None,
                    None,
                    None
                )
                .unwrap(),
            ContextServiceResponse::ScriptSource {
                muted_errors: false,
                ..
            }
        ));
    }

    fn script_test_creator(
        service: &ParentContextService,
        page_id: PageId,
    ) -> oryn_js::worker_wire::Creator {
        service
            .context_owner()
            .unwrap()
            .handle_message_wire(
                page_id,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        }
    }

    #[test]
    fn blob_script_loading_uses_native_store_and_preserves_policy_and_integrity() {
        struct NoNetwork;
        impl NetworkBroker for NoNetwork {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("Blob script must not enter the network broker")
            }
        }
        for blocked in [false, true] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://source.test/".into())
                .unwrap();
            let headers = if blocked {
                vec![("content-security-policy".into(), "script-src 'none'".into())]
            } else {
                vec![]
            };
            context
                .install_document_security(page_id, "https://source.test/".into(), &headers)
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(NoNetwork)),
            );
            let creator = script_test_creator(&service, page_id);
            let context = service.context_owner().unwrap();
            let owner = service.script_document_owner(page_id, &creator).unwrap();
            let url = context
                .register_native_blob(
                    owner.clone(),
                    b"globalThis.loaded = true".to_vec(),
                    String::new(),
                )
                .unwrap();
            let request = |url: String, module, integrity| ContextServiceRequest::LoadScript {
                fallback_encoding: None,
                parser_inserted: false,
                script_policy: None,
                page_id,
                creator: creator.clone(),
                url,
                module,
                nonce: None,
                integrity,
                cross_origin: None,
                initiator_url: None,
                referrer_policy: None,
                referrer_source: None,
            };
            for address in [url.clone(), format!("{url}#part")] {
                let response = service.handle(request(address, false, None)).unwrap();
                assert_eq!(
                    matches!(&response, ContextServiceResponse::ScriptSource { source, muted_errors: false, .. } if source == "globalThis.loaded = true"),
                    !blocked,
                    "{response:?}"
                );
            }
            if blocked {
                continue;
            }
            assert!(matches!(
                service.handle(request(url.clone(), true, None)).unwrap(),
                ContextServiceResponse::ScriptLoadFailed { .. }
            ));
            assert!(matches!(
                service
                    .handle(request(
                        url.clone(),
                        false,
                        Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into())
                    ))
                    .unwrap(),
                ContextServiceResponse::ScriptLoadFailed {
                    failure: crate::script_load_failure::ScriptLoadFailure::Integrity(_)
                }
            ));
            assert!(matches!(
                service
                    .handle(request(format!("{url}?different"), false, None))
                    .unwrap(),
                ContextServiceResponse::ScriptLoadFailed { .. }
            ));
            context.revoke_native_blob(&owner, &url).unwrap();
            assert!(matches!(
                service.handle(request(url, false, None)).unwrap(),
                ContextServiceResponse::ScriptLoadFailed { .. }
            ));
        }
    }

    #[test]
    fn script_wire_authority_ignores_forged_initiator_and_rejects_retired_generation() {
        struct Scripts(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Scripts {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let id = request.request_id;
                self.0.lock().unwrap().push(request);
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/javascript".into()),
                        (
                            "access-control-allow-origin".into(),
                            "https://actual.example".into(),
                        ),
                    ],
                    body: b"export default 1".to_vec(),
                })
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://actual.example/document".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://actual.example/document".into(), &[])
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Scripts(seen.clone()))),
        );
        let creator = script_test_creator(&service, page_id);
        let request = |creator| ContextServiceRequest::LoadScript {
            fallback_encoding: None,
            parser_inserted: false,
            script_policy: None,
            page_id,
            creator,
            url: "https://remote.example/module.js".into(),
            module: true,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: Some("https://forged.example/".into()),
            referrer_policy: None,
            referrer_source: Some("https://actual.example/importer.js".into()),
        };
        assert!(service.handle(request(creator)).is_ok());
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("origin")
                    && value == "https://actual.example")
        );
        drop(requests);
        let stale = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(service.handle(request(stale)).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn parent_stylesheet_mime_checks_empty_network_and_local_responses() {
        struct Sheet;
        impl NetworkBroker for Sheet {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/html".into()),
                        ("access-control-allow-origin".into(), "*".into()),
                    ],
                    body: Vec::new(),
                })
            }
        }
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://source.test/".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://source.test/".into(), &[])
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Sheet)),
        );
        let creator = script_test_creator(&service, page_id);
        for (url, quirks, cross, allowed) in [
            ("https://source.test/sheet", false, None, false),
            ("https://source.test/sheet", true, None, true),
            ("https://other.test/sheet", true, None, false),
            ("https://other.test/sheet", true, Some("anonymous"), true),
            ("https://other.test/sheet", false, Some("anonymous"), false),
            ("data:text/html,", false, None, false),
            ("data:text/html,", true, None, true),
            ("data:text/css,", false, None, true),
        ] {
            let initiator = ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: creator.clone(),
                prepared_policy: None,
                stylesheet: Some(StylesheetRequest {
                    quirks,
                    cross_origin: cross.map(str::to_owned),
                    ..Default::default()
                }),
            };
            let result = service.load_page_resource(
                page_id,
                url,
                "stylesheet",
                PageResourceOptions {
                    response_lease: None,
                    download: None,
                    script_authority: None,
                    initiator: &initiator,
                    form_authority: None,
                    initiator_url: None,
                    referrer_policy: None,
                    parent_encoding: None,
                    post: None,
                },
            );
            assert_eq!(
                result.is_ok(),
                allowed,
                "{url}, quirks={quirks}, crossorigin={cross:?}"
            );
        }
    }

    #[test]
    fn stylesheet_response_proof_tracks_redirect_taint_and_cors_completion() {
        use sha2::Digest;
        struct Responses;
        impl NetworkBroker for Responses {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let location = if request.url.ends_with("start.css") {
                    Some("https://cdn.test/hop.css")
                } else if request.url.ends_with("hop.css") {
                    Some("https://source.test/end.css")
                } else {
                    None
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if location.is_some() { 302 } else { 200 },
                    headers: location
                        .map(|url| {
                            vec![
                                ("location".into(), url.into()),
                                ("access-control-allow-origin".into(), "*".into()),
                            ]
                        })
                        .unwrap_or_else(|| {
                            vec![
                                ("content-type".into(), "text/css".into()),
                                ("access-control-allow-origin".into(), "*".into()),
                            ]
                        }),
                    body: b"p { color: green }".to_vec(),
                })
            }
        }
        for (address, cross, expected) in [
            ("https://source.test/end.css", None, true),
            ("https://source.test/start.css", None, false),
            ("https://source.test/start.css", Some("anonymous"), true),
        ] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://source.test/".into())
                .unwrap();
            context
                .install_document_security(page_id, "https://source.test/".into(), &[])
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(Responses)),
            );
            let creator = script_test_creator(&service, page_id);
            let initiator = ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: creator.clone(),
                prepared_policy: None,
                stylesheet: Some(StylesheetRequest {
                    policy: None,
                    render_blocking: false,
                    quirks: false,
                    nonce: None,
                    integrity: None,
                    cross_origin: cross.map(str::to_owned),
                }),
            };
            let response = service
                .load_page_resource(
                    page_id,
                    address,
                    "stylesheet",
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: None,
                        referrer_policy: None,
                        parent_encoding: None,
                        post: None,
                    },
                )
                .unwrap();
            let ContextServiceResponse::PageResource {
                stylesheet_response: Some(proof),
                url,
                source,
                ..
            } = response
            else {
                panic!("missing response proof");
            };
            assert_eq!(proof.origin_clean, expected);
            let owner = service.script_document_owner(page_id, &creator).unwrap();
            assert_eq!(
                service
                    .script_loads
                    .validate_stylesheet_response(
                        &owner,
                        proof.id,
                        &url,
                        &sha2::Sha256::digest(source.as_bytes()).into()
                    )
                    .unwrap(),
                Some(expected)
            );
            assert!(
                service
                    .script_loads
                    .validate_stylesheet_response(
                        &owner,
                        proof.id,
                        "https://source.test/other.css",
                        &proof.body_sha256
                    )
                    .is_err()
            );
            service
                .script_loads
                .release_stylesheet_response(&owner, proof.id)
                .unwrap();
            assert!(service.script_loads.is_drained().unwrap());
        }
    }
    #[test]
    fn native_stylesheet_policy_denials_remain_web_failures_without_network() {
        struct ForbiddenNetwork;
        impl NetworkBroker for ForbiddenNetwork {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("denied stylesheet must not reach transport")
            }
        }
        for (address, headers) in [
            ("https://source.test/blocked.css",vec![("content-security-policy".into(),"style-src 'none'".into())]),
            ("http://other.test/blocked.css",Vec::new()),
        ] {
            let context=Browser::in_process_probe().new_context();
            let page=context.new_page();
            let page_id=page.page_id();
            context.commit_page_url(page_id,"https://source.test/".into()).unwrap();
            context.install_document_security(page_id,"https://source.test/".into(),&headers).unwrap();
            let service=ParentContextService::new_with_network(context,page_id,Some(SharedNetworkBroker::new(ForbiddenNetwork)));
            let creator=script_test_creator(&service,page_id);
            let initiator=ResourceInitiator {
                embedded_type:None,request_state:None,navigation:Default::default(),
                creator,prepared_policy:None,
                stylesheet:Some(StylesheetRequest {policy:None,render_blocking:false,quirks:false,
                    nonce:None,integrity:None,cross_origin:None}),
            };
            let error=service.load_page_resource(page_id,address,"stylesheet",PageResourceOptions {
                response_lease:None,download:None,script_authority:None,initiator:&initiator,
                form_authority:None,initiator_url:None,referrer_policy:None,parent_encoding:None,post:None,
            }).unwrap_err();
            assert!(matches!(error,RuntimeError::StylesheetFailure(crate::script_load_failure::ScriptLoadFailure::Fetch(_))), "denied stylesheet case {address}");
            assert!(service.script_loads.is_drained().unwrap());
        }
    }
    #[test]
    fn native_stylesheet_absent_broker_is_operational_not_web_failure() {
        let context=Browser::in_process_probe().new_context();
        let page=context.new_page();let page_id=page.page_id();
        context.commit_page_url(page_id,"https://source.test/".into()).unwrap();
        context.install_document_security(page_id,"https://source.test/".into(),&[]).unwrap();
        let service=ParentContextService::new_with_network(context,page_id,None);
        let creator=script_test_creator(&service,page_id);
        let initiator=ResourceInitiator {embedded_type:None,request_state:None,navigation:Default::default(),
            creator,prepared_policy:None,stylesheet:Some(StylesheetRequest {policy:None,render_blocking:false,
                quirks:false,nonce:None,integrity:None,cross_origin:None})};
        let error=service.load_page_resource(page_id,"https://source.test/style.css","stylesheet",PageResourceOptions {
            response_lease:None,download:None,script_authority:None,initiator:&initiator,form_authority:None,
            initiator_url:None,referrer_policy:None,parent_encoding:None,post:None,
        }).unwrap_err();
        assert!(matches!(&error,RuntimeError::ContextService(message) if message=="page resource broker is unavailable"), "unexpected operational origin: {error:?}");
        assert!(service.script_loads.is_drained().unwrap());
    }
    #[test]
    fn local_stylesheet_proof_uses_native_blob_origin_and_cancelled_request_custody() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://source.test/".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://source.test/".into(), &[])
            .unwrap();
        let service = ParentContextService::new_with_network(context, page_id, None);
        let creator = script_test_creator(&service, page_id);
        let owner = service.script_document_owner(page_id, &creator).unwrap();
        let blob = service
            .context_owner()
            .unwrap()
            .register_native_blob(owner.clone(), b"p{}".to_vec(), "text/css".into())
            .unwrap();
        for address in ["data:text/css,p%7B%7D".to_owned(), blob] {
            let initiator = ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: creator.clone(),
                stylesheet: None,
                prepared_policy: None,
            };
            let response = service
                .load_page_resource(
                    page_id,
                    &address,
                    "stylesheet",
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: None,
                        referrer_policy: None,
                        parent_encoding: None,
                        post: None,
                    },
                )
                .unwrap();
            let ContextServiceResponse::PageResource {
                stylesheet_response: Some(proof),
                url,
                ..
            } = response
            else {
                panic!("missing local proof");
            };
            assert!(proof.origin_clean);
            service.script_loads.cancel(proof.id).unwrap();
            assert_eq!(
                service
                    .script_loads
                    .validate_stylesheet_response(&owner, proof.id, &url, &proof.body_sha256)
                    .unwrap(),
                None
            );
            service
                .script_loads
                .release_stylesheet_response(&owner, proof.id)
                .unwrap();
            assert!(service.script_loads.is_drained().unwrap());
        }
    }

    #[test]
    fn stylesheet_nonce_cors_integrity_and_embedder_policy_survive_redirects() {
        struct Sheet(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Sheet {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let redirect = request.url.ends_with("start.css");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "https://cdn.test/end.css".into())]
                    } else {
                        vec![
                            ("content-type".into(), "text/css".into()),
                            ("access-control-allow-origin".into(), "*".into()),
                        ]
                    },
                    body: b"hello".to_vec(),
                })
            }
        }
        let hash = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
        for (nonce, integrity, cross, coep, allowed, requests) in [
            ("good", Some(hash), Some("anonymous"), false, true, 2),
            ("bad", Some(hash), Some("anonymous"), false, false, 0),
            ("good", Some(hash), None, false, false, 2),
            (
                "good",
                Some("sha256-AA=="),
                Some("anonymous"),
                false,
                false,
                2,
            ),
            ("good", None, None, true, false, 2),
            ("good", Some(hash), Some("anonymous"), true, true, 2),
        ] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://source.test/".into())
                .unwrap();
            let mut headers = vec![(
                "content-security-policy".into(),
                "style-src 'nonce-good'".into(),
            )];
            if coep {
                headers.push(("cross-origin-embedder-policy".into(), "require-corp".into()));
            }
            context
                .install_document_security(page_id, "https://source.test/".into(), &headers)
                .unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(Sheet(seen.clone()))),
            );
            let creator = script_test_creator(&service, page_id);
            let initiator = ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator,
                prepared_policy: None,
                stylesheet: Some(StylesheetRequest {
                    policy: None,
                    render_blocking: false,
                    quirks: false,
                    nonce: Some(nonce.into()),
                    integrity: integrity.map(str::to_owned),
                    cross_origin: cross.map(str::to_owned),
                }),
            };
            let result = service.load_page_resource(
                page_id,
                "https://source.test/start.css",
                "stylesheet",
                PageResourceOptions {
                    response_lease: None,
                    download: None,
                    script_authority: None,
                    initiator: &initiator,
                    form_authority: None,
                    initiator_url: None,
                    referrer_policy: None,
                    parent_encoding: None,
                    post: None,
                },
            );
            assert_eq!(
                result.is_ok(),
                allowed,
                "nonce={nonce}, integrity={integrity:?}, cross={cross:?}, coep={coep}"
            );
            assert_eq!(seen.lock().unwrap().len(), requests);
        }
    }
    #[test]
    fn mixed_content_script_redirect_is_rejected_before_second_broker_request() {
        struct Redirect(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url);
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 302,
                    headers: vec![("location".into(), "http://insecure.test/end.js".into())],
                    body: Vec::new(),
                })
            }
        }
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://source.test/".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://source.test/".into(), &[])
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Redirect(seen.clone()))),
        );
        let creator = script_test_creator(&service, page_id);
        let result = service.handle(ContextServiceRequest::LoadScript {
            fallback_encoding: None,
            page_id,
            creator,
            url: "https://source.test/start.js".into(),
            module: false,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: None,
            referrer_policy: None,
            referrer_source: None,
            parser_inserted: true,
            script_policy: None,
        });
        assert!(
            matches!(result,Ok(ContextServiceResponse::ScriptLoadFailed{failure:crate::script_load_failure::ScriptLoadFailure::Fetch(message)}) if message.contains("mixed content"))
        );
        assert_eq!(*seen.lock().unwrap(), vec!["https://source.test/start.js"]);
    }

    #[test]
    fn typed_parent_destination_policy_and_redirect_metadata_match_module_type() {
        struct Redirect(crate::ModuleType);
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                assert_eq!(
                    request.security.fetch_metadata.destination.as_deref(),
                    Some(self.0.fetch_destination())
                );
                let redirect = request.url.ends_with("start");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "/end".into())]
                    } else {
                        vec![(
                            "content-type".into(),
                            if self.0 == crate::ModuleType::Css {
                                "text/css"
                            } else {
                                "application/json"
                            }
                            .into(),
                        )]
                    },
                    body: b"{}".to_vec(),
                })
            }
        }
        for kind in [
            crate::ModuleType::JavaScript,
            crate::ModuleType::Css,
            crate::ModuleType::Json,
            crate::ModuleType::Text,
        ] {
            for allowed in [false, true] {
                let context = Browser::in_process_probe().new_context();
                let page = context.new_page();
                let page_id = page.page_id();
                context
                    .commit_page_url(page_id, "https://source.test/".into())
                    .unwrap();
                let directive = if kind == crate::ModuleType::Css {
                    "style-src"
                } else {
                    "connect-src"
                };
                context
                    .install_document_security(
                        page_id,
                        "https://source.test/".into(),
                        &[(
                            "content-security-policy".into(),
                            format!(
                                "sandbox allow-same-origin; script-src 'none'; {directive} {}",
                                if allowed { "'self'" } else { "'none'" }
                            ),
                        )],
                    )
                    .unwrap();
                let service = ParentContextService::new_with_network(
                    context,
                    page_id,
                    Some(SharedNetworkBroker::new(Redirect(kind))),
                );
                let creator = script_test_creator(&service, page_id);
                let result = service
                    .handle(ContextServiceRequest::LoadTypedModule {
                        page_id,
                        creator,
                        url: "https://source.test/start".into(),
                        kind,
                        nonce: None,
                        integrity: None,
                        cross_origin: None,
                        initiator_url: None,
                        referrer_policy: None,
                        referrer_source: None,
                        parser_inserted: true,
                        script_policy: None,
                    })
                    .unwrap();
                assert_eq!(
                    matches!(result, ContextServiceResponse::ScriptSource { .. }),
                    allowed && kind != crate::ModuleType::JavaScript
                );
            }
        }
    }

    #[test]
    fn typed_module_parent_contract_enforces_mime_for_data_resources() {
        struct DataOnly;
        impl NetworkBroker for DataOnly {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("data modules must not access the network")
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/page".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/page".into(), &[])
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(DataOnly)),
        );
        let request = |kind| ContextServiceRequest::LoadTypedModule {
            nonce: None,
            parser_inserted: false,
            script_policy: None,
            page_id: page.page_id(),
            creator: script_test_creator(&service, page.page_id()),
            url: "data:application/json,%7B%22value%22%3A42%7D".into(),
            kind,
            integrity: None,
            cross_origin: None,
            initiator_url: None,
            referrer_policy: Some("no-referrer".into()),
            referrer_source: None,
        };
        assert!(
            matches!(service.handle(request(crate::ModuleType::Json)).unwrap(),
            ContextServiceResponse::ScriptSource { source, referrer_policy, .. }
                if source == "{\"value\":42}" && referrer_policy == "no-referrer")
        );
        for kind in [crate::ModuleType::Css, crate::ModuleType::JavaScript] {
            assert!(matches!(
                service.handle(request(kind)).unwrap(),
                ContextServiceResponse::ScriptLoadFailed {
                    failure: crate::script_load_failure::ScriptLoadFailure::Fetch(_),
                }
            ));
        }
    }

    #[test]
    fn script_response_carries_final_referrer_policy_and_data_inherits_options() {
        struct Script;
        impl NetworkBroker for Script {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/javascript".into()),
                        ("referrer-policy".into(), "unsafe-url,origin".into()),
                        ("Referrer-Policy".into(), "invalid,no-referrer".into()),
                    ],
                    body: b"export{}".to_vec(),
                })
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/page".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Script)),
        );
        assert!(
            matches!(service.load_page_script(page.page_id(), "/script.js", true,
            None, None, Some("unsafe-url")).unwrap(),
            ContextServiceResponse::ScriptSource { referrer_policy, .. } if referrer_policy == "no-referrer")
        );
        assert!(
            matches!(service.load_page_script(page.page_id(), "data:text/javascript,export%7B%7D", true,
            None, None, Some("origin")).unwrap(),
            ContextServiceResponse::ScriptSource { referrer_policy, .. } if referrer_policy == "origin")
        );
    }

    #[test]
    fn classic_script_integrity_obeys_crossorigin_credentials_and_response_headers() {
        use base64::Engine;
        use sha2::Digest;
        struct Scripts(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Scripts {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let mut headers = vec![
                    ("content-type".into(), "text/javascript".into()),
                    (
                        "access-control-allow-origin".into(),
                        if request.url.ends_with("wildcard.js") {
                            "*"
                        } else {
                            "https://example.test"
                        }
                        .into(),
                    ),
                ];
                if !request.url.ends_with("missing.js") {
                    headers.push(("access-control-allow-credentials".into(), "true".into()));
                }
                if request.url.ends_with("duplicate.js") {
                    headers.push(("Access-Control-Allow-Credentials".into(), "true".into()));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers,
                    body: b"void 0;".to_vec(),
                })
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Scripts(seen.clone()))),
        );
        let hash = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(b"void 0;"))
        );
        for mode in ["", "anonymous", "invalid-value"] {
            service
                .load_page_script_with_options(ScriptOptionsInputs {
                    page_id: page.page_id(),
                    requested_url: "https://cdn.test/wildcard.js",
                    module: false,
                    nonce: None,
                    initiator_url: None,
                    referrer_policy: None,
                    integrity: Some(&hash),
                    cross_origin: Some(mode),
                })
                .unwrap();
            let requests = seen.lock().unwrap();
            let request = requests.last().unwrap();
            assert!(!request.credentials);
            assert_eq!(
                request.security.credentials,
                crate::cookies::CookieCredentials::SameOrigin
            );
        }
        service
            .load_page_script_with_options(ScriptOptionsInputs {
                page_id: page.page_id(),
                requested_url: "https://cdn.test/ok.js",
                module: false,
                nonce: None,
                initiator_url: None,
                referrer_policy: None,
                integrity: Some(&hash),
                cross_origin: Some("USE-CREDENTIALS"),
            })
            .unwrap();
        {
            let requests = seen.lock().unwrap();
            let request = requests.last().unwrap();
            assert!(request.credentials);
            assert_eq!(
                request.security.credentials,
                crate::cookies::CookieCredentials::Include
            );
        }
        for path in ["wildcard.js", "missing.js", "duplicate.js"] {
            assert!(
                service
                    .load_page_script_with_options(ScriptOptionsInputs {
                        page_id: page.page_id(),
                        requested_url: &format!("https://cdn.test/{path}"),
                        module: false,
                        nonce: None,
                        initiator_url: None,
                        referrer_policy: None,
                        integrity: Some(&hash),
                        cross_origin: Some("use-credentials")
                    })
                    .is_err()
            );
        }
    }

    #[test]
    fn module_fetch_defaults_to_same_origin_credentials_and_checks_redirect_cors() {
        struct Scripts(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Scripts {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let mut headers = vec![("content-type".into(), "text/javascript".into())];
                let status = if request.url.ends_with("redirect.js") {
                    headers.push(("location".into(), "/ok.js".into()));
                    302
                } else {
                    headers.push(("access-control-allow-origin".into(), "*".into()));
                    if request.url.ends_with("duplicate.js") {
                        headers.push((
                            "Access-Control-Allow-Origin".into(),
                            "https://example.test".into(),
                        ));
                    }
                    200
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status,
                    headers,
                    body: b"export{}".to_vec(),
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(Scripts(requests.clone()))),
        );
        service
            .load_page_script(
                page.page_id(),
                "https://cdn.example/ok.js",
                true,
                None,
                None,
                None,
            )
            .unwrap();
        {
            let seen = requests.lock().unwrap();
            assert!(!seen[0].credentials);
            assert_eq!(
                seen[0].security.credentials,
                crate::cookies::CookieCredentials::SameOrigin
            );
            assert!(
                seen[0]
                    .headers
                    .contains(&("origin".into(), "https://example.test".into()))
            );
        }
        assert!(
            service
                .load_page_script(
                    page.page_id(),
                    "https://cdn.example/duplicate.js",
                    true,
                    None,
                    None,
                    None
                )
                .is_err()
        );
        let count = requests.lock().unwrap().len();
        assert!(
            service
                .load_page_script(
                    page.page_id(),
                    "https://cdn.example/redirect.js",
                    true,
                    None,
                    None,
                    None
                )
                .is_err()
        );
        assert_eq!(requests.lock().unwrap().len(), count + 1);
        service
            .load_page_script(page.page_id(), "/ok.js", true, None, None, None)
            .unwrap();
        assert!(requests.lock().unwrap().last().unwrap().credentials);
    }

    #[test]
    fn page_script_loader_accepts_bundles_above_old_cap_and_rejects_over_budget() {
        struct ScriptNetwork;
        impl NetworkBroker for ScriptNetwork {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let size = if request.url.ends_with("large.js") {
                    MAX_PAGE_SCRIPT_BYTES + 1
                } else {
                    2 * 1024 * 1024
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: vec![b' '; size],
                })
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(ScriptNetwork)),
        );
        let load = |path| service.load_page_script(page.page_id(), path, false, None, None, None);
        assert!(
            matches!(load("/bundle.js").unwrap(), ContextServiceResponse::ScriptSource { source, .. } if source.len() == 2 * 1024 * 1024)
        );
        assert!(load("/large.js").is_err());
    }

    #[test]
    fn worker_entry_requires_success_and_javascript_mime_for_both_script_types() {
        for module in [false, true] {
            for status in [101, 204, 200, 299, 301, 404, 500] {
                for mime in [
                    None,
                    Some("text/plain"),
                    Some("application/json"),
                    Some("text/javascript"),
                ] {
                    let response = NetworkResponse {
                        response_policy: None,
                        timing: None,
                        request_id: 1,
                        status,
                        headers: mime
                            .into_iter()
                            .map(|value| ("content-type".into(), value.into()))
                            .collect(),
                        body: b"self.ready=true".to_vec(),
                    };
                    assert_eq!(
                        decode_worker_script(&response, module).is_ok(),
                        (200..=299).contains(&status) && mime == Some("text/javascript"),
                        "{status} {mime:?} {module}"
                    );
                }
            }
        }
    }

    #[test]
    fn captured_script_encoding_survives_parent_transport_redirect_and_lease() {
        struct Scripts;
        impl NetworkBroker for Scripts {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                if request.url.ends_with("start.js") {
                    return Ok(NetworkResponse {
                        response_policy: None,
                        timing: None,
                        request_id: request.request_id,
                        status: 302,
                        headers: vec![("location".into(), "/final.js".into())],
                        body: Vec::new(),
                    });
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![(
                        "content-type".into(),
                        if request.url.ends_with("header.js") {
                            "text/javascript;charset=windows-1252"
                        } else {
                            "text/javascript"
                        }
                        .into(),
                    )],
                    body: vec![0xca],
                })
            }
        }
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://example.test/".into(), &[])
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Scripts)),
        );
        let creator = script_test_creator(&service, page_id);
        let (worker, parent) = UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let server = std::thread::spawn(move || serve_context_transport(parent, Arc::new(service)));
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        for (url, module, leased, expected) in [
            ("https://example.test/start.js", false, false, "К"),
            ("https://example.test/start.js", false, true, "К"),
            ("https://example.test/header.js", false, false, "Ê"),
            ("https://example.test/final.js", true, false, "�"),
            ("data:text/javascript,%CA", false, true, "К"),
        ] {
            let mut request = ContextServiceRequest::LoadScript {
                fallback_encoding: Some("windows-1251".into()),
                parser_inserted: true,
                script_policy: None,
                page_id,
                creator: creator.clone(),
                url: url.into(),
                module,
                nonce: None,
                integrity: None,
                cross_origin: None,
                initiator_url: None,
                referrer_policy: None,
                referrer_source: None,
            };
            if leased {
                let ContextServiceResponse::ScriptLoadReserved { token } = client
                    .request(ContextServiceRequest::ReserveScriptLoad {
                        page_id,
                        creator: creator.clone(),
                    })
                    .unwrap()
                else {
                    panic!("script lease")
                };
                request = ContextServiceRequest::RunScriptLoad {
                    render_blocking: false,
                    page_id,
                    token,
                    request: Box::new(request),
                };
            }
            let ContextServiceResponse::ScriptSource { source, .. } =
                client.request(request).unwrap()
            else {
                panic!("script source")
            };
            assert_eq!(source, expected, "{url} module={module} leased={leased}");
        }
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
    }

    #[test]
    fn script_failure_provenance_survives_parent_wire_without_classifying_admission_as_fetch() {
        use crate::script_load_failure::ScriptLoadFailure;
        struct Failures;
        impl NetworkBroker for Failures {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                if request.url.ends_with("cancelled.js") {
                    return Err(NetworkError::Cancelled);
                }
                if request.url.ends_with("capacity.js") {
                    return Err(NetworkError::ResponseTooLarge { limit: 17 });
                }
                if request.url.ends_with("unavailable.js") {
                    return Err(NetworkError::Unavailable);
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 404,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: Vec::new(),
                })
            }
        }
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://example.test/".into(), &[])
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Failures)),
        );
        let creator = script_test_creator(&service, page_id);
        let (worker, parent) = UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let server = std::thread::spawn(move || serve_context_transport(parent, Arc::new(service)));
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        let request = |path: &str| ContextServiceRequest::LoadScript {
            fallback_encoding: None,
            parser_inserted: false,
            script_policy: None,
            page_id,
            creator: creator.clone(),
            url: format!("https://example.test/{path}"),
            module: true,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: None,
            referrer_policy: None,
            referrer_source: None,
        };
        for (path, class) in [("missing.js", 0), ("cancelled.js", 1), ("capacity.js", 2)] {
            let response = client.request(request(path)).unwrap();
            assert!(matches!(
                (class, response),
                (
                    0,
                    ContextServiceResponse::ScriptLoadFailed {
                        failure: ScriptLoadFailure::Fetch(_)
                    }
                ) | (
                    1,
                    ContextServiceResponse::ScriptLoadFailed {
                        failure: ScriptLoadFailure::Cancelled
                    }
                ) | (
                    2,
                    ContextServiceResponse::ScriptLoadFailed {
                        failure: ScriptLoadFailure::Capacity(_)
                    }
                )
            ));
        }
        assert!(client.request(request("unavailable.js")).is_err());
        let mut stale = request("missing.js");
        if let ContextServiceRequest::LoadScript { creator, .. } = &mut stale {
            *creator = oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 999,
                stream_epoch: 0,
            };
        }
        assert!(client.request(stale).is_err());
        shutdown.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = server.join().unwrap();
    }
    #[test]
    fn worker_module_source_uses_utf8_instead_of_legacy_transport_encoding() {
        let source = "self.name='é'";
        let response = NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: 1,
            status: 200,
            headers: vec![(
                "content-type".into(),
                "text/javascript;charset=windows-1252".into(),
            )],
            body: source.as_bytes().to_vec(),
        };
        assert_eq!(decode_worker_script(&response, true).unwrap(), source);
        assert_eq!(decode_worker_script(&response, false).unwrap(), source);
    }
    #[test]
    fn module_script_rejects_missing_invalid_and_non_javascript_mime_types() {
        for header in [
            None,
            Some(""),
            Some("invalid"),
            Some("text/plain"),
            Some("application/json"),
            Some("application/example+javascript"),
            Some("text/javascript1.6"),
        ] {
            assert!(
                decode_page_script(b"export{}", header, true).is_err(),
                "{header:?}"
            );
        }
        for header in [
            "text/javascript",
            "APPLICATION/JAVASCRIPT;charset=windows-1252",
            "text/javascript1.5",
            "application/ecmascript",
        ] {
            assert_eq!(
                decode_page_script(b"export{}", Some(header), true).unwrap(),
                "export{}"
            );
        }
    }

    #[test]
    fn parent_script_decoder_honors_captured_fallback_without_changing_modules() {
        assert_eq!(
            decode_page_module(
                b"\xca",
                Some("text/javascript"),
                false,
                None,
                Some("windows-1251")
            )
            .unwrap(),
            "К"
        );
        assert_eq!(
            decode_page_module(
                b"\xca",
                Some("text/javascript;charset=windows-1252"),
                false,
                None,
                Some("windows-1251")
            )
            .unwrap(),
            "Ê"
        );
        assert_eq!(
            decode_page_module(
                b"\xef\xbb\xbf\xd0\x9a",
                Some("text/javascript;charset=windows-1252"),
                false,
                None,
                Some("windows-1251")
            )
            .unwrap(),
            "К"
        );
        assert_eq!(
            decode_page_module(
                b"\xca",
                Some("text/javascript"),
                true,
                None,
                Some("windows-1251")
            )
            .unwrap(),
            "�"
        );
    }

    #[test]
    fn module_script_decoding_ignores_legacy_transport_charset() {
        let source = "export const name='é';";
        let header = Some("text/javascript;charset=windows-1252");
        assert_eq!(
            decode_page_script(source.as_bytes(), header, true).unwrap(),
            source
        );
        assert_eq!(
            decode_page_script(source.as_bytes(), header, false).unwrap(),
            "export const name='Ã©';"
        );
        assert_eq!(
            decode_page_script(b"\xef\xbb\xbfexport{}", header, true).unwrap(),
            "export{}"
        );
        assert_eq!(
            decode_page_script(b"\xff\xfeA\0", header, true).unwrap(),
            "��A\0"
        );
    }

    #[test]
    fn page_script_budget_bounds_bytes_and_decoded_source() {
        assert_eq!(
            decode_page_script(&vec![b' '; MAX_PAGE_SCRIPT_BYTES], None, false)
                .unwrap()
                .len(),
            MAX_PAGE_SCRIPT_BYTES
        );
        assert!(decode_page_script(&vec![b' '; MAX_PAGE_SCRIPT_BYTES + 1], None, false).is_err());
        assert!(
            decode_page_script(
                &vec![0x80; MAX_PAGE_SCRIPT_BYTES / 3 + 1],
                Some("text/javascript; charset=windows-1252"),
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn context_json_writes_coalesce_receipts_and_preserve_large_frames() {
        #[derive(Default)]
        struct Writes {
            bytes: Vec<u8>,
            calls: usize,
        }
        impl Write for Writes {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let receipt = ResponseReceipt {
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: Some(WorkerRoute::issue(7, 8, 9).unwrap()),
            received_id: 42,
            accepted: true,
        };
        let mut writer = Writes::default();
        write_context_json_line(&mut writer, &receipt).unwrap();
        assert_eq!(
            writer.calls, 1,
            "one receipt must not issue a write per JSON token"
        );
        let mut expected = serde_json::to_vec(&receipt).unwrap();
        expected.push(b'\n');
        assert_eq!(writer.bytes, expected);
        let large = vec!["escaped\n\0\"".repeat(1024); 3];
        write_context_json_line(&mut writer, &large).unwrap();
        expected.extend(serde_json::to_vec(&large).unwrap());
        expected.push(b'\n');
        assert_eq!(writer.bytes, expected);
    }

    #[test]
    fn context_json_write_failure_does_not_retry_on_drop() {
        struct Failing(usize);
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                self.0 += 1;
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Failing(0);
        assert!(write_context_json_line(&mut writer, &"receipt").is_err());
        assert_eq!(writer.0, 1);
    }

    #[test]
    fn response_frame_budget_includes_json_expansion_and_delimiter() {
        let route = WorkerRoute::issue(7, 8, 9).unwrap();
        let make = |value: String| ResponseEnvelope {
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: Some(route.clone()),
            correlation_id: 42,
            response: Some(ContextServiceResponse::DocumentCookie { value }),
            error: None,
        };
        let response = make("ordinary".into());
        let exact = serde_json::to_vec(&response).unwrap().len() + 1;
        assert_eq!(
            encode_context_response(response, exact).unwrap().len(),
            exact
        );
        let encoded = encode_context_response(make("\0".repeat(512)), 1024).unwrap();
        assert!(encoded.len() <= 1024);
        assert_eq!(encoded.last(), Some(&b'\n'));
        let decoded: ResponseEnvelope = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.route, Some(route.clone()));
        assert_eq!(decoded.correlation_id, 42);
        assert!(decoded.response.is_none());
        assert!(decoded.error.unwrap().contains("frame budget"));
        assert!(encode_context_response(make("large".repeat(512)), 1).is_err());
    }

    #[test]
    fn oversized_response_does_not_corrupt_following_response() {
        struct OversizedOnce(AtomicBool);
        impl ContextTransport for OversizedOnce {
            fn request(
                &self,
                _: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, RuntimeError> {
                Ok(ContextServiceResponse::ReferrerPolicy {
                    value: if self.0.swap(false, Ordering::SeqCst) {
                        "\0".repeat(MAX_CONTEXT_FRAME_BYTES / 6)
                    } else {
                        "ok".into()
                    },
                })
            }
        }
        let (worker, parent) = UnixStream::pair().unwrap();
        worker
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            serve_context_transport(parent, Arc::new(OversizedOnce(AtomicBool::new(true))))
        });
        let mut client = IpcContextClient::new(worker);
        let request = || ContextServiceRequest::GetReferrerPolicy {
            page_id: PageId(1),
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
        };
        assert!(
            matches!(client.request(request()), Err(RuntimeError::ContextService(error)) if error.contains("frame budget"))
        );
        assert!(
            matches!(client.request(request()).unwrap(), ContextServiceResponse::ReferrerPolicy { value } if value == "ok")
        );
        drop(client);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn parent_dispatch_limit_releases_capacity_on_every_drop() {
        let limit = ContextDispatchLimit::default();
        let mut permits = (0..MAX_PARENT_CONTEXT_OPERATIONS)
            .map(|_| limit.admit().unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            limit.admit(),
            Err(RuntimeError::WorkerLimitExceeded { .. })
        ));
        permits.pop();
        let replacement = limit.admit().unwrap();
        assert!(limit.admit().is_err());
        drop(permits);
        drop(replacement);
        assert_eq!(limit.0.load(Ordering::Acquire), 0);
    }

    #[test]
    fn raw_context_peer_cannot_bypass_parent_dispatch_limit() {
        struct BlockedService(Arc<(Mutex<bool>, std::sync::Condvar)>);
        impl ContextTransport for BlockedService {
            fn request(
                &self,
                _: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, RuntimeError> {
                let (lock, changed) = &*self.0;
                let guard = lock.lock().unwrap();
                drop(changed.wait_while(guard, |released| !*released).unwrap());
                Err(RuntimeError::ContextService("released test service".into()))
            }
        }
        struct Release(Arc<(Mutex<bool>, std::sync::Condvar)>);
        impl Drop for Release {
            fn drop(&mut self) {
                *self.0.0.lock().unwrap() = true;
                self.0.1.notify_all();
            }
        }
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let _release = Release(gate.clone());
        let (mut worker, parent) = UnixStream::pair().unwrap();
        worker
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (finished, result) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let _ = finished.send(serve_context_transport(
                parent,
                Arc::new(BlockedService(gate)),
            ));
        });
        for message_id in 1..=MAX_PARENT_CONTEXT_OPERATIONS as u64 + 1 {
            serde_json::to_writer(
                &mut worker,
                &RequestEnvelope {
                    diagnostic: None,
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: None,
                    message_id,
                    request: ContextServiceRequest::GetReferrerPolicy {
                        page_id: PageId(1),
                        creator: oryn_js::worker_wire::Creator::Window {
                            target: oryn_js::message_wire::WindowTarget::Root,
                            document_generation: 1,
                            stream_epoch: 0,
                        },
                    },
                },
            )
            .unwrap();
            worker.write_all(b"\n").unwrap();
        }
        assert!(matches!(
            result.recv_timeout(Duration::from_secs(5)).unwrap(),
            Err(RuntimeError::WorkerLimitExceeded { .. })
        ));
        server.join().unwrap();
    }

    #[test]
    fn context_message_exhaustion_rejects_before_writing() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent.set_nonblocking(true).unwrap();
        let mut client = IpcContextClient::new(worker);
        client.next_message_id = u64::MAX;
        for _ in 0..2 {
            assert!(matches!(
                client.request(ContextServiceRequest::GetReferrerPolicy {
                    page_id: PageId(1),
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                }),
                Err(RuntimeError::ProtocolViolation(_))
            ));
            assert_eq!(client.next_message_id, u64::MAX);
        }
        let mut byte = [0];
        assert_eq!(
            parent.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn context_reply_correlation_does_not_replace_route_authentication() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(parent.try_clone().unwrap());
            let mut line = Vec::new();
            reader.read_until(b'\n', &mut line).unwrap();
            let request: RequestEnvelope = serde_json::from_slice(&line).unwrap();
            let route = request.route.unwrap();
            let forged =
                WorkerRoute::issue(route.process_session, route.environment, route.generation)
                    .unwrap();
            serde_json::to_writer(
                &mut parent,
                &ResponseEnvelope {
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: Some(forged),
                    correlation_id: request.message_id,
                    response: Some(ContextServiceResponse::ReferrerPolicy {
                        value: "unsafe-url".into(),
                    }),
                    error: None,
                },
            )
            .unwrap();
            parent.write_all(b"\n").unwrap();
        });
        let client = SharedContextClient::new(IpcContextClient::new(worker))
            .for_route(WorkerRoute::issue(1, 1, 0).unwrap());
        assert!(
            client
                .request(ContextServiceRequest::GetReferrerPolicy {
                    page_id: PageId(1),
                    creator: oryn_js::worker_wire::Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }
                })
                .is_err()
        );
        server.join().unwrap();
    }

    #[test]
    fn indexeddb_request_envelope_preserves_numeric_keys_and_values() {
        use crate::storage::{IdbCommand, IdbWrite, StorageRequest};

        for number in [9007199254740991.0_f64, 9007199254740992.0, 0.1, -0.1] {
            let envelope = RequestEnvelope {
                diagnostic: None,
                protocol_version: 1,
                route: None,
                message_id: 7,
                request: ContextServiceRequest::Storage {
                    creator: None,
                    request: StorageRequest::IndexedDb {
                        origin: "https://example.test".into(),
                        command: IdbCommand::Batch {
                            database: "precision".into(),
                            writes: vec![IdbWrite {
                                store: "records".into(),
                                key: Some(serde_json::json!(number)),
                                value: serde_json::json!({"id": number}),
                                add_only: true,
                            }],
                        },
                    },
                },
            };
            let bytes = serde_json::to_vec(&envelope).unwrap();
            let decoded: RequestEnvelope = serde_json::from_slice(&bytes).unwrap();
            let ContextServiceRequest::Storage {
                creator: _,
                request:
                    StorageRequest::IndexedDb {
                        command: IdbCommand::Batch { writes, .. },
                        ..
                    },
            } = decoded.request
            else {
                panic!("unexpected request after transport");
            };
            assert_eq!(
                writes[0].key.as_ref().unwrap().as_f64().unwrap().to_bits(),
                number.to_bits()
            );
            assert_eq!(
                writes[0].value["id"].as_f64().unwrap().to_bits(),
                number.to_bits()
            );
        }
    }

    struct FixtureNetwork(Option<&'static str>);

    impl NetworkBroker for FixtureNetwork {
        fn send(
            &self,
            request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, NetworkError> {
            assert!(
                request
                    .headers
                    .contains(&("Accept".into(), "text/plain".into()))
            );
            // A same-origin GET does not carry Origin. Cookie and Proxy-*
            // remain parent-owned, while Fetch permits author Authorization.
            assert_eq!(request.method, "GET");
            assert!(
                !request
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("origin"))
            );
            for forbidden in ["cookie", "proxy-authorization"] {
                assert!(
                    !request
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case(forbidden)),
                    "{forbidden}"
                );
            }
            if let Some(expected) = self.0 {
                let authorization: Vec<_> = request
                    .headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .map(|(_, value)| value.as_str())
                    .collect();
                assert_eq!(authorization, vec![expected]);
            } else {
                assert!(
                    !request
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                );
            }
            Ok(crate::network::NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("Content-Type".into(), "application/octet-stream".into()),
                    ("Set-Cookie".into(), "secret=yes".into()),
                ],
                body: vec![7; 150_000],
            })
        }
    }

    struct CountingReader {
        reads: Arc<AtomicU64>,
        body: std::io::Cursor<Vec<u8>>,
    }

    impl Read for CountingReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.body.read(output)
        }
    }

    struct PullNetwork {
        reads: Arc<AtomicU64>,
    }

    impl NetworkBroker for PullNetwork {
        fn send(
            &self,
            _request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, NetworkError> {
            unreachable!("streaming path must not buffer through send")
        }

        fn start_stream(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkStreamResponse, NetworkError> {
            Ok(NetworkStreamResponse {
                response_policy: None,
                encoded_bytes: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/plain".into()),
                    ("access-control-allow-origin".into(), "*".into()),
                ],
                content_length: Some(5),
                upload: None,
                body: Box::new(CountingReader {
                    reads: self.reads.clone(),
                    body: std::io::Cursor::new(b"hello".to_vec()),
                }),
            })
        }
    }

    struct ConcurrentStartNetwork {
        active: Arc<AtomicU64>,
        peak: Arc<AtomicU64>,
    }

    impl NetworkBroker for ConcurrentStartNetwork {
        fn send(
            &self,
            request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, NetworkError> {
            let requested_headers = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("access-control-request-headers"))
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            Ok(crate::network::NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 204,
                headers: vec![
                    ("access-control-allow-origin".into(), "*".into()),
                    ("access-control-allow-methods".into(), "GET".into()),
                    ("access-control-allow-headers".into(), requested_headers),
                ],
                body: Vec::new(),
            })
        }

        fn start_stream(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkStreamResponse, NetworkError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(25));
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(NetworkStreamResponse {
                response_policy: None,
                encoded_bytes: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/plain".into()),
                    ("access-control-allow-origin".into(), "*".into()),
                ],
                content_length: Some(2),
                upload: None,
                body: Box::new(std::io::Cursor::new(b"ok".to_vec())),
            })
        }
    }

    struct CancellationNetwork {
        cancelled: Arc<AtomicU64>,
    }

    impl NetworkBroker for CancellationNetwork {
        fn send(
            &self,
            _request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, NetworkError> {
            unreachable!("streaming path must not buffer through send")
        }

        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(2);
            sender
                .blocking_send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("access-control-allow-origin".into(), "*".into())],
                    content_length: None,
                    upload: None,
                })
                .unwrap();
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }

        fn cancel_operation(&self, _request_id: u64) -> Result<(), NetworkError> {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct StalledCancellationNetwork {
        cancelled: Arc<AtomicU64>,
    }

    impl NetworkBroker for StalledCancellationNetwork {
        fn send(
            &self,
            _request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, NetworkError> {
            unreachable!("scheduled resources use the operation path")
        }

        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(2);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(1));
                drop(sender);
            });
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }

        fn cancel_operation(&self, _request_id: u64) -> Result<(), NetworkError> {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn multiplexed_context_channel_starts_thirty_two_requests_concurrently() {
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let active = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU64::new(0));
        let (worker, parent) = UnixStream::pair().unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(ConcurrentStartNetwork {
                active: active.clone(),
                peak: peak.clone(),
            })),
        );
        let server = std::thread::spawn(move || serve_context_service(parent, service));
        let client = SharedContextClient::new(IpcContextClient::new(worker));
        let callers = (0..32)
            .map(|index| {
                let client = client.clone();
                let page_id = page.page_id();
                std::thread::spawn(move || {
                    client.request(ContextServiceRequest::HttpStreamStart {
                        page_id,
                        request: HttpStreamRequest {
                            upgrade_mixed_image: false,
                            fetch: None,
                            client: None,
                            use_cors_preflight: false,
                            request_id: index + 1,
                            method: "GET".into(),
                            url: format!("https://example.test/{index}"),
                            headers: Vec::new(),
                            body_base64: String::new(),
                            credentials: false,
                            credentials_mode: None,
                            initiator_url: None,
                            referrer_policy: None,
                        },
                    })
                })
            })
            .collect::<Vec<_>>();
        for caller in callers {
            assert!(matches!(
                caller.join().unwrap().unwrap(),
                ContextServiceResponse::HttpStreamStarted { .. }
            ));
        }
        assert_eq!(peak.load(Ordering::SeqCst), 32);
        drop(client);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn worker_service_teardown_cancels_every_active_stream() {
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let cancelled = Arc::new(AtomicU64::new(0));
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(CancellationNetwork {
                cancelled: cancelled.clone(),
            })),
        );
        for request_id in 1..=3 {
            assert!(matches!(
                service
                    .handle(ContextServiceRequest::HttpStreamStart {
                        page_id: page.page_id(),
                        request: HttpStreamRequest {
                            upgrade_mixed_image: false,
                            fetch: None,
                            client: None,
                            use_cors_preflight: false,
                            request_id,
                            method: "GET".into(),
                            url: format!("https://example.test/{request_id}"),
                            headers: Vec::new(),
                            body_base64: String::new(),
                            credentials: false,
                            credentials_mode: None,
                            initiator_url: None,
                            referrer_policy: None,
                        },
                    })
                    .unwrap(),
                ContextServiceResponse::HttpStreamStarted { .. }
            ));
        }
        let cleanup = service.http_cleanup.clone();
        drop(service);
        cleanup.drain(Duration::from_secs(2)).unwrap();
        assert_eq!(cancelled.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn script_cleanup_uses_retained_scope_after_context_or_service_admission_retires() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        let service = ParentContextService::new(context.clone(), page_id);
        let token = service.script_loads.reserve().unwrap();
        let lease = service.script_loads.claim(token).unwrap();
        service.shutting_down.store(true, Ordering::Release);
        drop(page);
        drop(context);
        let request = ContextServiceRequest::CancelScriptLoad { page_id, token };
        assert!(
            service
                .response_scope(None, Some(&request))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            service.handle(request).unwrap(),
            ContextServiceResponse::ScriptLoadCancellationPending
        ));
        assert!(lease.cancel.load(Ordering::Acquire));
        drop(lease);
        assert!(matches!(
            service
                .handle(ContextServiceRequest::CancelScriptLoad { page_id, token })
                .unwrap(),
            ContextServiceResponse::ScriptLoadCancelled
        ));
        assert!(
            service
                .handle(ContextServiceRequest::CancelScriptLoad {
                    page_id: PageId(page_id.0 + 1),
                    token
                })
                .is_err()
        );
        assert!(
            service
                .handle(ContextServiceRequest::GetDocumentCookie {
                    page_id,
                    creator: None
                })
                .is_err()
        );
    }

    #[test]
    fn owned_script_cancellation_reaches_network_without_retiring_sibling_service() {
        struct PendingScript {
            started: std::sync::mpsc::Sender<()>,
            retained: Mutex<Vec<tokio::sync::mpsc::Sender<crate::network::NetworkEvent>>>,
            cancelled: Arc<AtomicU64>,
        }
        impl NetworkBroker for PendingScript {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!()
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                let (sender, events) = tokio::sync::mpsc::channel(2);
                self.retained.lock().unwrap().push(sender);
                self.started.send(()).unwrap();
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: request.request_id,
                    events,
                    demand: None,
                })
            }
            fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        context
            .install_document_security(page_id, "https://example.test/".into(), &[])
            .unwrap();
        let (started, receiver) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicU64::new(0));
        let service = ParentContextService::new_with_network(
            context.clone(),
            page_id,
            Some(SharedNetworkBroker::new(PendingScript {
                started,
                retained: Mutex::new(Vec::new()),
                cancelled: cancelled.clone(),
            })),
        )
        .into_shared();
        let ContextServiceResponse::ScriptLoadReserved { token } = service
            .handle(ContextServiceRequest::ReserveScriptLoad {
                page_id,
                creator: script_test_creator(&service, page_id),
            })
            .unwrap()
        else {
            panic!("reservation response")
        };
        let script = || ContextServiceRequest::LoadScript {
            fallback_encoding: None,
            parser_inserted: false,
            script_policy: None,
            page_id,
            creator: script_test_creator(&service, page_id),
            url: "/slow.js".into(),
            module: false,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: None,
            referrer_policy: None,
            referrer_source: None,
        };
        let request = script();
        let worker = service.clone();
        let running = std::thread::spawn(move || {
            worker.handle(ContextServiceRequest::RunScriptLoad {
                render_blocking: false,
                page_id,
                token,
                request: Box::new(request),
            })
        });
        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            service
                .handle(ContextServiceRequest::CancelScriptLoad {
                    page_id: PageId(page_id.0 + 1),
                    token
                })
                .is_err()
        );
        service
            .handle(ContextServiceRequest::CancelScriptLoad { page_id, token })
            .unwrap();
        assert!(matches!(
            running.join().unwrap().unwrap(),
            ContextServiceResponse::ScriptLoadFailed {
                failure: crate::script_load_failure::ScriptLoadFailure::Cancelled,
            }
        ));
        assert_eq!(cancelled.load(Ordering::SeqCst), 1);
        assert!(
            service
                .handle(ContextServiceRequest::RunScriptLoad {
                    render_blocking: false,
                    page_id,
                    token,
                    request: Box::new(script())
                })
                .is_err()
        );
        assert!(!service.shutting_down.load(Ordering::Acquire));
        let ContextServiceResponse::ScriptLoadReserved { token: next } = service
            .handle(ContextServiceRequest::ReserveScriptLoad {
                page_id,
                creator: script_test_creator(&service, page_id),
            })
            .unwrap()
        else {
            panic!("reservation response")
        };
        service
            .handle(ContextServiceRequest::CancelScriptLoad {
                page_id,
                token: next,
            })
            .unwrap();
        assert!(
            service
                .handle(ContextServiceRequest::RunScriptLoad {
                    render_blocking: false,
                    page_id,
                    token: next,
                    request: Box::new(script())
                })
                .is_err()
        );
        assert!(
            receiver.try_recv().is_err(),
            "cancelled admission must not start another request"
        );
    }

    #[test]
    fn owned_style_font_and_frame_cancellation_reaches_pending_transport() {
        for destination in ["stylesheet", "font", "frame"] {
            for phase in ["headers", "redirect", "body"] {
                if destination == "frame" && phase == "body" {
                    continue;
                } // Published frame bodies use the retained stream cancellation tests.
                struct Pending {
                    phase: &'static str,
                    count: AtomicU64,
                    started: std::sync::mpsc::Sender<()>,
                    retained: Mutex<Vec<tokio::sync::mpsc::Sender<NetworkEvent>>>,
                    cancelled: Arc<AtomicU64>,
                }
                impl NetworkBroker for Pending {
                    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                        unreachable!()
                    }
                    fn start_operation(
                        &self,
                        request: NetworkRequest,
                    ) -> Result<NetworkOperation, NetworkError> {
                        let (sender, events) = tokio::sync::mpsc::channel(2);
                        let first = self.count.fetch_add(1, Ordering::SeqCst) == 0;
                        if self.phase == "redirect" && first {
                            sender
                                .try_send(NetworkEvent::Headers {
                                    response_policy: None,
                                    timing: None,
                                    request_id: request.request_id,
                                    status: 302,
                                    headers: vec![("location".into(), "/after".into())],
                                    content_length: Some(0),
                                    upload: None,
                                })
                                .unwrap();
                            sender
                                .try_send(NetworkEvent::Complete {
                                    encoded_body_size: None,
                                    request_id: request.request_id,
                                })
                                .unwrap();
                        } else {
                            if self.phase == "body" {
                                sender
                                    .try_send(NetworkEvent::Headers {
                                        response_policy: None,
                                        timing: None,
                                        request_id: request.request_id,
                                        status: 200,
                                        headers: Vec::new(),
                                        content_length: Some(100),
                                        upload: None,
                                    })
                                    .unwrap();
                            }
                            self.started.send(()).unwrap();
                        }
                        self.retained.lock().unwrap().push(sender);
                        Ok(NetworkOperation {
                            body_demand_pending: Default::default(),
                            prefetched: Default::default(),
                            request_id: request.request_id,
                            events,
                            demand: None,
                        })
                    }
                    fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
                        self.cancelled.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                }
                let context = Browser::in_process_probe().new_context();
                let page = context.new_page();
                let page_id = page.page_id();
                context
                    .commit_page_url(page_id, "https://example.test/".into())
                    .unwrap();
                context
                    .install_document_security(page_id, "https://example.test/".into(), &[])
                    .unwrap();
                let (started, receiver) = std::sync::mpsc::channel();
                let cancelled = Arc::new(AtomicU64::new(0));
                let service = ParentContextService::new_with_network(
                    context,
                    page_id,
                    Some(SharedNetworkBroker::new(Pending {
                        phase,
                        count: AtomicU64::new(0),
                        started,
                        retained: Mutex::new(Vec::new()),
                        cancelled: cancelled.clone(),
                    })),
                )
                .into_shared();
                let creator = script_test_creator(&service, page_id);
                let ContextServiceResponse::ScriptLoadReserved { token } = service
                    .handle(ContextServiceRequest::ReserveScriptLoad {
                        page_id,
                        creator: creator.clone(),
                    })
                    .unwrap()
                else {
                    panic!()
                };
                let request = ContextServiceRequest::LoadPageResource {
                    page_id,
                    url: "https://example.test/held".into(),
                    destination: destination.into(),
                    initiator: Box::new(ResourceInitiator {
                        embedded_type: None,
                        request_state: None,
                        navigation: Default::default(),
                        creator,
                        stylesheet: None,
                        prepared_policy: None,
                    }),
                    download: None,
                    script_authority: None,
                    form_authority: None,
                    initiator_url: None,
                    referrer_policy: None,
                    parent_encoding: None,
                    post: None,
                };
                let worker = service.clone();
                let running = std::thread::spawn(move || {
                    worker.handle(ContextServiceRequest::RunScriptLoad {
                        render_blocking: false,
                        page_id,
                        token,
                        request: Box::new(request),
                    })
                });
                receiver.recv_timeout(Duration::from_secs(1)).unwrap();
                service
                    .handle(ContextServiceRequest::CancelScriptLoad { page_id, token })
                    .unwrap();
                assert!(running.join().unwrap().is_err());
                assert!(!service.shutting_down.load(Ordering::Acquire));
                let deadline = std::time::Instant::now() + Duration::from_secs(1);
                while cancelled.load(Ordering::SeqCst) == 0 {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
                assert!(service.script_loads.is_drained().unwrap());
            }
        }
    }
    #[test]
    fn worker_replacement_cancels_scheduled_resource_waits() {
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/".into(), &[])
            .unwrap();
        let cancelled = Arc::new(AtomicU64::new(0));
        let service = ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(StalledCancellationNetwork {
                cancelled: cancelled.clone(),
            })),
        )
        .into_shared();
        let request_service = service.clone();
        let request = std::thread::spawn(move || {
            request_service.handle(ContextServiceRequest::LoadScript {
                fallback_encoding: None,
                parser_inserted: false,
                script_policy: None,
                page_id: page.page_id(),
                creator: script_test_creator(&request_service, page.page_id()),
                url: "/slow.js".into(),
                module: false,
                nonce: None,
                integrity: None,
                cross_origin: None,
                initiator_url: None,
                referrer_policy: None,
                referrer_source: None,
            })
        });
        std::thread::sleep(Duration::from_millis(10));
        service.cancel_all_network_operations();
        assert!(matches!(
            request.join().unwrap().unwrap(),
            ContextServiceResponse::ScriptLoadFailed {
                failure: crate::script_load_failure::ScriptLoadFailure::Cancelled,
            }
        ));
        assert_eq!(cancelled.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn accepted_headers_do_not_pull_response_bytes_until_worker_demand() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://example.test/app", "<p>ready</p>")
            .await
            .unwrap();
        let page_id = page.page_id();
        let reads = Arc::new(AtomicU64::new(0));
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(PullNetwork {
                reads: reads.clone(),
            })),
        );
        let started = service
            .handle(ContextServiceRequest::HttpStreamStart {
                page_id,
                request: HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: None,
                    client: None,
                    use_cors_preflight: false,
                    request_id: 8,
                    method: "GET".into(),
                    url: "https://example.test/pull".into(),
                    headers: Vec::new(),
                    body_base64: String::new(),
                    credentials: true,
                    credentials_mode: None,
                    initiator_url: None,
                    referrer_policy: None,
                },
            })
            .expect("accept response headers");
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = started else {
            panic!("unexpected stream response");
        };
        let chunk = service
            .handle(ContextServiceRequest::HttpStreamRead {
                creator: None,
                page_id,
                stream_id,
                max_bytes: 2,
            })
            .expect("pull response bytes");
        assert!(matches!(
            chunk,
            ContextServiceResponse::HttpStreamChunk { done: false, .. }
        ));
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn typed_http_streams_are_owned_bounded_and_header_filtered() {
        for authorization in [None, Some("secret")] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            page.load_html("https://example.test/app", "<p>ready</p>")
                .await
                .unwrap();
            let page_id = page.page_id();
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(FixtureNetwork(authorization))),
            );
            let started = service
                .handle(ContextServiceRequest::HttpStreamStart {
                    page_id,
                    request: HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: None,
                        client: None,
                        use_cors_preflight: false,
                        request_id: 7,
                        method: "GET".into(),
                        url: "https://example.test/stream".into(),
                        headers: {
                            let mut headers = vec![
                                ("Accept".into(), "text/plain".into()),
                                ("Cookie".into(), "session=forged".into()),
                                ("Proxy-Authorization".into(), "forged-proxy".into()),
                                ("Origin".into(), "https://forged.test".into()),
                            ];
                            if let Some(value) = authorization {
                                headers.push(("Authorization".into(), value.into()));
                            }
                            headers
                        },
                        body_base64: String::new(),
                        credentials: true,
                        credentials_mode: None,
                        initiator_url: None,
                        referrer_policy: None,
                    },
                })
                .expect("start stream");
            let ContextServiceResponse::HttpStreamStarted {
                stream_id,
                headers,
                total_bytes,
                ..
            } = started
            else {
                panic!("unexpected stream response");
            };
            assert_eq!(total_bytes, Some(150_000));
            assert!(matches!(
                service.handle(ContextServiceRequest::HttpStreamAbort {
                    creator: None,
                    page_id: PageId(92),
                    stream_id
                }),
                Err(RuntimeError::ContextService(_))
            ));
            assert_eq!(
                headers,
                vec![("Content-Type".into(), "application/octet-stream".into())]
            );

            let mut bytes = 0;
            let mut chunks = 0;
            loop {
                let response = service
                    .handle(ContextServiceRequest::HttpStreamRead {
                        creator: None,
                        page_id,
                        stream_id,
                        max_bytes: usize::MAX,
                    })
                    .expect("read stream");
                let ContextServiceResponse::HttpStreamChunk {
                    body_base64, done, ..
                } = response
                else {
                    panic!("unexpected chunk response");
                };
                let chunk =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                        .expect("valid chunk");
                assert!(chunk.len() <= 64 * 1024);
                bytes += chunk.len();
                chunks += 1;
                if done {
                    break;
                }
            }
            assert_eq!(bytes, 150_000);
            assert_eq!(chunks, 3);
            for _ in 0..2 {
                assert!(matches!(
                    service.handle(ContextServiceRequest::HttpStreamAbort {
                        creator: None,
                        page_id,
                        stream_id
                    }),
                    Ok(ContextServiceResponse::HttpStreamAborted)
                ));
            }
            assert!(matches!(
                service.handle(ContextServiceRequest::HttpStreamRead {
                    creator: None,
                    page_id: PageId(92),
                    stream_id,
                    max_bytes: 1,
                }),
                Err(RuntimeError::ContextService(_))
            ));
        }
    }

    #[tokio::test]
    async fn worker_originated_cross_origin_navigation_is_parent_policy_checked() {
        let mut options = ContextOptions::default();
        options
            .action_policy
            .rules
            .insert(ConsequentialAction::CrossOriginNavigation, PolicyRule::Deny);
        let context = Browser::in_process_probe().new_context_with_options(options);
        let page = context.new_page();
        page.load_html("https://app.example/", "<p>ready</p>")
            .await
            .unwrap();
        let service = ParentContextService::new(context.clone(), page.page_id());

        let response = service
            .handle(ContextServiceRequest::NavigatePage {
                page_id: page.page_id(),
                url: "https://other.example/".into(),
            })
            .unwrap();
        assert!(matches!(
            response,
            ContextServiceResponse::PageDenied { .. }
        ));
        assert_eq!(context.security_audit().len(), 1);
        assert_eq!(
            context.security_audit()[0].decision,
            crate::PolicyDecision::Denied
        );
    }

    #[tokio::test]
    async fn preconfirmed_navigation_chain_rechecks_every_redirect_origin() {
        let mut options = ContextOptions::default();
        options.action_policy.rules.insert(
            ConsequentialAction::CrossOriginNavigation,
            PolicyRule::RequireConfirmation,
        );
        options
            .action_policy
            .denied_origins
            .insert("https://blocked.example".into());
        let context = Browser::in_process_probe().new_context_with_options(options);
        let page = context.new_page();
        page.load_html("https://app.example/", "<p>ready</p>")
            .await
            .unwrap();
        let _grant = context.grant_navigation_chain(page.page_id());
        let service = ParentContextService::new(context.clone(), page.page_id());

        assert!(matches!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page.page_id(),
                    source: NavigationPolicySource::Document(
                        context
                            .capture_command_navigation_source(page.page_id())
                            .unwrap()
                    ),
                    url: "https://allowed.example/redirect-1".into(),
                })
                .unwrap(),
            ContextServiceResponse::NavigationAuthorized
        ));
        assert!(matches!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page.page_id(),
                    source: NavigationPolicySource::Document(
                        context
                            .capture_command_navigation_source(page.page_id())
                            .unwrap()
                    ),
                    url: "https://blocked.example/redirect-2".into(),
                })
                .unwrap(),
            ContextServiceResponse::PageDenied { .. }
        ));
        let audit = context.security_audit();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].decision, crate::PolicyDecision::Allowed);
        assert_eq!(audit[1].decision, crate::PolicyDecision::Denied);
        assert!(!audit[0].confirmation_consumed);
        assert!(!audit[1].confirmation_consumed);
    }

    #[tokio::test]
    async fn worker_originated_form_effect_is_parent_policy_checked() {
        let mut options = ContextOptions::default();
        options
            .action_policy
            .rules
            .insert(ConsequentialAction::FormSubmission, PolicyRule::Deny);
        let context = Browser::in_process_probe().new_context_with_options(options);
        let page = context.new_page();
        page.load_html("https://app.example/", "<form action=/charged></form>")
            .await
            .unwrap();
        let service = ParentContextService::new(context.clone(), page.page_id());

        let response = service
            .handle(ContextServiceRequest::AuthorizeFormSubmission {
                report_source: None,
                authority: None,
                page_id: page.page_id(),
                url: "https://app.example/charged".into(),
                method: "post".into(),
            })
            .unwrap();
        assert!(matches!(
            response,
            ContextServiceResponse::PageDenied { .. }
        ));
        assert_eq!(context.security_audit().len(), 1);
        assert_eq!(
            context.security_audit()[0].request.action,
            ConsequentialAction::FormSubmission
        );
        assert_eq!(
            context.security_audit()[0].decision,
            crate::PolicyDecision::Denied
        );
    }

    #[test]
    fn data_response_streams_preserve_bytes_ownership_cancellation_and_csp() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context.clone(),
            page_id,
            Some(SharedNetworkBroker::new(FixtureNetwork(None))),
        );
        let request = HttpStreamRequest {
            upgrade_mixed_image: false,
            fetch: None,
            client: None,
            use_cors_preflight: false,
            request_id: 500,
            method: "GET".into(),
            url: "data:text/plain,%00%80%FF".into(),
            headers: vec![],
            body_base64: String::new(),
            credentials: false,
            credentials_mode: Some("omit".into()),
            initiator_url: None,
            referrer_policy: None,
        };
        let start = || {
            service.handle(ContextServiceRequest::HttpStreamStart {
                page_id,
                request: request.clone(),
            })
        };
        let ContextServiceResponse::HttpStreamStarted {
            stream_id,
            status,
            total_bytes,
            ..
        } = start().unwrap()
        else {
            panic!("data response missing")
        };
        assert_eq!((status, total_bytes), (200, Some(3)));
        assert!(
            service
                .handle(ContextServiceRequest::HttpStreamRead {
                    creator: None,
                    page_id: PageId(999),
                    stream_id,
                    max_bytes: 3
                })
                .is_err()
        );
        let ContextServiceResponse::HttpStreamChunk {
            body_base64, done, ..
        } = service
            .handle(ContextServiceRequest::HttpStreamRead {
                creator: None,
                page_id,
                stream_id,
                max_bytes: 3,
            })
            .unwrap()
        else {
            panic!("data chunk missing")
        };
        assert_eq!(body_base64, "AID/");
        // Filling max_bytes consumes the body chunk, not its separately owned
        // completion event. Explicitly read EOF before expecting reclamation.
        assert!(!done);
        let ContextServiceResponse::HttpStreamChunk {
            body_base64, done, ..
        } = service
            .handle(ContextServiceRequest::HttpStreamRead {
                creator: None,
                page_id,
                stream_id,
                max_bytes: 3,
            })
            .unwrap()
        else {
            panic!("data completion missing")
        };
        assert!(body_base64.is_empty());
        assert!(done);
        assert!(
            service
                .handle(ContextServiceRequest::HttpStreamRead {
                    creator: None,
                    page_id,
                    stream_id,
                    max_bytes: 3
                })
                .is_err()
        );
        service
            .handle(ContextServiceRequest::HttpRequestAbort {
                page_id,
                request_id: 500,
            })
            .unwrap();
        assert!(start().is_err());
        context
            .install_document_security(
                page_id,
                "https://example.test/".into(),
                &[(
                    "content-security-policy".into(),
                    "connect-src 'none'".into(),
                )],
            )
            .unwrap();
        assert!(
            matches!(start(),Err(RuntimeError::ContextService(message)) if message.contains("Content-Security-Policy"))
        );
    }

    #[test]
    fn media_blob_admission_uses_native_owner_csp_and_retains_open_bytes_after_revoke() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        struct NoSocket;
        impl NetworkBroker for NoSocket {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                panic!("local media must not reach socket broker")
            }
        }
        for permitted in [false, true] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .install_document_security(
                    page_id,
                    "https://media.test/".into(),
                    &[(
                        "content-security-policy".into(),
                        if permitted {
                            "media-src blob:"
                        } else {
                            "media-src 'none'"
                        }
                        .into(),
                    )],
                )
                .unwrap();
            context
                .handle_message_wire(
                    page_id,
                    MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let (owner, _, _) = context
                .window_worker_creator(page_id, &WindowTarget::Root, 1, 0)
                .unwrap();
            let url = context
                .register_native_blob(owner.clone(), vec![0, 1, 2, 3], "audio/wav".into())
                .unwrap();
            let creator = Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let service = ParentContextService::new_with_network(
                context.clone(),
                page_id,
                Some(SharedNetworkBroker::new(NoSocket)),
            );
            let request = |id| ContextServiceRequest::MediaStreamStart {
                page_id,
                cors_enabled: false,
                destination: "audio".into(),
                request: HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: Some(oryn_js::platform::FetchOptions {
                        keepalive: false,
                        privileged_range: true,
                        cache: Default::default(),
                        mode: "no-cors".into(),
                        redirect: "follow".into(),
                        integrity: String::new(),
                    }),
                    client: Some(creator.clone()),
                    use_cors_preflight: false,
                    request_id: id,
                    method: "GET".into(),
                    url: format!("{url}#ignored"),
                    headers: vec![("range".into(), "bytes=1-2".into())],
                    body_base64: String::new(),
                    credentials: true,
                    credentials_mode: Some("include".into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
            };
            let response = service.handle(request(101));
            if !permitted {
                assert!(
                    matches!(response,Err(RuntimeError::ContextService(message))if message.contains("Content-Security-Policy"))
                );
                continue;
            }
            let ContextServiceResponse::HttpStreamStarted {
                stream_id,
                status,
                headers,
                ..
            } = response.unwrap()
            else {
                panic!()
            };
            assert_eq!(status, 206);
            assert!(headers.contains(&("content-range".into(), "bytes 1-2/4".into())));
            context.revoke_native_blob(&owner, &url).unwrap();
            assert!(service.handle(request(102)).is_err());
            let mut body = Vec::new();
            loop {
                let ContextServiceResponse::HttpStreamChunk {
                    body_base64, done, ..
                } = service
                    .handle(ContextServiceRequest::HttpStreamRead {
                        page_id,
                        creator: Some(creator.clone()),
                        stream_id,
                        max_bytes: 64,
                    })
                    .unwrap()
                else {
                    panic!()
                };
                body.extend(
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                        .unwrap(),
                );
                if done {
                    break;
                }
            }
            assert_eq!(body, vec![1, 2]);
        }
    }

    #[test]
    fn text_track_destination_enforces_media_policy_cors_and_active_mixed_content() {
        struct Track(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Track {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/vtt".into()),
                        ("access-control-allow-origin".into(), "*".into()),
                    ],
                    body: b"WEBVTT\n\n".to_vec(),
                })
            }
        }
        for (mode, url, policy, allowed) in [
            (
                "same-origin",
                "https://example.test/cues",
                "media-src *",
                true,
            ),
            (
                "same-origin",
                "https://other.test/cues",
                "media-src *",
                false,
            ),
            ("cors", "https://other.test/cues", "media-src *", true),
            ("cors", "http://other.test/cues", "media-src *", false),
            ("cors", "https://other.test/cues", "media-src 'none'", false),
        ] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let id = page.page_id();
            context
                .commit_page_url(id, "https://example.test/".into())
                .unwrap();
            context
                .install_document_security(
                    id,
                    "https://example.test/".into(),
                    &[("content-security-policy".into(), policy.into())],
                )
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                id,
                Some(SharedNetworkBroker::new(Track(seen.clone()))),
            );
            let result = service.handle(ContextServiceRequest::MediaStreamStart {
                page_id: id,
                cors_enabled: true,
                destination: "track".into(),
                request: HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: Some(oryn_js::platform::FetchOptions {
                        keepalive: false,
                        privileged_range: false,
                        cache: Default::default(),
                        mode: mode.into(),
                        redirect: "follow".into(),
                        integrity: String::new(),
                    }),
                    client: None,
                    use_cors_preflight: false,
                    request_id: 991,
                    method: "GET".into(),
                    url: url.into(),
                    headers: Vec::new(),
                    body_base64: String::new(),
                    credentials: false,
                    credentials_mode: Some("omit".into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
            });
            if allowed {
                let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = result.unwrap()
                else {
                    panic!("text track stream")
                };
                service.abort_http_stream(id, stream_id).unwrap();
                let requests = seen.lock().unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(
                    requests[0].security.kind,
                    crate::cookies::CookieRequestKind::Media
                );
                assert!(
                    requests[0]
                        .headers
                        .iter()
                        .any(|(name, value)| name.eq_ignore_ascii_case("sec-fetch-dest")
                            && value == "track")
                );
            } else {
                assert!(result.is_err());
                assert!(seen.lock().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn media_stream_uses_media_policy_cookie_destination_and_private_opaque_metadata() {
        struct Media(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Media {
            fn send(&self, r: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(r.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: r.request_id,
                    status: 206,
                    headers: vec![("content-range".into(), "bytes 0-2/100".into())],
                    body: b"wav".to_vec(),
                })
            }
        }
        for permitted in [true, false] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let id = page.page_id();
            context
                .commit_page_url(id, "https://example.test/".into())
                .unwrap();
            context
                .install_document_security(
                    id,
                    "https://example.test/".into(),
                    &[(
                        "content-security-policy".into(),
                        if permitted {
                            "default-src 'none'; media-src https://media.test"
                        } else {
                            "default-src *; media-src 'none'"
                        }
                        .into(),
                    )],
                )
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                id,
                Some(SharedNetworkBroker::new(Media(seen.clone()))),
            );
            let result = service.handle(ContextServiceRequest::MediaStreamStart {
                page_id: id,
                cors_enabled: false,
                destination: "audio".into(),
                request: HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: Some(oryn_js::platform::FetchOptions {
                        keepalive: false,
                        privileged_range: true,
                        cache: Default::default(),
                        mode: "no-cors".into(),
                        redirect: "follow".into(),
                        integrity: String::new(),
                    }),
                    client: None,
                    use_cors_preflight: false,
                    request_id: 91,
                    method: "GET".into(),
                    url: "https://media.test/a".into(),
                    headers: vec![("range".into(), "bytes=0-".into())],
                    body_base64: String::new(),
                    credentials: true,
                    credentials_mode: Some("include".into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
            });
            if permitted {
                let ContextServiceResponse::HttpStreamStarted {
                    status,
                    internal_response,
                    ..
                } = result.unwrap()
                else {
                    panic!("media response")
                };
                assert_eq!(status, 0);
                let hidden = internal_response.unwrap();
                assert_eq!(hidden.status, 206);
                assert!(hidden.range_requested);
                let requests = seen.lock().unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(
                    requests[0].security.kind,
                    crate::cookies::CookieRequestKind::Media
                );
            } else {
                assert!(result.is_err());
                assert!(seen.lock().unwrap().is_empty());
            }
        }
    }
    #[test]
    fn media_mixed_content_upgrade_is_destination_owned_and_keeps_custom_port() {
        struct Media(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Media {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("access-control-allow-origin".into(), "*".into())],
                    body: Vec::new(),
                })
            }
        }
        for cors in [false, true] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let id = page.page_id();
            context
                .commit_page_url(id, "https://page.test/".into())
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                id,
                Some(SharedNetworkBroker::new(Media(seen.clone()))),
            );
            let request = HttpStreamRequest {
                upgrade_mixed_image: false,
                fetch: Some(oryn_js::platform::FetchOptions {
                    keepalive: false,
                    privileged_range: true,
                    cache: Default::default(),
                    mode: if cors { "cors" } else { "no-cors" }.into(),
                    redirect: "follow".into(),
                    integrity: String::new(),
                }),
                client: None,
                use_cors_preflight: false,
                request_id: 94,
                method: "GET".into(),
                url: "http://media.test:8443/sound.wav".into(),
                headers: vec![("range".into(), "bytes=0-".into())],
                body_base64: String::new(),
                credentials: false,
                credentials_mode: Some("omit".into()),
                initiator_url: None,
                referrer_policy: None,
            };
            let result = service
                .handle(ContextServiceRequest::MediaStreamStart {
                    page_id: id,
                    request,
                    cors_enabled: cors,
                    destination: "audio".into(),
                })
                .unwrap();
            let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = result else {
                panic!()
            };
            service.abort_http_stream(id, stream_id).unwrap();
            assert_eq!(
                *seen.lock().unwrap(),
                vec!["https://media.test:8443/sound.wav"]
            );
        }
    }

    #[test]
    fn image_cors_mode_controls_cross_origin_response_authorization() {
        struct ImageResponse {
            headers: Vec<(String, String)>,
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
        }
        impl NetworkBroker for ImageResponse {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: self.headers.clone(),
                    body: b"image bytes".to_vec(),
                })
            }
        }
        for (cors_enabled, credentials, permission, allowed) in [
            (false, "include", 0, true),
            (true, "same-origin", 0, false),
            (true, "same-origin", 1, true),
            (true, "include", 1, false),
            (true, "include", 2, true),
        ] {
            let headers = match permission {
                1 => vec![("access-control-allow-origin".into(), "*".into())],
                2 => vec![
                    (
                        "access-control-allow-origin".into(),
                        "https://example.test".into(),
                    ),
                    ("access-control-allow-credentials".into(), "true".into()),
                ],
                _ => Vec::new(),
            };
            let requests = Arc::new(Mutex::new(Vec::new()));
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let page_id = page.page_id();
            context
                .commit_page_url(page_id, "https://example.test/page".into())
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                page_id,
                Some(SharedNetworkBroker::new(ImageResponse {
                    headers,
                    requests: requests.clone(),
                })),
            );
            let result = service.handle(ContextServiceRequest::ImageStreamStart {
                page_id,
                cors_enabled,
                request: HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: None,
                    client: None,
                    use_cors_preflight: false,
                    request_id: 77,
                    method: "GET".into(),
                    url: "https://images.test/image.png".into(),
                    headers: Vec::new(),
                    body_base64: String::new(),
                    credentials: true,
                    credentials_mode: Some(credentials.into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
            });
            assert_eq!(
                result.is_ok(),
                allowed,
                "cors={cors_enabled}, credentials={credentials}, permission={permission}"
            );
            let seen = requests.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].credentials, credentials == "include");
            assert_eq!(
                seen[0]
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("origin")),
                cors_enabled
            );
            drop(seen);
            if let Ok(ContextServiceResponse::HttpStreamStarted {
                stream_id,
                cors_filtered,
                ..
            }) = result
            {
                assert_eq!(
                    cors_filtered, !cors_enabled,
                    "opaque image origin must reach the renderer"
                );
                service
                    .handle(ContextServiceRequest::HttpStreamAbort {
                        creator: None,
                        page_id,
                        stream_id,
                    })
                    .unwrap();
            }
        }
    }

    #[test]
    fn image_stream_uses_image_csp_and_rejects_non_get_payloads() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://images.test/".into())
            .unwrap();
        context
            .install_document_security(
                page.page_id(),
                "https://images.test/".into(),
                &[(
                    "content-security-policy".into(),
                    "img-src 'none'; connect-src *".into(),
                )],
            )
            .unwrap();
        let service = ParentContextService::new(context, page.page_id());
        let request = HttpStreamRequest {
            upgrade_mixed_image: false,
            fetch: None,
            client: None,
            use_cors_preflight: false,
            request_id: 1,
            method: "GET".into(),
            url: "https://images.test/image.svg".into(),
            headers: vec![],
            body_base64: String::new(),
            credentials: false,
            credentials_mode: Some("omit".into()),
            initiator_url: None,
            referrer_policy: None,
        };
        let denied = service.handle(ContextServiceRequest::ImageStreamStart {
            cors_enabled: false,
            page_id: page.page_id(),
            request: request.clone(),
        });
        assert!(
            matches!(denied, Err(RuntimeError::ContextService(message)) if message.contains("Content-Security-Policy Image"))
        );
        let mut request = request;
        request.method = "POST".into();
        assert!(matches!(
            service.handle(ContextServiceRequest::ImageStreamStart {
                cors_enabled: false,
                page_id: page.page_id(),
                request
            }),
            Err(RuntimeError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn cors_permissions_handle_duplicates_wildcards_and_case() {
        let origin = "https://example.test";
        let mut response = NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: 1,
            status: 204,
            headers: vec![
                ("access-control-allow-origin".into(), origin.into()),
                ("access-control-allow-credentials".into(), "true".into()),
                ("access-control-allow-methods".into(), "*".into()),
                ("access-control-allow-headers".into(), "*".into()),
            ],
            body: vec![],
        };
        assert!(
            enforce_cors_preflight(origin, false, "PATCH", &["x-test".into()], &response).is_ok()
        );
        assert!(
            enforce_cors_preflight(origin, true, "PATCH", &["x-test".into()], &response).is_err()
        );
        assert!(
            enforce_cors_preflight(origin, false, "PATCH", &["authorization".into()], &response)
                .is_err()
        );
        response.headers.push((
            "Access-Control-Allow-Headers".into(),
            "Authorization, X-Test".into(),
        ));
        response
            .headers
            .push(("Access-Control-Allow-Methods".into(), "PATCH".into()));
        assert!(
            enforce_cors_preflight(
                origin,
                true,
                "PATCH",
                &["authorization".into(), "x-test".into()],
                &response
            )
            .is_ok()
        );
        assert!(enforce_cors_preflight(origin, true, "patch", &[], &response).is_err());
        // A safelisted method does not need an explicit allow-methods entry.
        assert!(
            enforce_cors_preflight(origin, true, "POST", &["x-test".into()], &response).is_ok()
        );
        response.headers[1].1 = "True".into();
        assert!(!cors_origin_allowed(origin, true, &response));
        response.headers[1].1 = "true".into();
        response
            .headers
            .push(("Access-Control-Allow-Origin".into(), origin.into()));
        assert!(!cors_origin_allowed(origin, false, &response));
        response.headers.pop();
        response
            .headers
            .push(("Access-Control-Allow-Credentials".into(), "true".into()));
        assert!(!cors_origin_allowed(origin, true, &response));
    }

    #[test]
    fn streamed_fetch_redirect_origin_stays_opaque_when_returning_home() {
        struct Chain(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Chain {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let mut seen = self.0.lock().unwrap();
                seen.push(request.clone());
                let index = seen.len();
                let mut headers = vec![(
                    "access-control-allow-origin".into(),
                    if index == 1 {
                        "https://example.test"
                    } else {
                        "null"
                    }
                    .into(),
                )];
                if index <= 2 {
                    headers.push((
                        "location".into(),
                        if index == 1 {
                            "https://second.test/middle"
                        } else {
                            "https://example.test/end"
                        }
                        .into(),
                    ));
                }
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if index <= 2 { 302 } else { 200 },
                    headers,
                    body: vec![],
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let page_id = page.page_id();
        context
            .commit_page_url(page_id, "https://example.test/".into())
            .unwrap();
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(Chain(requests.clone()))),
        );
        let result = service
            .start_http_stream(
                page_id,
                HttpStreamRequest {
                    upgrade_mixed_image: false,
                    fetch: None,
                    client: None,
                    use_cors_preflight: false,
                    request_id: 901,
                    method: "GET".into(),
                    url: "https://first.test/start".into(),
                    headers: vec![],
                    body_base64: String::new(),
                    credentials: true,
                    credentials_mode: Some("same-origin".into()),
                    initiator_url: None,
                    referrer_policy: None,
                },
                crate::csp::CspDestination::Connect,
            )
            .unwrap();
        let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = result else {
            panic!("stream")
        };
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 3);
        for (index, request) in seen.iter().enumerate() {
            assert!(!request.credentials);
            assert!(
                request.headers.contains(&(
                    "origin".into(),
                    if index == 0 {
                        "https://example.test"
                    } else {
                        "null"
                    }
                    .into()
                ))
            );
        }
        assert_ne!(seen[0].request_id, seen[1].request_id);
        assert_ne!(seen[1].request_id, seen[2].request_id);
        drop(seen);
        service
            .handle(ContextServiceRequest::HttpStreamAbort {
                creator: None,
                page_id,
                stream_id,
            })
            .unwrap();
    }

    #[test]
    fn parent_cors_checks_credentials_and_preflight_permissions() {
        let response = NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: 1,
            status: 204,
            headers: vec![
                (
                    "Access-Control-Allow-Origin".into(),
                    "https://app.example".into(),
                ),
                ("Access-Control-Allow-Credentials".into(), "true".into()),
                ("Access-Control-Allow-Methods".into(), "PUT".into()),
                (
                    "Access-Control-Allow-Headers".into(),
                    "content-type, if-match".into(),
                ),
            ],
            body: Vec::new(),
        };
        assert!(enforce_cors_response("https://app.example", true, &response).is_ok());
        assert!(
            enforce_cors_preflight(
                "https://app.example",
                true,
                "PUT",
                &["content-type".into(), "if-match".into()],
                &response,
            )
            .is_ok()
        );
        assert!(
            enforce_cors_preflight(
                "https://app.example",
                true,
                "DELETE",
                &["x-forbidden".into()],
                &response,
            )
            .is_err()
        );

        let wildcard = NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: 2,
            status: 200,
            headers: vec![("Access-Control-Allow-Origin".into(), "*".into())],
            body: Vec::new(),
        };
        assert!(enforce_cors_response("https://app.example", false, &wildcard).is_ok());
        assert!(enforce_cors_response("https://app.example", true, &wildcard).is_err());
        assert!(
            enforce_mixed_content(
                "https://app.example",
                &Url::parse("http://api.example/data").unwrap(),
            )
            .is_err()
        );
        assert!(
            enforce_mixed_content(
                "http://app.example",
                &Url::parse("http://api.example/data").unwrap(),
            )
            .is_ok()
        );
    }
}

fn clipboard_service_error(error: RuntimeError) -> oryn_js::clipboard::ClipboardError {
    use oryn_js::clipboard::ClipboardError;
    match error {
        RuntimeError::Closed | RuntimeError::PageNotFound(_) => ClipboardError::Retired,
        RuntimeError::PolicyDenied { .. } | RuntimeError::ConfirmationRequired { .. } => {
            ClipboardError::NotAllowed
        }
        _ => ClipboardError::Unavailable,
    }
}
#[cfg(feature = "v8-host")]
pub(crate) struct NativeClipboardBackend {
    client: SharedContextClient,
    page_id: PageId,
}
#[cfg(feature = "v8-host")]
impl NativeClipboardBackend {
    pub(crate) fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::clipboard::ClipboardBackend for NativeClipboardBackend {
    fn authorize_script_for(
        &self,
        creator: &oryn_js::worker_wire::Creator,
        _origins: &[String],
        write: bool,
    ) -> Result<(), oryn_js::clipboard::ClipboardError> {
        match self
            .client
            .request(ContextServiceRequest::ClipboardAuthorizeScript {
                page_id: self.page_id,
                creator: creator.clone(),
                write,
            }) {
            Ok(ContextServiceResponse::ClipboardScriptAuthorized { result }) => result,
            _ => Err(oryn_js::clipboard::ClipboardError::Unavailable),
        }
    }
    fn read_for(
        &self,
        creator: &oryn_js::worker_wire::Creator,
    ) -> Result<oryn_js::clipboard::ClipboardData, oryn_js::clipboard::ClipboardError> {
        match self.client.request(ContextServiceRequest::ClipboardRead {
            page_id: self.page_id,
            creator: creator.clone(),
        }) {
            Ok(ContextServiceResponse::ClipboardRead { result }) => result,
            _ => Err(oryn_js::clipboard::ClipboardError::Unavailable),
        }
    }
    fn write_for(
        &self,
        creator: &oryn_js::worker_wire::Creator,
        data: oryn_js::clipboard::ClipboardData,
    ) -> Result<(), oryn_js::clipboard::ClipboardError> {
        data.validate()?;
        match self.client.request(ContextServiceRequest::ClipboardWrite {
            page_id: self.page_id,
            creator: creator.clone(),
            data,
        }) {
            Ok(ContextServiceResponse::ClipboardWrite { result }) => result,
            _ => Err(oryn_js::clipboard::ClipboardError::Unavailable),
        }
    }
    fn read(
        &self,
    ) -> Result<oryn_js::clipboard::ClipboardData, oryn_js::clipboard::ClipboardError> {
        Err(oryn_js::clipboard::ClipboardError::NotAllowed)
    }
    fn write(
        &self,
        _: oryn_js::clipboard::ClipboardData,
    ) -> Result<(), oryn_js::clipboard::ClipboardError> {
        Err(oryn_js::clipboard::ClipboardError::NotAllowed)
    }
}

/// Installed by native realm creation; the author context bridge rejects this RPC.
#[cfg(feature = "v8-host")]
pub struct NativeLocationNavigationBackend {
    client: SharedContextClient,
    page_id: PageId,
}
#[cfg(feature = "v8-host")]
impl NativeLocationNavigationBackend {
    pub fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::v8_bindings::LocationNavigationBackend for NativeLocationNavigationBackend {
    fn target_page(&self) -> Option<u64> {
        Some(self.page_id.0)
    }
    fn authorize_selected(
        &self,
        creator: oryn_js::worker_wire::Creator,
        target_page: u64,
        target: oryn_js::worker_wire::Creator,
        url: String,
        source_url: String,
        referrer_policy: String,
    ) -> Result<Option<oryn_js::navigation::ScriptNavigationAuthority>, String> {
        match self
            .client
            .request(ContextServiceRequest::IssueScriptNavigationAuthority {
                page_id: self.page_id,
                target_page_id: Some(PageId(target_page)),
                creator,
                target,
                url,
                source_url,
                referrer_policy,
            })
            .map_err(|error| error.to_string())?
        {
            ContextServiceResponse::ScriptNavigationAuthority { authority } => Ok(Some(authority)),
            ContextServiceResponse::PageDenied { .. } => Ok(None),
            _ => Err("invalid selected-target navigation authority response".into()),
        }
    }
    fn authorize_download(
        &self,
        creator: oryn_js::worker_wire::Creator,
        url: String,
    ) -> Result<bool, String> {
        match self
            .client
            .request(ContextServiceRequest::AuthorizePageDownload {
                page_id: self.page_id,
                creator,
                url,
            })
            .map_err(|error| error.to_string())?
        {
            ContextServiceResponse::PageDownloadAuthorized => Ok(true),
            ContextServiceResponse::PageDenied { .. } => Ok(false),
            _ => Err("invalid native download authorization response".into()),
        }
    }
    fn authorize(
        &self,
        creator: oryn_js::worker_wire::Creator,
        target: oryn_js::worker_wire::Creator,
        url: String,
        source_url: String,
        referrer_policy: String,
    ) -> Result<Option<oryn_js::navigation::ScriptNavigationAuthority>, String> {
        match self
            .client
            .request(ContextServiceRequest::IssueScriptNavigationAuthority {
                page_id: self.page_id,
                target_page_id: None,
                creator,
                target,
                url,
                source_url,
                referrer_policy,
            })
            .map_err(|error| error.to_string())?
        {
            ContextServiceResponse::ScriptNavigationAuthority { authority } => Ok(Some(authority)),
            ContextServiceResponse::PageDenied { .. } => Ok(None),
            _ => Err("invalid native Location navigation authorization response".into()),
        }
    }
}

pub struct NativeActivationBackend {
    client: SharedContextClient,
    page_id: PageId,
}
impl NativeActivationBackend {
    pub fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
impl oryn_js::user_activation::ActivationBackend for NativeActivationBackend {
    fn sticky(&self, creator: oryn_js::worker_wire::Creator) -> Result<bool, String> {
        match self
            .client
            .request(ContextServiceRequest::StickyUserActivation {
                page_id: self.page_id,
                creator,
            }) {
            Ok(ContextServiceResponse::TransientUserActivation { result }) => result,
            _ => Err("native sticky activation route unavailable".into()),
        }
    }

    fn transient(
        &self,
        creator: oryn_js::worker_wire::Creator,
        consume: bool,
    ) -> Result<bool, String> {
        match self
            .client
            .request(ContextServiceRequest::TransientUserActivation {
                page_id: self.page_id,
                creator,
                consume,
            }) {
            Ok(ContextServiceResponse::TransientUserActivation { result }) => result,
            _ => Err("native transient activation route unavailable".into()),
        }
    }

    fn notify(&self, creator: oryn_js::worker_wire::Creator) -> Result<(), String> {
        match self
            .client
            .request(ContextServiceRequest::NotifyUserActivation {
                page_id: self.page_id,
                creator,
            }) {
            Ok(ContextServiceResponse::UserActivationRecorded { result }) => result,
            _ => Err("native activation route unavailable".into()),
        }
    }
}

#[cfg(test)]
mod form_fetch_metadata_regressions {
    use super::*;
    use crate::network::{
        NetworkBroker, NetworkError, NetworkRequest, NetworkResponse, SharedNetworkBroker,
    };
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    struct Redirects(Arc<Mutex<Vec<NetworkRequest>>>);
    impl NetworkBroker for Redirects {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let mut seen = self.0.lock().unwrap();
            seen.push(request.clone());
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: if seen.len() < 3 { 302 } else { 200 },
                headers: if seen.len() == 1 {
                    vec![
                        ("location".into(), "https://next.test/end".into()),
                        ("referrer-policy".into(), "no-referrer".into()),
                    ]
                } else if seen.len() == 2 {
                    vec![("location".into(), "https://receipt.test/end".into())]
                } else {
                    vec![("content-type".into(), "text/html".into())]
                },
                body: b"<title>receipt</title>".to_vec(),
            })
        }
    }
    #[test]
    fn frame_form_fetch_keeps_original_referrer_cookie_source_across_destination_and_redirect() {
        let context = crate::Browser::new().new_context();
        let source = context.new_page().page_id();
        let destination = context.new_page().page_id();
        context
            .install_document_security(source, "https://source.test/forms/edit".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                source,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .install_document_security(destination, "https://destination.test/".into(), &[])
            .unwrap();
        context
            .commit_page_url(destination, "https://destination.test/".into())
            .unwrap();
        context
            .handle_message_wire(
                destination,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let authority = context
            .issue_form_authority_with_referrer(source, creator, Some("unsafe-url"))
            .unwrap();
        let authority = context
            .retarget_form_authority(&authority, source, destination)
            .unwrap();
        context
            .handle_message_wire(
                source,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = ParentContextService::new_with_network(
            context.clone(),
            destination,
            Some(SharedNetworkBroker::new(Redirects(seen.clone()))),
        );
        service
            .load_page_resource(
                destination,
                "https://receipt.test/start",
                "frame",
                PageResourceOptions {
                    response_lease: None,
                    download: None,
                    script_authority: None,
                    initiator: &ResourceInitiator {
                        embedded_type: None,
                        request_state: None,
                        navigation: Default::default(),
                        creator: Creator::Window {
                            target: WindowTarget::Root,
                            document_generation: 1,
                            stream_epoch: 0,
                        },
                        stylesheet: None,
                        prepared_policy: None,
                    },
                    form_authority: Some(authority),
                    initiator_url: Some("https://forged.test/"),
                    referrer_policy: None,
                    parent_encoding: None,
                    post: None,
                },
            )
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(
            seen[0]
                .headers
                .contains(&("referer".into(), "https://source.test/forms/edit".into()))
        );
        assert!(
            !seen[1]
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
        );
        for request in seen.iter() {
            assert_eq!(
                request.security.initiator_url.as_deref(),
                Some("https://source.test/forms/edit")
            );
            assert_eq!(
                request.security.top_level_url.as_deref(),
                Some("https://source.test/forms/edit")
            );
        }
        assert!(seen[1].security.redirect_chain_cross_site);
        assert!(seen[2].security.redirect_chain_cross_site);
    }
    #[test]
    fn script_frame_fetch_uses_sealed_source_after_initiator_retires() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://top.test/".into(), &[])
            .unwrap();
        context
            .commit_page_url(page, "https://top.test/".into())
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let frame = "8".repeat(64);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://source.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(page, "https://source.test/path", &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(
                page,
                frame.clone(),
                2,
                0,
                "https://source.test/path",
                &ticket,
            )
            .unwrap();
        context
            .register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
                page,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        context
            .register_isolation_container(
                crate::runtime::worker_service::IsolationContainerInputs {
                    page,
                    frame: frame.clone(),
                    generation: 2,
                    epoch: 0,
                    allow: String::new(),
                    source: "https://source.test/path".into(),
                    sandbox: None,
                },
            )
            .unwrap();
        let source = Creator::Window {
            target: WindowTarget::Frame(frame.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        let owner = context
            .window_worker_creator(page, &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap()
            .0;
        context.record_native_activation(owner).unwrap();
        let authority = context
            .issue_script_navigation_authority(crate::runtime::ScriptNavigationInputs {
                source: page,
                source_creator: source,
                destination: page,
                target_creator: root.clone(),
                destination_url: "https://receipt.test/start",
                source_url: "https://source.test/path",
                referrer_policy: "unsafe-url",
            })
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = ParentContextService::new_with_network(
            context.clone(),
            page,
            Some(SharedNetworkBroker::new(Redirects(seen.clone()))),
        );
        let admission = oryn_js::navigation::ScriptNavigationAdmission {
            authority,
            target: root.clone(),
        };
        let initiator = ResourceInitiator {
            embedded_type: None,
            request_state: None,
            navigation: Default::default(),
            creator: root,
            stylesheet: None,
            prepared_policy: None,
        };
        let wrong = service.load_page_resource(
            page,
            "https://receipt.test/wrong",
            "frame",
            PageResourceOptions {
                response_lease: None,
                download: None,
                script_authority: Some(admission.clone()),
                initiator: &initiator,
                form_authority: None,
                initiator_url: None,
                referrer_policy: None,
                parent_encoding: None,
                post: None,
            },
        );
        assert!(wrong.is_err());
        assert!(seen.lock().unwrap().is_empty());
        service
            .load_page_resource(
                page,
                "https://receipt.test/start",
                "frame",
                PageResourceOptions {
                    response_lease: None,
                    download: None,
                    script_authority: Some(admission),
                    initiator: &initiator,
                    form_authority: None,
                    initiator_url: Some("https://forged.test/"),
                    referrer_policy: Some("no-referrer"),
                    parent_encoding: None,
                    post: None,
                },
            )
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(
            seen[0]
                .headers
                .contains(&("referer".into(), "https://source.test/path".into()))
        );
        assert!(
            !seen[1]
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
        );
        for request in seen.iter() {
            assert_eq!(
                request.security.initiator_url.as_deref(),
                Some("https://source.test/path")
            );
            assert_eq!(
                request.security.top_level_url.as_deref(),
                Some("https://top.test/")
            );
            assert_eq!(
                request.security.ancestor_urls,
                vec!["https://source.test/path", "https://top.test/"]
            );
        }
        assert!(seen[1].security.redirect_chain_cross_site);
        assert!(seen[2].security.redirect_chain_cross_site);
    }
    #[test]
    fn script_frame_fetch_rejects_target_retirement_during_io() {
        struct RetireTarget {
            context: crate::BrowserContext,
            page: PageId,
            frame: String,
            status: u16,
            calls: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl NetworkBroker for RetireTarget {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.context
                    .handle_message_wire(
                        self.page,
                        MessageRequest::RetireWindow {
                            frame: self.frame.clone(),
                            document_generation: 2,
                            stream_epoch: 0,
                        },
                    )
                    .unwrap();
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: self.status,
                    headers: vec![
                        ("location".into(), "https://receipt.test/redirected".into()),
                        ("content-type".into(), "text/html".into()),
                    ],
                    body: b"<p>retired target response</p>".to_vec(),
                })
            }
        }
        for status in [200, 302] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page().page_id();
            context
                .install_document_security(page, "https://top.test/".into(), &[])
                .unwrap();
            context
                .commit_page_url(page, "https://top.test/".into())
                .unwrap();
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let root = Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let frame = "7".repeat(64);
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "https://child.test".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            let ticket = context
                .prepare_frame_worker_policy(page, "https://child.test/", &[], "")
                .unwrap();
            context
                .bind_frame_worker_policy(page, frame.clone(), 2, 0, "https://child.test/", &ticket)
                .unwrap();
            context
                .register_activation_parent(
                    crate::runtime::worker_service::ActivationParentInputs {
                        page,
                        frame: frame.clone(),
                        generation: 2,
                        epoch: 0,
                        parent: WindowTarget::Root,
                        parent_generation: 1,
                        parent_epoch: 0,
                    },
                )
                .unwrap();
            context
                .register_isolation_container(
                    crate::runtime::worker_service::IsolationContainerInputs {
                        page,
                        frame: frame.clone(),
                        generation: 2,
                        epoch: 0,
                        allow: String::new(),
                        source: "https://child.test/".into(),
                        sandbox: None,
                    },
                )
                .unwrap();
            let target = Creator::Window {
                target: WindowTarget::Frame(frame.clone()),
                document_generation: 2,
                stream_epoch: 0,
            };
            let authority = context
                .issue_script_navigation_authority(crate::runtime::ScriptNavigationInputs {
                    source: page,
                    source_creator: root.clone(),
                    destination: page,
                    target_creator: target.clone(),
                    destination_url: "https://receipt.test/start",
                    source_url: "https://top.test/",
                    referrer_policy: "unsafe-url",
                })
                .unwrap();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let network = SharedNetworkBroker::new(RetireTarget {
                context: context.clone(),
                page,
                frame,
                status,
                calls: calls.clone(),
            });
            let service = ParentContextService::new_with_network(context, page, Some(network));
            let result = service.load_page_resource(
                page,
                "https://receipt.test/start",
                "frame",
                PageResourceOptions {
                    response_lease: None,
                    download: None,
                    script_authority: Some(oryn_js::navigation::ScriptNavigationAdmission {
                        authority,
                        target,
                    }),
                    initiator: &ResourceInitiator {
                        embedded_type: None,
                        request_state: None,
                        navigation: Default::default(),
                        creator: root,
                        stylesheet: None,
                        prepared_policy: None,
                    },
                    form_authority: None,
                    initiator_url: None,
                    referrer_policy: None,
                    parent_encoding: None,
                    post: None,
                },
            );
            assert!(
                result.is_err(),
                "neither a terminal body nor a redirect may outlive its target"
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }
}

pub struct NativePlatformBackend {
    client: SharedContextClient,
    page_id: PageId,
}
impl NativePlatformBackend {
    pub fn new(client: SharedContextClient, page_id: PageId) -> Self {
        Self { client, page_id }
    }
}
impl oryn_js::platform_control::Backend for NativePlatformBackend {
    fn request(
        &self,
        creator: oryn_js::worker_wire::Creator,
        request: oryn_js::platform_control::Request,
    ) -> Result<oryn_js::platform_control::Response, String> {
        match self
            .client
            .request(ContextServiceRequest::NativePlatformQuery {
                page_id: self.page_id,
                creator,
                request,
            }) {
            Ok(ContextServiceResponse::NativePlatformResult { result }) => result,
            _ => Err("native platform route unavailable".into()),
        }
    }
}

#[cfg(test)]
mod document_resource_authority_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    fn fixture() -> (BrowserContext, PageId, ParentContextService, Creator) {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "6".repeat(64);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                page,
                "https://child.test/",
                &[(
                    "content-security-policy".into(),
                    "frame-src 'none'; img-src 'none'".into(),
                )],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(page, frame.clone(), 2, 0, "https://child.test/", &ticket)
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Frame(frame),
            document_generation: 2,
            stream_epoch: 0,
        };
        let service = ParentContextService::new_with_network(
            context.clone(),
            page,
            Some(SharedNetworkBroker::new(crate::network::UnavailableBroker)),
        );
        (context, page, service, creator)
    }
    #[test]
    fn support_original_child_guard_rejects_wrong_child_and_retired_generation() {
        let (context, page, _service, creator) = fixture();
        let root = Creator::Window { target: WindowTarget::Root, document_generation: 1, stream_epoch: 0 };
        let Creator::Window { target: WindowTarget::Frame(frame), .. } = &creator else { panic!() };
        context.register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
            page, frame: frame.clone(), generation: 2, epoch: 0,
            parent: WindowTarget::Root, parent_generation: 1, parent_epoch: 0,
        }).unwrap();
        let original = Mutex::new(Some(creator.clone()));
        assert!(context.with_support_original_child(page, &root, &original, |child, _, _| {
            assert_eq!(child, &creator);
            Ok(())
        }).is_ok());
        let wrong = Mutex::new(Some(Creator::Window { target: WindowTarget::Frame("7".repeat(64)), document_generation: 2, stream_epoch: 0 }));
        let called = std::cell::Cell::new(false);
        assert!(context.with_support_original_child(page, &root, &wrong, |_, _, _| { called.set(true); Ok(()) }).is_err());
        assert!(!called.get());
        context.handle_message_wire(page, MessageRequest::RetireWindow { frame: frame.clone(), document_generation: 2, stream_epoch: 0 }).unwrap();
        assert!(context.with_support_original_child(page, &root, &original, |_, _, _| { called.set(true); Ok(()) }).is_err());
        assert!(!called.get());
    }

    #[test]
    fn pinned_support_terminal_occurrences_share_original_http_32_slot_limit() {
        // Explicit pinned input root: absence is failure, never a skipped test
        // or a test-only descriptor issuer. from_fixture checks every raw hash.
        let export = std::path::PathBuf::from(std::env::var_os("ORYN_PINNED_SUPPORT_EXPORT")
            .expect("set ORYN_PINNED_SUPPORT_EXPORT to the pinned WPT export"));
        let read = |path: &str| std::fs::read(export.join(path.trim_start_matches('/'))).unwrap();
        let parent_bytes = read(crate::support_descriptor::PARENT);
        let child_bytes = read(crate::support_descriptor::CHILD);
        let imports: [Vec<u8>; 6] = std::array::from_fn(|index|
            read(crate::support_descriptor::IMPORTS[index].0));
        let parent_url = url::Url::parse("https://top.test/IndexedDB/idb-partitioned-coverage.sub.html").unwrap();
        let child_url = url::Url::parse("https://child.test/IndexedDB/resources/idb-partitioned-coverage-iframe.html").unwrap();
        let hash = "9b102136523d37559a98e9a101b6c7f4f379f43f01160f840a8248e51fd8a8d7";
        let descriptor_hash: [u8; 32] = std::array::from_fn(|index|
            u8::from_str_radix(&hash[index * 2..index * 2 + 2], 16).unwrap());
        let descriptor = crate::support_descriptor::PinnedSupportDescriptor::from_fixture(
            [1; 32], descriptor_hash, &parent_url, &child_url,
            crate::support_descriptor::REVISION, &parent_bytes, &child_bytes,
            std::array::from_fn(|index| imports[index].as_slice())).unwrap();
        let (context, page, service, child) = fixture();
        let root = Creator::Window { target: WindowTarget::Root, document_generation: 1, stream_epoch: 0 };
        let Creator::Window { target: WindowTarget::Frame(frame), .. } = &child else { panic!("child") };
        context.register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
            page, frame: frame.clone(), generation: 2, epoch: 0,
            parent: WindowTarget::Root, parent_generation: 1, parent_epoch: 0,
        }).unwrap();
        let original = Mutex::new(Some(child.clone()));
        let (pe, ce) = context.with_support_original_child(page, &root, &original,
            |_, pe, ce| Ok((pe, ce))).unwrap();
        let make = |id: u64| {
            let mut proof = crate::support_broker_occurrence::BrokerOccurrence::start(
                descriptor, page.0, id, id, &root, pe, parent_url.as_str(), child_url.as_str()).unwrap();
            assert!(proof.bind(page.0, &root, pe, &child, ce));
            let (sender, events) = tokio::sync::mpsc::channel(2);
            sender.try_send(NetworkEvent::Chunk { request_id: id, bytes: child_bytes.clone().into() }).unwrap();
            sender.try_send(NetworkEvent::Complete { request_id: id, encoded_body_size: Some(child_bytes.len() as u64) }).unwrap();
            Arc::new(HttpStreamEntry {
                support: Mutex::new(Some(proof)), keepalive_context: None, keepalive_drain: None,
                page_id: page, creator: Mutex::new(Some(child.clone())),
                prepared_frame_policy: None, frame_deadline: None, request_id: id, cleanup: None,
                state: Mutex::new(HttpStreamState {
                    preload: None, operation: Some(NetworkOperation {
                        body_demand_pending: Default::default(), prefetched: Default::default(),
                        request_id: id, events, demand: None,
                    }), current: bytes::Bytes::new(), offset: 0, bytes_read: 0,
                    content_length: Some(child_bytes.len()),
                }),
            })
        };
        for id in 1..=32 {
            service.socket_lifecycle.admit_http(id, make(id)).unwrap();
            let ContextServiceResponse::HttpStreamChunk { done, .. } =
                service.read_http_stream(page, id, 16 * 1024 * 1024).unwrap()
                else { panic!("HTTP body response") };
            assert!(done, "actual broker Complete must settle original response");
            for (index, (path, _)) in crate::support_descriptor::IMPORTS.iter().enumerate() {
                service.record_support_import(page, &child, &child_url.join(path).unwrap(), &imports[index]).unwrap();
            }
            let receipt = service.support_response_snapshot(page, id).unwrap().unwrap();
            assert_eq!(receipt.child.occurrence, id);
            assert_eq!(receipt.body_bytes, child_bytes.len() as u64);
            let registry = service.http_streams.lock().unwrap();
            let entry = registry.get(&id).unwrap();
            assert!(entry.state.lock().unwrap().operation.is_none(), "EOF releases original IO receiver");
        }
        assert_eq!(service.http_streams.lock().unwrap().len(), 32);
        assert!(matches!(service.socket_lifecycle.admit_http(33, make(33)),
            Err(RuntimeError::WorkerLimitExceeded { resource }) if resource == "pending HTTP streams"));
        // Explicit release revokes the original receipt and frees exactly one
        // original capability slot, not an independent evidence registry.
        service.socket_lifecycle.release_result(Some(&child), Some(1), None).unwrap();
        assert!(service.support_response_snapshot(page, 1).unwrap().is_none());
        service.socket_lifecycle.admit_http(33, make(33)).unwrap();
        for id in 2..=33 {
            service.socket_lifecycle.release_result(Some(&child), Some(id), None).unwrap();
        }
        assert!(service.http_streams.lock().unwrap().is_empty());
    }

    #[test]
    fn navigation_authority_uses_exact_child_origin_and_rejects_replacement() {
        let (context, page, service, creator) = fixture();
        let request = || ContextServiceRequest::AuthorizeNavigation {
            page_id: page,
            source: NavigationPolicySource::Document(creator.clone()),
            url: "https://child.test/next".into(),
        };
        assert!(matches!(
            service.handle(request()).unwrap(),
            ContextServiceResponse::NavigationAuthorized
        ));
        assert!(
            context.security_audit().is_empty(),
            "same-child origin must not borrow the cross-origin active root"
        );
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(service.handle(request()).is_err());
        assert!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page,
                    source: NavigationPolicySource::InitialAboutBlank,
                    url: "https://top.test/next".into()
                })
                .is_err()
        );
        assert!(serde_json::from_value::<ContextServiceRequest>(serde_json::json!({"operation":"authorize_navigation","page_id":page,"url":"https://top.test/"})).is_err());
    }
    #[test]
    fn native_command_snapshot_captures_initial_once_and_never_repairs_stale_authority() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        let service = ParentContextService::new(context.clone(), page);
        let query = || ContextServiceRequest::CaptureCommandNavigationSource {
            page_id: page,
            initial_only: false,
        };
        assert!(!query().author_compatibility_operation());
        let ContextServiceResponse::CommandNavigationSource { creator: first } =
            service.handle(query()).unwrap()
        else {
            panic!()
        };
        let ContextServiceResponse::CommandNavigationSource { creator: again } =
            service.handle(query()).unwrap()
        else {
            panic!()
        };
        assert_eq!(first, again);
        context
            .install_document_security(page, "https://replacement.test/".into(), &[])
            .unwrap();
        // A nonblank page with no committed matching root cannot synthesize one.
        assert!(service.handle(query()).is_err());
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let ContextServiceResponse::CommandNavigationSource {
            creator: replacement,
        } = service.handle(query()).unwrap()
        else {
            panic!()
        };
        assert_ne!(first, replacement);
        assert!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page,
                    source: NavigationPolicySource::Document(first),
                    url: "https://replacement.test/next".into()
                })
                .is_err()
        );
        assert!(matches!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page,
                    source: NavigationPolicySource::Document(replacement),
                    url: "https://replacement.test/next".into()
                })
                .unwrap(),
            ContextServiceResponse::NavigationAuthorized
        ));
    }
    #[test]
    fn retired_page_only_worker_and_metadata_routes_never_fall_back_to_live_root() {
        let (context, page, service, _) = fixture();
        let before = context.page_referrer_policy(page).unwrap();
        for request in [
            ContextServiceRequest::LoadWorkerScript {
                page_id: page,
                url: "https://top.test/worker.js".into(),
                module: false,
            },
            ContextServiceRequest::InstallMetaSecurity {
                page_id: page,
                serialized: "default-src 'none'".into(),
            },
            ContextServiceRequest::InstallReferrerPolicy {
                page_id: page,
                serialized: "no-referrer".into(),
            },
        ] {
            assert!(matches!(
                service.handle(request),
                Err(RuntimeError::ProtocolViolation(_))
            ));
        }
        assert_eq!(context.page_referrer_policy(page).unwrap(), before);
    }
    #[test]
    fn referrer_query_reads_exact_child_metadata_and_rejects_retirement() {
        let (context, page, service, creator) = fixture();
        service
            .handle(ContextServiceRequest::InstallRealmMetadata {
                page_id: page,
                creator: creator.clone(),
                meta_csp: None,
                meta_referrer: Some("no-referrer".into()),
            })
            .unwrap();
        let query = || ContextServiceRequest::GetReferrerPolicy {
            page_id: page,
            creator: creator.clone(),
        };
        let ContextServiceResponse::ReferrerPolicy { value } = service.handle(query()).unwrap()
        else {
            panic!()
        };
        assert_eq!(value, "no-referrer");
        assert_ne!(
            context.page_referrer_policy(page).unwrap().as_str(),
            "no-referrer"
        );
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(service.handle(query()).is_err());
        // Missing native source is a protocol decoding error, not root lookup.
        assert!(
            serde_json::from_value::<ContextServiceRequest>(
                serde_json::json!({"operation":"get_referrer_policy","page_id":page})
            )
            .is_err()
        );
    }
    #[test]
    fn binary_css_response_preserves_non_text_bytes_and_native_policy() {
        let (_context, page, service, child) = fixture();
        let request = |creator, destination: &str| ContextServiceRequest::LoadPageResource {
            download: None,
            script_authority: None,
            page_id: page,
            initiator: Box::new(ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator,
                stylesheet: None,
                prepared_policy: None,
            }),
            form_authority: None,
            url: "data:application/octet-stream;base64,AP/+gA==".into(),
            destination: destination.into(),
            parent_encoding: None,
            initiator_url: None,
            referrer_policy: None,
            post: None,
        };
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        for destination in ["font", "image"] {
            let ContextServiceResponse::PageBinaryResource {
                body_base64,
                destination: actual,
                ..
            } = service.handle(request(root.clone(), destination)).unwrap()
            else {
                panic!("binary resource decoded as text")
            };
            assert_eq!(actual, destination);
            assert_eq!(
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                    .unwrap(),
                vec![0, 255, 254, 128]
            );
        }
        assert!(service.handle(request(child, "image")).is_err());
    }
    #[test]
    fn parser_resource_uses_exact_embedder_csp_and_rejects_retired_creator() {
        let (context, page, service, creator) = fixture();
        let request = || ContextServiceRequest::LoadPageResource {
            download: None,
            script_authority: None,
            page_id: page,
            initiator: Box::new(ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: creator.clone(),
                stylesheet: None,
                prepared_policy: None,
            }),
            form_authority: None,
            url: "data:text/html,child".into(),
            destination: "frame".into(),
            parent_encoding: None,
            initiator_url: Some("https://top.test/".into()),
            referrer_policy: None,
            post: None,
        };
        let error = service.handle(request()).unwrap_err().to_string();
        assert!(error.contains("Content-Security-Policy"));
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(service.handle(request()).is_err());
    }
    #[test]
    fn precommit_child_resource_uses_native_response_policy_ticket() {
        let (context, page, service, _) = fixture();
        let ticket = context
            .prepare_frame_worker_policy(
                page,
                "https://pending.test/",
                &[("content-security-policy".into(), "img-src 'none'".into())],
                "",
            )
            .unwrap();
        let request = ContextServiceRequest::LoadPageResource {
            download: None,
            script_authority: None,
            page_id: page,
            initiator: Box::new(ResourceInitiator {
                embedded_type: None,
                request_state: None,
                navigation: Default::default(),
                creator: Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                stylesheet: None,
                prepared_policy: Some(ticket),
            }),
            form_authority: None,
            url: "data:image/png,bytes".into(),
            destination: "image".into(),
            parent_encoding: None,
            initiator_url: None,
            referrer_policy: None,
            post: None,
        };
        assert!(
            service
                .handle(request)
                .unwrap_err()
                .to_string()
                .contains("Content-Security-Policy")
        );
    }
    #[test]
    fn http_stream_creator_is_preserved_and_retirement_reclaims_only_owned_body() {
        let (context, page, service, creator) = fixture();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let start = |id, creator| ContextServiceRequest::HttpStreamStart {
            page_id: page,
            request: HttpStreamRequest {
                upgrade_mixed_image: false,
                fetch: None,
                client: Some(creator),
                use_cors_preflight: false,
                request_id: id,
                method: "GET".into(),
                url: "data:text/plain,owned".into(),
                headers: vec![],
                body_base64: String::new(),
                credentials: false,
                credentials_mode: Some("omit".into()),
                initiator_url: None,
                referrer_policy: None,
            },
        };
        let ContextServiceResponse::HttpStreamStarted {
            stream_id: child, ..
        } = service.handle(start(71, creator.clone())).unwrap()
        else {
            panic!()
        };
        let ContextServiceResponse::HttpStreamStarted { stream_id: top, .. } =
            service.handle(start(72, root.clone())).unwrap()
        else {
            panic!()
        };
        assert!(
            service
                .handle(ContextServiceRequest::HttpStreamRead {
                    page_id: page,
                    creator: Some(root.clone()),
                    stream_id: child,
                    max_bytes: 1
                })
                .is_err()
        );
        assert!(
            service
                .handle(ContextServiceRequest::HttpStreamAbort {
                    page_id: page,
                    creator: Some(root.clone()),
                    stream_id: child
                })
                .is_err()
        );
        let held = service
            .http_streams
            .lock()
            .unwrap()
            .get(&child)
            .cloned()
            .unwrap();
        let transport_guard = held.state.lock().unwrap();
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(!service.http_streams.lock().unwrap().contains_key(&child));
        assert!(service.http_streams.lock().unwrap().contains_key(&top));
        drop(transport_guard);
        drop(held);
        assert!(
            service
                .handle(ContextServiceRequest::HttpStreamAbort {
                    page_id: page,
                    creator: Some(creator),
                    stream_id: child
                })
                .is_ok()
        );
        service
            .handle(ContextServiceRequest::ReleaseNetworkResult {
                page_id: page,
                creator: Some(root),
                stream_id: Some(top),
                socket_id: None,
            })
            .unwrap();
        assert!(service.http_streams.lock().unwrap().is_empty());
    }
    #[test]
    fn native_storage_access_requires_current_nonopaque_document_before_store_io() {
        use oryn_js::platform_control::{Request, WindowStorageOperation};
        let (context, page, _service, creator) = fixture();
        assert!(
            context
                .native_platform_query(
                    page,
                    creator.clone(),
                    Request::Storage {
                        local: true,
                        operation: WindowStorageOperation::Access
                    }
                )
                .is_ok()
        );
        let opaque = context.new_page().page_id();
        let initial = context.native_initial_document_creator(opaque).unwrap();
        let denied = context
            .native_platform_query(
                opaque,
                initial,
                Request::Storage {
                    local: false,
                    operation: WindowStorageOperation::Access,
                },
            )
            .unwrap_err();
        assert!(denied.starts_with("SecurityError:"));
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = creator.clone()
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_platform_query(
                    page,
                    creator,
                    Request::Storage {
                        local: true,
                        operation: WindowStorageOperation::Access
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn native_storage_events_capture_writer_url_and_exact_recipient_lifetime() {
        use oryn_js::platform_control::{Request, Response};
        let (context, page, service, creator) = fixture();
        let recipient = context.new_page().page_id();
        context
            .install_document_security(recipient, "https://child.test/reader".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                recipient,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let owner = context
            .window_worker_creator(
                page,
                &match &creator {
                    Creator::Window { target, .. } => target.clone(),
                    _ => unreachable!(),
                },
                2,
                0,
            )
            .unwrap()
            .0;
        context
            .update_realm_document_url(&owner, "https://child.test/writer#current")
            .unwrap();
        service
            .handle(ContextServiceRequest::Storage {
                creator: Some(creator.clone()),
                request: StorageRequest::Set {
                    area: StorageArea::Local,
                    scope: StorageScope {
                        origin: "https://child.test".into(),
                        top_level_page_id: page,
                    },
                    key: "key".into(),
                    value: "value".into(),
                },
            })
            .unwrap();
        let Response::StorageEvents { events } = context
            .native_platform_query(recipient, root.clone(), Request::StorageEvents { after: 0 })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].url, "https://child.test/writer#current");
        assert_eq!(events[0].new_value.as_deref(), Some("value"));
        let Response::StorageEvents { events: own } = context
            .native_platform_query(page, creator.clone(), Request::StorageEvents { after: 0 })
            .unwrap()
        else {
            panic!()
        };
        assert!(own.is_empty());
        use oryn_js::platform_control::WindowStorageOperation as StorageOp;
        context
            .native_platform_query(
                page,
                creator.clone(),
                Request::Storage {
                    local: true,
                    operation: StorageOp::Set {
                        key: "key".into(),
                        value: "value".into(),
                    },
                },
            )
            .unwrap();
        context
            .native_platform_query(
                page,
                creator.clone(),
                Request::Storage {
                    local: true,
                    operation: StorageOp::Remove {
                        key: "absent".into(),
                    },
                },
            )
            .unwrap();
        let Response::StorageEvents { events: unchanged } = context
            .native_platform_query(
                recipient,
                root.clone(),
                Request::StorageEvents {
                    after: events[0].id,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(
            unchanged.is_empty(),
            "no-op mutations must not broadcast storage events"
        );
        context
            .install_document_security(recipient, "https://child.test/replacement".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                recipient,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let Response::StorageEvents { events: old } = context
            .native_platform_query(
                recipient,
                Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 2,
                    stream_epoch: 0,
                },
                Request::StorageEvents { after: 0 },
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(
            old.is_empty(),
            "new document cannot replay predecessor's storage event"
        );
        assert!(
            context
                .update_realm_document_url(&owner, "https://foreign.test/")
                .is_err()
        );
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .update_realm_document_url(&owner, "https://child.test/stale")
                .is_err()
        );
    }

    #[test]
    fn compatibility_events_follow_frame_origin_and_exclude_exact_source_document() {
        let (context, page, service, creator) = fixture();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let change = |origin: &str| StorageChange {
            area: StorageArea::Local,
            scope: StorageScope {
                origin: origin.into(),
                top_level_page_id: page,
            },
            key: Some("key".into()),
            old_value: None,
            new_value: Some("private".into()),
        };
        context
            .push_context_event(ContextEvent::Storage {
                source_page_id: PageId(999),
                source_creator: root.clone(),
                source_url: "https://source.test/".into(),
                targets: vec![(page, root.clone()), (page, creator.clone())],
                change: change("https://top.test"),
            })
            .unwrap();
        context
            .push_context_event(ContextEvent::Storage {
                source_page_id: page,
                source_creator: creator.clone(),
                source_url: "https://source.test/".into(),
                targets: vec![(page, root.clone()), (page, creator.clone())],
                change: change("https://child.test"),
            })
            .unwrap();
        let poll = |creator| ContextServiceRequest::PollEvents {
            page_id: page,
            creator: Some(creator),
            after_event_id: 0,
            max_events: 32,
        };
        let ContextServiceResponse::Events { events } =
            service.handle(poll(creator.clone())).unwrap()
        else {
            panic!()
        };
        assert!(events.is_empty());
        context
            .push_context_event(ContextEvent::Storage {
                source_page_id: PageId(999),
                source_creator: root.clone(),
                source_url: "https://source.test/".into(),
                targets: vec![(page, root.clone()), (page, creator.clone())],
                change: change("https://child.test"),
            })
            .unwrap();
        let ContextServiceResponse::Events { events } =
            service.handle(poll(creator.clone())).unwrap()
        else {
            panic!()
        };
        assert_eq!(events.len(), 1);
        let ContextServiceResponse::Events { events } = service.handle(poll(root)).unwrap() else {
            panic!()
        };
        assert_eq!(events.len(), 1);
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(service.handle(poll(creator)).is_err());
    }
    #[test]
    fn websocket_operations_require_exact_document_and_retirement_reclaims_owner() {
        let (context, page, service, creator) = fixture();
        service
            .socket_owners
            .lock()
            .unwrap()
            .insert(7, creator.clone());
        assert!(
            service
                .validate_websocket_creator(page, 7, Some(&creator), true)
                .is_ok()
        );
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            service
                .validate_websocket_creator(page, 7, Some(&root), true)
                .is_err()
        );
        assert!(
            service
                .validate_websocket_creator(page, 7, None, true)
                .is_err()
        );
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            service
                .validate_websocket_creator(page, 7, Some(&creator), true)
                .is_err()
        );
        assert!(!service.socket_owners.lock().unwrap().contains_key(&7));
        assert!(
            service
                .validate_websocket_creator(page, 7, Some(&creator), false)
                .is_err()
        );
        assert!(
            service
                .validate_websocket_creator(page, 7, Some(&root), false)
                .is_err()
        );
    }
    #[test]
    fn accepted_socket_retirement_closes_exact_owner_and_rejects_late_handshake() {
        use std::io::Read;
        let (context, page, service, creator) = fixture();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = || {
            let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (peer, _) = listener.accept().unwrap();
            (
                tungstenite::WebSocket::from_raw_socket(
                    tungstenite::stream::MaybeTlsStream::Plain(client),
                    tungstenite::protocol::Role::Client,
                    None,
                ),
                peer,
            )
        };
        let (first, mut first_peer) = socket();
        let (second, _second_peer) = socket();
        service
            .socket_lifecycle
            .admit(7, &creator, first, None)
            .unwrap();
        service
            .socket_lifecycle
            .admit(8, &root, second, None)
            .unwrap();
        let in_flight = service.sockets.lock().unwrap().get(&7).cloned().unwrap();
        let transport_guard = in_flight.socket.lock().unwrap();
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(!service.sockets.lock().unwrap().contains_key(&7));
        assert!(service.sockets.lock().unwrap().contains_key(&8));
        first_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(first_peer.read(&mut [0; 1]).unwrap(), 0);
        drop(transport_guard);
        drop(in_flight);
        let (late, _late_peer) = socket();
        assert!(
            service
                .socket_lifecycle
                .admit(9, &creator, late, None)
                .is_err()
        );
        context
            .install_document_security(page, "https://replacement.test/".into(), &[])
            .unwrap();
        assert!(service.sockets.lock().unwrap().is_empty());
        let (late_root, _late_root_peer) = socket();
        assert!(
            service
                .socket_lifecycle
                .admit(10, &root, late_root, None)
                .is_err()
        );
    }
    #[test]
    fn websocket_open_uses_frame_connect_policy_before_network_io() {
        let (context, page, service, creator) = fixture();
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 3,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                page,
                "https://child.test/",
                &[(
                    "content-security-policy".into(),
                    "connect-src 'none'".into(),
                )],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(page, frame.clone(), 3, 0, "https://child.test/", &ticket)
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Frame(frame.clone()),
            document_generation: 3,
            stream_epoch: 0,
        };
        let error = service
            .handle(ContextServiceRequest::WebSocketOpen {
                request_id: 1,
                page_id: page,
                creator: Some(creator),
                url: "wss://child.test/socket".into(),
                protocols: vec![],
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Content-Security-Policy connect-src")
        );
    }
    #[test]
    fn frame_storage_and_cookie_calls_require_current_exact_origin_and_lifetime() {
        let (context, page, service, creator) = fixture();
        let read = |origin: &str| ContextServiceRequest::Storage {
            creator: Some(creator.clone()),
            request: StorageRequest::Entries {
                area: StorageArea::Local,
                scope: StorageScope {
                    origin: origin.into(),
                    top_level_page_id: page,
                },
            },
        };
        assert!(service.handle(read("https://top.test")).is_err());
        assert!(service.handle(read("https://child.test")).is_ok());
        assert!(
            service
                .handle(ContextServiceRequest::GetDocumentCookie {
                    page_id: page,
                    creator: Some(creator.clone())
                })
                .is_ok()
        );
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creator
        else {
            panic!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(service.handle(read("https://child.test")).is_err());
        assert!(
            service
                .handle(ContextServiceRequest::GetDocumentCookie {
                    page_id: page,
                    creator: Some(creator)
                })
                .is_err()
        );
    }
}

#[cfg(test)]
mod author_compatibility_admission_tests {
    use super::*;
    #[test]
    fn native_script_navigation_upload_and_policy_routes_are_not_author_json_operations() {
        let page = PageId(1);
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let requests = vec![
            ContextServiceRequest::ReserveScriptLoad {
                page_id: page,
                creator,
            },
            ContextServiceRequest::CancelScriptLoad {
                page_id: page,
                token: 1,
            },
            ContextServiceRequest::MaterializeUpload {
                page_id: page,
                label: "secret".into(),
            },
            ContextServiceRequest::LoadWorkerScript {
                page_id: page,
                url: "https://example.test/private".into(),
                module: false,
            },
            ContextServiceRequest::InstallMetaSecurity {
                page_id: page,
                serialized: "default-src *".into(),
            },
            ContextServiceRequest::NavigatePage {
                page_id: page,
                url: "https://foreign.test/".into(),
            },
            ContextServiceRequest::CommitPage {
                page_id: page,
                url: "https://foreign.test/".into(),
            },
            ContextServiceRequest::HttpRequestAbort {
                page_id: page,
                request_id: 1,
            },
        ];
        for request in requests {
            assert!(!request.author_compatibility_operation());
        }
        assert!(
            ContextServiceRequest::GetDocumentCookie {
                page_id: page,
                creator: None
            }
            .author_compatibility_operation()
        );
        assert!(
            ContextServiceRequest::HttpStreamAbort {
                page_id: page,
                stream_id: 1,
                creator: None
            }
            .author_compatibility_operation()
        );
    }
}

#[cfg(test)]
mod native_csp_event_custody_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        platform_control::{Request, Response},
        worker_wire::Creator,
    };
    #[test]
    fn same_url_frames_do_not_share_csp_events_and_retired_owners_cannot_poll() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        let url = "https://same.test/";
        context
            .install_document_security(page, url.into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let mut creators = Vec::new();
        for id in ["7".repeat(64), "8".repeat(64)] {
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterWindow {
                        frame: id.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "https://same.test".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            let ticket = context
                .prepare_frame_worker_policy(
                    page,
                    url,
                    &[("content-security-policy".into(), "img-src 'none'".into())],
                    "",
                )
                .unwrap();
            context
                .bind_frame_worker_policy(page, id.clone(), 2, 0, url, &ticket)
                .unwrap();
            creators.push(Creator::Window {
                target: WindowTarget::Frame(id),
                document_generation: 2,
                stream_epoch: 0,
            });
        }
        let service = ParentContextService::new_with_network(
            context.clone(),
            page,
            Some(SharedNetworkBroker::new(crate::network::UnavailableBroker)),
        );
        service
            .handle(ContextServiceRequest::ReportRealmCspRequest {
                page_id: page,
                creator: creators[0].clone(),
                redirected: false,
                original_url: None,
                request: crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Image,
                    url: "https://same.test/filter.svg".into(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                },
            })
            .unwrap();
        let Response::CspEvents { events } = context
            .native_platform_query(page, creators[0].clone(), Request::CspEvents { after: 0 })
            .unwrap()
        else {
            panic!("native CSP response")
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].effective_directive, "img-src");
        assert_eq!(events[0].blocked_uri, "https://same.test/filter.svg");
        for creator in [
            creators[1].clone(),
            Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
        ] {
            let Response::CspEvents { events } = context
                .native_platform_query(page, creator, Request::CspEvents { after: 0 })
                .unwrap()
            else {
                panic!("native CSP response")
            };
            assert!(events.is_empty());
        }
        let Response::CspEvents { events: later } = context
            .native_platform_query(
                page,
                creators[0].clone(),
                Request::CspEvents {
                    after: events[0].id,
                },
            )
            .unwrap()
        else {
            panic!("native CSP response")
        };
        assert!(later.is_empty());
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = &creators[0]
        else {
            unreachable!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_platform_query(page, creators[0].clone(), Request::CspEvents { after: 0 })
                .is_err()
        );
    }
}

#[cfg(test)]
mod frame_response_stream_tests {
    use super::*;
    use crate::network::NetworkError;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    struct PrefixBroker {
        sender: Arc<Mutex<Option<tokio::sync::mpsc::Sender<NetworkEvent>>>>,
        cancelled: Arc<AtomicBool>,
    }
    impl NetworkBroker for PrefixBroker {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            panic!("frame preparation must use owned stream")
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(4);
            let request_id = request.request_id;
            sender
                .try_send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: None,
                    request_id,
                    status: 200,
                    headers: vec![
                        ("content-type".into(), "text/html; charset=utf-8".into()),
                        ("Content-Language".into(), "tr".into()),
                    ],
                    content_length: None,
                    upload: None,
                })
                .unwrap();
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id,
                    bytes: bytes::Bytes::from_static(b"<script>early()</script>"),
                })
                .unwrap();
            *self.sender.lock().unwrap() = Some(sender);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id,
                events,
                demand: None,
            })
        }
        fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
            self.cancelled.store(true, Ordering::Release);
            self.sender.lock().unwrap().take();
            Ok(())
        }
    }
    fn fixture() -> (
        BrowserContext,
        PageId,
        Arc<ParentContextService>,
        ContextServiceResponse,
    ) {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://example.test/".into(), &[])
            .unwrap();
        context
            .commit_page_url(page, "https://example.test/".into())
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let network = SharedNetworkBroker::new(PrefixBroker {
            sender: Default::default(),
            cancelled: Default::default(),
        });
        let service = ParentContextService::new_with_network(context.clone(), page, Some(network))
            .into_shared();
        let worker = service.clone();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = worker.handle(ContextServiceRequest::LoadPageResource {
                page_id: page,
                url: "https://example.test/frame".into(),
                destination: "frame".into(),
                download: None,
                script_authority: None,
                initiator: Box::new(ResourceInitiator {
                    embedded_type: None,
                    request_state: None,
                    navigation: Default::default(),
                    creator: Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                    stylesheet: None,
                    prepared_policy: None,
                }),
                form_authority: None,
                parent_encoding: None,
                initiator_url: None,
                referrer_policy: None,
                post: None,
            });
            let _ = sent.send(result);
        });
        let response = received
            .recv_timeout(Duration::from_secs(2))
            .expect("headers publish without body EOF")
            .unwrap();
        (context, page, service, response)
    }
    fn bind(
        context: &BrowserContext,
        page: PageId,
        service: &ParentContextService,
        token: &str,
    ) -> Creator {
        let frame = "c".repeat(64);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://example.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        service
            .handle(ContextServiceRequest::BindFrameWorkerPolicy {
                page_id: page,
                frame: frame.clone(),
                document_generation: 2,
                stream_epoch: 0,
                url: "https://example.test/frame".into(),
                token: token.into(),
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
                container_allow: String::new(),
                container_source: "https://example.test/frame".into(),
                container_sandbox: None,
            })
            .unwrap();
        Creator::Window {
            target: WindowTarget::Frame(frame),
            document_generation: 2,
            stream_epoch: 0,
        }
    }
    #[test]
    fn frame_headers_and_prefix_publish_before_eof_only_after_native_child_binding() {
        let (context, page, service, response) = fixture();
        let ContextServiceResponse::PageResource {
            stream_id: Some(id),
            worker_policy_token: Some(token),
            source,
            content_language,
            ..
        } = response
        else {
            panic!("stream grant")
        };
        assert!(source.is_empty());
        assert_eq!(content_language.as_deref(), Some("tr"));
        assert!(
            service
                .validate_http_stream_creator(page, id, None, true)
                .is_err()
        );
        let child = bind(&context, page, &service, &token);
        assert!(
            service
                .validate_http_stream_creator(
                    page,
                    id,
                    Some(&Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: 1,
                        stream_epoch: 0
                    }),
                    true
                )
                .is_err()
        );
        let worker = service.clone();
        let owner = child.clone();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(worker.handle(ContextServiceRequest::HttpStreamRead {
                page_id: page,
                stream_id: id,
                max_bytes: 64 * 1024,
                creator: Some(owner),
            }));
        });
        let ContextServiceResponse::HttpStreamChunk {
            body_base64, done, ..
        } = received
            .recv_timeout(Duration::from_secs(2))
            .expect("prefix does not wait to fill 64KiB")
            .unwrap()
        else {
            panic!()
        };
        assert!(!done);
        assert_eq!(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                .unwrap(),
            b"<script>early()</script>"
        );
        let Creator::Window {
            target: WindowTarget::Frame(frame),
            ..
        } = child
        else {
            unreachable!()
        };
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(!service.http_streams.lock().unwrap().contains_key(&id));
    }
    #[test]
    fn rejected_frame_stream_receipt_reclaims_stream_and_prepared_policy() {
        let (context, page, service, response) = fixture();
        let ContextServiceResponse::PageResource {
            stream_id: Some(id),
            worker_policy_token: Some(token),
            ..
        } = &response
        else {
            panic!()
        };
        let scope = ContextTransport::response_scope(service.as_ref(), None, None).unwrap();
        drop(response_custody::ResponseGuard::new(
            scope,
            Default::default(),
            Some(&response),
        ));
        context.retry_context_response_reclaims().unwrap();
        assert!(!service.http_streams.lock().unwrap().contains_key(id));
        assert!(context.prepared_frame_resource_policy(page, token).is_err());
    }
    #[test]
    fn retired_resource_scope_releases_grants_after_live_service_is_dropped() {
        let (context, page, service, response) = fixture();
        let ContextServiceResponse::PageResource {
            stream_id: Some(id),
            worker_policy_token: Some(policy),
            ..
        } = response
        else {
            panic!("stream grant")
        };
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let (local, _, _, _, _) = context
            .prepare_local_navigation(page, creator, "data:text/html,late", false)
            .unwrap();
        let cleanup = service.navigation_resource_cleanup();
        assert!(!cleanup.is_drained().unwrap());
        // Live service destruction seals network work, but retained policy and local
        // grants still have an exact release-only owner until the late ACK arrives.
        drop(service);
        assert!(!cleanup.is_drained().unwrap());
        cleanup.release_frame(id, &policy).unwrap();
        assert!(!cleanup.is_drained().unwrap());
        cleanup.discard_local(&local).unwrap();
        assert!(cleanup.is_drained().unwrap());
        cleanup.release_frame(id, &policy).unwrap();
        cleanup.discard_local(&local).unwrap();
        assert!(
            context
                .commit_prepared_local_navigation(page, &local)
                .is_err()
        );
    }
    #[test]
    fn resource_cleanup_rejects_foreign_page_and_policy_without_consuming_grants() {
        let (context, page, service, response) = fixture();
        let ContextServiceResponse::PageResource {
            stream_id: Some(id),
            worker_policy_token: Some(policy),
            ..
        } = response
        else {
            panic!("stream grant")
        };
        let other = context.new_page();
        let foreign = NavigationResourceCleanup {
            page: other.page_id(),
            context: context.downgrade(),
            streams: service.http_streams.clone(),
        };
        let (local, _, _, _, _) = context
            .prepare_local_navigation(
                page,
                Creator::Window {
                    target: WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                "data:text/html,owned",
                false,
            )
            .unwrap();
        assert!(foreign.release_frame(id, &policy).is_err());
        assert!(foreign.discard_local(&local).is_err());
        let cleanup = service.navigation_resource_cleanup();
        assert!(cleanup.release_frame(id, "forged").is_err());
        assert!(service.http_streams.lock().unwrap().contains_key(&id));
        assert!(
            context
                .prepared_frame_resource_policy(page, &policy)
                .is_ok()
        );
        cleanup.release_frame(id, &policy).unwrap();
        assert!(!cleanup.is_drained().unwrap());
        cleanup.discard_local(&local).unwrap();
        assert!(cleanup.is_drained().unwrap());
    }
    #[test]
    fn frame_response_release_is_grant_scoped_before_native_commit() {
        let (context, page, service, response) = fixture();
        let ContextServiceResponse::PageResource {
            stream_id: Some(id),
            worker_policy_token: Some(token),
            ..
        } = response
        else {
            panic!()
        };
        assert!(
            service
                .handle(ContextServiceRequest::ReleaseFrameResponseStream {
                    page_id: page,
                    stream_id: id,
                    policy: "foreign".into()
                })
                .is_err()
        );
        assert!(service.http_streams.lock().unwrap().contains_key(&id));
        service
            .handle(ContextServiceRequest::ReleaseFrameResponseStream {
                page_id: page,
                stream_id: id,
                policy: token.clone(),
            })
            .unwrap();
        assert!(!service.http_streams.lock().unwrap().contains_key(&id));
        assert!(
            context
                .prepared_frame_resource_policy(page, &token)
                .is_err()
        );
    }
}

/// Parent-authenticated structural view; URL disclosure is independently
/// same-origin guarded. Ordered children come from native task-boundary
/// publication; no target actor or authored script executes during this read.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RelatedWindowSnapshot {
    pub children: Vec<(String, oryn_js::message_wire::WindowTarget)>,
    pub same_origin_domain: bool,
    pub creator: oryn_js::worker_wire::Creator,
    pub parent: oryn_js::message_wire::WindowTarget,
    pub opener: Option<(PageId, oryn_js::message_wire::WindowTarget)>,
    pub length: usize,
    pub url: Option<String>,
}

#[cfg(test)]
mod realm_inline_script_authority_tests {
    use super::*;
    use crate::network::NetworkError;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    #[test]
    fn native_inline_kind_allowance_is_not_general_script_authority() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(
                id,
                "https://rules.test/".into(),
                &[(
                    "content-security-policy".into(),
                    "script-src 'inline-speculation-rules'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        assert!(matches!(
            service.authorize_inline_script_kind(InlineScriptAuthorization {
                page_id: id,
                creator: Some(&creator),
                source: "{}",
                nonce: None,
                parser_inserted: true,
                script_policy: None,
                speculation_rules: true
            }),
            Ok(ContextServiceResponse::ScriptAuthorized)
        ));
        assert!(matches!(
            service.authorize_inline_script_kind(InlineScriptAuthorization {
                page_id: id,
                creator: Some(&creator),
                source: "{}",
                nonce: None,
                parser_inserted: true,
                script_policy: None,
                speculation_rules: false
            }),
            Ok(ContextServiceResponse::ScriptLoadFailed {
                failure: crate::script_load_failure::ScriptLoadFailure::Fetch(_)
            })
        ));
        let stale = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            service
                .authorize_inline_script_kind(InlineScriptAuthorization {
                    page_id: id,
                    creator: Some(&stale),
                    source: "{}",
                    nonce: None,
                    parser_inserted: true,
                    script_policy: None,
                    speculation_rules: true
                })
                .is_err()
        );
    }
    #[test]
    fn captured_inline_policy_survives_later_meta_but_not_owner_retirement() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://root.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        let ContextServiceResponse::CapturedScriptPolicy { authority } = service
            .handle(ContextServiceRequest::CaptureScriptPolicy {
                page_id: id,
                creator: creator.clone(),
            })
            .unwrap()
        else {
            panic!("snapshot")
        };
        service
            .handle(ContextServiceRequest::InstallRealmMetadata {
                page_id: id,
                creator: creator.clone(),
                meta_csp: Some("script-src 'none'".into()),
                meta_referrer: None,
            })
            .unwrap();
        assert!(matches!(
            service.authorize_inline_script(id, Some(&creator), "0", None, true),
            Ok(ContextServiceResponse::ScriptLoadFailed {
                failure: crate::script_load_failure::ScriptLoadFailure::Fetch(_)
            })
        ));
        assert!(
            service
                .authorize_inline_script_with_policy(
                    id,
                    Some(&creator),
                    "0",
                    None,
                    true,
                    Some(&authority)
                )
                .is_ok()
        );
        let stale = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            service
                .authorize_inline_script_with_policy(
                    id,
                    Some(&stale),
                    "0",
                    None,
                    true,
                    Some(&authority)
                )
                .is_err()
        );
        service.cancel_all_network_operations();
        assert!(
            service
                .authorize_inline_script_with_policy(
                    id,
                    Some(&creator),
                    "0",
                    None,
                    true,
                    Some(&authority)
                )
                .is_err()
        );
    }
    #[test]
    fn captured_policy_pruning_retains_cached_owners_until_eviction() {
        use oryn_js::message_wire::MessageResponse;
        let context = crate::Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://root.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = |generation| Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        let first_owner = service.script_document_owner(id, &creator(1)).unwrap();
        let ContextServiceResponse::CapturedScriptPolicy { authority: first } = service
            .handle(ContextServiceRequest::CaptureScriptPolicy {
                page_id: id,
                creator: creator(1),
            })
            .unwrap()
        else {
            panic!("capture")
        };
        let MessageResponse::DocumentParked(parked) = context
            .handle_message_wire(
                id,
                MessageRequest::ParkDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        else {
            panic!("park")
        };
        context
            .install_document_security(id, "https://next.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        service
            .handle(ContextServiceRequest::CaptureScriptPolicy {
                page_id: id,
                creator: creator(2),
            })
            .unwrap();
        assert!(service.script_policies.get(&first, &first_owner).is_ok());
        assert!(
            service
                .authorize_inline_script_with_policy(
                    id,
                    Some(&creator(1)),
                    "0",
                    None,
                    true,
                    Some(&first)
                )
                .is_err()
        );
        assert!(
            service
                .authorize_inline_script_with_policy(
                    id,
                    Some(&creator(2)),
                    "0",
                    None,
                    true,
                    Some(&first)
                )
                .is_err()
        );
        context
            .handle_message_wire(id, MessageRequest::DiscardDocument { token: parked })
            .unwrap();
        service
            .handle(ContextServiceRequest::CaptureScriptPolicy {
                page_id: id,
                creator: creator(2),
            })
            .unwrap();
        assert!(service.script_policies.get(&first, &first_owner).is_err());
    }
    #[test]
    fn captured_report_only_policy_keeps_original_disposition_and_creator() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(
                id,
                "https://root.test/".into(),
                &[(
                    "Content-Security-Policy-Report-Only".into(),
                    "script-src 'none'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        let ContextServiceResponse::CapturedScriptPolicy { authority } = service
            .handle(ContextServiceRequest::CaptureScriptPolicy {
                page_id: id,
                creator: creator.clone(),
            })
            .unwrap()
        else {
            panic!("capture")
        };
        service
            .handle(ContextServiceRequest::InstallRealmMetadata {
                page_id: id,
                creator: creator.clone(),
                meta_csp: Some("script-src 'none'".into()),
                meta_referrer: None,
            })
            .unwrap();
        assert!(
            service
                .authorize_inline_script_with_policy(
                    id,
                    Some(&creator),
                    "0",
                    None,
                    true,
                    Some(&authority)
                )
                .is_ok()
        );
        let events = context.poll_context_events(id, &creator, 0, 100).unwrap();
        let violations = events
            .iter()
            .filter_map(|record| match &record.event {
                ContextEvent::CspViolation {
                    target_creator,
                    violation,
                    ..
                } => Some((target_creator, violation)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].0.as_ref(), Some(&creator));
        assert_eq!(
            violations[0].1.disposition,
            crate::csp::CspDisposition::Report
        );
        assert_eq!(violations[0].1.original_policy, "script-src 'none'");
        assert_eq!(violations[0].1.blocked_uri, "inline");
        assert_eq!(violations[0].1.document_uri, "https://root.test/");
    }
    #[test]
    fn captured_external_policy_is_used_at_every_redirect() {
        struct Scripts(Arc<Mutex<Vec<String>>>);
        impl NetworkBroker for Scripts {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.url.clone());
                let redirect = request.url.ends_with("start.js");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if redirect { 302 } else { 200 },
                    headers: if redirect {
                        vec![("location".into(), "/final.js".into())]
                    } else {
                        vec![("content-type".into(), "text/javascript".into())]
                    },
                    body: if redirect {
                        Vec::new()
                    } else {
                        b"void 0;".to_vec()
                    },
                })
            }
        }
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(
                id,
                "https://root.test/".into(),
                &[(
                    "Content-Security-Policy".into(),
                    "script-src 'nonce-good' 'strict-dynamic'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let service = ParentContextService::new_with_network(
            context.clone(),
            id,
            Some(SharedNetworkBroker::new(Scripts(seen.clone()))),
        );
        let ContextServiceResponse::CapturedScriptPolicy { authority } = service
            .handle(ContextServiceRequest::CaptureScriptPolicy {
                page_id: id,
                creator: creator.clone(),
            })
            .unwrap()
        else {
            panic!("capture")
        };
        service
            .handle(ContextServiceRequest::InstallRealmMetadata {
                page_id: id,
                creator: creator.clone(),
                meta_csp: Some("script-src 'none'".into()),
                meta_referrer: None,
            })
            .unwrap();
        let request = |script_policy| ContextServiceRequest::LoadScript {
            fallback_encoding: None,
            parser_inserted: false,
            page_id: id,
            creator: creator.clone(),
            script_policy,
            url: "https://root.test/start.js".into(),
            module: false,
            nonce: None,
            integrity: None,
            cross_origin: None,
            initiator_url: None,
            referrer_policy: None,
            referrer_source: None,
        };
        assert!(matches!(
            service.handle(request(None)).unwrap(),
            ContextServiceResponse::ScriptLoadFailed { .. }
        ));
        assert!(seen.lock().unwrap().is_empty());
        let mut parser_request = request(Some(authority.clone()));
        if let ContextServiceRequest::LoadScript {
            parser_inserted, ..
        } = &mut parser_request
        {
            *parser_inserted = true;
        }
        assert!(matches!(
            service.handle(parser_request).unwrap(),
            ContextServiceResponse::ScriptLoadFailed { .. }
        ));
        assert!(
            seen.lock().unwrap().is_empty(),
            "parser provenance must not inherit dynamic trust"
        );
        assert!(matches!(
            service.handle(request(Some(authority))).unwrap(),
            ContextServiceResponse::ScriptSource { .. }
        ));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "https://root.test/start.js".to_owned(),
                "https://root.test/final.js".to_owned()
            ]
        );
    }
    #[test]
    fn inline_frame_policy_is_separate_from_root_and_requires_live_creator() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://root.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let token = "c".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: token.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let policy = context
            .prepare_frame_worker_policy(
                id,
                "https://child.test/",
                &[(
                    "Content-Security-Policy".into(),
                    "script-src 'nonce-good'".into(),
                )],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(id, token.clone(), 2, 0, "https://child.test/", &policy)
            .unwrap();
        let child = Creator::Window {
            target: WindowTarget::Frame(token.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        assert!(
            service
                .authorize_inline_script(id, None, "0", None, true)
                .is_err()
        );
        assert!(matches!(
            service.authorize_inline_script(
                id,
                Some(&root),
                "globalThis.rootAllowed=true",
                None,
                true
            ),
            Ok(ContextServiceResponse::ScriptAuthorized)
        ));
        assert!(matches!(
            service.authorize_inline_script(
                id,
                Some(&child),
                "globalThis.childAllowed=true",
                None,
                true
            ),
            Ok(ContextServiceResponse::ScriptLoadFailed {
                failure: crate::script_load_failure::ScriptLoadFailure::Fetch(_)
            })
        ));
        assert!(matches!(
            service.authorize_inline_script(
                id,
                Some(&child),
                "globalThis.childAllowed=true",
                Some("good"),
                true
            ),
            Ok(ContextServiceResponse::ScriptAuthorized)
        ));
        let stale = Creator::Window {
            target: WindowTarget::Frame(token.clone()),
            document_generation: 2,
            stream_epoch: 1,
        };
        assert!(
            service
                .authorize_inline_script(id, Some(&stale), "0", Some("good"), true)
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame: token,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            service
                .authorize_inline_script(id, Some(&child), "0", Some("good"), true)
                .is_err()
        );
    }
}

#[cfg(test)]
mod realm_trusted_types_authority_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        trusted_types::PolicyCheck,
        worker_wire::Creator,
    };
    fn allowed(value: ContextServiceResponse) -> bool {
        match value {
            ContextServiceResponse::TrustedTypesAuthorization { allowed } => allowed,
            _ => panic!("typed Trusted Types receipt"),
        }
    }
    #[test]
    fn trusted_types_reports_remain_with_exact_frame_lifetime() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://root.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let frame = "9".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                id,
                "https://child.test/",
                &[
                    (
                        "Content-Security-Policy".into(),
                        "trusted-types allowed".into(),
                    ),
                    (
                        "Content-Security-Policy-Report-Only".into(),
                        "require-trusted-types-for 'script'; trusted-types report-name".into(),
                    ),
                ],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
            .unwrap();
        let child = Creator::Window {
            target: WindowTarget::Frame(frame.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        assert!(!allowed(
            service
                .authorize_trusted_types(id, &root, &PolicyCheck::SinkRequired)
                .unwrap()
        ));
        assert!(allowed(
            service
                .authorize_trusted_types(id, &child, &PolicyCheck::SinkRequired)
                .unwrap()
        ));
        assert!(allowed(
            service
                .authorize_trusted_types(
                    id,
                    &child,
                    &PolicyCheck::Sink {
                        sink: "Element innerHTML".into(),
                        input: "raw".into(),
                        location: Default::default()
                    }
                )
                .unwrap()
        ));
        assert!(allowed(
            service
                .authorize_trusted_types(
                    id,
                    &child,
                    &PolicyCheck::Policy {
                        name: "allowed".into(),
                        already_created: false,
                        location: Default::default()
                    }
                )
                .unwrap()
        ));
        assert!(!allowed(
            service
                .authorize_trusted_types(
                    id,
                    &child,
                    &PolicyCheck::Policy {
                        name: "report-name".into(),
                        already_created: false,
                        location: Default::default()
                    }
                )
                .unwrap()
        ));
        let events = context.poll_context_events(id, &child, 0, 100).unwrap();
        let violations = events
            .iter()
            .filter_map(|record| match &record.event {
                ContextEvent::CspViolation {
                    target_creator: Some(creator),
                    violation,
                    ..
                } if creator == &child => Some(violation),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(violations.len(), 3);
        assert_eq!(violations[0].sample, "Element innerHTML|raw");
        assert_eq!(
            violations[0].disposition,
            crate::csp::CspDisposition::Report
        );
        assert_eq!(violations[1].sample, "allowed");
        assert_eq!(
            violations[2].disposition,
            crate::csp::CspDisposition::Enforce
        );
        assert!(
            !context
                .poll_context_events(id, &root, 0, 100)
                .unwrap()
                .iter()
                .any(|record| matches!(&record.event, ContextEvent::CspViolation { .. }))
        );
        let stale = Creator::Window {
            target: WindowTarget::Frame(frame.clone()),
            document_generation: 2,
            stream_epoch: 1,
        };
        assert!(
            service
                .authorize_trusted_types(id, &stale, &PolicyCheck::SinkRequired)
                .is_err()
        );
        let other = context.new_page();
        assert!(
            service
                .authorize_trusted_types(other.page_id(), &root, &PolicyCheck::SinkRequired)
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            service
                .authorize_trusted_types(id, &child, &PolicyCheck::SinkRequired)
                .is_err()
        );
    }
}

#[cfg(test)]
mod realm_inline_style_authority_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    fn allowed(value: ContextServiceResponse) -> bool {
        match value {
            ContextServiceResponse::InlineStyleAuthorization { allowed } => allowed,
            _ => panic!("typed inline style receipt"),
        }
    }
    #[test]
    fn frame_style_element_attribute_and_reports_use_exact_native_creator() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://root.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let token = "d".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: token.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let policy = context
            .prepare_frame_worker_policy(
                id,
                "https://child.test/",
                &[
                    (
                        "Content-Security-Policy".into(),
                        "style-src-elem 'nonce-good'; style-src-attr 'none'".into(),
                    ),
                    (
                        "Content-Security-Policy-Report-Only".into(),
                        "style-src-elem 'none'".into(),
                    ),
                ],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(id, token.clone(), 2, 0, "https://child.test/", &policy)
            .unwrap();
        let child = Creator::Window {
            target: WindowTarget::Frame(token.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        assert!(allowed(
            service
                .authorize_inline_style(id, &root, "body{color:red}", None, false)
                .unwrap()
        ));
        assert!(!allowed(
            service
                .authorize_inline_style(id, &child, "body{color:red}", None, false)
                .unwrap()
        ));
        assert!(allowed(
            service
                .authorize_inline_style(id, &child, "body{color:red}", Some("good"), false)
                .unwrap()
        ));
        assert!(!allowed(
            service
                .authorize_inline_style(id, &child, "color:red", Some("good"), true)
                .unwrap()
        ));
        let events = context.poll_context_events(id, &child, 0, 100).unwrap();
        assert!(events.iter().any(|record|matches!(&record.event,ContextEvent::CspViolation{target_creator:Some(creator),violation,..} if creator==&child && violation.effective_directive=="style-src-attr" && violation.blocked_uri=="inline")));
        assert!(events.iter().any(|record|matches!(&record.event,ContextEvent::CspViolation{violation,..} if violation.disposition==crate::csp::CspDisposition::Report)));
        assert!(
            !context
                .poll_context_events(id, &root, 0, 100)
                .unwrap()
                .iter()
                .any(|record| matches!(&record.event, ContextEvent::CspViolation { .. }))
        );
        let stale = Creator::Window {
            target: WindowTarget::Frame(token.clone()),
            document_generation: 2,
            stream_epoch: 1,
        };
        assert!(
            service
                .authorize_inline_style(id, &stale, "x", Some("good"), false)
                .is_err()
        );
        let other = context.new_page();
        assert!(
            service
                .authorize_inline_style(other.page_id(), &root, "x", None, false)
                .is_err()
        );
        assert!(
            service
                .authorize_inline_style(
                    id,
                    &root,
                    &"x".repeat(MAX_PAGE_SCRIPT_BYTES + 1),
                    None,
                    false
                )
                .is_err()
        );
        assert!(
            service
                .authorize_inline_style(id, &root, "x", Some(&"n".repeat(4097)), false)
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame: token,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            service
                .authorize_inline_style(id, &child, "x", Some("good"), false)
                .is_err()
        );
    }
    #[test]
    fn inherited_srcdoc_style_policy_remains_native_and_attribute_nonces_do_not_authorize() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(
                id,
                "https://root.test/".into(),
                &[(
                    "Content-Security-Policy".into(),
                    "style-src 'nonce-inherited'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "e".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://root.test".into(),
                    inherit_origin: Some(WindowTarget::Root),
                },
            )
            .unwrap();
        context
            .inherit_frame_worker_policy_at_url(
                crate::runtime::worker_service::InheritedFramePolicyInputs {
                    page: id,
                    frame: frame.clone(),
                    generation: 2,
                    epoch: 0,
                    parent: &WindowTarget::Root,
                    parent_generation: 1,
                    parent_epoch: 0,
                    document_url: "about:srcdoc",
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Frame(frame),
            document_generation: 2,
            stream_epoch: 0,
        };
        let service = ParentContextService::new(context.clone(), id);
        assert!(!allowed(
            service
                .authorize_inline_style(id, &creator, "body{}", None, false)
                .unwrap()
        ));
        assert!(allowed(
            service
                .authorize_inline_style(id, &creator, "body{}", Some("inherited"), false)
                .unwrap()
        ));
        assert!(!allowed(
            service
                .authorize_inline_style(id, &creator, "color:red", Some("inherited"), true)
                .unwrap()
        ));
    }
}

#[cfg(test)]
mod websocket_close_receipt_tests {
    use super::*;
    #[test]
    fn close_receipt_uses_peer_handshake_and_cancellation_is_unclean() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut socket = tungstenite::WebSocket::from_raw_socket(
                stream,
                tungstenite::protocol::Role::Server,
                None,
            );
            let frame = socket.read().unwrap();
            assert!(
                matches!(frame,Message::Close(Some(frame))if u16::from(frame.code)==1000&&frame.reason=="reason")
            );
            // Tungstenite's server flush sends the queued close reply, then
            // reports terminal ConnectionClosed once the handshake is complete.
            assert!(matches!(
                socket.flush(),
                Err(tungstenite::Error::ConnectionClosed)
            ));
        });
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        let mut socket = WebSocket::from_raw_socket(
            MaybeTlsStream::Plain(stream),
            tungstenite::protocol::Role::Client,
            None,
        );
        assert_eq!(
            finish_websocket_close(&mut socket, None, "reason".into(), || false),
            (1000, "reason".into(), true)
        );
        peer.join().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let mut socket = WebSocket::from_raw_socket(
            MaybeTlsStream::Plain(stream),
            tungstenite::protocol::Role::Client,
            None,
        );
        assert_eq!(
            finish_websocket_close(&mut socket, Some(1000), String::new(), || true),
            (1006, String::new(), false)
        );
    }
}

#[cfg(test)]
mod websocket_handshake_custody_tests {
    use super::*;
    fn scope() -> Arc<SocketLifecycle> {
        Arc::new(SocketLifecycle {
            page: PageId(1),
            handshakes: Mutex::new(BTreeMap::new()),
            retired: Mutex::new(BTreeSet::new()),
            sealed: AtomicBool::new(false),
            sockets: Arc::new(Mutex::new(BTreeMap::new())),
            owners: Arc::new(Mutex::new(BTreeMap::new())),
            audits: Arc::new(Mutex::new(BTreeMap::new())),
            streams: Arc::new(Mutex::new(BTreeMap::new())),
            http_cleanup: http_cleanup_custody::Scope::default(),
            cancelled_http_requests: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }
    fn owner(epoch: u64) -> crate::agent_registry::AgentOwner {
        crate::agent_registry::AgentOwner::Window {
            page: PageId(1),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation: 1,
            epoch,
        }
    }
    #[test]
    fn reset_network_drain_counts_ordinary_http_streams_and_unattached_admissions() {
        let sibling = scope();
        let scope = scope();
        let network = SharedNetworkBroker::new(crate::network::UnavailableBroker);
        let pending =
            http_cleanup_custody::Ticket::reserve(&scope.http_cleanup, 17, network).unwrap();
        assert!(!scope.is_drained().unwrap());
        assert!(sibling.is_drained().unwrap());
        let (_sender, events) = tokio::sync::mpsc::channel(1);
        let entry = Arc::new(HttpStreamEntry {
            support: Mutex::new(None),
            keepalive_context: None,
            keepalive_drain: None,
            page_id: PageId(1),
            creator: Mutex::new(None),
            prepared_frame_policy: None,
            frame_deadline: None,
            request_id: 17,
            cleanup: None,
            state: Mutex::new(HttpStreamState {
                preload: None,
                operation: Some(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: 17,
                    events,
                    demand: None,
                }),
                current: bytes::Bytes::new(),
                offset: 0,
                bytes_read: 0,
                content_length: None,
            }),
        });
        scope.streams.lock().unwrap().insert(17, entry);
        drop(pending);
        assert!(
            !scope.is_drained().unwrap(),
            "ordinary HTTP stream still owns its response"
        );
        scope.streams.lock().unwrap().remove(&17);
        assert!(scope.is_drained().unwrap());
        assert!(sibling.is_drained().unwrap());
    }
    #[test]
    fn handshake_cancel_before_and_after_connect_is_exact_and_retirement_scoped() {
        let scope = scope();
        scope.abort_request(1).unwrap();
        let early = SocketHandshakeLease::begin(scope.clone(), 1, owner(0)).unwrap();
        assert!(early.check().is_err());
        drop(early);
        let active = SocketHandshakeLease::begin(scope.clone(), 2, owner(0)).unwrap();
        let sibling = SocketHandshakeLease::begin(scope.clone(), 3, owner(1)).unwrap();
        assert!(SocketHandshakeLease::begin(scope.clone(), 2, owner(0)).is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        active.attach(&stream).unwrap();
        scope.abort_request(2).unwrap();
        assert!(active.check().is_err());
        assert!(sibling.check().is_ok());
        let mut byte = [0];
        assert_eq!(std::io::Read::read(&mut peer, &mut byte).unwrap(), 0);
        scope.retire(Some(&owner(0))).unwrap();
        assert!(sibling.check().is_ok());
        assert!(SocketHandshakeLease::begin(scope.clone(), 4, owner(0)).is_err());
        scope.retire(None).unwrap();
        assert!(sibling.check().is_err());
        assert!(!scope.is_drained().unwrap());
        drop(active);
        drop(sibling);
        assert!(scope.handshakes.lock().unwrap().is_empty());
        assert!(scope.is_drained().unwrap());
    }
}

// Both native transports offer protocols only; no extension negotiation is enabled.
pub(crate) fn validate_websocket_negotiation(
    headers: &tungstenite::http::HeaderMap,
    protocols: &[String],
) -> Result<String, String> {
    let values = headers.get_all("sec-websocket-protocol");
    let mut selected = values.iter();
    let protocol = selected
        .next()
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| "invalid WebSocket protocol response".to_string())?
        .unwrap_or_default();
    if selected.next().is_some()
        || (!protocol.is_empty() && !protocols.contains(&protocol))
        || (protocol.is_empty() && !protocols.is_empty())
        || headers.contains_key("sec-websocket-extensions")
    {
        return Err("unrequested or missing WebSocket negotiation response".into());
    }
    Ok(protocol)
}
#[cfg(test)]
mod websocket_negotiation_tests {
    use super::validate_websocket_negotiation;
    #[test]
    fn negotiation_requires_exact_single_offered_protocol_and_no_extensions() {
        let mut headers = tungstenite::http::HeaderMap::new();
        assert_eq!(validate_websocket_negotiation(&headers, &[]).unwrap(), "");
        let offered = vec!["FOO".to_string()];
        assert!(validate_websocket_negotiation(&headers, &offered).is_err());
        headers.insert("sec-websocket-protocol", "foo".parse().unwrap());
        assert!(validate_websocket_negotiation(&headers, &offered).is_err());
        headers.insert("sec-websocket-protocol", "FOO".parse().unwrap());
        assert_eq!(
            validate_websocket_negotiation(&headers, &offered).unwrap(),
            "FOO"
        );
        assert!(validate_websocket_negotiation(&headers, &[]).is_err());
        headers.append("sec-websocket-protocol", "FOO".parse().unwrap());
        assert!(validate_websocket_negotiation(&headers, &offered).is_err());
        headers.remove("sec-websocket-protocol");
        headers.insert(
            "sec-websocket-extensions",
            "permessage-deflate".parse().unwrap(),
        );
        assert!(validate_websocket_negotiation(&headers, &[]).is_err());
    }
}

#[cfg(test)]
mod selected_navigation_request_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    #[test]
    fn blank_prerender_final_selection_matches_get_url_without_queueing_another_load() {
        for (method, defer, noopener, matching, form) in [
            ("GET", false, true, true, true),
            ("GET", false, true, true, false),
            ("POST", false, true, true, true),
            ("GET", true, true, true, true),
            ("GET", false, false, true, true),
            ("GET", false, true, false, true),
        ] {
            let context = crate::Browser::native_test_with_config(Default::default()).new_context();
            let source = context.new_page();
            let id = source.page_id();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(
                    source.load_html("https://selection.test/", "<!doctype html><p>source</p>"),
                )
                .unwrap();
            let creator = context.capture_command_navigation_source(id).unwrap();
            let candidate_url = "https://selection.test/form?x=one";
            let (candidate, _, scope) = context
                .reserve_blank_prerender(id, &creator, candidate_url)
                .unwrap();
            let provenance = context
                .capture_document_source(
                    id,
                    creator.clone(),
                    "https://selection.test/",
                    "no-referrer",
                )
                .unwrap();
            let form_authority = (method == "POST")
                .then(|| context.document_form_authority(id, &provenance).unwrap());
            let service = ParentContextService::new(context.clone(), id);
            let response = service
                .handle(ContextServiceRequest::OpenPage {
                    hyperlink_audit: None,
                    local_inventory: Default::default(),
                    link_navigation: false,
                    user_initiated: false,
                    defer_navigation: defer,
                    local_related_navigation: false,
                    selected_target: None,
                    document_source: form.then_some(provenance),
                    opener_page_id: id,
                    creator: Some(creator.clone()),
                    noopener,
                    target_name: "_BLANK".into(),
                    form_authority,
                    url: Some(
                        if matching {
                            candidate_url
                        } else {
                            "https://selection.test/form?x=two"
                        }
                        .into(),
                    ),
                    post: (method == "POST").then(|| oryn_js::navigation::PostResource {
                        bytes: b"x=one".to_vec(),
                        content_type: "application/x-www-form-urlencoded".into(),
                    }),
                    referrer_policy: None,
                })
                .unwrap();
            let ContextServiceResponse::PageOpened {
                page_id,
                created,
                blank_prerender,
                related_root,
                related_navigation_url,
                ..
            } = response
            else {
                panic!("expected final selection")
            };
            assert!(
                related_navigation_url.is_none(),
                "form and deferred selection retain parent-owned navigation"
            );
            if method == "GET" && !defer && noopener && matching {
                let receipt = blank_prerender.expect("matching candidate receipt");
                assert_eq!(receipt.page_id, candidate);
                assert_eq!(page_id, candidate);
                assert_eq!(receipt.token, scope.token);
                assert_eq!(receipt.url, candidate_url);
                assert!(!created);
                assert!(related_root.is_none());
                assert!(
                    context
                        .claim_blank_prerender(id, &creator, None, candidate_url)
                        .unwrap()
                        .is_none(),
                    "claim is consumed once"
                );
            } else {
                assert!(blank_prerender.is_none());
                assert!(created);
                assert_ne!(page_id, candidate);
                assert_eq!(
                    context
                        .reserve_blank_prerender_target(id, &creator, candidate_url)
                        .unwrap(),
                    Some(candidate),
                    "ineligible navigation leaves candidate available"
                );
            }
            assert!(
                context.hidden_prerender_page(candidate),
                "only actor promotion publishes the candidate"
            );
            let pending = context.queued_navigation_status(candidate).unwrap();
            assert_eq!(pending.active, 0);
            assert!(pending.outcomes.is_empty());
        }
    }
    #[test]
    fn related_window_open_transfers_only_nonblank_initial_navigation() {
        for destination in ["about:blank", "https://selection.test/child"] {
            let context = crate::Browser::native_test_with_config(Default::default()).new_context();
            let source = context.new_page();
            let id = source.page_id();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(source.load_html(
                    "https://selection.test/source",
                    "<!doctype html><body>source</body>",
                ))
                .unwrap();
            let creator = context.capture_command_navigation_source(id).unwrap();
            let service = ParentContextService::new(context.clone(), id);
            let response = service
                .handle(ContextServiceRequest::OpenPage {
                    hyperlink_audit: None,
                    local_inventory: Default::default(),
                    link_navigation: false,
                    user_initiated: false,
                    defer_navigation: false,
                    local_related_navigation: true,
                    selected_target: None,
                    document_source: None,
                    opener_page_id: id,
                    creator: Some(creator),
                    noopener: false,
                    target_name: "_blank".into(),
                    form_authority: None,
                    url: Some(destination.into()),
                    post: None,
                    referrer_policy: None,
                })
                .unwrap();
            let ContextServiceResponse::PageOpened {
                page_id,
                created,
                related_root,
                related_navigation_url,
                ..
            } = response
            else {
                panic!("popup response")
            };
            assert!(created);
            assert!(related_root.is_some());
            assert_eq!(
                related_navigation_url.as_deref(),
                (destination != "about:blank").then_some(destination)
            );
            let queue = context.queued_navigation_status(page_id).unwrap();
            assert_eq!(queue.active, 0);
            assert!(queue.outcomes.is_empty());
            // No actor has received this test response: revoke the unconstructed
            // admitted root through the same parent rollback used by creation.
            ParentContextService::new(context.clone(), page_id)
                .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id,
                    expected_generation: None,
                })
                .unwrap();
        }
    }

    #[test]
    fn selection_only_keeps_captured_page_and_does_not_queue_a_load() {
        let context = crate::Browser::new().new_context();
        let source = context.new_page();
        let id = source.page_id();
        context
            .install_document_security(id, "https://selection.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let destination = context.new_popup_from_creator(id, &creator).unwrap();
        context
            .native_initial_document_creator(destination.page_id())
            .unwrap();
        let receipt = context
            .capture_navigation_target(id, &creator, destination.page_id(), None)
            .unwrap();
        let service = ParentContextService::new(context.clone(), id);
        let response = service
            .handle(ContextServiceRequest::OpenPage {
                hyperlink_audit: None,
                local_inventory: Default::default(),
                link_navigation: false,
                user_initiated: false,
                defer_navigation: true,
                local_related_navigation: false,
                selected_target: Some(receipt.clone()),
                document_source: None,
                opener_page_id: id,
                creator: Some(creator.clone()),
                noopener: false,
                target_name: "entirely-different-name".into(),
                form_authority: None,
                url: Some("https://selection.test/later".into()),
                post: None,
                referrer_policy: None,
            })
            .unwrap();
        let ContextServiceResponse::PageOpened {
            page_id,
            created,
            selected_target,
            ..
        } = response
        else {
            panic!("selection response")
        };
        assert_eq!(page_id, destination.page_id());
        assert!(!created);
        assert_eq!(selected_target, Some(receipt));
        let pending = context
            .queued_navigation_status(destination.page_id())
            .unwrap();
        assert_eq!(pending.active, 0);
        assert!(pending.outcomes.is_empty());
        assert_eq!(
            context
                .pages()
                .iter()
                .find(|page| page.page.page_id == destination.page_id())
                .unwrap()
                .page
                .url,
            "about:blank"
        );
        let target = context
            .root_navigation_creator(destination.page_id())
            .unwrap();
        let response = service
            .handle(ContextServiceRequest::IssueScriptNavigationAuthority {
                page_id: id,
                target_page_id: Some(destination.page_id()),
                creator: creator.clone(),
                target: target.clone(),
                url: "https://selection.test/later".into(),
                source_url: "https://selection.test/".into(),
                referrer_policy: "no-referrer".into(),
            })
            .unwrap();
        let ContextServiceResponse::ScriptNavigationAuthority { authority } = response else {
            panic!("script authority response")
        };
        assert!(
            context
                .script_navigation_source(
                    &authority,
                    destination.page_id(),
                    &target,
                    "https://selection.test/later"
                )
                .is_ok()
        );
        assert!(
            context
                .script_navigation_source(&authority, id, &target, "https://selection.test/later")
                .is_err(),
            "destination authority must not collapse to the source page"
        );
        let source = context
            .capture_document_source(id, creator, "https://selection.test/", "no-referrer")
            .unwrap();
        assert!(matches!(
            service
                .handle(ContextServiceRequest::CheckDocumentTargetNavigation {
                    page_id: id,
                    target_page_id: Some(destination.page_id()),
                    source: source.clone(),
                    target
                })
                .unwrap(),
            ContextServiceResponse::NavigationAuthorized
        ));
        let authority = context.document_form_authority(id, &source).unwrap();
        let response = service
            .handle(ContextServiceRequest::RetargetFormNavigationAuthority {
                page_id: id,
                target_page_id: destination.page_id(),
                authority,
            })
            .unwrap();
        let ContextServiceResponse::FormNavigationAuthority { authority } = response else {
            panic!("retargeted form authority response")
        };
        assert_eq!(
            context
                .form_fetch_source(&authority, destination.page_id())
                .unwrap()
                .url,
            "https://selection.test/"
        );
        assert!(context.form_fetch_source(&authority, id).is_err());
    }
}

#[cfg(test)]
mod http_terminal_receipt_tests {
    use super::*;
    use crate::Browser;
    fn entry(
        request_id: u64,
        length: Option<usize>,
        events: tokio::sync::mpsc::Receiver<NetworkEvent>,
        page: PageId,
    ) -> Arc<HttpStreamEntry> {
        Arc::new(HttpStreamEntry {
            support: Mutex::new(None),
            keepalive_context: None,
            keepalive_drain: None,
            page_id: page,
            creator: Mutex::new(None),
            prepared_frame_policy: None,
            frame_deadline: None,
            request_id,
            cleanup: None,
            state: Mutex::new(HttpStreamState {
                preload: None,
                operation: Some(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id,
                    events,
                    demand: None,
                }),
                current: bytes::Bytes::new(),
                offset: 0,
                bytes_read: 0,
                content_length: length,
            }),
        })
    }
    #[test]
    fn measured_encoded_size_is_published_only_with_exact_stream_terminal() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let service = ParentContextService::new_with_network(context, id, None);
        let (sender, events) = tokio::sync::mpsc::channel(4);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 1,
                bytes: b"decoded".as_slice().into(),
            })
            .unwrap();
        service
            .http_streams
            .lock()
            .unwrap()
            .insert(1, entry(1, None, events, id));
        let ContextServiceResponse::HttpStreamChunk {
            done,
            encoded_body_size,
            ..
        } = service.read_http_stream(id, 1, 7).unwrap()
        else {
            panic!("prefix")
        };
        assert!(!done);
        assert_eq!(encoded_body_size, None);
        sender
            .try_send(NetworkEvent::Complete {
                request_id: 1,
                encoded_body_size: Some(31),
            })
            .unwrap();
        let ContextServiceResponse::HttpStreamChunk {
            done,
            encoded_body_size,
            ..
        } = service.read_http_stream(id, 1, 7).unwrap()
        else {
            panic!("terminal")
        };
        assert!(done);
        assert_eq!(encoded_body_size, Some(31));
        assert!(service.read_http_stream(id, 1, 7).is_err());
    }
    #[test]
    fn content_length_does_not_release_stream_before_terminal_ack() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let service = ParentContextService::new_with_network(context, id, None);
        let (sender, events) = tokio::sync::mpsc::channel(4);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 1,
                bytes: b"a".as_slice().into(),
            })
            .unwrap();
        service
            .http_streams
            .lock()
            .unwrap()
            .insert(1, entry(1, Some(1), events, id));
        let ContextServiceResponse::HttpStreamChunk { done, .. } =
            service.read_http_stream(id, 1, 1).unwrap()
        else {
            panic!("body")
        };
        assert!(!done);
        assert!(service.http_streams.lock().unwrap().contains_key(&1));
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: 1,
            })
            .unwrap();
        let ContextServiceResponse::HttpStreamChunk { done, .. } =
            service.read_http_stream(id, 1, 1).unwrap()
        else {
            panic!("terminal")
        };
        assert!(done);
        assert!(!service.http_streams.lock().unwrap().contains_key(&1));
    }
    #[test]
    fn early_terminal_extra_bytes_and_disconnect_fail_closed() {
        for (length, bytes, complete) in [
            (Some(2), b"a".as_slice(), true),
            (Some(1), b"ab".as_slice(), true),
            (Some(1), b"a".as_slice(), false),
        ] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let id = page.page_id();
            let service = ParentContextService::new_with_network(context, id, None);
            let (sender, events) = tokio::sync::mpsc::channel(4);
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id: 1,
                    bytes: bytes.to_vec().into(),
                })
                .unwrap();
            if complete {
                sender
                    .try_send(NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: 1,
                    })
                    .unwrap();
            }
            drop(sender);
            service
                .http_streams
                .lock()
                .unwrap()
                .insert(1, entry(1, length, events, id));
            assert!(service.read_http_stream(id, 1, 64).is_err());
            assert!(!service.http_streams.lock().unwrap().contains_key(&1));
        }
    }
    #[test]
    fn late_extra_body_is_rejected_after_length_sized_read() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let service = ParentContextService::new_with_network(context, id, None);
        let (sender, events) = tokio::sync::mpsc::channel(4);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 1,
                bytes: b"a".as_slice().into(),
            })
            .unwrap();
        service
            .http_streams
            .lock()
            .unwrap()
            .insert(1, entry(1, Some(1), events, id));
        let ContextServiceResponse::HttpStreamChunk { done, .. } =
            service.read_http_stream(id, 1, 1).unwrap()
        else {
            panic!("body")
        };
        assert!(!done);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 1,
                bytes: b"b".as_slice().into(),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: 1,
            })
            .unwrap();
        assert!(service.read_http_stream(id, 1, 64).is_err());
        assert!(!service.http_streams.lock().unwrap().contains_key(&1));
    }
    #[test]
    fn zero_length_requires_terminal_and_rejects_body() {
        for extra in [false, true] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let id = page.page_id();
            let service = ParentContextService::new_with_network(context, id, None);
            let (sender, events) = tokio::sync::mpsc::channel(4);
            if extra {
                sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: 1,
                        bytes: b"a".as_slice().into(),
                    })
                    .unwrap();
            }
            sender
                .try_send(NetworkEvent::Complete {
                    encoded_body_size: None,
                    request_id: 1,
                })
                .unwrap();
            service
                .http_streams
                .lock()
                .unwrap()
                .insert(1, entry(1, Some(0), events, id));
            let result = service.read_http_stream(id, 1, 64);
            assert_eq!(result.is_err(), extra);
            assert!(!service.http_streams.lock().unwrap().contains_key(&1));
        }
    }
    #[test]
    fn preload_bytes_are_published_only_after_exact_terminal() {
        for outcome in ["complete", "short", "extra", "disconnect", "cancel"] {
            let context = Browser::in_process_probe().new_context();
            let page = context.new_page();
            let id = page.page_id();
            let service = ParentContextService::new_with_network(context, id, None);
            let key = crate::preload_cache::Key {
                page: id,
                creator: oryn_js::worker_wire::Creator::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    document_generation: 1,
                    stream_epoch: 0,
                },
                url: "https://example.test/image".into(),
                cors: false,
                isolation: Default::default(),
                credentials: "include".into(),
                integrity: String::new(),
                range: None,
            };
            let (sender, events) = tokio::sync::mpsc::channel(4);
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id: 1,
                    bytes: b"a".as_slice().into(),
                })
                .unwrap();
            let stream = entry(1, Some(if outcome == "short" { 2 } else { 1 }), events, id);
            stream.state.lock().unwrap().preload = Some(crate::preload_cache::Candidate {
                key: key.clone(),
                response: crate::preload_cache::Response {
                    timing_allowed: true,
                    hops: vec![key.url.clone()],
                    response: oryn_js::worker_wire::AgentFetchResponse {
                        timing: None,
                        redirected: false,
                        range_requested: false,
                        status: 200,
                        status_text: String::new(),
                        headers: Vec::new(),
                        body: Vec::new(),
                        response_url: key.url.clone(),
                        response_type: "basic".into(),
                    },
                },
            });
            service.http_streams.lock().unwrap().insert(1, stream);
            service.read_http_stream(id, 1, 1).unwrap();
            assert!(service.preloads.take(&key).is_none());
            if outcome == "cancel" {
                service.abort_http_stream(id, 1).unwrap();
            } else {
                if outcome == "extra" {
                    sender
                        .try_send(NetworkEvent::Chunk {
                            request_id: 1,
                            bytes: b"b".as_slice().into(),
                        })
                        .unwrap();
                }
                if outcome != "disconnect" {
                    sender
                        .try_send(NetworkEvent::Complete {
                            encoded_body_size: None,
                            request_id: 1,
                        })
                        .unwrap();
                }
                drop(sender);
                assert_eq!(
                    service.read_http_stream(id, 1, 64).is_ok(),
                    outcome == "complete"
                );
            }
            let cached = service.preloads.take(&key);
            assert_eq!(cached.is_some(), outcome == "complete");
            if let Some(cached) = cached {
                assert_eq!(cached.response.body, b"a");
            }
        }
    }
    #[test]
    fn fulfilled_length_stream_remains_cancellable() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let service = ParentContextService::new_with_network(context, id, None);
        let (sender, events) = tokio::sync::mpsc::channel(4);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 1,
                bytes: b"a".as_slice().into(),
            })
            .unwrap();
        service
            .http_streams
            .lock()
            .unwrap()
            .insert(1, entry(1, Some(1), events, id));
        service.read_http_stream(id, 1, 1).unwrap();
        service.abort_http_stream(id, 1).unwrap();
        assert!(service.read_http_stream(id, 1, 1).is_err());
    }
}

#[cfg(feature = "v8-host")]
struct ContextHyperlinkAudit {
    token: oryn_js::navigation::HyperlinkAuditToken,
    ticket: Mutex<Option<script_cleanup_custody::Ticket>>,
}
#[cfg(feature = "v8-host")]
impl oryn_js::navigation::HyperlinkAudit for ContextHyperlinkAudit {
    fn token(&self) -> oryn_js::navigation::HyperlinkAuditToken {
        self.token.clone()
    }
    fn accepted(&self) {
        if let Some(ticket) = self
            .ticket
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            ticket.accept_hyperlink_audit();
            drop(ticket);
        }
    }
    fn discarded(&self) {
        self.ticket
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
    fn transferred(&self) {
        if let Some(ticket) = self
            .ticket
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            ticket.finish_if_not_cancelled();
        }
    }
}
impl SharedContextClient {
    #[cfg(feature = "v8-host")]
    pub(crate) fn prepare_hyperlink_audit(
        &self,
        page_id: PageId,
        creator: oryn_js::worker_wire::Creator,
        destination: String,
        urls: Vec<String>,
    ) -> Result<Arc<dyn oryn_js::navigation::HyperlinkAudit>, RuntimeError> {
        let mut capability = vec![0; 32];
        getrandom::fill(&mut capability)
            .map_err(|_| RuntimeError::ContextService("audit entropy unavailable".into()))?;
        let token = oryn_js::navigation::HyperlinkAuditToken {
            page_id: page_id.0,
            capability,
        };
        let ticket = script_cleanup_custody::Ticket::reserve(self.clone(), page_id)
            .map_err(RuntimeError::ContextService)?;
        ticket.attach_request(ContextServiceRequest::FinishHyperlinkAudit {
            page_id,
            token: token.clone(),
            commit: false,
        });
        match self.request(ContextServiceRequest::PrepareHyperlinkAudit {
            page_id,
            creator,
            token: token.clone(),
            destination,
            urls,
        })? {
            ContextServiceResponse::HyperlinkAuditPrepared => Ok(Arc::new(ContextHyperlinkAudit {
                token,
                ticket: Mutex::new(Some(ticket)),
            })),
            _ => Err(RuntimeError::ProtocolViolation(
                "invalid audit preparation response".into(),
            )),
        }
    }
}

pub(crate) use crate::frame_resource_response::embedded_document_mime;

#[cfg(feature = "v8-host")]
pub(crate) struct NativeDeferredFetchBackend {
    client: SharedContextClient,
    page_id: PageId,
    creator: std::rc::Rc<dyn Fn() -> Result<oryn_js::worker_wire::Creator, String>>,
    prepared_creator: std::cell::RefCell<Option<oryn_js::worker_wire::Creator>>,
}
#[cfg(feature = "v8-host")]
impl NativeDeferredFetchBackend {
    pub(crate) fn new(
        client: SharedContextClient,
        page_id: PageId,
        creator: std::rc::Rc<dyn Fn() -> Result<oryn_js::worker_wire::Creator, String>>,
    ) -> Self {
        Self {
            client,
            page_id,
            creator,
            prepared_creator: std::cell::RefCell::new(None),
        }
    }
    fn send_state(
        &self,
        request: ContextServiceRequest,
    ) -> Result<oryn_js::fetch_later::DeferredFetchState, oryn_js::fetch_later::DeferredFetchError>
    {
        match self.client.request(request) {
            Ok(ContextServiceResponse::DeferredFetchState { result }) => result,
            Ok(_) => Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                "invalid deferred Fetch state response".into(),
            )),
            Err(error) => Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                error.to_string(),
            )),
        }
    }
    fn send(
        &self,
        request: ContextServiceRequest,
    ) -> Result<(), oryn_js::fetch_later::DeferredFetchError> {
        match self.client.request(request) {
            Ok(ContextServiceResponse::DeferredFetchResult { result }) => result,
            Ok(_) => Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                "invalid deferred Fetch response".into(),
            )),
            Err(error) => Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                error.to_string(),
            )),
        }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::fetch_later::DeferredFetchBackend for NativeDeferredFetchBackend {
    fn prepare(
        &self,
        token: &oryn_js::fetch_later::DeferredFetchToken,
        request: oryn_js::platform::PlatformHttpRequest,
        activate_after: Option<std::time::Duration>,
    ) -> Result<(), oryn_js::fetch_later::DeferredFetchError> {
        let creator =
            (self.creator)().map_err(oryn_js::fetch_later::DeferredFetchError::Transport)?;
        *self.prepared_creator.borrow_mut() = Some(creator.clone());
        self.send(ContextServiceRequest::PrepareDeferredFetch {
            page_id: self.page_id,
            creator,
            token: token.clone(),
            request,
            activate_after,
        })
    }
    fn activate(
        &self,
        token: &oryn_js::fetch_later::DeferredFetchToken,
    ) -> Result<oryn_js::fetch_later::DeferredFetchState, oryn_js::fetch_later::DeferredFetchError>
    {
        self.send_state(ContextServiceRequest::ActivateDeferredFetch {
            page_id: self.page_id,
            token: token.clone(),
        })
    }
    fn cancel(
        &self,
        token: &oryn_js::fetch_later::DeferredFetchToken,
    ) -> Result<oryn_js::fetch_later::DeferredFetchState, oryn_js::fetch_later::DeferredFetchError>
    {
        let Some(creator) = self.prepared_creator.borrow().clone() else {
            return Ok(oryn_js::fetch_later::DeferredFetchState::Canceled);
        };
        self.send_state(ContextServiceRequest::CancelDeferredFetch {
            page_id: self.page_id,
            creator,
            token: token.clone(),
        })
    }
    fn acknowledge(
        &self,
        token: &oryn_js::fetch_later::DeferredFetchToken,
    ) -> Result<(), oryn_js::fetch_later::DeferredFetchError> {
        self.send(ContextServiceRequest::AcknowledgeDeferredFetch {
            page_id: self.page_id,
            token: token.clone(),
        })
    }
    fn state(
        &self,
        token: &oryn_js::fetch_later::DeferredFetchToken,
    ) -> Result<oryn_js::fetch_later::DeferredFetchState, oryn_js::fetch_later::DeferredFetchError>
    {
        self.send_state(ContextServiceRequest::QueryDeferredFetch {
            page_id: self.page_id,
            token: token.clone(),
        })
    }
}

#[cfg(test)]
mod deferred_fetch_wire_authority_tests {
    use super::*;
    use oryn_js::{
        fetch_later::{DeferredFetchError, DeferredFetchToken},
        message_wire::WindowTarget,
        worker_wire::Creator,
    };

    #[test]
    fn cancellation_wire_requires_original_document_authority() {
        let creator = Creator::Window {
            target: WindowTarget::Frame("ab".repeat(32)),
            document_generation: 37,
            stream_epoch: 91,
        };
        let mut bytes = vec![0x5a; 32];
        bytes[..8].copy_from_slice(&123_u64.to_be_bytes());
        let token = DeferredFetchToken(bytes);
        let request = ContextServiceRequest::CancelDeferredFetch {
            page_id: PageId(7),
            creator: creator.clone(),
            token: token.clone(),
        };
        let value = serde_json::to_value(request).unwrap();
        let decoded: ContextServiceRequest = serde_json::from_value(value.clone()).unwrap();
        match decoded {
            ContextServiceRequest::CancelDeferredFetch {
                page_id,
                creator: decoded,
                token: decoded_token,
            } => {
                assert_eq!(page_id, PageId(7));
                assert_eq!(decoded, creator);
                assert_eq!(decoded_token, token);
            }
            _ => panic!("cancellation changed operation"),
        }
        let mut missing_creator = value.clone();
        missing_creator.as_object_mut().unwrap().remove("creator");
        assert!(serde_json::from_value::<ContextServiceRequest>(missing_creator).is_err());
        for missing in ["document_generation", "stream_epoch", "target"] {
            let mut malformed = value.clone();
            malformed["creator"]["Window"]
                .as_object_mut()
                .unwrap()
                .remove(missing);
            assert!(
                serde_json::from_value::<ContextServiceRequest>(malformed).is_err(),
                "{missing}"
            );
        }
    }

    #[test]
    fn deferred_response_discriminator_cannot_alias_terminal_outcome() {
        // The enum discriminator is `result`; a same-named payload would
        // collide with that tag instead of preserving the typed terminal ACK.
        for outcome in [
            Ok(()),
            Err(DeferredFetchError::Quota("full".into())),
            Err(DeferredFetchError::Type("obsolete capability".into())),
            Err(DeferredFetchError::Transport("lost ACK".into())),
        ] {
            let response = ContextServiceResponse::DeferredFetchResult {
                result: outcome.clone(),
            };
            let value = serde_json::to_value(response).unwrap();
            assert_eq!(value["result"], "deferred_fetch_result");
            assert!(value.get("outcome").is_some());
            match serde_json::from_value::<ContextServiceResponse>(value.clone()).unwrap() {
                ContextServiceResponse::DeferredFetchResult { result } => {
                    assert_eq!(result, outcome)
                }
                _ => panic!("terminal ACK changed variant"),
            }
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove("outcome");
            assert!(serde_json::from_value::<ContextServiceResponse>(missing).is_err());
            for malformed in [
                serde_json::Value::Bool(true),
                serde_json::Value::Null,
                serde_json::json!({"Err": "untyped failure"}),
            ] {
                let mut invalid = value.clone();
                invalid["outcome"] = malformed;
                assert!(serde_json::from_value::<ContextServiceResponse>(invalid).is_err());
            }
        }
    }
}

#[cfg(test)]
mod compatibility_event_delivery_integration_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        platform_control::{Request, Response},
        worker_wire::Creator,
    };

    #[test]
    fn compatibility_receipt_does_not_consume_native_csp_delivery() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(
                page,
                "https://example.test/".into(),
                &[("content-security-policy".into(), "img-src 'none'".into())],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let service = ParentContextService::new_with_network(
            context.clone(),
            page,
            Some(SharedNetworkBroker::new(crate::network::UnavailableBroker)),
        );
        service
            .handle(ContextServiceRequest::ReportRealmCspRequest {
                page_id: page,
                creator: creator.clone(),
                redirected: false,
                original_url: None,
                request: crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Image,
                    url: "https://example.test/blocked.png".into(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                },
            })
            .unwrap();
        let inspect = |after| {
            service
                .handle(ContextServiceRequest::PollEvents {
                    page_id: page,
                    creator: Some(creator.clone()),
                    after_event_id: after,
                    max_events: 32,
                })
                .unwrap()
        };
        let ContextServiceResponse::Events { events } = inspect(0) else {
            panic!("inspection response")
        };
        let id = events
            .iter()
            .find(|entry| matches!(entry.event, ContextEvent::CspViolation { .. }))
            .expect("native violation")
            .event_id;
        let ContextServiceResponse::Events { events } = inspect(id) else {
            panic!("inspection ACK")
        };
        assert!(events.iter().all(|entry| entry.event_id > id));

        let Response::CspEvents { events } = context
            .native_platform_query(page, creator.clone(), Request::CspEvents { after: 0 })
            .unwrap()
        else {
            panic!("native delivery")
        };
        assert_eq!(
            events.len(),
            1,
            "inspection must not steal the native event"
        );
        assert_eq!(events[0].id, id);
        assert_eq!(events[0].blocked_uri, "https://example.test/blocked.png");
        // A lost native response retries the same event; only the native ACK
        // releases its delivery obligation.
        let Response::CspEvents { events } = context
            .native_platform_query(page, creator.clone(), Request::CspEvents { after: 0 })
            .unwrap()
        else {
            panic!("native retry")
        };
        assert_eq!(events[0].id, id);
        let Response::CspEvents { events } = context
            .native_platform_query(page, creator.clone(), Request::CspEvents { after: id })
            .unwrap()
        else {
            panic!("native ACK")
        };
        assert!(events.is_empty());
        let Response::CspEvents { events } = context
            .native_platform_query(page, creator, Request::CspEvents { after: 0 })
            .unwrap()
        else {
            panic!("stale native cursor")
        };
        assert!(
            events.is_empty(),
            "stale cursor cannot resurrect acknowledged events"
        );

        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let successor = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            context
                .native_platform_query(page, successor, Request::CspEvents { after: id })
                .is_err(),
            "successor cannot inherit delivery receipts"
        );
    }
}

#[cfg(test)]
mod deferred_fetch_state_wire_tests {
    use super::*;
    use oryn_js::fetch_later::{DeferredFetchError, DeferredFetchState};

    #[test]
    fn parent_deadline_winner_is_a_typed_state_not_an_admission_ack() {
        for outcome in [
            Ok(DeferredFetchState::Pending),
            Ok(DeferredFetchState::Activated),
            Ok(DeferredFetchState::Canceled),
            Err(DeferredFetchError::Type("expired".into())),
            Err(DeferredFetchError::Transport("retry".into())),
        ] {
            let value = serde_json::to_value(ContextServiceResponse::DeferredFetchState {
                result: outcome.clone(),
            })
            .unwrap();
            assert_eq!(value["result"], "deferred_fetch_state");
            let decoded: ContextServiceResponse = serde_json::from_value(value.clone()).unwrap();
            match decoded {
                ContextServiceResponse::DeferredFetchState { result } => {
                    assert_eq!(result, outcome)
                }
                _ => panic!("state reply became admission ACK"),
            }
            for malformed in [
                serde_json::json!({"Ok": null}),
                serde_json::json!({"Ok": true}),
                serde_json::json!({"Ok": "Unknown"}),
                serde_json::json!({"Err": "expired"}),
            ] {
                let mut invalid = value.clone();
                invalid["outcome"] = malformed;
                assert!(serde_json::from_value::<ContextServiceResponse>(invalid).is_err());
            }
        }
    }
}

/// Produced from the target's native full session history, never an author
/// history.length property. Only the authenticated current root may supply it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptCloseHistoryProof {
    pub creator: oryn_js::worker_wire::Creator,
    pub single_entry: bool,
}

#[cfg(all(test, feature = "in-process-probe"))]
mod script_close_history_tests {
    use super::*;
    use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
    #[test]
    fn close_uses_parent_projection_and_rejects_foreign_and_stale_authority() {
        let context = crate::runtime::Browser::in_process_probe().new_context();
        let page = context.new_page().page_id();
        let other = context.new_page().page_id();
        let creator = context.native_initial_document_creator(page).unwrap();
        let service = ParentContextService::new(context.clone(), page);
        let request = |target, creator| ContextServiceRequest::ClosePage {
            source_page_id: page,
            target_page_id: target,
            creator,
            expected: None,
        };
        assert!(service.handle(request(other, creator.clone())).is_err());
        // Missing projection is an infrastructure error, never a made-up
        // single-entry answer from an author history.length property.
        assert!(service.handle(request(page, creator.clone())).is_err());
        service
            .handle(ContextServiceRequest::BeginScriptCloseHistory {
                page_id: page,
                creator: creator.clone(),
                revision: 1,
                count: 2,
            })
            .unwrap();
        service
            .handle(ContextServiceRequest::PublishScriptCloseHistory {
                page_id: page,
                creator: creator.clone(),
                revision: 1,
                count: 2,
            })
            .unwrap();
        assert!(matches!(
            service.handle(request(page, creator.clone())),
            Ok(ContextServiceResponse::PageDenied { .. })
        ));
        let Creator::Window {
            document_generation,
            stream_epoch,
            ..
        } = creator
        else {
            unreachable!()
        };
        assert!(
            service
                .handle(request(
                    page,
                    Creator::Window {
                        target: WindowTarget::Root,
                        document_generation: document_generation + 1,
                        stream_epoch
                    }
                ))
                .is_err()
        );
        assert!(
            service
                .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: page,
                    expected_generation: Some(document_generation)
                })
                .is_err(),
            "ordinary host page is not auxiliary rollback custody"
        );
    }
    #[test]
    fn auxiliary_rollback_rejects_foreign_endpoint_and_replacement_generation() {
        let context = crate::runtime::Browser::in_process_probe().new_context();
        let opener = context.new_page().page_id();
        let creator = context.native_initial_document_creator(opener).unwrap();
        let popup = context
            .new_popup_from_creator(opener, &creator)
            .unwrap()
            .page_id();
        let popup_creator = context.native_initial_document_creator(popup).unwrap();
        let Creator::Window {
            document_generation,
            ..
        } = popup_creator
        else {
            unreachable!()
        };
        let source = ParentContextService::new(context.clone(), opener);
        assert!(
            source
                .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: popup,
                    expected_generation: Some(document_generation)
                })
                .is_err()
        );
        let target = ParentContextService::new(context.clone(), popup);
        assert!(
            target
                .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: popup,
                    expected_generation: Some(document_generation + 1)
                })
                .is_err()
        );
        context
            .install_document_security(popup, "https://replacement.test/".into(), &[])
            .unwrap();
        assert!(
            target
                .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: popup,
                    expected_generation: Some(document_generation)
                })
                .is_err()
        );
    }
}

#[cfg(test)]
mod external_template_policy_regressions {
    use super::*;
    #[test]
    fn template_admission_requires_window_cors_get_before_network() {
        let context = crate::runtime::Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let creator = context.native_initial_document_creator(id).unwrap();
        let service = ParentContextService::new_with_network(context, id, None);
        for (client, method, mode, body) in [
            (None, "GET", "cors", ""),
            (Some(creator.clone()), "POST", "cors", ""),
            (Some(creator.clone()), "GET", "no-cors", ""),
            (Some(creator), "GET", "cors", "YQ=="),
        ] {
            let request = HttpStreamRequest {
                upgrade_mixed_image: false,
                fetch: Some(oryn_js::platform::FetchOptions {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    mode: mode.into(),
                    redirect: "follow".into(),
                    integrity: String::new(),
                }),
                client,
                use_cors_preflight: false,
                request_id: 7,
                method: method.into(),
                url: "https://example.test/patch".into(),
                headers: Vec::new(),
                body_base64: body.into(),
                credentials: true,
                credentials_mode: Some("same-origin".into()),
                initiator_url: None,
                referrer_policy: None,
            };
            assert!(matches!(
                service.handle(ContextServiceRequest::TemplateStreamStart {
                    page_id: id,
                    request,
                    nonce: None
                }),
                Err(RuntimeError::ProtocolViolation(_))
            ));
            assert!(service.http_streams.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn external_template_nonce_and_script_policy_are_independent_of_connect_policy() {
        use crate::csp::{CspDestination, CspPolicySet, CspRequest};
        let policy = CspPolicySet::from_headers(
            "https://origin.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'nonce-accepted'; connect-src *".into(),
            )],
        );
        let mut request = CspRequest {
            destination: CspDestination::Script,
            url: "https://cross.test/patch".into(),
            nonce: Some("wrong".into()),
            inline_source: None,
            parser_inserted: true,
            initiated_by_trusted_script: false,
        };
        assert!(!policy.authorize(&request).allowed);
        request.nonce = Some("accepted".into());
        assert!(policy.authorize(&request).allowed);
        let policy = CspPolicySet::from_headers(
            "https://origin.test/",
            &[(
                "Content-Security-Policy".into(),
                "script-src 'self'; connect-src *".into(),
            )],
        );
        assert!(!policy.authorize(&request).allowed);
        request.url = "https://origin.test/patch".into();
        assert!(policy.authorize(&request).allowed);
    }
    #[test]
    fn template_integrity_must_accept_complete_response_before_publication() {
        let integrity = "sha256-B9fOqAtaB7EXwH61rP5cQQlWSqoRukf/UC9TatLkCec=";
        assert!(crate::integrity::verify(integrity, b"SRI_OK", true).is_ok());
        assert!(crate::integrity::verify(integrity, b"SRI_", true).is_err());
        assert!(crate::integrity::verify(integrity, b"SRI_OK", false).is_err());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ScriptClosePreparation {
    Pending,
    Allowed(ScriptCloseHistoryProof),
    Canceled,
}

#[path = "ordinary_keepalive.rs"]
pub(crate) mod ordinary_keepalive;

#[cfg(test)]
mod ordinary_keepalive_response_tests {
    use super::*;
    #[test]
    fn orphan_response_consumes_body_and_releases_quota_only_at_eof() {
        struct Ack;
        impl NetworkBroker for Ack {
            fn send(
                &self,
                _: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                unreachable!()
            }
            fn cancel_operation(&self, _: u64) -> Result<(), crate::network::NetworkError> {
                Ok(())
            }
        }
        let budget = crate::keepalive::Budget::default();
        let owner = crate::agent_registry::AgentOwner::Window {
            page: PageId(88),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let cleanup = http_cleanup_custody::Ticket::reserve(
            &Default::default(),
            88,
            SharedNetworkBroker::new(Ack),
        )
        .unwrap();
        cleanup.retain_keepalive(Arc::new(budget.reserve(owner.clone(), 65536).unwrap()));
        cleanup.attach(88);
        let (sender, events) = tokio::sync::mpsc::channel(2);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 88,
                bytes: bytes::Bytes::from_static(b"ok"),
            })
            .unwrap();
        let entry = Arc::new(HttpStreamEntry {
            keepalive_context: None,
            keepalive_drain: Some(crate::keepalive_drain::Ticket::reserve().unwrap()),
            support: Mutex::new(None),
            page_id: PageId(88),
            creator: Mutex::new(None),
            prepared_frame_policy: None,
            frame_deadline: None,
            request_id: 88,
            cleanup: Some(cleanup),
            state: Mutex::new(HttpStreamState {
                preload: None,
                operation: Some(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id: 88,
                    events,
                    demand: None,
                }),
                current: bytes::Bytes::new(),
                offset: 0,
                bytes_read: 0,
                content_length: Some(2),
            }),
        });
        let observed = entry.clone();
        orphan_keepalive_response(entry);
        let deadline = Instant::now() + Duration::from_secs(2);
        while observed.state.lock().unwrap().bytes_read < 2 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            budget.reserve(owner.clone(), 1).is_err(),
            "headers and chunks do not end fetch-group accounting"
        );
        sender
            .try_send(NetworkEvent::Complete {
                request_id: 88,
                encoded_body_size: Some(2),
            })
            .unwrap();
        loop {
            if budget.reserve(owner.clone(), 65536).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}

#[cfg(feature = "v8-host")]
pub(crate) struct NativeHistoryProjectionBackend {
    client: SharedContextClient,
    page: PageId,
}
#[cfg(feature = "v8-host")]
impl NativeHistoryProjectionBackend {
    pub(crate) fn new(client: SharedContextClient, page: PageId) -> Self {
        Self { client, page }
    }
    fn send(&self, request: ContextServiceRequest) -> Result<(), String> {
        match self.client.request(request) {
            Ok(ContextServiceResponse::ScriptCloseHistoryPublished) => Ok(()),
            Ok(_) => Err("native history projection ACK mismatch".into()),
            Err(error) => Err(error.to_string()),
        }
    }
}
#[cfg(feature = "v8-host")]
impl oryn_js::navigation_api::NativeHistoryPublisher for NativeHistoryProjectionBackend {
    fn begin(
        &self,
        creator: &oryn_js::worker_wire::Creator,
        revision: u64,
        count: usize,
    ) -> Result<(), String> {
        self.send(ContextServiceRequest::BeginScriptCloseHistory {
            page_id: self.page,
            creator: creator.clone(),
            revision,
            count,
        })
    }
    fn publish(
        &self,
        creator: &oryn_js::worker_wire::Creator,
        revision: u64,
        count: usize,
    ) -> Result<(), String> {
        self.send(ContextServiceRequest::PublishScriptCloseHistory {
            page_id: self.page,
            creator: creator.clone(),
            revision,
            count,
        })
    }
    fn abort(
        &self,
        creator: &oryn_js::worker_wire::Creator,
        revision: u64,
        count: usize,
    ) -> Result<(), String> {
        self.send(ContextServiceRequest::AbortScriptCloseHistory {
            page_id: self.page,
            creator: creator.clone(),
            revision,
            count,
        })
    }
}

#[cfg(test)]
mod script_close_projection_wire_tests {
    use super::*;
    #[test]
    fn projection_and_closing_reads_require_exact_native_creator() {
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 7,
            stream_epoch: 3,
        };
        let page_id = PageId(1);
        for request in [
            ContextServiceRequest::BeginScriptCloseHistory {
                page_id,
                creator: creator.clone(),
                revision: 4,
                count: 2,
            },
            ContextServiceRequest::PublishScriptCloseHistory {
                page_id,
                creator: creator.clone(),
                revision: 4,
                count: 2,
            },
            ContextServiceRequest::AbortScriptCloseHistory {
                page_id,
                creator: creator.clone(),
                revision: 4,
                count: 2,
            },
            ContextServiceRequest::ReadScriptCloseState {
                source_page_id: page_id,
                creator: creator.clone(),
                target_page_id: PageId(2),
                expected: None,
            },
            ContextServiceRequest::ClosePage {
                source_page_id: page_id,
                creator: creator.clone(),
                target_page_id: PageId(2),
                expected: None,
            },
        ] {
            let value = serde_json::to_value(request).unwrap();
            let decoded: ContextServiceRequest = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), value);
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove("creator");
            assert!(serde_json::from_value::<ContextServiceRequest>(missing).is_err());
            let mut missing_epoch = value;
            missing_epoch["creator"]["Window"]
                .as_object_mut()
                .unwrap()
                .remove("stream_epoch");
            assert!(serde_json::from_value::<ContextServiceRequest>(missing_epoch).is_err());
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlankPrerenderActivation {
    pub page_id: PageId,
    pub token: oryn_js::message_wire::PrerenderToken,
    pub generation: u64,
    pub epoch: u64,
    pub url: String,
}

#[cfg(test)]
mod fetch_origin_header_regressions {
    use super::*;
    #[test]
    fn parent_http_origin_header_uses_response_taint_method_and_policy() {
        struct Reply(Option<&'static str>);
        impl NetworkBroker for Reply {
            fn send(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkResponse, crate::network::NetworkError> {
                let origins = request
                    .headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("origin"))
                    .map(|(_, value)| value.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(origins, self.0.into_iter().collect::<Vec<_>>());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("access-control-allow-origin".into(), "*".into())],
                    body: Vec::new(),
                })
            }
        }
        for (method, target, cors, policy, expected) in [
            (
                "GET",
                "https://source.test/item",
                true,
                "strict-origin-when-cross-origin",
                None,
            ),
            (
                "POST",
                "https://source.test/item",
                true,
                "strict-origin-when-cross-origin",
                Some("https://source.test"),
            ),
            (
                "POST",
                "https://source.test/item",
                true,
                "no-referrer",
                Some("null"),
            ),
            (
                "GET",
                "https://remote.test/item",
                true,
                "no-referrer",
                Some("https://source.test"),
            ),
            (
                "POST",
                "https://remote.test/item",
                false,
                "same-origin",
                Some("null"),
            ),
        ] {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let id = page.page_id();
            context
                .install_document_security(id, "https://source.test/".into(), &[])
                .unwrap();
            context
                .handle_message_wire(
                    id,
                    oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let service = ParentContextService::new_with_network(
                context,
                id,
                Some(SharedNetworkBroker::new(Reply(expected))),
            );
            let creator = oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            };
            let result = service
                .start_http_stream_with_cors(
                    id,
                    HttpStreamRequest {
                        upgrade_mixed_image: false,
                        fetch: None,
                        client: Some(creator),
                        use_cors_preflight: false,
                        request_id: 5,
                        method: method.into(),
                        url: target.into(),
                        headers: Vec::new(),
                        body_base64: String::new(),
                        credentials: false,
                        credentials_mode: Some("omit".into()),
                        initiator_url: None,
                        referrer_policy: Some(policy.into()),
                    },
                    crate::csp::CspDestination::Connect,
                    cors,
                )
                .unwrap();
            let ContextServiceResponse::HttpStreamStarted { stream_id, .. } = result else {
                panic!("HTTP headers")
            };
            service.abort_http_stream(id, stream_id).unwrap();
        }
    }
}

#[cfg(test)]
mod prefetch_admission_regressions {
    use super::*;
    #[test]
    fn prefetch_cannot_fallback_to_page_authority_or_override_fetch_category() {
        let context = crate::runtime::Browser::in_process_probe().new_context();
        let page = context.new_page().page_id();
        let creator = context.native_initial_document_creator(page).unwrap();
        let service = ParentContextService::new_with_network(context.clone(), page, None);
        for (client, method, mode, body) in [
            (None, "GET", "no-cors", ""),
            (Some(creator.clone()), "POST", "no-cors", ""),
            (Some(creator.clone()), "GET", "cors", ""),
            (Some(creator), "GET", "no-cors", "YQ=="),
        ] {
            let request = HttpStreamRequest {
                upgrade_mixed_image: false,
                fetch: Some(oryn_js::platform::FetchOptions {
                    keepalive: false,
                    privileged_range: false,
                    cache: Default::default(),
                    mode: mode.into(),
                    redirect: "follow".into(),
                    integrity: String::new(),
                }),
                client,
                use_cors_preflight: false,
                request_id: 7,
                method: method.into(),
                url: "https://example.test/future".into(),
                headers: Vec::new(),
                body_base64: body.into(),
                credentials: true,
                credentials_mode: Some("include".into()),
                initiator_url: None,
                referrer_policy: None,
            };
            assert!(matches!(
                service.handle(ContextServiceRequest::PrefetchStreamStart {
                    page_id: page,
                    request
                }),
                Err(RuntimeError::ProtocolViolation(_))
            ));
            assert!(service.http_streams.lock().unwrap().is_empty());
        }
    }
}

#[cfg(test)]
mod embedded_type_request_tests {
    use super::*;
    use crate::Browser;
    use crate::network::NetworkError;
    #[test]
    fn embedded_type_wire_defaults_and_selected_metadata_remain_distinct() {
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let mut initiator = ResourceInitiator {
            embedded_type: Some("image/svg+xml".into()),
            request_state: None,
            navigation: Default::default(),
            stylesheet: None,
            creator,
            prepared_policy: None,
        };
        let encoded = serde_json::to_value(&initiator).unwrap();
        assert_eq!(encoded["embedded_type"], "image/svg+xml");
        let mut legacy = encoded.clone();
        legacy.as_object_mut().unwrap().remove("embedded_type");
        assert!(
            serde_json::from_value::<ResourceInitiator>(legacy)
                .unwrap()
                .embedded_type
                .is_none()
        );
        struct Resource;
        impl NetworkBroker for Resource {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                assert_eq!(
                    request.security.fetch_metadata.mode,
                    Some(crate::fetch_metadata::Mode::Navigate)
                );
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![],
                    body: b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
                })
            }
        }
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://embedded.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let service = ParentContextService::new_with_network(
            context.clone(),
            page,
            Some(SharedNetworkBroker::new(Resource)),
        );
        let response = service
            .load_page_resource(
                page,
                "https://embedded.test/item",
                "object",
                PageResourceOptions {
                    response_lease: None,
                    download: None,
                    script_authority: None,
                    initiator: &initiator,
                    form_authority: None,
                    initiator_url: None,
                    referrer_policy: None,
                    parent_encoding: None,
                    post: None,
                },
            )
            .unwrap();
        let ContextServiceResponse::PageResource {
            content_type,
            response_headers,
            stream_id,
            ..
        } = response
        else {
            panic!("object response")
        };
        assert_eq!(content_type.as_deref(), Some("image/svg+xml"));
        assert!(response_headers.is_empty());
        assert!(stream_id.is_some());
        assert!(
            service
                .load_page_resource(
                    page,
                    "https://embedded.test/item",
                    "frame",
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: None,
                        referrer_policy: None,
                        parent_encoding: None,
                        post: None
                    }
                )
                .is_err()
        );
        initiator.embedded_type = Some("application/x-no-handler".into());
        assert!(matches!(
            service
                .load_page_resource(
                    page,
                    "https://embedded.test/item",
                    "object",
                    PageResourceOptions {
                        response_lease: None,
                        download: None,
                        script_authority: None,
                        initiator: &initiator,
                        form_authority: None,
                        initiator_url: None,
                        referrer_policy: None,
                        parent_encoding: None,
                        post: None
                    }
                )
                .unwrap(),
            ContextServiceResponse::EmbeddedResourceUnavailable
        ));
    }
}

#[cfg(test)]
mod credentialless_navigation_request_tests {
    use super::*;
    use crate::Browser;
    use crate::network::NetworkError;
    #[test]
    fn document_destinations_keep_include_while_no_cors_images_omit() {
        struct Capture(Arc<Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Capture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request);
                Err(NetworkError::Transport(
                    "stop after request inspection".into(),
                ))
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(
                page,
                "https://source.test/".into(),
                &[(
                    "cross-origin-embedder-policy".into(),
                    "credentialless".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let service = ParentContextService::new_with_network(
            context.clone(),
            page,
            Some(SharedNetworkBroker::new(Capture(seen.clone()))),
        );
        let initiator = ResourceInitiator {
            embedded_type: None,
            request_state: None,
            navigation: Default::default(),
            stylesheet: None,
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            prepared_policy: None,
        };
        for destination in ["frame", "object", "embed", "image"] {
            assert!(
                service
                    .load_page_resource(
                        page,
                        "https://cross.test/item",
                        destination,
                        PageResourceOptions {
                            response_lease: None,
                            download: None,
                            script_authority: None,
                            initiator: &initiator,
                            form_authority: None,
                            initiator_url: None,
                            referrer_policy: None,
                            parent_encoding: None,
                            post: None
                        }
                    )
                    .is_err()
            );
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        for request in &seen[..3] {
            assert!(request.credentials);
            assert_eq!(
                request.security.credentials,
                crate::cookies::CookieCredentials::Include
            );
            assert_eq!(
                request.security.fetch_metadata.mode,
                Some(crate::fetch_metadata::Mode::Navigate)
            );
        }
        assert!(!seen[3].credentials);
        assert_eq!(
            seen[3].security.credentials,
            crate::cookies::CookieCredentials::Omit
        );
        assert_eq!(
            seen[3].security.fetch_metadata.mode,
            Some(crate::fetch_metadata::Mode::NoCors)
        );
    }
}

#[cfg(test)]
mod auxiliary_rollback_identity_tests {
    use super::*;
    use oryn_js::{message_wire::MessageRequest, worker_wire::Creator};
    fn fixture() -> (BrowserContext, PageId, u64) {
        let context = crate::runtime::Browser::in_process_probe().new_context();
        let opener = context.new_page();
        let creator = context
            .native_initial_document_creator(opener.page_id())
            .unwrap();
        let popup = context
            .new_popup_from_creator(opener.page_id(), &creator)
            .unwrap();
        let Creator::Window {
            document_generation,
            ..
        } = context
            .native_initial_document_creator(popup.page_id())
            .unwrap()
        else {
            unreachable!()
        };
        (context, popup.page_id(), document_generation)
    }
    #[test]
    fn rollback_of_retired_initial_identity_does_not_rematerialize_and_repeats_safely() {
        let (context, page, generation) = fixture();
        let service = ParentContextService::new(context.clone(), page);
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        for _ in 0..2 {
            assert!(matches!(
                service
                    .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                        page_id: page,
                        expected_generation: Some(generation)
                    })
                    .unwrap(),
                ContextServiceResponse::PageClosed
            ));
            assert!(context.native_initial_document_creator(page).is_err());
        }
    }
    #[test]
    fn rollback_cannot_close_a_new_initial_generation_after_retirement() {
        let (context, page, generation) = fixture();
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let replacement = context.native_initial_document_creator(page).unwrap();
        let Creator::Window {
            document_generation: new_generation,
            ..
        } = replacement
        else {
            unreachable!()
        };
        assert_ne!(generation, new_generation);
        let service = ParentContextService::new(context.clone(), page);
        assert!(
            service
                .handle(ContextServiceRequest::RollbackAuxiliaryRoot {
                    page_id: page,
                    expected_generation: Some(generation)
                })
                .is_err()
        );
        assert_eq!(
            context.native_initial_document_creator(page).unwrap(),
            replacement
        );
    }
    #[cfg(feature = "v8-host")]
    #[test]
    fn lost_rollback_ack_retains_source_custody_until_target_ack() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        struct Target {
            service: ParentContextService,
            calls: AtomicUsize,
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Target {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, RuntimeError> {
                let response = self.service.handle(request)?;
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(RuntimeError::ContextService(
                        "injected lost rollback acknowledgement".into(),
                    ));
                }
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                Ok(response)
            }
        }
        let (context, page, generation) = fixture();
        let source_page = context.new_page();
        let source = SharedContextClient::direct(context.clone(), source_page.page_id());
        let (entered, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let target = SharedContextClient(
            Arc::new(Target {
                service: ParentContextService::new(context.clone(), page),
                calls: AtomicUsize::new(0),
                entered,
                release: Mutex::new(wait),
            }),
            None,
            Default::default(),
            Default::default(),
            None,
        );
        let mut rollback = source
            .reserve_auxiliary_rollback(source_page.page_id())
            .unwrap();
        assert!(source.drain_script_cleanup(Duration::ZERO).is_err());
        rollback.bind(target, page);
        rollback.set_generation(generation);
        assert!(rollback.revoke().is_err());
        assert!(context.native_initial_document_creator(page).is_err());
        drop(rollback);
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(source.drain_script_cleanup(Duration::ZERO).is_err());
        release.send(()).unwrap();
        source.drain_script_cleanup(Duration::from_secs(5)).unwrap();
    }
}

#[cfg(test)]
mod context_rpc_diagnostic_tests {
    use super::*;
    #[test]
    fn diagnostic_operation_never_formats_nested_authority_payload() {
        let request = ContextServiceRequest::AuthorizePreloadConsumption {
            proof: "secret authority proof must not appear".into(),
            request: Box::new(ContextServiceRequest::WebFeatureProfile { page_id: PageId(7) }),
        };
        assert_eq!(
            request.diagnostic_operation(),
            "AuthorizePreloadConsumption"
        );
        assert_eq!(
            ContextServiceRequest::WebFeatureProfile { page_id: PageId(7) }.diagnostic_operation(),
            "WebFeatureProfile"
        );
    }
    #[test]
    fn diagnostic_platform_subtypes_are_literal_and_distinct() {
        use oryn_js::platform_control::Request;
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        for (request, label) in [
            (Request::DocumentFocus, "NativePlatformQuery.DocumentFocus"),
            (Request::WindowState, "NativePlatformQuery.WindowState"),
            (
                Request::CspEvents { after: 81 },
                "NativePlatformQuery.CspEvents",
            ),
            (
                Request::StorageEvents { after: 72 },
                "NativePlatformQuery.StorageEvents",
            ),
            (
                Request::RegisterProtocol {
                    scheme: "secret".into(),
                    url: "https://secret.invalid".into(),
                },
                "NativePlatformQuery.RegisterProtocol",
            ),
        ] {
            let query = ContextServiceRequest::NativePlatformQuery {
                page_id: PageId(7),
                creator: creator.clone(),
                request,
            };
            assert_eq!(query.diagnostic_operation(), label);
            assert!(!label.contains("secret"));
        }
    }
    #[test]
    fn diagnostic_envelope_missing_stamp_remains_compatible() {
        let request = ContextServiceRequest::WebFeatureProfile { page_id: PageId(7) };
        let envelope = RequestEnvelope {
            diagnostic: None,
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: None,
            message_id: 17,
            request,
        };
        let encoded = serde_json::to_value(&envelope).unwrap();
        assert!(encoded.get("diagnostic").is_none());
        let decoded: RequestEnvelope = serde_json::from_value(encoded).unwrap();
        assert!(decoded.diagnostic.is_none());
    }
}
