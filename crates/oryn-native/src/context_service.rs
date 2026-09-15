#![cfg(unix)]

//! Typed parent services available to a sandboxed page worker.
//!
//! This channel deliberately has no generic filesystem, environment, shell,
//! socket, or secret operation. Networking continues to use the separately
//! policy-enforced network broker; this service coordinates context state.

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
    time::Duration,
};

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

pub const CONTEXT_SERVICE_PROTOCOL_VERSION: u32 = 11;
pub const MAX_CONTEXT_FRAME_BYTES: usize = 24 * 1024 * 1024;
// Page bundles use the same ceiling as the default resource-cache entry.
// Wire JSON remains independently bounded, including escape expansion.
const MAX_PAGE_SCRIPT_BYTES: usize = 8 * 1024 * 1024;

fn decode_page_script(body: &[u8], content_type: Option<&str>) -> Result<String, RuntimeError> {
    if body.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "page script resource".into(),
        });
    }
    let (source, _) =
        oryn_net::decode_resource_text(body, content_type, oryn_net::TextResourceKind::JavaScript);
    if source.len() > MAX_PAGE_SCRIPT_BYTES {
        return Err(RuntimeError::WorkerLimitExceeded {
            resource: "decoded page script source".into(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ContextServiceRequest {
    GetReferrerPolicy {
        page_id: PageId,
    },
    HttpStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    ImageStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    HttpStreamRead {
        page_id: PageId,
        stream_id: u64,
        max_bytes: usize,
    },
    HttpStreamAbort {
        page_id: PageId,
        stream_id: u64,
    },
    HttpRequestAbort {
        page_id: PageId,
        request_id: u64,
    },
    Storage {
        request: StorageRequest,
    },
    OpenPage {
        opener_page_id: PageId,
        url: Option<String>,
        #[serde(default)]
        post: Option<oryn_js::navigation::PostResource>,
        #[serde(default)]
        referrer_policy: Option<String>,
    },
    ActivatePage {
        source_page_id: PageId,
        target_page_id: PageId,
    },
    CommitPage {
        page_id: PageId,
        url: String,
    },
    InstallDocumentSecurity {
        page_id: PageId,
        document_url: String,
        headers: Vec<(String, String)>,
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
    },
    SetDocumentCookie {
        page_id: PageId,
        value: String,
    },
    GetSecurityOrigin {
        page_id: PageId,
    },
    GetRelatedPageOrigin {
        source_page_id: PageId,
        target_page_id: PageId,
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
    AuthorizeNavigation {
        page_id: PageId,
        url: String,
    },
    /// Authorize the actual default form-submission effect after validation
    /// and the cancelable submit event have completed.
    AuthorizeFormSubmission {
        page_id: PageId,
        url: String,
        method: String,
    },
    ClosePage {
        source_page_id: PageId,
        target_page_id: PageId,
    },
    PostMessage {
        source_page_id: PageId,
        target_page_id: PageId,
        target_origin: String,
        data: serde_json::Value,
    },
    PollEvents {
        page_id: PageId,
        after_event_id: u64,
        max_events: usize,
    },
    LoadWorkerScript {
        page_id: PageId,
        url: String,
        module: bool,
    },
    LoadScript {
        page_id: PageId,
        url: String,
        module: bool,
        nonce: Option<String>,
        #[serde(default)]
        initiator_url: Option<String>,
        #[serde(default)]
        referrer_policy: Option<String>,
    },
    LoadPageResource {
        page_id: PageId,
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
    AuthorizeInlineScript {
        page_id: PageId,
        source: String,
        nonce: Option<String>,
    },
    WebSocketOpen {
        page_id: PageId,
        url: String,
        protocols: Vec<String>,
    },
    WebSocketSend {
        page_id: PageId,
        socket_id: u64,
        frame: WebSocketFrame,
    },
    WebSocketPoll {
        page_id: PageId,
        socket_id: u64,
        max_frames: usize,
    },
    WebSocketClose {
        page_id: PageId,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    },
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
    ReferrerPolicy {
        value: String,
    },
    HttpStreamStarted {
        stream_id: u64,
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        total_bytes: Option<usize>,
    },
    HttpStreamChunk {
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
        page_id: PageId,
    },
    PageDenied {
        reason: String,
    },
    PageActivated {
        page_id: PageId,
    },
    PageClosed,
    PageCommitted,
    DocumentSecurityInstalled,
    CspViolationsAccepted,
    DocumentCookie {
        value: String,
    },
    DocumentCookieSet,
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
    MessageQueued,
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
    ScriptSource {
        url: String,
        source: String,
        module: bool,
    },
    ScriptAuthorized,
    PageResource {
        url: String,
        source: String,
        encoding: String,
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
    WebSocketOpened {
        socket_id: u64,
        protocol: String,
    },
    WebSocketSent,
    WebSocketEvents {
        frames: Vec<WebSocketFrame>,
    },
    WebSocketClosed,
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
    protocol_version: u32,
    route: Option<WorkerRoute>,
    message_id: u64,
    request: ContextServiceRequest,
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

fn encode_context_response(
    mut response: ResponseEnvelope,
    limit: usize,
) -> Result<Vec<u8>, RuntimeError> {
    let mut frame = BoundedContextFrame {
        bytes: Vec::new(),
        limit: limit.saturating_sub(1),
    };
    if serde_json::to_writer(&mut frame, &response).is_err() {
        // Nothing has reached the socket. Replace the entire oversized result,
        // retaining authenticated route and correlation rather than truncating JSON.
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
    Ok(frame.bytes)
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
        let mut reader = BufReader::new(
            self.stream
                .try_clone()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?,
        );
        let mut line = Vec::new();
        read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        let response: ResponseEnvelope = serde_json::from_slice(&line)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if response.protocol_version != CONTEXT_SERVICE_PROTOCOL_VERSION
            || response.route.is_some()
            || response.correlation_id != message_id
        {
            return Err(RuntimeError::ProtocolViolation(
                "context-service response correlation mismatch".into(),
            ));
        }
        if let Some(error) = response.error {
            return Err(RuntimeError::ContextService(error));
        }
        response.response.ok_or_else(|| {
            RuntimeError::ProtocolViolation("context service returned no response".into())
        })
    }
}

pub(crate) trait ContextTransport: Send + Sync {
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

type ContextReply = Result<ContextServiceResponse, String>;
type PendingContextReplies = BTreeMap<u64, (Option<WorkerRoute>, mpsc::Sender<ContextReply>)>;

struct IpcContextTransport {
    writer: Mutex<UnixStream>,
    next_message_id: AtomicU64,
    pending: Arc<Mutex<PendingContextReplies>>,
}

impl IpcContextTransport {
    fn new(client: IpcContextClient) -> Result<Self, RuntimeError> {
        let reader_stream = client
            .stream
            .try_clone()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let pending = Arc::new(Mutex::new(PendingContextReplies::new()));
        let reader_pending = pending.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader_stream);
            loop {
                let mut line = Vec::new();
                let read = read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES);
                let Ok(read) = read else { break };
                if read == 0 {
                    break;
                }
                let Ok(response) = serde_json::from_slice::<ResponseEnvelope>(&line) else {
                    continue;
                };
                let sender = reader_pending
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&response.correlation_id));
                if let Some((route, sender)) = sender {
                    let result = if response.protocol_version != CONTEXT_SERVICE_PROTOCOL_VERSION
                        || response.route != route
                    {
                        Err("context response protocol mismatch".into())
                    } else if let Some(error) = response.error {
                        Err(error)
                    } else {
                        response
                            .response
                            .ok_or_else(|| "context service returned no response".into())
                    };
                    let _ = sender.send(result);
                }
            }
            if let Ok(mut pending) = reader_pending.lock() {
                for (_, (_, sender)) in std::mem::take(&mut *pending) {
                    let _ = sender.send(Err("context service connection closed".into()));
                }
            }
        });
        Ok(Self {
            writer: Mutex::new(client.stream),
            next_message_id: AtomicU64::new(client.next_message_id),
            pending,
        })
    }
}

impl Drop for IpcContextTransport {
    fn drop(&mut self) {
        if let Ok(writer) = self.writer.get_mut() {
            let _ = writer.shutdown(Shutdown::Both);
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
        let (sender, receiver) = mpsc::channel();
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
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RuntimeError::ContextService("context pending lock poisoned".into()))?;
        if pending.len() >= MAX_CONTEXT_OPERATIONS {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending context operations".into(),
            });
        }
        pending.insert(message_id, (route.cloned(), sender));
        drop(pending);
        let envelope = RequestEnvelope {
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: route.cloned(),
            message_id,
            request,
        };
        let written = (|| {
            serde_json::to_writer(&mut *writer, &envelope)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
            writer
                .write_all(b"\n")
                .and_then(|_| writer.flush())
                .map_err(|error| RuntimeError::ContextService(error.to_string()))
        })();
        drop(writer);
        if let Err(error) = written {
            self.pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&message_id));
            return Err(error);
        }
        receiver
            .recv()
            .map_err(|_| RuntimeError::ContextService("context response channel closed".into()))?
            .map_err(RuntimeError::ContextService)
    }
}

#[derive(Clone)]
pub struct SharedContextClient(Arc<dyn ContextTransport>, Option<WorkerRoute>);

impl SharedContextClient {
    pub fn new(client: IpcContextClient) -> Self {
        Self(
            Arc::new(IpcContextTransport::new(client).expect("context transport initialization")),
            None,
        )
    }

    pub(crate) fn for_route(&self, route: WorkerRoute) -> Self {
        Self(self.0.clone(), Some(route))
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn direct(context: BrowserContext, source_page_id: PageId) -> Self {
        Self(
            Arc::new(ParentContextService::new_direct(context, source_page_id)),
            None,
        )
    }

    pub fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.0.request_routed(self.1.as_ref(), request)
    }
}

pub struct ParentContextService {
    context: crate::runtime::WeakBrowserContext,
    source_page_id: PageId,
    next_socket_id: AtomicU64,
    sockets: Mutex<BTreeMap<u64, WebSocket<MaybeTlsStream<TcpStream>>>>,
    socket_audits: Mutex<BTreeMap<u64, crate::resource_audit::WebSocketObservation>>,
    network: Option<SharedNetworkBroker>,
    next_http_stream_id: AtomicU64,
    http_streams: Mutex<BTreeMap<u64, Arc<HttpStreamEntry>>>,
    cancelled_http_requests: Mutex<BTreeSet<u64>>,
    authorized_storage_origins: Mutex<BTreeSet<String>>,
    shutting_down: AtomicBool,
    trusted_in_process_runtime: bool,
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
    operation: NetworkOperation,
    current: bytes::Bytes,
    offset: usize,
    bytes_read: usize,
    content_length: Option<usize>,
}

struct PageResourceOptions<'a> {
    initiator_url: Option<&'a str>,
    referrer_policy: Option<&'a str>,
    parent_encoding: Option<&'a str>,
    post: Option<oryn_js::navigation::PostResource>,
}

struct HttpStreamEntry {
    page_id: PageId,
    request_id: u64,
    state: Mutex<HttpStreamState>,
}

impl Drop for ParentContextService {
    fn drop(&mut self) {
        self.cancel_all_network_operations();
    }
}

fn receive_network_event(
    events: &mut tokio::sync::mpsc::Receiver<NetworkEvent>,
    timeout: Duration,
) -> Result<Option<NetworkEvent>, RuntimeError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match events.try_recv() {
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
        let authorized_storage_origins = context
            .page_origin(source_page_id)
            .into_iter()
            .collect::<BTreeSet<_>>();
        Self {
            context: context.downgrade(),
            source_page_id,
            next_socket_id: AtomicU64::new(1),
            sockets: Mutex::new(BTreeMap::new()),
            socket_audits: Mutex::new(BTreeMap::new()),
            network,
            next_http_stream_id: AtomicU64::new(1),
            http_streams: Mutex::new(BTreeMap::new()),
            cancelled_http_requests: Mutex::new(BTreeSet::new()),
            authorized_storage_origins: Mutex::new(authorized_storage_origins),
            shutting_down: AtomicBool::new(false),
            trusted_in_process_runtime: false,
        }
    }

    pub(crate) fn cancel_all_network_operations(&self) {
        self.shutting_down.store(true, Ordering::Release);
        if let Ok(mut sockets) = self.sockets.lock() {
            sockets.clear();
        }
        if let Ok(mut audits) = self.socket_audits.lock() {
            audits.clear();
        }
        let request_ids = self
            .http_streams
            .lock()
            .map(|mut streams| {
                std::mem::take(&mut *streams)
                    .into_values()
                    .map(|entry| entry.request_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(network) = &self.network {
            for request_id in request_ids {
                let _ = network.cancel_operation(request_id);
            }
        }
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
        // Keep the context alive for this bounded request, never for the
        // lifetime of the service retained by one of its own page actors.
        let context_owner = self.context_owner()?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::WorkerCrashed(
                "context service is shutting down".into(),
            ));
        }
        match request {
            ContextServiceRequest::GetReferrerPolicy { page_id } => {
                self.validate_socket_owner(page_id)?;
                Ok(ContextServiceResponse::ReferrerPolicy {
                    value: self
                        .context_owner()?
                        .page_referrer_policy(page_id)?
                        .as_str()
                        .into(),
                })
            }
            ContextServiceRequest::HttpStreamStart { page_id, request } => {
                self.start_http_stream(page_id, request, crate::csp::CspDestination::Connect)
            }
            ContextServiceRequest::ImageStreamStart { page_id, request } => {
                if request.method != "GET" || !request.body_base64.is_empty() {
                    return Err(RuntimeError::ProtocolViolation(
                        "image stream requires a bodyless GET".into(),
                    ));
                }
                self.start_http_stream(page_id, request, crate::csp::CspDestination::Image)
            }
            ContextServiceRequest::HttpStreamRead {
                page_id,
                stream_id,
                max_bytes,
            } => self.read_http_stream(page_id, stream_id, max_bytes),
            ContextServiceRequest::HttpStreamAbort { page_id, stream_id } => {
                self.abort_http_stream(page_id, stream_id)
            }
            ContextServiceRequest::HttpRequestAbort {
                page_id,
                request_id,
            } => self.abort_http_request(page_id, request_id),
            ContextServiceRequest::Storage { request } => {
                self.validate_storage_scope(&request)?;
                let response = self
                    .context_owner()?
                    .context_store()
                    .execute(request)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                if let StorageResponse::Changed { change } = &response {
                    self.context_owner()?
                        .push_context_event(ContextEvent::Storage {
                            source_page_id: self.source_page_id,
                            change: change.clone(),
                        });
                }
                Ok(ContextServiceResponse::Storage {
                    response: Box::new(response),
                })
            }
            ContextServiceRequest::OpenPage {
                opener_page_id,
                url,
                post,
                referrer_policy,
            } => {
                if opener_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot open a popup for another page".into(),
                    ));
                }
                if !self.context_owner()?.popups_allowed() {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "popup policy denied page creation".into(),
                    });
                }
                if self
                    .context_owner()?
                    .csp_sandbox(opener_page_id)?
                    .is_some_and(|sandbox| !sandbox.allow_popups)
                {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "Content-Security-Policy sandbox denied popup creation".into(),
                    });
                }
                let opener_origin = self.context_owner()?.page_origin(opener_page_id)?;
                let target_origin = url.as_deref().and_then(|value| {
                    context_owner
                        .resolve_page_url(self.source_page_id, value)
                        .ok()
                        .map(|url| url.origin().ascii_serialization())
                });
                if let Err(error) = self.context_owner()?.require_policy(
                    PolicyRequest {
                        action: ConsequentialAction::Popup,
                        source_origin: opener_origin.clone(),
                        target_origin,
                        detail: url.clone(),
                    },
                    None,
                ) {
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
                        .and_then(|url| context_owner.resolve_page_url(opener_page_id, url).ok())
                        .is_none_or(|url| !resource.valid_for(&url))
                    {
                        return Err(RuntimeError::ContextService(
                            "invalid popup POST resource".into(),
                        ));
                    }
                    match self.handle(ContextServiceRequest::AuthorizeFormSubmission {
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
                // Clone sessionStorage before the popup's first document can
                // execute, matching the browser's initial-copy semantics.
                let initial_url = url;
                let page = self.context_owner()?.new_popup(opener_page_id, None);
                let opener_scope = StorageScope {
                    origin: opener_origin.clone(),
                    top_level_page_id: opener_page_id,
                };
                let popup_scope = StorageScope {
                    origin: opener_origin,
                    top_level_page_id: page.page_id(),
                };
                if let StorageResponse::Entries { entries } = self
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
                if let Some(url) = initial_url {
                    self.context_owner()?.queue_resource_navigation(
                        page.page_id(),
                        url,
                        post,
                        referrer_policy,
                    )?;
                }
                Ok(ContextServiceResponse::PageOpened {
                    page_id: page.page_id(),
                })
            }
            ContextServiceRequest::ActivatePage {
                source_page_id,
                target_page_id,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot impersonate another page".into(),
                    ));
                }
                self.context_owner()?.activate_page(target_page_id)?;
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
                    #[cfg(test)]
                    self.context_owner()?.install_document_security(
                        page_id,
                        document_url,
                        &headers,
                    )?;
                    #[cfg(not(test))]
                    self.context_owner()?.verify_document_security(
                        page_id,
                        &document_url,
                        &headers,
                    )?;
                }
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::InstallMetaSecurity {
                page_id,
                serialized,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot install meta policy for another page".into(),
                    ));
                }
                self.context_owner()?
                    .install_meta_security(page_id, &serialized)?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
            }
            ContextServiceRequest::InstallReferrerPolicy {
                page_id,
                serialized,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot install referrer policy for another page".into(),
                    ));
                }
                self.context_owner()?
                    .install_meta_referrer(page_id, &serialized)?;
                Ok(ContextServiceResponse::DocumentSecurityInstalled)
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
            ContextServiceRequest::GetDocumentCookie { page_id } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot read another page's cookies".into(),
                    ));
                }
                Ok(ContextServiceResponse::DocumentCookie {
                    value: self.context_owner()?.document_cookie(page_id)?,
                })
            }
            ContextServiceRequest::SetDocumentCookie { page_id, value } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot write another page's cookies".into(),
                    ));
                }
                self.context_owner()?.set_document_cookie(page_id, value)?;
                Ok(ContextServiceResponse::DocumentCookieSet)
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
            ContextServiceRequest::GetRelatedPageOrigin {
                source_page_id,
                target_page_id,
            } => {
                if source_page_id != self.source_page_id
                    || !self
                        .context_owner()?
                        .may_read_related_page_origin(source_page_id, target_page_id)
                {
                    return Err(RuntimeError::ContextService(
                        "worker cannot read an unrelated page's security origin".into(),
                    ));
                }
                Ok(ContextServiceResponse::SecurityOrigin {
                    serialized: self.context_owner()?.page_origin(target_page_id)?,
                    opaque: self.context_owner()?.is_opaque_origin(target_page_id)?,
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
                let resolved = self.context_owner()?.resolve_page_url(page_id, &url)?;
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
            ContextServiceRequest::AuthorizeNavigation { page_id, url } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot authorize navigation for another page".into(),
                    ));
                }
                self.context_owner()?.validate_page_url(page_id, &url)?;
                let source_origin = self.context_owner()?.page_origin(page_id)?;
                let resolved = self.context_owner()?.resolve_page_url(page_id, &url)?;
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
                page_id,
                url,
                method,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot authorize form submission for another page".into(),
                    ));
                }
                self.context_owner()?.validate_page_url(page_id, &url)?;
                if self
                    .context_owner()?
                    .csp_sandbox(page_id)?
                    .is_some_and(|sandbox| !sandbox.allow_forms)
                {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "Content-Security-Policy sandbox denied form submission".into(),
                    });
                }
                let source_origin = self.context_owner()?.page_origin(page_id)?;
                let resolved = self.context_owner()?.resolve_page_url(page_id, &url)?;
                if !self
                    .context_owner()?
                    .authorize_csp(
                        page_id,
                        &crate::csp::CspRequest {
                            destination: crate::csp::CspDestination::Form,
                            url: resolved.to_string(),
                            nonce: None,
                            inline_source: None,
                            parser_inserted: false,
                            initiated_by_trusted_script: false,
                        },
                    )?
                    .allowed
                {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "form submission blocked by Content-Security-Policy form-action"
                            .into(),
                    });
                }
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
            ContextServiceRequest::ClosePage {
                source_page_id,
                target_page_id,
            } => {
                if source_page_id != self.source_page_id
                    || !self
                        .context_owner()?
                        .may_script_close(source_page_id, target_page_id)
                {
                    return Err(RuntimeError::ContextService(
                        "page is not permitted to close the target".into(),
                    ));
                }
                let context = self.context_owner()?.clone();
                std::thread::spawn(move || {
                    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        let _ = runtime.block_on(context.close_page_from_script(target_page_id));
                    }
                });
                Ok(ContextServiceResponse::PageClosed)
            }
            ContextServiceRequest::PostMessage {
                source_page_id,
                target_page_id,
                target_origin,
                data,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot impersonate another message source".into(),
                    ));
                }
                let source_origin = self.context_owner()?.page_origin(source_page_id)?;
                let actual_target_origin = self.context_owner()?.page_origin(target_page_id)?;
                if target_origin != "*"
                    && (self.context_owner()?.is_opaque_origin(target_page_id)?
                        || target_origin != actual_target_origin)
                {
                    return Err(RuntimeError::ContextService(
                        "postMessage target origin did not match".into(),
                    ));
                }
                self.context_owner()?
                    .push_context_event(ContextEvent::Message {
                        source_page_id,
                        target_page_id,
                        source_origin,
                        data,
                    });
                self.context_owner()?.wake_context_events(target_page_id);
                Ok(ContextServiceResponse::MessageQueued)
            }
            ContextServiceRequest::PollEvents {
                page_id,
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
                        after_event_id,
                        max_events.min(32),
                    )?,
                })
            }
            ContextServiceRequest::LoadWorkerScript {
                page_id,
                url,
                module,
            } => self.load_worker_script(page_id, &url, module),
            ContextServiceRequest::LoadScript {
                page_id,
                url,
                module,
                nonce,
                initiator_url,
                referrer_policy,
            } => self.load_page_script(
                page_id,
                &url,
                module,
                nonce.as_deref(),
                initiator_url.as_deref(),
                referrer_policy.as_deref(),
            ),
            ContextServiceRequest::LoadPageResource {
                page_id,
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
                    initiator_url: initiator_url.as_deref(),
                    referrer_policy: referrer_policy.as_deref(),
                    parent_encoding: parent_encoding.as_deref(),
                    post,
                },
            ),
            ContextServiceRequest::AuthorizeInlineScript {
                page_id,
                source,
                nonce,
            } => self.authorize_inline_script(page_id, &source, nonce.as_deref()),
            ContextServiceRequest::WebSocketOpen {
                page_id,
                url,
                protocols,
            } => self.open_websocket(page_id, &url, &protocols),
            ContextServiceRequest::WebSocketSend {
                page_id,
                socket_id,
                frame,
            } => self.send_websocket(page_id, socket_id, frame),
            ContextServiceRequest::WebSocketPoll {
                page_id,
                socket_id,
                max_frames,
            } => self.poll_websocket(page_id, socket_id, max_frames),
            ContextServiceRequest::WebSocketClose {
                page_id,
                socket_id,
                code,
                reason,
            } => self.close_websocket(page_id, socket_id, code, reason),
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
        self.validate_socket_owner(page_id)?;
        if self
            .cancelled_http_requests
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP cancellation lock poisoned".into()))?
            .remove(&request.request_id)
        {
            return Err(RuntimeError::Network("HTTP request cancelled".into()));
        }
        let body = base64::Engine::decode(
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
        let page_url = self.context_owner()?.page_url(page_id)?;
        let source_url = request.initiator_url.as_deref().unwrap_or(&page_url);
        let page_base =
            Url::parse(&page_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let source = Url::parse(source_url)
            .or_else(|_| page_base.join(source_url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let source_origin = source.origin().ascii_serialization();
        let mut target = Url::parse(&request.url)
            .or_else(|_| source.join(&request.url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let base_headers = sanitize_worker_headers(request.headers);
        let method = request.method;
        let credentials_mode = request.credentials_mode.clone();
        let public_request_id = request.request_id;
        let referrer_policy = request
            .referrer_policy
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map_or(
                self.context_owner()?.page_referrer_policy(page_id)?,
                crate::referrer::ReferrerPolicy::parse_list,
            );
        let mut redirects = 0_u8;
        let (operation, response_status, response_headers, content_length) = loop {
            if let Some(upgraded) = self.context_owner()?.upgrade_csp_url(page_id, &target)? {
                target = upgraded;
            }
            let csp = self.context_owner()?.authorize_csp(
                page_id,
                &crate::csp::CspRequest {
                    destination,
                    url: target.to_string(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                },
            )?;
            if !csp.violations.is_empty() {
                self.context_owner()?
                    .handle_csp_violations(page_id, csp.violations.clone())?;
            }
            if !csp.allowed {
                return Err(RuntimeError::ContextService(format!(
                    "request blocked by Content-Security-Policy {destination:?}"
                )));
            }
            if target.scheme() == "data" {
                if redirects != 0 {
                    return Err(RuntimeError::Network(
                        "HTTP redirect to data URL denied".into(),
                    ));
                }
                // Fetch data URLs produce a local basic response. Keep them
                // behind the same native ownership, CSP, stream quota and
                // cancellation checks as HTTP without sending them to a socket
                // broker or requiring a CORS preflight for an opaque origin.
                let (content_type, bytes) = crate::network::decode_data_url(&target)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
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
                        request_id: public_request_id,
                    })
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                break (
                    NetworkOperation {
                        request_id: public_request_id,
                        events,
                        demand: None,
                    },
                    200,
                    vec![(
                        "content-type".into(),
                        content_type.unwrap_or_else(|| "text/plain;charset=US-ASCII".into()),
                    )],
                    Some(length),
                );
            }
            enforce_mixed_content(source.as_str(), &target)?;
            let cross_origin = target.origin().ascii_serialization() != source_origin;
            let credentials = match credentials_mode.as_deref() {
                Some("include") => true,
                Some("omit") => false,
                Some("same-origin") => !cross_origin,
                _ => request.credentials,
            };
            let mut headers = base_headers.clone();
            headers.push(("origin".into(), source_origin.clone()));
            if let Some(referrer) = referrer_policy.header(&source, &target) {
                headers.push(("referer".into(), referrer));
            }
            if cross_origin && cors_preflight_required(&method, &headers) {
                let requested_headers = cors_non_safelisted_headers(&headers);
                let mut preflight_headers = vec![
                    ("origin".into(), source_origin.clone()),
                    (
                        "access-control-request-method".into(),
                        method.to_uppercase(),
                    ),
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
                        request_id: self.next_http_stream_id.fetch_add(1, Ordering::Relaxed)
                            | (1_u64 << 63),
                        method: "OPTIONS".into(),
                        url: target.to_string(),
                        headers: preflight_headers,
                        body: Vec::new(),
                        credentials: false,
                        security: crate::cookies::CookieRequestContext {
                            credentials: crate::cookies::CookieCredentials::Omit,
                            kind: crate::cookies::CookieRequestKind::Fetch,
                            initiator_url: Some(source_origin.clone()),
                            top_level_url: Some(self.context_owner()?.page_url(page_id)?),
                            ..Default::default()
                        },
                        resolved_addrs: Vec::new(),
                    },
                    Duration::from_secs(10),
                    64 * 1024,
                    &self.shutting_down,
                )
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
                enforce_cors_preflight(&source_origin, &method, &requested_headers, &preflight)?;
            }
            let operation_request_id = if target.as_str() == request.url {
                public_request_id
            } else {
                self.next_http_stream_id.fetch_add(1, Ordering::Relaxed) | (1_u64 << 62)
            };
            let mut operation = network
                .start_operation(NetworkRequest {
                    request_id: operation_request_id,
                    method: method.clone(),
                    url: target.to_string(),
                    headers,
                    body: body.clone(),
                    credentials,
                    security: crate::cookies::CookieRequestContext {
                        credentials: if credentials {
                            crate::cookies::CookieCredentials::Include
                        } else {
                            crate::cookies::CookieCredentials::Omit
                        },
                        kind: if destination == crate::csp::CspDestination::Image {
                            crate::cookies::CookieRequestKind::Image
                        } else {
                            crate::cookies::CookieRequestKind::Fetch
                        },
                        initiator_url: Some(source_origin.clone()),
                        top_level_url: Some(self.context_owner()?.page_url(page_id)?),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                })
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
            if self
                .cancelled_http_requests
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("HTTP cancellation lock poisoned".into())
                })?
                .remove(&public_request_id)
            {
                let _ = network.cancel_operation(operation.request_id);
                return Err(RuntimeError::Network("HTTP request cancelled".into()));
            }
            let header_event =
                match receive_network_event(&mut operation.events, Duration::from_secs(30)) {
                    Ok(event) => event,
                    Err(error) => {
                        let _ = network.cancel_operation(operation.request_id);
                        return Err(error);
                    }
                };
            if self
                .cancelled_http_requests
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("HTTP cancellation lock poisoned".into())
                })?
                .remove(&operation.request_id)
            {
                let _ = network.cancel_operation(operation.request_id);
                return Err(RuntimeError::Network("HTTP request cancelled".into()));
            }
            let (response_status, response_headers, content_length) = match header_event {
                Some(NetworkEvent::Headers {
                    request_id,
                    status,
                    headers,
                    content_length,
                }) if request_id == operation.request_id => (status, headers, content_length),
                Some(NetworkEvent::Failed { request_id, error })
                    if request_id == operation.request_id =>
                {
                    return Err(RuntimeError::Network(error));
                }
                Some(_) => {
                    let _ = network.cancel_operation(operation.request_id);
                    return Err(RuntimeError::ProtocolViolation(
                        "HTTP stream received a stale, unmatched, or out-of-order event".into(),
                    ));
                }
                None => {
                    let _ = network.cancel_operation(operation.request_id);
                    return Err(RuntimeError::Network(
                        "HTTP stream ended before response headers".into(),
                    ));
                }
            };
            if (300..400).contains(&response_status) {
                if redirects >= 10 {
                    let _ = network.cancel_operation(operation.request_id);
                    return Err(RuntimeError::Network("HTTP redirect limit exceeded".into()));
                }
                let location = response_headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        RuntimeError::Network("HTTP redirect omitted Location".into())
                    })?;
                let next = target
                    .join(location)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                let _ = network.cancel_operation(operation.request_id);
                target = next;
                redirects = redirects.saturating_add(1);
                continue;
            }
            if cross_origin && !cors_headers_allowed(&source_origin, credentials, &response_headers)
            {
                let _ = network.cancel_operation(operation.request_id);
                return Err(RuntimeError::ContextService(format!(
                    "cross-origin response denied by CORS: initiator={source_origin}, response={}",
                    target.origin().ascii_serialization()
                )));
            }
            break (operation, response_status, response_headers, content_length);
        };
        if content_length.is_some_and(|length| length > 16 * 1024 * 1024) {
            let _ = network.cancel_operation(operation.request_id);
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "decoded HTTP response body".into(),
            });
        }
        let total_bytes = content_length;
        let stream_id = self.next_http_stream_id.fetch_add(1, Ordering::Relaxed);
        let mut streams = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?;
        if streams.len() >= 32 {
            let _ = network.cancel_operation(operation.request_id);
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending HTTP streams".into(),
            });
        }
        streams.insert(
            stream_id,
            Arc::new(HttpStreamEntry {
                page_id,
                request_id: public_request_id,
                state: Mutex::new(HttpStreamState {
                    operation,
                    current: bytes::Bytes::new(),
                    offset: 0,
                    bytes_read: 0,
                    content_length: total_bytes,
                }),
            }),
        );
        Ok(ContextServiceResponse::HttpStreamStarted {
            stream_id,
            request_id: public_request_id,
            status: response_status,
            headers: sanitize_parent_response_headers(response_headers),
            total_bytes,
        })
    }

    fn abort_http_request(
        &self,
        page_id: PageId,
        request_id: u64,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        self.cancelled_http_requests
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP cancellation lock poisoned".into()))?
            .insert(request_id);
        let stream_ids = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
            .iter()
            .filter_map(|(stream_id, entry)| {
                (entry.page_id == page_id && entry.request_id == request_id).then_some(*stream_id)
            })
            .collect::<Vec<_>>();
        for stream_id in stream_ids {
            let _ = self.abort_http_stream(page_id, stream_id);
        }
        if let Some(network) = &self.network {
            let _ = network.cancel_operation(request_id);
        }
        Ok(ContextServiceResponse::HttpRequestAborted)
    }

    fn read_http_stream(
        &self,
        page_id: PageId,
        stream_id: u64,
        max_bytes: usize,
    ) -> Result<ContextServiceResponse, RuntimeError> {
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
            let requested = max_bytes.clamp(1, 64 * 1024);
            let mut terminal = false;
            let mut chunk = Vec::with_capacity(requested);
            while chunk.len() < requested && !terminal {
                while stream.offset >= stream.current.len() {
                    stream.current = bytes::Bytes::new();
                    stream.offset = 0;
                    stream.operation.request_body_chunk();
                    match receive_network_event(
                        &mut stream.operation.events,
                        Duration::from_secs(30),
                    )? {
                        Some(NetworkEvent::Chunk { request_id, bytes })
                            if request_id == stream.operation.request_id =>
                        {
                            stream.current = bytes
                        }
                        Some(NetworkEvent::Complete { request_id })
                            if request_id == stream.operation.request_id =>
                        {
                            terminal = true;
                            break;
                        }
                        Some(NetworkEvent::Failed { request_id, error })
                            if request_id == stream.operation.request_id =>
                        {
                            return Err(RuntimeError::Network(error));
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
            }
            stream.bytes_read = stream.bytes_read.saturating_add(chunk.len());
            if stream.bytes_read > 16 * 1024 * 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "decoded HTTP response body".into(),
                });
            }
            Ok((
                chunk,
                terminal
                    || stream
                        .content_length
                        .is_some_and(|length| stream.bytes_read >= length),
            ))
        })();
        let (chunk, done) = match result {
            Ok(result) => result,
            Err(error) => {
                self.http_streams
                    .lock()
                    .ok()
                    .and_then(|mut streams| streams.remove(&stream_id));
                if let Some(network) = &self.network {
                    let _ = network.cancel_operation(entry.request_id);
                }
                return Err(error);
            }
        };
        if done {
            self.http_streams
                .lock()
                .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
                .remove(&stream_id);
        }
        Ok(ContextServiceResponse::HttpStreamChunk {
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
        if let Some(network) = &self.network {
            network
                .cancel_operation(entry.request_id)
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
        }
        Ok(ContextServiceResponse::HttpStreamAborted)
    }

    fn load_worker_script(
        &self,
        page_id: PageId,
        requested_url: &str,
        module: bool,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let network = self.network.clone().ok_or_else(|| {
            RuntimeError::ContextService("worker-script broker is unavailable".into())
        })?;
        let source_origin = self.context_owner()?.page_origin(page_id)?;
        if source_origin == "null" {
            return Err(RuntimeError::ContextService(
                "SecurityError: an opaque-origin document cannot create an external worker".into(),
            ));
        }
        let mut url = self
            .context_owner()?
            .resolve_page_url(page_id, requested_url)?;
        if url.origin().ascii_serialization() != source_origin {
            return Err(RuntimeError::ContextService(
                "SecurityError: dedicated worker scripts must be same-origin".into(),
            ));
        }
        for redirect in 0..=10 {
            let decision = self.context_owner()?.authorize_csp(
                page_id,
                &crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Worker,
                    url: url.to_string(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: true,
                },
            )?;
            if !decision.violations.is_empty() {
                self.context_owner()?
                    .handle_csp_violations(page_id, decision.violations.clone())?;
            }
            if !decision.allowed {
                return Err(RuntimeError::ContextService(
                    "worker script blocked by Content-Security-Policy worker-src".into(),
                ));
            }
            let response = collect_operation_bounded_with_cancel(
                &network,
                NetworkRequest {
                    request_id: self.next_http_stream_id.fetch_add(1, Ordering::Relaxed),
                    method: "GET".into(),
                    url: url.to_string(),
                    headers: vec![(
                        "accept".into(),
                        "text/javascript,application/javascript,*/*;q=0.1".into(),
                    )],
                    body: Vec::new(),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        kind: crate::cookies::CookieRequestKind::Worker,
                        initiator_url: Some(self.context_owner()?.page_url(page_id)?),
                        top_level_url: Some(self.context_owner()?.page_url(page_id)?),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                Duration::from_secs(30),
                1024 * 1024,
                &self.shutting_down,
            )
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
            if (300..400).contains(&response.status) {
                if redirect == 10 {
                    return Err(RuntimeError::Network(
                        "worker redirect limit exceeded".into(),
                    ));
                }
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        RuntimeError::Network("worker redirect omitted Location".into())
                    })?;
                url = url
                    .join(location)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                if url.origin().ascii_serialization() != source_origin {
                    return Err(RuntimeError::ContextService(
                        "SecurityError: worker redirect crossed origins".into(),
                    ));
                }
                continue;
            }
            if response.body.len() > 1024 * 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "dedicated worker script".into(),
                });
            }
            let content_type = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str());
            let (source, _) = oryn_net::decode_resource_text(
                &response.body,
                content_type,
                oryn_net::TextResourceKind::JavaScript,
            );
            let referrer_policy = response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let referrer_policy = if referrer_policy.trim().is_empty() {
                "strict-origin-when-cross-origin".to_string()
            } else {
                referrer_policy
            };
            return Ok(ContextServiceResponse::WorkerScript {
                url: url.to_string(),
                source,
                module,
                referrer_policy,
            });
        }
        Err(RuntimeError::Network(
            "worker request did not produce a response".into(),
        ))
    }

    fn authorize_inline_script(
        &self,
        page_id: PageId,
        source: &str,
        nonce: Option<&str>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let decision = self.context_owner()?.authorize_csp(
            page_id,
            &crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Script,
                url: "inline".into(),
                nonce: nonce.map(str::to_owned),
                inline_source: Some(source.to_owned()),
                parser_inserted: false,
                initiated_by_trusted_script: true,
            },
        )?;
        if !decision.violations.is_empty() {
            self.context_owner()?
                .handle_csp_violations(page_id, decision.violations.clone())?;
        }
        if !decision.allowed {
            return Err(RuntimeError::ContextService(
                "dynamic inline script blocked by Content-Security-Policy".into(),
            ));
        }
        Ok(ContextServiceResponse::ScriptAuthorized)
    }

    fn load_page_script(
        &self,
        page_id: PageId,
        requested_url: &str,
        module: bool,
        nonce: Option<&str>,
        initiator_url: Option<&str>,
        referrer_policy: Option<&str>,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let network = self.network.clone().ok_or_else(|| {
            RuntimeError::ContextService("script resource broker is unavailable".into())
        })?;
        let page_url = self.context_owner()?.page_url(page_id)?;
        let source_url = initiator_url.unwrap_or(&page_url);
        let source =
            Url::parse(source_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let source_origin = source.origin().ascii_serialization();
        let policy = referrer_policy
            .filter(|value| !value.trim().is_empty())
            .map_or(
                self.context_owner()?.page_referrer_policy(page_id)?,
                crate::referrer::ReferrerPolicy::parse_list,
            );
        let mut url = source
            .join(requested_url)
            .or_else(|_| Url::parse(requested_url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        for redirect in 0..=10 {
            if let Some(upgraded) = self.context_owner()?.upgrade_csp_url(page_id, &url)? {
                url = upgraded;
            }
            let decision = self.context_owner()?.authorize_csp(
                page_id,
                &crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Script,
                    url: url.to_string(),
                    nonce: nonce.map(str::to_owned),
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: true,
                },
            )?;
            if !decision.violations.is_empty() {
                self.context_owner()?
                    .handle_csp_violations(page_id, decision.violations.clone())?;
            }
            if !decision.allowed {
                return Err(RuntimeError::ContextService(
                    "dynamic script blocked by Content-Security-Policy script-src".into(),
                ));
            }
            if url.scheme() == "data" {
                let (media_type, body) = crate::network::decode_data_url(&url)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                if body.len() > MAX_PAGE_SCRIPT_BYTES {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "decoded dynamic script data URL".into(),
                    });
                }
                let source = decode_page_script(&body, media_type.as_deref())?;
                return Ok(ContextServiceResponse::ScriptSource {
                    url: url.to_string(),
                    source,
                    module,
                });
            }
            let mut headers = vec![(
                "accept".into(),
                "text/javascript,application/javascript,*/*;q=0.1".into(),
            )];
            if let Some(referrer) = policy.header(&source, &url) {
                headers.push(("referer".into(), referrer));
            }
            let response = collect_operation_bounded_with_cancel(
                &network,
                NetworkRequest {
                    request_id: self.next_http_stream_id.fetch_add(1, Ordering::Relaxed),
                    method: "GET".into(),
                    url: url.to_string(),
                    headers,
                    body: Vec::new(),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        kind: crate::cookies::CookieRequestKind::Script,
                        initiator_url: Some(source_url.into()),
                        top_level_url: Some(page_url.clone()),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                Duration::from_secs(30),
                MAX_PAGE_SCRIPT_BYTES,
                &self.shutting_down,
            )
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
            if (300..400).contains(&response.status) {
                if redirect == 10 {
                    return Err(RuntimeError::Network(
                        "dynamic script redirect limit exceeded".into(),
                    ));
                }
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        RuntimeError::Network("script redirect omitted Location".into())
                    })?;
                url = url
                    .join(location)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                continue;
            }
            if response.body.len() > MAX_PAGE_SCRIPT_BYTES {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "dynamic script resource".into(),
                });
            }
            if module && url.origin().ascii_serialization() != source_origin {
                let allowed = response.headers.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("access-control-allow-origin")
                        && (value.trim() == "*" || value.trim() == source_origin)
                });
                if !allowed {
                    return Err(RuntimeError::ContextService(
                        "cross-origin module script failed CORS".into(),
                    ));
                }
            }
            let content_type = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str());
            let source = decode_page_script(&response.body, content_type)?;
            return Ok(ContextServiceResponse::ScriptSource {
                url: url.to_string(),
                source,
                module,
            });
        }
        Err(RuntimeError::Network(
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
        let PageResourceOptions {
            initiator_url,
            referrer_policy,
            parent_encoding,
            mut post,
        } = options;
        self.validate_socket_owner(page_id)?;
        let form_navigation = post.is_some();
        if let Some(resource) = &post {
            if destination != "frame"
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
        let network = self.network.clone().ok_or_else(|| {
            RuntimeError::ContextService("page resource broker is unavailable".into())
        })?;
        let page_url = self.context_owner()?.page_url(page_id)?;
        let source_url = initiator_url.unwrap_or(&page_url);
        let source =
            Url::parse(source_url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        let policy = referrer_policy
            .filter(|value| !value.trim().is_empty())
            .map_or(
                self.context_owner()?.page_referrer_policy(page_id)?,
                crate::referrer::ReferrerPolicy::parse_list,
            );
        let mut url = source
            .join(requested_url)
            .or_else(|_| Url::parse(requested_url))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        for redirect in 0..=10 {
            if let Some(upgraded) = self.context_owner()?.upgrade_csp_url(page_id, &url)? {
                url = upgraded;
            }
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
            let decision = self.context_owner()?.authorize_csp(
                page_id,
                &crate::csp::CspRequest {
                    destination: csp_destination,
                    url: url.to_string(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: true,
                    initiated_by_trusted_script: false,
                },
            )?;
            if !decision.violations.is_empty() {
                self.context_owner()?
                    .handle_csp_violations(page_id, decision.violations.clone())?;
            }
            if !decision.allowed {
                return Err(RuntimeError::ContextService(format!(
                    "{destination} blocked by Content-Security-Policy"
                )));
            }
            if url.scheme() == "data" {
                let (media_type, body) = crate::network::decode_data_url(&url)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                if body.len() > 16 * 1024 * 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: format!("decoded {destination} data URL"),
                    });
                }
                let (source, encoding) = oryn_net::decode_resource_text_with_fallback(
                    &body,
                    media_type.as_deref(),
                    page_resource_text_kind(text_kind, media_type.as_deref()),
                    None,
                );
                return Ok(ContextServiceResponse::PageResource {
                    url: url.to_string(),
                    source,
                    encoding: encoding.into(),
                    destination: destination.into(),
                    content_type: media_type,
                    document_referrer: String::new(),
                    referrer_policy: crate::referrer::ReferrerPolicy::default().as_str().into(),
                    refresh_url: None,
                });
            }
            let mut request_headers = vec![("accept".into(), accept.into())];
            if let Some(resource) = &post {
                request_headers.push(("content-type".into(), resource.content_type.clone()));
            }
            if let Some(referrer) = policy.header(&source, &url) {
                request_headers.push(("referer".into(), referrer));
            }
            let response = collect_operation_bounded_with_cancel(
                &network,
                NetworkRequest {
                    request_id: self.next_http_stream_id.fetch_add(1, Ordering::Relaxed),
                    method: if post.is_some() { "POST" } else { "GET" }.into(),
                    url: url.to_string(),
                    headers: request_headers,
                    body: post
                        .as_ref()
                        .map_or_else(Vec::new, |resource| resource.bytes.clone()),
                    credentials: true,
                    security: crate::cookies::CookieRequestContext {
                        kind: cookie_kind,
                        initiator_url: Some(source_url.to_owned()),
                        top_level_url: Some(page_url.clone()),
                        ..Default::default()
                    },
                    resolved_addrs: Vec::new(),
                },
                Duration::from_secs(30),
                16 * 1024 * 1024,
                &self.shutting_down,
            )
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                oryn_js::navigation::PostResource::redirect(&mut post, response.status);
                if redirect == 10 {
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
                        RuntimeError::Network(format!("{destination} redirect omitted Location"))
                    })?;
                url = url
                    .join(location)
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                continue;
            }
            let content_type = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str());
            let (source, encoding) = oryn_net::decode_resource_text_with_fallback(
                &response.body,
                content_type,
                page_resource_text_kind(text_kind, content_type),
                if destination == "frame" && source.origin() == url.origin() {
                    parent_encoding
                } else {
                    None
                },
            );
            let document_referrer = policy
                .header(
                    &source_url.parse().map_err(|error| {
                        RuntimeError::Network(format!("invalid page-resource initiator: {error}"))
                    })?,
                    &url,
                )
                .unwrap_or_default();
            let response_policy = response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
                .map(|(_, value)| crate::referrer::ReferrerPolicy::parse_list(value))
                .next_back()
                .unwrap_or_default();
            let refresh_url = response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("refresh"))
                .filter_map(|(_, value)| value.split_once(';').map(|(_, target)| target))
                .filter_map(|target| target.split_once('=').map(|(_, target)| target.trim()))
                .next_back()
                .and_then(|target| url.join(target).ok())
                .map(|target| target.to_string());
            if destination == "frame" {
                self.authorized_storage_origins
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService(
                            "authorized frame-origin registry is poisoned".into(),
                        )
                    })?
                    .insert(url.origin().ascii_serialization());
            }
            return Ok(ContextServiceResponse::PageResource {
                url: url.to_string(),
                source,
                encoding: encoding.into(),
                destination: destination.into(),
                content_type: content_type.map(str::to_owned),
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
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let socket_id = self.next_socket_id.fetch_add(1, Ordering::Relaxed);
        let mut observation = self.network.as_ref().and_then(|network| {
            network
                .resource_audit()
                .begin_websocket(page_id.0, socket_id, url)
        });
        let mut url = Url::parse(url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(upgraded) = self.context_owner()?.upgrade_csp_url(page_id, &url)? {
            url = upgraded;
        }
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(RuntimeError::ContextService(
                "WebSocket URL must use ws or wss".into(),
            ));
        }
        let decision = self.context_owner()?.authorize_csp(
            page_id,
            &crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Connect,
                url: url.to_string(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            },
        )?;
        if !decision.violations.is_empty() {
            self.context_owner()?
                .handle_csp_violations(page_id, decision.violations.clone())?;
        }
        if !decision.allowed {
            return Err(RuntimeError::ContextService(
                "WebSocket blocked by Content-Security-Policy connect-src".into(),
            ));
        }
        enforce_mixed_content(&self.context_owner()?.page_url(page_id)?, &url)?;
        let addresses =
            crate::network::resolve_and_authorize(self.context_owner()?.network_policy(), &url)
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let connector = fixture_websocket_connector(
            &url,
            !addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()),
            std::env::var_os("ORYN_WPT_CA_CERT").as_deref(),
        )?;
        let timeout = self.context_owner()?.request_timeout();
        let mut stream = None;
        let mut last_error = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(candidate) => {
                    stream = Some(candidate);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let stream = stream.ok_or_else(|| {
            RuntimeError::Network(
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "WebSocket resolver returned no address".into()),
            )
        })?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|_| stream.set_write_timeout(Some(timeout)))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let origin = self.context_owner()?.page_origin(page_id)?;
        request.headers_mut().insert(
            "origin",
            origin
                .parse()
                .map_err(|error| RuntimeError::Network(format!("invalid Origin: {error}")))?,
        );
        if let Some(cookie) = self.context_owner()?.cookie_header_for_request(
            page_id,
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
        let protocol = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::Closed);
        }
        if let Some(mut audit) = observation.take() {
            audit.opened(response.status().as_u16());
            self.socket_audits
                .lock()
                .map_err(|_| RuntimeError::ContextService("WebSocket audit lock poisoned".into()))?
                .insert(socket_id, audit);
        }
        sockets.insert(socket_id, socket);
        Ok(ContextServiceResponse::WebSocketOpened {
            socket_id,
            protocol,
        })
    }

    fn send_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        frame: WebSocketFrame,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?;
        let socket = sockets
            .get_mut(&socket_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
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
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?;
        let socket = sockets
            .get_mut(&socket_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
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
                    frames.push(WebSocketFrame::Close {
                        code: None,
                        reason: String::new(),
                    });
                    break;
                }
                Err(
                    tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ),
                ) => {
                    frames.push(WebSocketFrame::Close {
                        code: None,
                        reason: String::new(),
                    });
                    break;
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
        let mut socket = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?
            .remove(&socket_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        let close = socket.close(Some(CloseFrame {
            code: code.map(CloseCode::from).unwrap_or(CloseCode::Normal),
            reason: reason.into(),
        }));
        if let Ok(mut audits) = self.socket_audits.lock()
            && let Some(mut audit) = audits.remove(&socket_id)
        {
            audit.finish(if close.is_ok() {
                "close_requested"
            } else {
                "failed"
            });
        }
        if let Err(error) = close
            && !matches!(
                error,
                tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    )
            )
        {
            return Err(RuntimeError::Network(error.to_string()));
        }
        Ok(ContextServiceResponse::WebSocketClosed)
    }

    fn validate_storage_scope(&self, request: &StorageRequest) -> Result<(), RuntimeError> {
        if self
            .context_owner()?
            .is_opaque_origin(self.source_page_id)?
        {
            return Err(RuntimeError::ContextService(
                "SecurityError: sandboxed opaque origins cannot access storage".into(),
            ));
        }
        let current_page_origin = self.context_owner()?.page_origin(self.source_page_id)?;
        let origin_is_authorized = |origin: &str| -> Result<bool, RuntimeError> {
            Ok(origin == current_page_origin
                || self
                    .authorized_storage_origins
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService(
                            "authorized frame-origin registry is poisoned".into(),
                        )
                    })?
                    .contains(origin))
        };
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
    let service = Arc::new(service);
    let result = serve_context_transport(stream, service.clone());
    service.cancel_all_network_operations();
    result
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
    loop {
        let mut line = Vec::new();
        let read = read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if read == 0 {
            break;
        }
        let request = serde_json::from_slice::<RequestEnvelope>(&line);
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
            Err(error) => (
                0,
                None,
                None,
                Some(format!("malformed context request: {error}")),
            ),
        };
        let service = service.clone();
        let writer = writer.clone();
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
                let _permit = permit;
                let (response, error) = if let Some(request) = request {
                    match service.request_routed(route.as_ref(), request) {
                        Ok(response) => (Some(response), None),
                        Err(error) => (None, Some(error.to_string())),
                    }
                } else {
                    (None, error)
                };
                let mut writer = writer.lock().map_err(|_| {
                    RuntimeError::ContextService("context writer lock poisoned".into())
                })?;
                // Serialize under the writer lock: at most one bounded encoded
                // response buffer is retained by this channel, not one per dispatch.
                let encoded = encode_context_response(
                    ResponseEnvelope {
                        protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                        route,
                        correlation_id: message_id,
                        response,
                        error,
                    },
                    MAX_CONTEXT_FRAME_BYTES,
                );
                let encoded = match encoded {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        let _ = writer.shutdown(Shutdown::Both);
                        return Err(error);
                    }
                };
                let written = writer
                    .write_all(&encoded)
                    .and_then(|_| writer.flush())
                    .map_err(|error| RuntimeError::ContextService(error.to_string()));
                if written.is_err() {
                    let _ = writer.shutdown(Shutdown::Both);
                }
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

fn cors_non_safelisted_headers(headers: &[(String, String)]) -> Vec<String> {
    let mut names = headers
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            // `Origin` and `Referer` are user-agent controlled forbidden
            // request headers. They participate in CORS validation but do not
            // themselves make an otherwise simple request preflighted.
            let safe = matches!(
                lower.as_str(),
                "accept" | "accept-language" | "origin" | "referer"
            ) || (lower == "content-type"
                && matches!(
                    value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
                ));
            (!safe).then_some(lower)
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn enforce_mixed_content(source_origin: &str, target: &Url) -> Result<(), RuntimeError> {
    let secure_source =
        source_origin.starts_with("https://") || source_origin.starts_with("wss://");
    if secure_source && matches!(target.scheme(), "http" | "ws") {
        return Err(RuntimeError::ContextService(format!(
            "active mixed content blocked: {source_origin} -> {target}"
        )));
    }
    Ok(())
}

fn cors_preflight_required(method: &str, headers: &[(String, String)]) -> bool {
    !matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "POST"
    ) || !cors_non_safelisted_headers(headers).is_empty()
}

fn response_header<'a>(response: &'a NetworkResponse, requested: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(requested))
        .map(|(_, value)| value.as_str())
}

fn cors_origin_allowed(source_origin: &str, credentials: bool, response: &NetworkResponse) -> bool {
    cors_headers_allowed(source_origin, credentials, &response.headers)
}

fn cors_headers_allowed(
    source_origin: &str,
    credentials: bool,
    headers: &[(String, String)],
) -> bool {
    let header = |requested: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(requested))
            .map(|(_, value)| value.as_str())
    };
    header("access-control-allow-origin")
        .is_some_and(|allowed| allowed == source_origin || (!credentials && allowed == "*"))
        && (!credentials
            || header("access-control-allow-credentials")
                .is_some_and(|value| value.eq_ignore_ascii_case("true")))
}

fn enforce_cors_preflight(
    source_origin: &str,
    method: &str,
    requested_headers: &[String],
    response: &NetworkResponse,
) -> Result<(), RuntimeError> {
    let allowed_methods = response_header(response, "access-control-allow-methods")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    let allowed_headers = response_header(response, "access-control-allow-headers")
        .unwrap_or_default()
        .split(',')
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    if !(200..300).contains(&response.status)
        || !cors_origin_allowed(source_origin, false, response)
        || !allowed_methods
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(method))
        || requested_headers
            .iter()
            .any(|header| !allowed_headers.iter().any(|allowed| allowed == header))
    {
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
        oryn_net::TextResourceKind::Fetch
    } else {
        default
    }
}

#[cfg(all(test, feature = "in-process-probe"))]
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
    fn page_script_budget_bounds_bytes_and_decoded_source() {
        assert_eq!(
            decode_page_script(&vec![b' '; MAX_PAGE_SCRIPT_BYTES], None)
                .unwrap()
                .len(),
            MAX_PAGE_SCRIPT_BYTES
        );
        assert!(decode_page_script(&vec![b' '; MAX_PAGE_SCRIPT_BYTES + 1], None).is_err());
        assert!(
            decode_page_script(
                &vec![0x80; MAX_PAGE_SCRIPT_BYTES / 3 + 1],
                Some("text/javascript; charset=windows-1252")
            )
            .is_err()
        );
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
        let request = || ContextServiceRequest::GetReferrerPolicy { page_id: PageId(1) };
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
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: None,
                    message_id,
                    request: ContextServiceRequest::GetReferrerPolicy { page_id: PageId(1) },
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
                client.request(ContextServiceRequest::GetReferrerPolicy { page_id: PageId(1) }),
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
                .request(ContextServiceRequest::GetReferrerPolicy { page_id: PageId(1) })
                .is_err()
        );
        server.join().unwrap();
    }

    #[test]
    fn indexeddb_request_envelope_preserves_numeric_keys_and_values() {
        use crate::storage::{IdbCommand, IdbWrite, StorageRequest};

        for number in [9007199254740991.0_f64, 9007199254740992.0, 0.1, -0.1] {
            let envelope = RequestEnvelope {
                protocol_version: 1,
                route: None,
                message_id: 7,
                request: ContextServiceRequest::Storage {
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

    struct FixtureNetwork;

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
            assert!(
                request
                    .headers
                    .contains(&("origin".into(), "https://example.test".into()))
            );
            assert!(
                !request
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            );
            Ok(crate::network::NetworkResponse {
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
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/plain".into()),
                    ("access-control-allow-origin".into(), "*".into()),
                ],
                content_length: Some(5),
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
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/plain".into()),
                    ("access-control-allow-origin".into(), "*".into()),
                ],
                content_length: Some(2),
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
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("access-control-allow-origin".into(), "*".into())],
                    content_length: None,
                })
                .unwrap();
            Ok(NetworkOperation {
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
        drop(service);
        assert_eq!(cancelled.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn worker_replacement_cancels_scheduled_resource_waits() {
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        let cancelled = Arc::new(AtomicU64::new(0));
        let service = Arc::new(ParentContextService::new_with_network(
            context,
            page.page_id(),
            Some(SharedNetworkBroker::new(StalledCancellationNetwork {
                cancelled: cancelled.clone(),
            })),
        ));
        let request_service = service.clone();
        let request = std::thread::spawn(move || {
            request_service.handle(ContextServiceRequest::LoadScript {
                page_id: page.page_id(),
                url: "/slow.js".into(),
                module: false,
                nonce: None,
                initiator_url: None,
                referrer_policy: None,
            })
        });
        std::thread::sleep(Duration::from_millis(10));
        service.cancel_all_network_operations();
        let error = request.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("cancelled"));
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
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://example.test/app", "<p>ready</p>")
            .await
            .unwrap();
        let page_id = page.page_id();
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(FixtureNetwork)),
        );
        let started = service
            .handle(ContextServiceRequest::HttpStreamStart {
                page_id,
                request: HttpStreamRequest {
                    request_id: 7,
                    method: "GET".into(),
                    url: "https://example.test/stream".into(),
                    headers: vec![
                        ("Accept".into(), "text/plain".into()),
                        ("Authorization".into(), "secret".into()),
                    ],
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
                service.handle(ContextServiceRequest::HttpStreamAbort { page_id, stream_id }),
                Ok(ContextServiceResponse::HttpStreamAborted)
            ));
        }
        assert!(matches!(
            service.handle(ContextServiceRequest::HttpStreamRead {
                page_id: PageId(92),
                stream_id,
                max_bytes: 1,
            }),
            Err(RuntimeError::ContextService(_))
        ));
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
        context.grant_navigation_chain(page.page_id());
        let service = ParentContextService::new(context.clone(), page.page_id());

        assert!(matches!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page.page_id(),
                    url: "https://allowed.example/redirect-1".into(),
                })
                .unwrap(),
            ContextServiceResponse::NavigationAuthorized
        ));
        assert!(matches!(
            service
                .handle(ContextServiceRequest::AuthorizeNavigation {
                    page_id: page.page_id(),
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
            Some(SharedNetworkBroker::new(FixtureNetwork)),
        );
        let request = HttpStreamRequest {
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
                page_id,
                stream_id,
                max_bytes: 3,
            })
            .unwrap()
        else {
            panic!("data chunk missing")
        };
        assert_eq!(body_base64, "AID/");
        assert!(done);
        assert!(
            service
                .handle(ContextServiceRequest::HttpStreamRead {
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
                page_id: page.page_id(),
                request
            }),
            Err(RuntimeError::ProtocolViolation(_))
        ));
    }

    #[test]
    fn parent_cors_checks_credentials_and_preflight_permissions() {
        let response = NetworkResponse {
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
                "PUT",
                &["content-type".into(), "if-match".into()],
                &response,
            )
            .is_ok()
        );
        assert!(
            enforce_cors_preflight(
                "https://app.example",
                "DELETE",
                &["x-forbidden".into()],
                &response,
            )
            .is_err()
        );

        let wildcard = NetworkResponse {
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
