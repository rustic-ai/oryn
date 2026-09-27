use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    os::unix::net::UnixStream,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::StreamExt;
use oryn_net::HttpTransport as _;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use url::Url;

#[path = "response_policy_token.rs"]
mod response_policy_token;
pub use response_policy_token::ResponsePolicyToken;

use oryn_net::cache::{CacheEntry, CacheKey, MemoryCache};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::{
    cookies::{ContextCookieStore, CookieRequestContext},
    security::{NetworkPolicy, PolicyDenied},
};

const BROKER_PROTOCOL_VERSION: u32 = 9;
use crate::worker_route::WorkerRoute;
const MAX_BROKER_FRAME_BYTES: usize = 24 * 1024 * 1024;
const MAX_DECODED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const PERSISTENT_HTTP_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("http_cache_v1");

/// Decode a document response using the same MIME distinction as native parsing.
#[cfg(any(test, not(feature = "v8-host")))]
pub(crate) fn decode_navigation_text(
    bytes: &[u8],
    content_type: Option<&str>,
) -> (String, &'static str) {
    if oryn_dom::xml::media_document_tag(content_type.unwrap_or("")).is_some() {
        return (String::new(), "UTF-8");
    }
    let mut decoder =
        oryn_net::IncrementalTextDecoder::new(content_type, navigation_text_kind(content_type))
            .with_html_detection();
    let mut source = decoder.push(bytes);
    source.push_str(&decoder.finish());
    (source, decoder.encoding_name().unwrap_or("UTF-8"))
}

pub(crate) fn navigation_text_kind(content_type: Option<&str>) -> oryn_net::TextResourceKind {
    let essence = content_type
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("text/html")
        .trim()
        .to_ascii_lowercase();
    if matches!(essence.as_str(), "text/xml" | "application/xml") || essence.ends_with("+xml") {
        oryn_net::TextResourceKind::Xml
    } else if oryn_dom::xml::is_text_mime(&essence) {
        oryn_net::TextResourceKind::PlainText
    } else {
        oryn_net::TextResourceKind::Html
    }
}

pub(crate) fn decode_data_url(url: &Url) -> Result<(Option<String>, Vec<u8>), NetworkError> {
    // Fetch's data-URL processor excludes the fragment, preserves malformed
    // percent escapes, and uses forgiving base64 rather than strict RFC 4648.
    let mut url = url.clone();
    url.set_fragment(None);
    let (metadata, payload) = url
        .as_str()
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .ok_or_else(|| NetworkError::Transport("malformed data URL".into()))?;
    let ascii_space = |ch: char| matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' ');
    let mut metadata = metadata.trim_matches(ascii_space);
    let base64 = metadata.rsplit_once(';').filter(|(_, suffix)| {
        suffix
            .trim_start_matches(' ')
            .eq_ignore_ascii_case("base64")
    });
    if let Some((mime, _)) = base64 {
        metadata = mime;
    }
    let mut bytes = Vec::with_capacity(payload.len().min(MAX_DECODED_RESPONSE_BYTES));
    let input = payload.as_bytes();
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%'
            && let Some(pair) = input.get(index + 1..index + 3)
            && let Ok(pair) = std::str::from_utf8(pair)
            && let Ok(byte) = u8::from_str_radix(pair, 16)
        {
            bytes.push(byte);
            index += 3;
        } else {
            bytes.push(input[index]);
            index += 1;
        }
        if bytes.len() > MAX_DECODED_RESPONSE_BYTES * 4 / 3 + 4 {
            return Err(NetworkError::Transport(
                "data URL body exceeds limit".into(),
            ));
        }
    }
    if base64.is_some() {
        bytes.retain(|byte| !matches!(byte, b'\t' | b'\n' | 0x0c | b'\r' | b' '));
        let config = base64::engine::general_purpose::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true);
        bytes = base64::engine::GeneralPurpose::new(&base64::alphabet::STANDARD, config)
            .decode(bytes)
            .map_err(|error| {
                NetworkError::Transport(format!("invalid data URL base64: {error}"))
            })?;
    }
    if bytes.len() > MAX_DECODED_RESPONSE_BYTES {
        return Err(NetworkError::Transport(
            "data URL body exceeds limit".into(),
        ));
    }
    let metadata = if metadata.starts_with(';') {
        format!("text/plain{metadata}")
    } else {
        metadata.to_owned()
    };
    let media_type = oryn_js::mime_type::MimeType::parse(&metadata)
        .map(|mime| mime.serialize())
        .unwrap_or_else(|| "text/plain;charset=US-ASCII".into());
    Ok((Some(media_type), bytes))
}

fn operation_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("oryn-network-events")
            .build()
            .expect("network event runtime")
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub request_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    #[serde(default = "default_credentials")]
    pub credentials: bool,
    #[serde(default)]
    pub security: CookieRequestContext,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub resolved_addrs: Vec<SocketAddr>,
}

const fn default_credentials() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkResponse {
    /// Private parent-issued navigation authority; never exposed as an HTTP header.
    #[serde(default)]
    pub response_policy: Option<ResponsePolicyToken>,
    #[serde(default)]
    pub timing: Option<oryn_net::TransportTiming>,
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A policy-approved response whose body remains owned by the parent and is
/// consumed only when the worker asks for another bounded chunk.
pub struct NetworkStreamResponse {
    pub response_policy: Option<ResponsePolicyToken>,
    pub encoded_bytes: Option<Arc<std::sync::atomic::AtomicU64>>,
    pub timing: Option<oryn_net::TransportTiming>,
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub content_length: Option<usize>,
    pub upload: Option<oryn_net::UploadProgress>,
    pub body: Box<dyn Read + Send>,
}

/// Events emitted by one independently cancellable browser-network operation.
/// Headers are always emitted before body chunks and completion.  Keeping this
/// contract in `oryn-native` lets policy/cache/cookie adapters decorate a live
/// body without turning it back into a blocking `Read` implementation.
/// Failure metadata received only through the authenticated host broker route.
/// The legacy string form stays an ordinary failure, regardless of its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(untagged)]
pub enum NetworkFailure {
    #[error("server certificate rejected: {certificate_rejected}")]
    Certificate { certificate_rejected: String },
    #[error("{0}")]
    Message(String),
}
impl std::ops::Deref for NetworkFailure {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Self::Message(message) => message,
            Self::Certificate {
                certificate_rejected,
            } => certificate_rejected,
        }
    }
}
impl From<NetworkError> for NetworkFailure {
    fn from(error: NetworkError) -> Self {
        match error {
            NetworkError::CertificateRejected(detail) => Self::Certificate {
                certificate_rejected: detail,
            },
            other => Self::Message(other.to_string()),
        }
    }
}
impl From<String> for NetworkFailure {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}
impl From<&str> for NetworkFailure {
    fn from(message: &str) -> Self {
        Self::Message(message.into())
    }
}
impl From<oryn_net::TransportError> for NetworkFailure {
    fn from(error: oryn_net::TransportError) -> Self {
        match error {
            oryn_net::TransportError::CertificateRejected(detail) => Self::Certificate {
                certificate_rejected: detail,
            },
            other => Self::Message(other.to_string()),
        }
    }
}
impl From<NetworkFailure> for NetworkError {
    fn from(error: NetworkFailure) -> Self {
        match error {
            NetworkFailure::Certificate {
                certificate_rejected,
            } => Self::CertificateRejected(certificate_rejected),
            NetworkFailure::Message(message) => Self::Transport(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkEvent {
    Headers {
        response_policy: Option<ResponsePolicyToken>,
        timing: Option<Box<oryn_net::TransportTiming>>,
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        content_length: Option<usize>,
        upload: Option<oryn_net::UploadProgress>,
    },
    Chunk {
        request_id: u64,
        bytes: bytes::Bytes,
    },
    Complete {
        encoded_body_size: Option<u64>,
        request_id: u64,
    },
    Failed {
        request_id: u64,
        error: NetworkFailure,
    },
}

pub struct NetworkOperation {
    pub(crate) body_demand_pending: std::cell::Cell<bool>,
    pub(crate) prefetched: std::collections::VecDeque<NetworkEvent>,
    pub request_id: u64,
    pub events: tokio::sync::mpsc::Receiver<NetworkEvent>,
    pub(crate) demand: Option<tokio::sync::mpsc::Sender<()>>,
}

impl NetworkOperation {
    fn observe_event(&self, event: &NetworkEvent) {
        if matches!(
            event,
            NetworkEvent::Chunk { .. }
                | NetworkEvent::Complete { .. }
                | NetworkEvent::Failed { .. }
        ) {
            self.body_demand_pending.set(false);
        }
    }
    pub fn try_event(&mut self) -> Result<NetworkEvent, tokio::sync::mpsc::error::TryRecvError> {
        if let Some(event) = self.prefetched.pop_front() {
            return Ok(event);
        }
        let event = self.events.try_recv()?;
        self.observe_event(&event);
        Ok(event)
    }
    pub fn blocking_event(&mut self) -> Option<NetworkEvent> {
        if let Some(event) = self.prefetched.pop_front() {
            return Some(event);
        }
        let event = self.events.blocking_recv()?;
        self.observe_event(&event);
        Some(event)
    }
    pub async fn next_event(&mut self) -> Option<NetworkEvent> {
        if let Some(event) = self.prefetched.pop_front() {
            return Some(event);
        }
        let event = self.events.recv().await?;
        self.observe_event(&event);
        Some(event)
    }
    pub fn request_body_chunk(&self) {
        if !self.prefetched.is_empty() || self.body_demand_pending.get() {
            return;
        }
        if let Some(demand) = &self.demand {
            match demand.try_send(()) {
                Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    self.body_demand_pending.set(true)
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
    }
}

/// Retained MIME sniffing stage for page actors. Poll never waits for IO;
/// admitted demand, replay chunks and terminal ACK retain one operation owner.
#[cfg(any(feature = "v8-host", test))]
pub(crate) struct DocumentMimeSniff {
    operation: Option<NetworkOperation>,
    state: MimePrefixState,
}
#[cfg(any(feature = "v8-host", test))]
impl DocumentMimeSniff {
    pub(crate) fn new(
        operation: NetworkOperation,
        supplied: Option<String>,
        no_sniff: bool,
    ) -> Self {
        Self {
            operation: Some(operation),
            state: MimePrefixState::new(supplied, no_sniff),
        }
    }
    pub(crate) fn poll(&mut self) -> std::task::Poll<Result<(NetworkOperation, String), String>> {
        let Some(operation) = self.operation.as_mut() else {
            return std::task::Poll::Ready(Err("MIME sniff already consumed".into()));
        };
        match self.state.poll(operation) {
            std::task::Poll::Pending => std::task::Poll::Pending,
            std::task::Poll::Ready(result) => {
                std::task::Poll::Ready(result.map(|mime| (self.operation.take().unwrap(), mime)))
            }
        }
    }
}
struct MimePrefixState {
    supplied: Option<String>,
    no_sniff: bool,
    deadline: std::time::Instant,
    prefix: Vec<u8>,
    retained: std::collections::VecDeque<NetworkEvent>,
    done: bool,
}
impl MimePrefixState {
    fn new(supplied: Option<String>, no_sniff: bool) -> Self {
        Self {
            supplied,
            no_sniff,
            deadline: std::time::Instant::now() + Duration::from_millis(300),
            prefix: Vec::new(),
            retained: Default::default(),
            done: false,
        }
    }
    fn finish(
        &mut self,
        operation: &mut NetworkOperation,
    ) -> std::task::Poll<Result<String, String>> {
        self.done = true;
        self.retained.append(&mut operation.prefetched);
        operation.prefetched = std::mem::take(&mut self.retained);
        std::task::Poll::Ready(Ok(oryn_net::mime_sniff::document_type(
            self.supplied.as_deref(),
            self.no_sniff,
            &self.prefix,
        )))
    }
    fn poll(
        &mut self,
        operation: &mut NetworkOperation,
    ) -> std::task::Poll<Result<String, String>> {
        use std::task::Poll;
        if self.done {
            return Poll::Ready(Err("MIME prefix already consumed".into()));
        }
        if !oryn_net::mime_sniff::needs_prefix(self.supplied.as_deref(), self.no_sniff) {
            return self.finish(operation);
        }
        for _ in 0..16 {
            if self.prefix.len() >= oryn_net::mime_sniff::PREFIX_LIMIT
                || std::time::Instant::now() >= self.deadline
            {
                return self.finish(operation);
            }
            match operation.try_event() {
                Ok(event) => {
                    match &event {
                        NetworkEvent::Chunk { request_id, bytes }
                            if *request_id == operation.request_id =>
                        {
                            let count = bytes
                                .len()
                                .min(oryn_net::mime_sniff::PREFIX_LIMIT - self.prefix.len());
                            self.prefix.extend_from_slice(&bytes[..count]);
                        }
                        NetworkEvent::Complete {
                            encoded_body_size,
                            request_id,
                        } if *request_id == operation.request_id => {
                            self.retained.push_back(event);
                            return self.finish(operation);
                        }
                        NetworkEvent::Failed { request_id, error }
                            if *request_id == operation.request_id =>
                        {
                            return Poll::Ready(Err(error.to_string()));
                        }
                        _ => {
                            return Poll::Ready(Err(
                                "MIME prefix response identity/order mismatch".into(),
                            ));
                        }
                    }
                    self.retained.push_back(event);
                    if self.retained.len() > oryn_net::mime_sniff::PREFIX_LIMIT + 1 {
                        return Poll::Ready(Err("MIME prefix event budget exceeded".into()));
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    operation.request_body_chunk();
                    return Poll::Pending;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Poll::Ready(Err(
                        "MIME prefix disconnected before terminal acknowledgement".into(),
                    ));
                }
            }
        }
        Poll::Pending
    }
}
/// Worker-side compatibility driver. Page actors use DocumentMimeSniff directly.
pub(crate) fn sniff_document_operation(
    operation: &mut NetworkOperation,
    supplied: Option<&str>,
    no_sniff: bool,
    mut checkpoint: impl FnMut() -> Result<(), String>,
) -> Result<String, String> {
    let mut state = MimePrefixState::new(supplied.map(str::to_owned), no_sniff);
    let result = loop {
        if let Err(error) = checkpoint() {
            break Err(error);
        }
        match state.poll(operation) {
            std::task::Poll::Ready(result) => break result,
            std::task::Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
        }
    };
    state.retained.append(&mut operation.prefetched);
    operation.prefetched = std::mem::take(&mut state.retained);
    result
}
pub(crate) fn response_no_sniff(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("x-content-type-options"))
        .flat_map(|(_, value)| value.split(','))
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("nosniff"))
}

/// Compatibility collector for APIs whose public result is intentionally a
/// complete byte buffer (for example script compilation). It still drives the
/// asynchronous operation contract and explicit demand; production transport
/// implementations never re-enter their blocking `send` path.
pub fn collect_operation(
    broker: &(impl NetworkBroker + ?Sized),
    request: NetworkRequest,
) -> Result<NetworkResponse, NetworkError> {
    collect_operation_bounded(
        broker,
        request,
        Duration::from_secs(30),
        MAX_DECODED_RESPONSE_BYTES,
    )
}

/// Complete a deliberately buffered resource over the operation protocol.
/// Unlike the historical collector, this path is deadline-, size-, and
/// correlation-bounded and explicitly cancels the transport on every
/// non-successful terminal path. It is reserved for APIs whose result is
/// intrinsically a complete byte sequence (scripts, images, and preflights).
pub fn collect_operation_bounded(
    broker: &(impl NetworkBroker + ?Sized),
    request: NetworkRequest,
    timeout: Duration,
    max_body_bytes: usize,
) -> Result<NetworkResponse, NetworkError> {
    collect_operation_bounded_inner(broker, request, timeout, max_body_bytes, None)
}

/// Complete a bounded operation while an external page/worker lifetime remains
/// valid. The flag is observed between every transport event and wait; setting
/// it guarantees the parent broker is cancelled before a late completion can
/// be consumed by a replacement document.
/// Drain an already admitted response after its headers. Used when integrity
/// requires complete representation bytes before exposing a Response. The
/// caller retains cancellation custody for the operation throughout this call.
#[cfg(test)]
pub(crate) fn collect_started_body(
    operation: &mut NetworkOperation,
    deadline: std::time::Instant,
    limit: usize,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Vec<u8>, NetworkError> {
    collect_started_body_with_check(operation, deadline, limit, cancelled, None)
}
#[cfg(test)]
pub(crate) fn collect_started_body_with_check(
    operation: &mut NetworkOperation,
    deadline: std::time::Instant,
    limit: usize,
    cancelled: &std::sync::atomic::AtomicBool,
    external: Option<&dyn Fn() -> bool>,
) -> Result<Vec<u8>, NetworkError> {
    collect_started_body_with_metadata(operation, deadline, limit, cancelled, external)
        .map(|body| body.bytes)
}

/// Representation bytes and the transport's independent encoded-byte count.
/// Missing metadata stays missing; decoded length is never an encoded count.
pub(crate) struct CollectedBody {
    pub bytes: Vec<u8>,
    pub encoded_body_size: Option<u64>,
}

pub(crate) fn collect_started_body_with_metadata(
    operation: &mut NetworkOperation,
    deadline: std::time::Instant,
    limit: usize,
    cancelled: &std::sync::atomic::AtomicBool,
    external: Option<&dyn Fn() -> bool>,
) -> Result<CollectedBody, NetworkError> {
    let mut body = Vec::new();
    operation.request_body_chunk();
    loop {
        if cancelled.load(std::sync::atomic::Ordering::Acquire)
            || external.is_some_and(|check| check())
        {
            return Err(NetworkError::Cancelled);
        }
        if std::time::Instant::now() >= deadline {
            return Err(NetworkError::TimedOut {
                timeout: Duration::ZERO,
            });
        }
        let event = match operation.try_event() {
            Ok(event) => event,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                std::thread::park_timeout(Duration::from_millis(1));
                continue;
            }
            Err(_) => {
                return Err(NetworkError::Transport(
                    "response closed before completion".into(),
                ));
            }
        };
        match event {
            NetworkEvent::Chunk { request_id, bytes } if request_id == operation.request_id => {
                if body.len().saturating_add(bytes.len()) > limit {
                    return Err(NetworkError::ResponseTooLarge { limit });
                }
                body.extend_from_slice(&bytes);
                operation.request_body_chunk();
            }
            NetworkEvent::Complete {
                encoded_body_size,
                request_id,
            } if request_id == operation.request_id => {
                return Ok(CollectedBody {
                    bytes: body,
                    encoded_body_size,
                });
            }
            NetworkEvent::Failed { request_id, error } if request_id == operation.request_id => {
                return Err(NetworkError::from(error));
            }
            _ => {
                return Err(NetworkError::Transport(
                    "unmatched or out-of-order response body event".into(),
                ));
            }
        }
    }
}

pub(crate) fn buffered_response_operation(
    request_id: u64,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Result<NetworkOperation, NetworkError> {
    let (sender, events) = tokio::sync::mpsc::channel(3);
    sender
        .try_send(NetworkEvent::Headers {
            response_policy: None,
            timing: None,
            request_id,
            status,
            headers,
            content_length: Some(body.len()),
            upload: None,
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    sender
        .try_send(NetworkEvent::Chunk {
            request_id,
            bytes: body.into(),
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    sender
        .try_send(NetworkEvent::Complete {
            encoded_body_size: None,
            request_id,
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    Ok(NetworkOperation {
        body_demand_pending: Default::default(),
        prefetched: Default::default(),
        request_id,
        events,
        demand: None,
    })
}

pub(crate) fn buffered_body_operation(
    request_id: u64,
    body: CollectedBody,
) -> Result<NetworkOperation, NetworkError> {
    let (sender, events) = tokio::sync::mpsc::channel(2);
    sender
        .try_send(NetworkEvent::Chunk {
            request_id,
            bytes: body.bytes.into(),
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    sender
        .try_send(NetworkEvent::Complete {
            encoded_body_size: body.encoded_body_size,
            request_id,
        })
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    Ok(NetworkOperation {
        body_demand_pending: Default::default(),
        prefetched: Default::default(),
        request_id,
        events,
        demand: None,
    })
}

pub fn collect_operation_bounded_with_cancel(
    broker: &(impl NetworkBroker + ?Sized),
    request: NetworkRequest,
    timeout: Duration,
    max_body_bytes: usize,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<NetworkResponse, NetworkError> {
    collect_operation_bounded_inner(broker, request, timeout, max_body_bytes, Some(cancelled))
}

fn collect_operation_bounded_inner(
    broker: &(impl NetworkBroker + ?Sized),
    request: NetworkRequest,
    timeout: Duration,
    max_body_bytes: usize,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<NetworkResponse, NetworkError> {
    let mut operation = broker.start_operation(request)?;
    let expected_request_id = operation.request_id;
    let deadline = std::time::Instant::now() + timeout;
    let mut response_policy = None;
    let mut transport_timing: Option<oryn_net::TransportTiming> = None;
    let mut timing_headers = None;
    let mut status = None;
    let mut headers = Vec::new();
    let mut body = Vec::new();
    loop {
        let event = loop {
            if cancelled
                .is_some_and(|cancelled| cancelled.load(std::sync::atomic::Ordering::Acquire))
            {
                let _ = broker.cancel_operation(expected_request_id);
                return Err(NetworkError::Cancelled);
            }
            match operation.try_event() {
                Ok(event) => break event,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    let _ = broker.cancel_operation(expected_request_id);
                    return Err(NetworkError::Transport(
                        "network operation closed without completion".into(),
                    ));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = broker.cancel_operation(expected_request_id);
                        return Err(NetworkError::TimedOut { timeout });
                    }
                    std::thread::park_timeout(Duration::from_millis(1));
                }
            }
        };
        let event_request_id = match &event {
            NetworkEvent::Headers { request_id, .. }
            | NetworkEvent::Chunk { request_id, .. }
            | NetworkEvent::Complete { request_id, .. }
            | NetworkEvent::Failed { request_id, .. } => *request_id,
        };
        if event_request_id != expected_request_id {
            let _ = broker.cancel_operation(expected_request_id);
            return Err(NetworkError::Transport(format!(
                "stale or unmatched network completion {event_request_id} for operation {expected_request_id}"
            )));
        }
        match event {
            NetworkEvent::Headers {
                response_policy: policy,
                timing,
                status: value,
                headers: values,
                ..
            } => {
                response_policy = policy;
                transport_timing = timing.map(|timing| *timing);
                timing_headers = Some(std::time::Instant::now());
                status = Some(value);
                headers = values;
                operation.request_body_chunk();
            }
            NetworkEvent::Chunk { bytes, .. } => {
                if body.len().saturating_add(bytes.len()) > max_body_bytes {
                    let _ = broker.cancel_operation(expected_request_id);
                    return Err(NetworkError::ResponseTooLarge {
                        limit: max_body_bytes,
                    });
                }
                body.extend_from_slice(&bytes);
                operation.request_body_chunk();
            }
            NetworkEvent::Complete {
                encoded_body_size,
                request_id,
            } => {
                return Ok(NetworkResponse {
                    response_policy,
                    timing: transport_timing.map(|mut timing| {
                        timing.encoded_body_size = encoded_body_size.or(timing.encoded_body_size);
                        timing.response_end = Some(timing.response_start.saturating_add(
                            timing_headers.map_or(0, |headers: std::time::Instant| {
                                headers.elapsed().as_micros().min(u64::MAX as u128) as u64
                            }),
                        ));
                        timing
                    }),
                    request_id,
                    status: status.ok_or_else(|| {
                        NetworkError::Transport("operation completed before headers".into())
                    })?,
                    headers,
                    body,
                });
            }
            NetworkEvent::Failed { error, .. } => {
                let _ = broker.cancel_operation(expected_request_id);
                return Err(NetworkError::from(error));
            }
        }
    }
}

pub(crate) fn operation_from_response(response: NetworkResponse) -> NetworkOperation {
    let request_id = response.request_id;
    let (sender, events) = tokio::sync::mpsc::channel(3);
    let (demand, mut requested) = tokio::sync::mpsc::channel(1);
    operation_runtime().spawn(async move {
        let length = response.body.len();
        let encoded_body_size = response
            .timing
            .as_ref()
            .and_then(|timing| timing.encoded_body_size);
        if sender
            .send(NetworkEvent::Headers {
                response_policy: response.response_policy,
                timing: response.timing.map(Box::new),
                request_id,
                status: response.status,
                headers: response.headers,
                content_length: Some(length),
                upload: None,
            })
            .await
            .is_err()
        {
            return;
        }
        if !response.body.is_empty()
            && requested.recv().await.is_some()
            && sender
                .send(NetworkEvent::Chunk {
                    request_id,
                    bytes: bytes::Bytes::from(response.body),
                })
                .await
                .is_err()
        {
            return;
        }
        let _ = sender
            .send(NetworkEvent::Complete {
                encoded_body_size,
                request_id,
            })
            .await;
    });
    NetworkOperation {
        body_demand_pending: Default::default(),
        prefetched: Default::default(),
        request_id,
        events,
        demand: Some(demand),
    }
}

pub trait NetworkBroker: Send + Sync {
    /// Token-specific cleanup does not cancel a later operation that reuses a
    /// numeric request ID. Only the owning native route may implement it.
    fn discard_response_policy_routed(
        &self,
        _route: Option<&WorkerRoute>,
        _token: &str,
    ) -> Result<(), NetworkError> {
        Err(NetworkError::Unavailable)
    }
    /// Read-only activity of this native owner, excluding sibling namespaces.
    /// SharedNetworkBroker's identity layer supplies transport-independent
    /// admission and acknowledged completion accounting.
    fn owned_operation_activity(&self) -> (usize, Option<std::time::Instant>) {
        (0, None)
    }
    /// Drain only this native owner's accepted operations; sibling namespaces
    /// sharing the same underlying transport must not enter this barrier.
    fn drain_owned_operations(&self, _timeout: Duration) -> Result<(), NetworkError> {
        Ok(())
    }
    fn start_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        if route.is_some() {
            return Err(NetworkError::Transport(
                "network transport does not accept route authority".into(),
            ));
        }
        self.start_operation(request)
    }
    fn cancel_routed(
        &self,
        route: Option<&WorkerRoute>,
        request_id: u64,
    ) -> Result<(), NetworkError> {
        if route.is_some() {
            return Err(NetworkError::Transport(
                "network transport does not accept route authority".into(),
            ));
        }
        self.cancel_operation(request_id)
    }
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError>;

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        let response = self.send(request)?;
        let content_length = Some(response.body.len());
        let encoded_bytes = response
            .timing
            .as_ref()
            .and_then(|timing| timing.encoded_body_size)
            .map(|bytes| Arc::new(std::sync::atomic::AtomicU64::new(bytes)));
        Ok(NetworkStreamResponse {
            response_policy: response.response_policy,
            encoded_bytes,
            timing: response.timing,
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            content_length,
            upload: None,
            body: Box::new(std::io::Cursor::new(response.body)),
        })
    }

    /// Start a bounded, event-driven request.  The default is deliberately a
    /// compatibility adapter for fixtures and embedders which still implement
    /// the historical blocking broker. Production transports override it.
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let request_id = request.request_id;
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let (demand, mut requested) = tokio::sync::mpsc::channel(1);
        let mut response = self.start_stream(request)?;
        let background_owner = network_identity::capture_background_owner();
        std::thread::spawn(move || {
            network_identity::with_background_owner(background_owner, || {
                let _ = sender.blocking_send(NetworkEvent::Headers {
                    response_policy: response.response_policy,
                    timing: response.timing.map(Box::new),
                    request_id,
                    status: response.status,
                    headers: response.headers,
                    content_length: response.content_length,
                    upload: response.upload,
                });
                let mut buffer = vec![0_u8; 64 * 1024];
                loop {
                    if requested.blocking_recv().is_none() {
                        break;
                    }
                    match response.body.read(&mut buffer) {
                        Ok(0) => {
                            let _ = sender.blocking_send(NetworkEvent::Complete {
                                encoded_body_size: response
                                    .encoded_bytes
                                    .as_ref()
                                    .map(|count| count.load(Ordering::Relaxed)),
                                request_id,
                            });
                            break;
                        }
                        Ok(read) => {
                            if sender
                                .blocking_send(NetworkEvent::Chunk {
                                    request_id,
                                    bytes: bytes::Bytes::copy_from_slice(&buffer[..read]),
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.blocking_send(NetworkEvent::Failed {
                                request_id,
                                error: error.to_string().into(),
                            });
                            break;
                        }
                    }
                }
            })
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand: Some(demand),
        })
    }

    fn cancel_operation(&self, _request_id: u64) -> Result<(), NetworkError> {
        Ok(())
    }
}

impl NetworkBroker for Box<dyn NetworkBroker> {
    fn discard_response_policy_routed(
        &self,
        route: Option<&WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        self.as_ref().discard_response_policy_routed(route, token)
    }
    fn owned_operation_activity(&self) -> (usize, Option<std::time::Instant>) {
        (**self).owned_operation_activity()
    }
    fn drain_owned_operations(&self, timeout: Duration) -> Result<(), NetworkError> {
        (**self).drain_owned_operations(timeout)
    }
    fn start_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        self.as_ref().start_routed(route, request)
    }
    fn cancel_routed(
        &self,
        route: Option<&WorkerRoute>,
        request_id: u64,
    ) -> Result<(), NetworkError> {
        self.as_ref().cancel_routed(route, request_id)
    }
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        self.as_ref().send(request)
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        self.as_ref().start_stream(request)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        self.as_ref().start_operation(request)
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.as_ref().cancel_operation(request_id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error(transparent)]
    Policy(#[from] PolicyDenied),
    #[error("no host network transport is configured")]
    Unavailable,
    #[error("host resolution failed for {host}: {detail}")]
    Resolution { host: String, detail: String },
    #[error("server certificate rejected: {0}")]
    CertificateRejected(String),
    #[error("network transport failed: {0}")]
    Transport(String),
    #[error("network operation exceeded its {timeout:?} deadline")]
    TimedOut { timeout: Duration },
    #[error("network operation was cancelled")]
    Cancelled,
    #[error("response exceeded the configured {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("unsupported content encoding {0:?}")]
    UnsupportedContentEncoding(String),
    #[error("deterministic network replay failed: {0}")]
    Replay(String),
}

pub struct PolicyBroker<B> {
    policy: NetworkPolicy,
    inner: B,
    endpoint_routes: Option<EndpointRoutes>,
}

/// Browser cookie semantics around any transport/cache stack. This adapter is
/// deliberately outside reqwest so every cache hit and redirect is evaluated
/// with the current site-for-cookies context.
pub struct CookieBroker<B> {
    inner: B,
    store: Arc<ContextCookieStore>,
}

impl<B> CookieBroker<B> {
    pub fn new(inner: B, store: Arc<ContextCookieStore>) -> Self {
        Self { inner, store }
    }

    fn prepare(&self, mut request: NetworkRequest) -> Result<(NetworkRequest, Url), NetworkError> {
        crate::fetch_metadata::apply(&mut request)?;
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        let mut context = request.security.clone();
        if !request.credentials {
            context.credentials = crate::cookies::CookieCredentials::Omit;
        }
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("cookie"));
        if let Some(header) = self
            .store
            .header_for_method(&url, &request.method, &context)
            .0
        {
            request.headers.push(("cookie".into(), header));
        }
        request.security = context;
        Ok((request, url))
    }

    fn store_response(
        &self,
        request: &NetworkRequest,
        url: &Url,
        headers: &[(String, String)],
    ) -> Result<(), NetworkError> {
        self.store
            .store_response(
                url,
                &request.security,
                headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
                    .map(|(_, value)| value.clone()),
            )
            .map_err(|error| NetworkError::Transport(error.to_string()))
    }
}

impl<B: NetworkBroker> NetworkBroker for CookieBroker<B> {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let (request, url) = self.prepare(request)?;
        let response = self.inner.send(request.clone())?;
        self.store_response(&request, &url, &response.headers)?;
        Ok(response)
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        let (request, url) = self.prepare(request)?;
        let response = self.inner.start_stream(request.clone())?;
        self.store_response(&request, &url, &response.headers)?;
        Ok(response)
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let (request, url) = self.prepare(request)?;
        let request_id = request.request_id;
        let mut operation = self.inner.start_operation(request.clone())?;
        let demand = operation.demand.take();
        let store = self.store.clone();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        operation_runtime().spawn(async move {
            while let Some(event) = operation.next_event().await {
                if let NetworkEvent::Headers { headers, .. } = &event {
                    let _ = store.store_response(
                        &url,
                        &request.security,
                        headers
                            .iter()
                            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
                            .map(|(_, value)| value.clone()),
                    );
                }
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner.cancel_operation(request_id)
    }
}

#[path = "network_identity.rs"]
mod network_identity;
pub struct SharedNetworkBroker {
    inner: Option<Arc<Box<dyn NetworkBroker>>>,
    audit: crate::resource_audit::ResourceAudit,
    audit_page_id: Option<u64>,
    audit_worker_pid: Option<u32>,
}

/// Context-owned browser memory cache. Policy remains outside this adapter so
/// every hit and miss is still authorized against the current request URL.
/// Applies Fetch cache semantics when storage is disabled, without buffering bodies.
pub(crate) struct UncachedBroker<B>(pub B);
impl<B: NetworkBroker> NetworkBroker for UncachedBroker<B> {
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        cache_miss_requires_error(&request)?;
        prepare_cache_network(&mut request, None);
        self.0.send(request)
    }
    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        cache_miss_requires_error(&request)?;
        prepare_cache_network(&mut request, None);
        self.0.start_stream(request)
    }
    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        cache_miss_requires_error(&request)?;
        prepare_cache_network(&mut request, None);
        self.0.start_operation(request)
    }
    fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
        self.0.cancel_operation(id)
    }
    fn owned_operation_activity(&self) -> (usize, Option<std::time::Instant>) {
        self.0.owned_operation_activity()
    }
}

pub struct MemoryCachingBroker<B> {
    revalidating: Arc<Mutex<std::collections::BTreeSet<CacheKey>>>,
    inner: B,
    partition: String,
    cache: Arc<Mutex<MemoryCache>>,
    max_entry_bytes: usize,
}

impl<B> MemoryCachingBroker<B> {
    pub fn new(
        inner: B,
        partition: impl Into<String>,
        max_entries: usize,
        max_bytes: usize,
        max_entry_bytes: usize,
    ) -> Self {
        Self {
            inner,
            partition: partition.into(),
            revalidating: Default::default(),
            cache: Arc::new(Mutex::new(MemoryCache::new(
                max_entries,
                max_bytes,
                max_entry_bytes,
            ))),
            max_entry_bytes,
        }
    }

    fn key(&self, request: &NetworkRequest) -> CacheKey {
        CacheKey {
            partition: self.partition.clone(),
            method: request.method.to_ascii_uppercase(),
            url: request.url.clone(),
            credential_partition: if request.credentials {
                "include".into()
            } else {
                "omit".into()
            },
        }
    }

    fn cached(&self, request: &NetworkRequest) -> Option<NetworkResponse> {
        if !cache_lookup_allowed(request) {
            return None;
        }
        let key = self.key(request);
        let entry = self
            .cache
            .lock()
            .ok()?
            .get_matching(&key, &request.headers, None)
            .filter(|entry| entry.status != 206)
            .cloned()?;
        if !entry.is_fresh(unix_time_ms())
            && !cache_accepts_stale(request)
            && !(cache_swr_allowed(request, &entry)
                && schedule_cache_revalidation(
                    request,
                    key,
                    &self.revalidating,
                    self.max_entry_bytes,
                ))
        {
            return None;
        }
        Some(NetworkResponse {
            response_policy: None,
            timing: Some(oryn_net::TransportTiming {
                encoded_body_size: entry.encoded_body_size,
                delivery: oryn_common::transport_timing::ResponseDelivery::LocalCache,
                ..Default::default()
            }),
            request_id: request.request_id,
            status: entry.status,
            headers: cache_representation_headers(&entry.headers),
            body: entry.body,
        })
    }

    fn prepare(&self, request: &mut NetworkRequest) -> Option<CacheEntry> {
        let entry = self.cache.lock().ok().and_then(|mut cache| {
            cache
                .get_matching(&self.key(request), &request.headers, None)
                .filter(|entry| entry.status != 206)
                .cloned()
        });
        prepare_cache_network(request, entry)
    }

    fn entry(
        &self,
        request: &NetworkRequest,
        status: u16,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Option<(CacheKey, CacheEntry)> {
        if status == 206 || body.len() > self.max_entry_bytes || !cache_authorized(request, headers)
        {
            return None;
        }
        let freshness_lifetime_ms = cache_freshness_ms(headers)?;
        let vary = cache_vary(headers, &request.headers)?;
        Some((
            self.key(request),
            CacheEntry {
                encoded_body_size: None,
                status,
                headers: cache_representation_headers(headers),
                body,
                stored_at_ms: unix_time_ms(),
                freshness_lifetime_ms,
                vary,
            },
        ))
    }

    fn store(&self, request: &NetworkRequest, response: &NetworkResponse) {
        if let Some((key, mut entry)) = self.entry(
            request,
            response.status,
            &response.headers,
            response.body.clone(),
        ) && let Ok(mut cache) = self.cache.lock()
        {
            entry.encoded_body_size = response
                .timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size);
            cache.insert(key, entry);
        }
    }

    fn invalidate_after_mutation(&self, request: &NetworkRequest) {
        if !matches!(request.method.as_str(), "GET" | "HEAD")
            && let Ok(mut cache) = self.cache.lock()
        {
            cache.invalidate_url(&self.partition, &request.url);
        }
    }
}

struct CachingReader {
    encoded_bytes: Option<Arc<std::sync::atomic::AtomicU64>>,
    inner: Box<dyn Read + Send>,
    cache: Arc<Mutex<MemoryCache>>,
    key: Option<CacheKey>,
    entry: Option<CacheEntry>,
    max_entry_bytes: usize,
    expected_length: Option<usize>,
    complete: bool,
}

impl Read for CachingReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let read = match self.inner.read(output) {
            Ok(read) => read,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::Interrupted {
                    self.entry = None;
                    self.key = None;
                    self.complete = true;
                }
                return Err(error);
            }
        };
        if read == 0 {
            if self.expected_length.is_some_and(|length| {
                self.entry
                    .as_ref()
                    .is_some_and(|entry| entry.body.len() != length)
            }) {
                self.entry = None;
                self.key = None;
                return Err(std::io::Error::other(
                    "cached response length differs at EOF",
                ));
            }
            if !self.complete {
                self.complete = true;
                if let (Some(key), Some(mut entry)) = (self.key.take(), self.entry.take())
                    && let Ok(mut cache) = self.cache.lock()
                {
                    entry.encoded_body_size = self
                        .encoded_bytes
                        .as_ref()
                        .map(|count| count.load(Ordering::Relaxed));
                    cache.insert(key, entry);
                }
            }
            return Ok(0);
        }
        if let Some(entry) = &mut self.entry {
            if entry.body.len().saturating_add(read) <= self.max_entry_bytes {
                entry.body.extend_from_slice(&output[..read]);
            } else {
                self.key = None;
                self.entry = None;
            }
        }
        Ok(read)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentCacheRecord {
    key: CacheKey,
    entry: CacheEntry,
}

struct PersistentCacheStore {
    database: Database,
    max_entries: usize,
    max_bytes: usize,
    max_entry_bytes: usize,
}

impl PersistentCacheStore {
    fn open(
        profile_path: &Path,
        max_entries: usize,
        max_bytes: usize,
        max_entry_bytes: usize,
    ) -> Result<Self, NetworkError> {
        let database = Database::create(profile_path.join("oryn-http-cache.redb"))
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let write = database
            .begin_write()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        write
            .open_table(PERSISTENT_HTTP_CACHE)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        write
            .commit()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        Ok(Self {
            database,
            max_entries,
            max_bytes,
            max_entry_bytes,
        })
    }

    fn encoded_key(key: &CacheKey) -> Result<String, NetworkError> {
        serde_json::to_string(key).map_err(|error| NetworkError::Transport(error.to_string()))
    }

    fn get(
        &self,
        key: &CacheKey,
        request_headers: &[(String, String)],
        fresh_at: Option<u64>,
    ) -> Result<Option<CacheEntry>, NetworkError> {
        let encoded_key = Self::encoded_key(key)?;
        let read = self
            .database
            .begin_read()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let table = read
            .open_table(PERSISTENT_HTTP_CACHE)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let record = table
            .get(encoded_key.as_str())
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .map(|value| serde_json::from_slice::<PersistentCacheRecord>(value.value()))
            .transpose()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        Ok(record.and_then(|record| {
            (fresh_at.is_none_or(|now| record.entry.is_fresh(now))
                && record.entry.matches_vary(request_headers))
            .then_some(record.entry)
        }))
    }

    fn insert(&self, key: CacheKey, entry: CacheEntry) -> Result<(), NetworkError> {
        if entry.body.len() > self.max_entry_bytes || self.max_entries == 0 || self.max_bytes == 0 {
            return Ok(());
        }
        let encoded_key = Self::encoded_key(&key)?;
        let mut records = self.records()?;
        records.retain(|(candidate, _, _)| candidate != &encoded_key);
        let mut bytes = records.iter().map(|(_, size, _)| *size).sum::<usize>();
        records.sort_by_key(|(_, _, stored_at)| *stored_at);
        while records.len() >= self.max_entries
            || bytes.saturating_add(entry.body.len()) > self.max_bytes
        {
            let Some((oldest, size, _)) = records.first().cloned() else {
                return Ok(());
            };
            self.remove_encoded(&oldest)?;
            records.remove(0);
            bytes = bytes.saturating_sub(size);
        }
        let encoded = serde_json::to_vec(&PersistentCacheRecord { key, entry })
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let write = self
            .database
            .begin_write()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        {
            let mut table = write
                .open_table(PERSISTENT_HTTP_CACHE)
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
            table
                .insert(encoded_key.as_str(), encoded.as_slice())
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
        }
        write
            .commit()
            .map_err(|error| NetworkError::Transport(error.to_string()))
    }

    fn records(&self) -> Result<Vec<(String, usize, u64)>, NetworkError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let table = read
            .open_table(PERSISTENT_HTTP_CACHE)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        table
            .iter()
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .map(|item| {
                let (key, value) =
                    item.map_err(|error| NetworkError::Transport(error.to_string()))?;
                let record: PersistentCacheRecord = serde_json::from_slice(value.value())
                    .map_err(|error| NetworkError::Transport(error.to_string()))?;
                Ok((
                    key.value().to_string(),
                    record.entry.body.len(),
                    record.entry.stored_at_ms,
                ))
            })
            .collect()
    }

    fn remove_encoded(&self, encoded_key: &str) -> Result<(), NetworkError> {
        let write = self
            .database
            .begin_write()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        {
            let mut table = write
                .open_table(PERSISTENT_HTTP_CACHE)
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
            table
                .remove(encoded_key)
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
        }
        write
            .commit()
            .map_err(|error| NetworkError::Transport(error.to_string()))
    }

    fn invalidate_url(&self, partition: &str, url: &str) -> Result<(), NetworkError> {
        let keys = self
            .records_with_keys()?
            .into_iter()
            .filter(|(_, record)| record.key.partition == partition && record.key.url == url)
            .map(|(encoded, _)| encoded)
            .collect::<Vec<_>>();
        for key in keys {
            self.remove_encoded(&key)?;
        }
        Ok(())
    }

    fn records_with_keys(&self) -> Result<Vec<(String, PersistentCacheRecord)>, NetworkError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let table = read
            .open_table(PERSISTENT_HTTP_CACHE)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        table
            .iter()
            .map_err(|error| NetworkError::Transport(error.to_string()))?
            .map(|item| {
                let (key, value) =
                    item.map_err(|error| NetworkError::Transport(error.to_string()))?;
                let record = serde_json::from_slice(value.value())
                    .map_err(|error| NetworkError::Transport(error.to_string()))?;
                Ok((key.value().to_string(), record))
            })
            .collect()
    }
}

pub struct PersistentCachingBroker<B> {
    revalidating: Arc<Mutex<std::collections::BTreeSet<CacheKey>>>,
    inner: B,
    partition: String,
    store: Arc<PersistentCacheStore>,
}

impl<B> PersistentCachingBroker<B> {
    pub fn open(
        inner: B,
        profile_path: &Path,
        partition: impl Into<String>,
        max_entries: usize,
        max_bytes: usize,
        max_entry_bytes: usize,
    ) -> Result<Self, NetworkError> {
        Ok(Self {
            revalidating: Default::default(),
            inner,
            partition: partition.into(),
            store: Arc::new(PersistentCacheStore::open(
                profile_path,
                max_entries,
                max_bytes,
                max_entry_bytes,
            )?),
        })
    }

    fn key(&self, request: &NetworkRequest) -> CacheKey {
        CacheKey {
            partition: self.partition.clone(),
            method: request.method.to_ascii_uppercase(),
            url: request.url.clone(),
            credential_partition: if request.credentials {
                "include".into()
            } else {
                "omit".into()
            },
        }
    }

    fn cached(&self, request: &NetworkRequest) -> Result<Option<NetworkResponse>, NetworkError> {
        if !cache_lookup_allowed(request) {
            return Ok(None);
        }
        let key = self.key(request);
        let Some(entry) = self
            .store
            .get(&key, &request.headers, None)?
            .filter(|entry| entry.status != 206)
        else {
            return Ok(None);
        };
        if !entry.is_fresh(unix_time_ms())
            && !cache_accepts_stale(request)
            && !(cache_swr_allowed(request, &entry)
                && schedule_cache_revalidation(
                    request,
                    key,
                    &self.revalidating,
                    self.store.max_entry_bytes,
                ))
        {
            return Ok(None);
        }
        Ok(Some(NetworkResponse {
            response_policy: None,
            timing: Some(oryn_net::TransportTiming {
                encoded_body_size: entry.encoded_body_size,
                delivery: oryn_common::transport_timing::ResponseDelivery::LocalCache,
                ..Default::default()
            }),
            request_id: request.request_id,
            status: entry.status,
            headers: cache_representation_headers(&entry.headers),
            body: entry.body,
        }))
    }

    fn prepare(&self, request: &mut NetworkRequest) -> Result<Option<CacheEntry>, NetworkError> {
        let entry = self
            .store
            .get(&self.key(request), &request.headers, None)?
            .filter(|entry| entry.status != 206);
        Ok(prepare_cache_network(request, entry))
    }

    fn entry(
        &self,
        request: &NetworkRequest,
        status: u16,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Option<(CacheKey, CacheEntry)> {
        if status == 206
            || body.len() > self.store.max_entry_bytes
            || !cache_authorized(request, headers)
        {
            return None;
        }
        Some((
            self.key(request),
            CacheEntry {
                encoded_body_size: None,
                status,
                headers: cache_representation_headers(headers),
                body,
                stored_at_ms: unix_time_ms(),
                freshness_lifetime_ms: cache_freshness_ms(headers)?,
                vary: cache_vary(headers, &request.headers)?,
            },
        ))
    }
}

struct PersistentCachingReader {
    encoded_bytes: Option<Arc<std::sync::atomic::AtomicU64>>,
    inner: Box<dyn Read + Send>,
    store: Arc<PersistentCacheStore>,
    key: Option<CacheKey>,
    entry: Option<CacheEntry>,
    expected_length: Option<usize>,
    complete: bool,
}

impl Read for PersistentCachingReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let read = match self.inner.read(output) {
            Ok(read) => read,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::Interrupted {
                    self.entry = None;
                    self.key = None;
                    self.complete = true;
                }
                return Err(error);
            }
        };
        if read == 0 {
            if self.expected_length.is_some_and(|length| {
                self.entry
                    .as_ref()
                    .is_some_and(|entry| entry.body.len() != length)
            }) {
                self.entry = None;
                self.key = None;
                return Err(std::io::Error::other(
                    "cached response length differs at EOF",
                ));
            }
            if !self.complete {
                self.complete = true;
                if let (Some(key), Some(mut entry)) = (self.key.take(), self.entry.take()) {
                    entry.encoded_body_size = self
                        .encoded_bytes
                        .as_ref()
                        .map(|count| count.load(Ordering::Relaxed));
                    self.store
                        .insert(key, entry)
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                }
            }
            return Ok(0);
        }
        if let Some(entry) = &mut self.entry {
            if entry.body.len().saturating_add(read) <= self.store.max_entry_bytes {
                entry.body.extend_from_slice(&output[..read]);
            } else {
                self.key = None;
                self.entry = None;
            }
        }
        Ok(read)
    }
}

impl SharedNetworkBroker {
    pub(crate) fn for_route(self, route: WorkerRoute) -> Self {
        Self::new(RoutedNetworkBroker { inner: self, route })
    }
    pub fn new(broker: impl NetworkBroker + 'static) -> Self {
        Self {
            inner: Some(Arc::new(Box::new(network_identity::IdentityBroker::new(
                Arc::new(Box::new(broker)),
            )))),
            audit: Default::default(),
            audit_page_id: None,
            audit_worker_pid: None,
        }
    }
    pub fn resource_audit(&self) -> crate::resource_audit::ResourceAudit {
        self.audit.clone()
    }
    /// New native owner, preserving transport and audit attribution while
    /// separating caller IDs from all previous owners of that transport.
    pub(crate) fn new_request_scope(mut self) -> Self {
        let inner = self.inner.take().expect("live network scope");
        self.inner = Some(Arc::new(Box::new(network_identity::IdentityBroker::new(
            inner,
        ))));
        self
    }
    pub fn for_page(mut self, page_id: u64) -> Self {
        self.audit_page_id = Some(page_id);
        self
    }
    pub fn for_worker(mut self, pid: u32) -> Self {
        self.audit_worker_pid = Some(pid);
        self
    }
    fn observe(&self, request: &NetworkRequest) -> Option<crate::resource_audit::Observation> {
        let mut observation = self.audit.begin(request, self.audit_page_id)?;
        observation.bind_worker(self.audit_worker_pid);
        Some(observation)
    }
}

struct RoutedNetworkBroker {
    inner: SharedNetworkBroker,
    route: WorkerRoute,
}

impl NetworkBroker for RoutedNetworkBroker {
    fn discard_response_policy_routed(
        &self,
        route: Option<&WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        if route.is_some_and(|route| route != &self.route) {
            return Err(NetworkError::Transport(
                "foreign response policy route".into(),
            ));
        }
        self.inner
            .discard_response_policy_routed(Some(&self.route), token)
    }
    fn start_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        self.inner
            .inner
            .as_ref()
            .expect("live route transport")
            .start_routed(route.or(Some(&self.route)), request)
    }
    fn cancel_routed(
        &self,
        route: Option<&WorkerRoute>,
        request_id: u64,
    ) -> Result<(), NetworkError> {
        self.inner
            .inner
            .as_ref()
            .expect("live route transport")
            .cancel_routed(route.or(Some(&self.route)), request_id)
    }

    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        collect_operation_bounded(
            self,
            request,
            Duration::from_secs(30),
            MAX_DECODED_RESPONSE_BYTES,
        )
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        self.inner
            .inner
            .as_ref()
            .expect("live route transport")
            .start_routed(Some(&self.route), request)
    }
    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner
            .inner
            .as_ref()
            .expect("live route transport")
            .cancel_routed(Some(&self.route), request_id)
    }
}

impl Clone for SharedNetworkBroker {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            audit: self.audit.clone(),
            audit_page_id: self.audit_page_id,
            audit_worker_pid: self.audit_worker_pid,
        }
    }
}

impl Drop for SharedNetworkBroker {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            // reqwest's blocking client owns a Tokio runtime and must not be
            // destroyed from within an async executor thread.
            std::thread::spawn(move || drop(inner));
        }
    }
}

impl NetworkBroker for SharedNetworkBroker {
    fn discard_response_policy_routed(
        &self,
        route: Option<&WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        self.inner
            .as_ref()
            .expect("broker handle is live")
            .discard_response_policy_routed(route, token)
    }
    fn owned_operation_activity(&self) -> (usize, Option<std::time::Instant>) {
        self.inner
            .as_ref()
            .expect("live network scope")
            .owned_operation_activity()
    }
    fn drain_owned_operations(&self, timeout: Duration) -> Result<(), NetworkError> {
        self.inner
            .as_ref()
            .expect("live network scope")
            .drain_owned_operations(timeout)
    }
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        crate::fetch_metadata::apply(&mut request)?;
        let mut observation = self.observe(&request);
        let result = self
            .inner
            .as_ref()
            .expect("broker handle is live")
            .send(request);
        if let Some(observation) = &mut observation {
            match &result {
                Ok(response) => {
                    observation.headers(response.status, &response.headers);
                    observation.chunk(&response.body);
                    observation.finish("complete");
                }
                Err(_) => observation.finish("failed"),
            }
        }
        result
    }

    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        crate::fetch_metadata::apply(&mut request)?;
        let observation = self.observe(&request);
        let result = self
            .inner
            .as_ref()
            .expect("broker handle is live")
            .start_stream(request);
        match (result, observation) {
            (Ok(mut response), Some(mut observation)) => {
                observation.headers(response.status, &response.headers);
                response.body = Box::new(crate::resource_audit::AuditedRead {
                    inner: response.body,
                    observation,
                });
                Ok(response)
            }
            (Err(error), Some(mut observation)) => {
                observation.finish("failed");
                Err(error)
            }
            (result, None) => result,
        }
    }

    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        crate::fetch_metadata::apply(&mut request)?;
        let observation = self.observe(&request);
        let result = self
            .inner
            .as_ref()
            .expect("broker handle is live")
            .start_operation(request);
        let Some(mut observation) = observation else {
            return result;
        };
        let mut operation = match result {
            Ok(operation) => operation,
            Err(error) => {
                observation.finish("failed");
                return Err(error);
            }
        };
        let request_id = operation.request_id;
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        operation_runtime().spawn(async move {
            loop {
                let event = tokio::select! {
                    event=operation.next_event()=>event,
                    _=sender.closed()=>break,
                };
                let Some(event) = event else {
                    break;
                };
                match &event {
                    NetworkEvent::Headers {
                        status, headers, ..
                    } => observation.headers(*status, headers),
                    NetworkEvent::Chunk { bytes, .. } => observation.chunk(bytes),
                    NetworkEvent::Complete { .. } => observation.finish("complete"),
                    NetworkEvent::Failed { .. } => observation.finish("failed"),
                }
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.audit.cancel(request_id, self.audit_page_id);
        self.inner
            .as_ref()
            .expect("broker handle is live")
            .cancel_operation(request_id)
    }
}

impl<B: NetworkBroker> NetworkBroker for MemoryCachingBroker<B> {
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        if let Some(response) = self.cached(&request) {
            return Ok(response);
        }
        cache_miss_requires_error(&request)?;
        self.invalidate_after_mutation(&request);
        let retained = self.prepare(&mut request);
        let mut response = self.inner.send(request.clone())?;
        if response.status == 304
            && let Some(entry) = retained
        {
            let timing = response.timing.take();
            response = revalidated_response(request.request_id, entry, &response.headers);
            response.timing = validated_cache_timing_with_size(
                timing,
                response
                    .timing
                    .as_ref()
                    .and_then(|timing| timing.encoded_body_size),
            );
        }
        self.store(&request, &response);
        Ok(response)
    }

    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        if let Some(response) = self.cached(&request) {
            let content_length = Some(response.body.len());
            let encoded_bytes = response
                .timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size)
                .map(|bytes| Arc::new(std::sync::atomic::AtomicU64::new(bytes)));
            return Ok(NetworkStreamResponse {
                response_policy: response.response_policy,
                encoded_bytes,
                timing: response.timing,
                request_id: response.request_id,
                status: response.status,
                headers: response.headers,
                content_length,
                upload: None,
                body: Box::new(std::io::Cursor::new(response.body)),
            });
        }
        cache_miss_requires_error(&request)?;
        self.invalidate_after_mutation(&request);
        let retained = self.prepare(&mut request);
        let mut response = self.inner.start_stream(request.clone())?;
        if response.status == 304
            && let Some(entry) = retained
        {
            finish_empty_revalidation_body(&mut response.body)?;
            let merged = revalidated_response(request.request_id, entry, &response.headers);
            response = NetworkStreamResponse {
                response_policy: None,
                encoded_bytes: merged
                    .timing
                    .as_ref()
                    .and_then(|timing| timing.encoded_body_size)
                    .map(|bytes| Arc::new(std::sync::atomic::AtomicU64::new(bytes))),
                timing: validated_cache_timing_with_size(
                    response.timing.take(),
                    merged
                        .timing
                        .as_ref()
                        .and_then(|timing| timing.encoded_body_size),
                ),
                request_id: merged.request_id,
                status: merged.status,
                headers: merged.headers,
                content_length: Some(merged.body.len()),
                upload: None,
                body: Box::new(std::io::Cursor::new(merged.body)),
            };
        }
        let candidate = self
            .entry(&request, response.status, &response.headers, Vec::new())
            .filter(|_| {
                response
                    .content_length
                    .is_none_or(|length| length <= self.max_entry_bytes)
            });
        let (key, entry) = candidate.unzip();
        Ok(NetworkStreamResponse {
            response_policy: response.response_policy,
            encoded_bytes: response.encoded_bytes.clone(),
            timing: response.timing,
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            content_length: response.content_length,
            upload: response.upload,
            body: Box::new(CachingReader {
                encoded_bytes: response.encoded_bytes.clone(),
                inner: response.body,
                cache: self.cache.clone(),
                key,
                entry,
                max_entry_bytes: self.max_entry_bytes,
                expected_length: response.content_length,
                complete: false,
            }),
        })
    }

    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        if let Some(response) = self.cached(&request) {
            return Ok(operation_from_response(response));
        }
        cache_miss_requires_error(&request)?;
        self.invalidate_after_mutation(&request);
        let retained = self.prepare(&mut request);
        let request_id = request.request_id;
        let key = self.key(&request);
        let request_headers = request.headers.clone();
        let cacheable_request = request.clone();
        let eligible = matches!(request.method.as_str(), "GET" | "HEAD");
        let cache = self.cache.clone();
        let max_entry_bytes = self.max_entry_bytes;
        let mut operation = revalidation_operation(self.inner.start_operation(request)?, retained);
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        operation_runtime().spawn(async move {
            let mut candidate: Option<CacheEntry> = None;
            while let Some(event) = operation.next_event().await {
                match &event {
                    NetworkEvent::Headers {
                        status,
                        headers,
                        content_length,
                        ..
                    } => {
                        if eligible
                            && content_length.is_none_or(|length| length <= max_entry_bytes)
                            && *status != 206
                            && cache_authorized(&cacheable_request, headers)
                        {
                            candidate =
                                cache_freshness_ms(headers).and_then(|freshness_lifetime_ms| {
                                    cache_vary(headers, &request_headers).map(|vary| CacheEntry {
                                        encoded_body_size: None,
                                        status: *status,
                                        headers: cache_representation_headers(headers),
                                        body: Vec::new(),
                                        stored_at_ms: unix_time_ms(),
                                        freshness_lifetime_ms,
                                        vary,
                                    })
                                });
                        }
                    }
                    NetworkEvent::Chunk { bytes, .. } => {
                        if let Some(entry) = &mut candidate {
                            if entry.body.len().saturating_add(bytes.len()) <= max_entry_bytes {
                                entry.body.extend_from_slice(bytes);
                            } else {
                                candidate = None;
                            }
                        }
                    }
                    NetworkEvent::Complete {
                        encoded_body_size, ..
                    } => {
                        if let Some(mut entry) = candidate.take()
                            && let Ok(mut cache) = cache.lock()
                        {
                            entry.encoded_body_size = *encoded_body_size;
                            cache.insert(key.clone(), entry);
                        }
                    }
                    NetworkEvent::Failed { .. } => candidate = None,
                }
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner.cancel_operation(request_id)
    }
}

impl<B: NetworkBroker> NetworkBroker for PersistentCachingBroker<B> {
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        if let Some(response) = self.cached(&request)? {
            return Ok(response);
        }
        cache_miss_requires_error(&request)?;
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            self.store.invalidate_url(&self.partition, &request.url)?;
        }
        let retained = self.prepare(&mut request)?;
        let mut response = self.inner.send(request.clone())?;
        if response.status == 304
            && let Some(entry) = retained
        {
            let timing = response.timing.take();
            response = revalidated_response(request.request_id, entry, &response.headers);
            response.timing = validated_cache_timing_with_size(
                timing,
                response
                    .timing
                    .as_ref()
                    .and_then(|timing| timing.encoded_body_size),
            );
        }
        if let Some((key, mut entry)) = self.entry(
            &request,
            response.status,
            &response.headers,
            response.body.clone(),
        ) {
            entry.encoded_body_size = response
                .timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size);
            self.store.insert(key, entry)?;
        }
        Ok(response)
    }

    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        if let Some(response) = self.cached(&request)? {
            let content_length = Some(response.body.len());
            let encoded_bytes = response
                .timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size)
                .map(|bytes| Arc::new(std::sync::atomic::AtomicU64::new(bytes)));
            return Ok(NetworkStreamResponse {
                response_policy: response.response_policy,
                encoded_bytes,
                timing: response.timing,
                request_id: response.request_id,
                status: response.status,
                headers: response.headers,
                content_length,
                upload: None,
                body: Box::new(std::io::Cursor::new(response.body)),
            });
        }
        cache_miss_requires_error(&request)?;
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            self.store.invalidate_url(&self.partition, &request.url)?;
        }
        let retained = self.prepare(&mut request)?;
        let mut response = self.inner.start_stream(request.clone())?;
        if response.status == 304
            && let Some(entry) = retained
        {
            finish_empty_revalidation_body(&mut response.body)?;
            let merged = revalidated_response(request.request_id, entry, &response.headers);
            response = NetworkStreamResponse {
                response_policy: None,
                encoded_bytes: merged
                    .timing
                    .as_ref()
                    .and_then(|timing| timing.encoded_body_size)
                    .map(|bytes| Arc::new(std::sync::atomic::AtomicU64::new(bytes))),
                timing: validated_cache_timing_with_size(
                    response.timing.take(),
                    merged
                        .timing
                        .as_ref()
                        .and_then(|timing| timing.encoded_body_size),
                ),
                request_id: merged.request_id,
                status: merged.status,
                headers: merged.headers,
                content_length: Some(merged.body.len()),
                upload: None,
                body: Box::new(std::io::Cursor::new(merged.body)),
            };
        }
        let candidate = self
            .entry(&request, response.status, &response.headers, Vec::new())
            .filter(|_| {
                response
                    .content_length
                    .is_none_or(|length| length <= self.store.max_entry_bytes)
            });
        let (key, entry) = candidate.unzip();
        Ok(NetworkStreamResponse {
            response_policy: response.response_policy,
            encoded_bytes: response.encoded_bytes.clone(),
            timing: response.timing,
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            content_length: response.content_length,
            upload: response.upload,
            body: Box::new(PersistentCachingReader {
                encoded_bytes: response.encoded_bytes.clone(),
                inner: response.body,
                store: self.store.clone(),
                key,
                entry,
                expected_length: response.content_length,
                complete: false,
            }),
        })
    }

    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        if let Some(response) = self.cached(&request)? {
            return Ok(operation_from_response(response));
        }
        cache_miss_requires_error(&request)?;
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            self.store.invalidate_url(&self.partition, &request.url)?;
        }
        let retained = self.prepare(&mut request)?;
        let request_id = request.request_id;
        let key = self.key(&request);
        let request_headers = request.headers.clone();
        let cacheable_request = request.clone();
        let max_entry_bytes = self.store.max_entry_bytes;
        let store = self.store.clone();
        let mut operation = revalidation_operation(self.inner.start_operation(request)?, retained);
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        operation_runtime().spawn(async move {
            let mut candidate: Option<CacheEntry> = None;
            while let Some(event) = operation.next_event().await {
                match &event {
                    NetworkEvent::Headers {
                        status,
                        headers,
                        content_length,
                        ..
                    } => {
                        if content_length.is_none_or(|length| length <= max_entry_bytes)
                            && *status != 206
                            && cache_authorized(&cacheable_request, headers)
                        {
                            candidate =
                                cache_freshness_ms(headers).and_then(|freshness_lifetime_ms| {
                                    cache_vary(headers, &request_headers).map(|vary| CacheEntry {
                                        encoded_body_size: None,
                                        status: *status,
                                        headers: cache_representation_headers(headers),
                                        body: Vec::new(),
                                        stored_at_ms: unix_time_ms(),
                                        freshness_lifetime_ms,
                                        vary,
                                    })
                                });
                        }
                    }
                    NetworkEvent::Chunk { bytes, .. } => {
                        if let Some(entry) = &mut candidate {
                            if entry.body.len().saturating_add(bytes.len()) <= max_entry_bytes {
                                entry.body.extend_from_slice(bytes);
                            } else {
                                candidate = None;
                            }
                        }
                    }
                    NetworkEvent::Complete {
                        encoded_body_size, ..
                    } => {
                        if let Some(mut entry) = candidate.take() {
                            entry.encoded_body_size = *encoded_body_size;
                            let _ = store.insert(key.clone(), entry);
                        }
                    }
                    NetworkEvent::Failed { .. } => candidate = None,
                }
                if sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner.cancel_operation(request_id)
    }
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn cache_timing(headers: &[(String, String)], now: std::time::SystemTime) -> Option<(u64, u64)> {
    let control = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(",");
    if control
        .split(',')
        .any(|v| v.trim().eq_ignore_ascii_case("no-store"))
    {
        return None;
    }
    if control
        .split(',')
        .any(|v| v.trim().eq_ignore_ascii_case("no-cache"))
    {
        return Some((0, 0));
    }
    let age = response_header(headers, "age")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1000);
    let date = response_header(headers, "date")
        .and_then(|v| httpdate::parse_http_date(v).ok())
        .unwrap_or(now);
    let lifetime = control
        .split(',')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.eq_ignore_ascii_case("max-age")
                .then(|| value.trim().trim_matches('"').parse::<u64>().ok())
                .flatten()
                .map(|v| v.saturating_mul(1000))
        })
        .or_else(|| {
            response_header(headers, "expires")
                .and_then(|v| httpdate::parse_http_date(v).ok())
                .map(|expires| {
                    expires
                        .duration_since(date)
                        .unwrap_or_default()
                        .as_millis()
                        .min(u128::from(u64::MAX)) as u64
                })
        })
        .or_else(|| {
            response_header(headers, "last-modified")
                .and_then(|v| httpdate::parse_http_date(v).ok())
                .map(|last| {
                    date.duration_since(last)
                        .unwrap_or_default()
                        .as_millis()
                        .min(u128::from(u64::MAX)) as u64
                        / 10
                })
        })
        .unwrap_or(0);
    let apparent = now
        .duration_since(date)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    Some((lifetime, age.max(apparent)))
}

fn cache_freshness_ms(headers: &[(String, String)]) -> Option<u64> {
    cache_timing(headers, std::time::SystemTime::now())
        .map(|(lifetime, age)| lifetime.saturating_sub(age))
}
fn cache_swr_allowed(request: &NetworkRequest, entry: &CacheEntry) -> bool {
    if request.security.fetch_cache != oryn_js::platform::FetchCacheMode::Default
        || cache_has_condition(request)
        || cache_requires_network(&request.headers)
    {
        return false;
    }
    let controls = entry
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
        .collect::<Vec<_>>();
    if controls.iter().any(|part| {
        [
            "no-cache",
            "no-store",
            "must-revalidate",
            "proxy-revalidate",
        ]
        .iter()
        .any(|name| part.eq_ignore_ascii_case(name))
    }) {
        return false;
    }
    let Some(window) = controls
        .iter()
        .find_map(|part| {
            let (name, value) = part.split_once('=')?;
            name.eq_ignore_ascii_case("stale-while-revalidate")
                .then(|| value.trim().trim_matches('"').parse::<u64>().ok())
                .flatten()
        })
        .map(|seconds| seconds.saturating_mul(1000))
    else {
        return false;
    };
    let Some((lifetime, initial_age)) = cache_timing(
        &entry.headers,
        std::time::UNIX_EPOCH + Duration::from_millis(entry.stored_at_ms),
    ) else {
        return false;
    };
    let age = initial_age.saturating_add(unix_time_ms().saturating_sub(entry.stored_at_ms));
    age >= lifetime && age < lifetime.saturating_add(window)
}
fn schedule_cache_revalidation(
    request: &NetworkRequest,
    key: CacheKey,
    pending: &Arc<Mutex<std::collections::BTreeSet<CacheKey>>>,
    limit: usize,
) -> bool {
    let Some(owner) = network_identity::capture_background_owner() else {
        return false;
    };
    if owner.cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return false;
    }
    {
        let mut active = pending.lock().unwrap_or_else(|error| error.into_inner());
        if active.contains(&key) {
            return true;
        }
        if active.len() >= 16 {
            return false;
        }
        active.insert(key.clone());
    }
    struct Permit {
        pending: Arc<Mutex<std::collections::BTreeSet<CacheKey>>>,
        key: CacheKey,
    }
    impl Drop for Permit {
        fn drop(&mut self) {
            self.pending
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&self.key);
        }
    }
    let permit = Permit {
        pending: pending.clone(),
        key,
    };
    let mut request = request.clone();
    request.security.fetch_cache = oryn_js::platform::FetchCacheMode::NoCache;
    crate::resource_io_executor::submit(move || {
        let broker = owner.broker();
        // Re-enter policy/cookie/cache admission with a private identity. The
        // normal NoCache route owns validators, 304 merging and publication.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owner.run(|| {
                collect_operation_bounded_with_cancel(
                    &broker,
                    request,
                    Duration::from_secs(30),
                    limit,
                    &owner.cancelled,
                )
            })
        }));
        operation_runtime().spawn(async move {
            while broker.drain_owned_operations(Duration::ZERO).is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            drop((owner, permit, broker));
        });
    })
    .is_ok()
}

fn cache_representation_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                "set-cookie" | "set-cookie2"
            )
        })
        .cloned()
        .collect()
}
fn cache_has_condition(request: &NetworkRequest) -> bool {
    request.headers.iter().any(|(name, _)| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "if-modified-since" | "if-none-match" | "if-unmodified-since" | "if-match" | "if-range"
        )
    })
}
fn prepare_cache_network(
    request: &mut NetworkRequest,
    entry: Option<CacheEntry>,
) -> Option<CacheEntry> {
    use oryn_js::platform::FetchCacheMode as Mode;
    if cache_request_no_store(request)
        || (request.security.fetch_cache == Mode::Default && cache_has_condition(request))
    {
        request.security.fetch_cache = Mode::NoStore;
    }
    let mode = request.security.fetch_cache;
    let may_validate = matches!(mode, Mode::Default | Mode::NoCache)
        && !cache_has_condition(request)
        && !request
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("range"));
    let retained = entry.filter(|_| may_validate);
    let mut conditional = false;
    if let Some(entry) = &retained {
        for (response, condition) in [
            ("etag", "if-none-match"),
            ("last-modified", "if-modified-since"),
        ] {
            if let Some(value) = response_header(&entry.headers, response) {
                request.headers.push((condition.into(), value.into()));
                conditional = true;
            }
        }
    }
    apply_fetch_cache_headers(request);
    conditional.then_some(retained).flatten()
}
// This final transport operation is idempotent and does not reinterpret native
// conditional validators as authored headers after a cache adapter prepared them.
fn apply_fetch_cache_headers(request: &mut NetworkRequest) {
    use oryn_js::platform::FetchCacheMode as Mode;
    let mode = request.security.fetch_cache;
    let has = |name: &str| {
        request
            .headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(name))
    };
    let (control, pragma) = (has("cache-control"), has("pragma"));
    if matches!(mode, Mode::NoStore | Mode::Reload) {
        if !control {
            request
                .headers
                .push(("cache-control".into(), "no-cache".into()));
        }
        if !pragma {
            request.headers.push(("pragma".into(), "no-cache".into()));
        }
    } else if mode == Mode::NoCache && !control {
        request
            .headers
            .push(("cache-control".into(), "max-age=0".into()));
    }
}

fn finish_empty_revalidation_body(body: &mut Box<dyn Read + Send>) -> Result<(), NetworkError> {
    let mut byte = [0];
    loop {
        match body.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                return Err(NetworkError::Transport(
                    "conditional 304 response contained body bytes".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(NetworkError::Transport(error.to_string())),
        }
    }
}
fn validated_cache_timing_with_size(
    timing: Option<oryn_net::TransportTiming>,
    encoded_body_size: Option<u64>,
) -> Option<oryn_net::TransportTiming> {
    let mut timing = timing.unwrap_or_default();
    timing.encoded_body_size = encoded_body_size;
    validated_cache_timing(Some(timing))
}
fn validated_cache_timing(
    timing: Option<oryn_net::TransportTiming>,
) -> Option<oryn_net::TransportTiming> {
    let mut timing = timing.unwrap_or_default();
    timing.delivery = oryn_common::transport_timing::ResponseDelivery::ValidatedCache;
    Some(timing)
}
fn revalidated_response(
    request_id: u64,
    mut entry: CacheEntry,
    headers: &[(String, String)],
) -> NetworkResponse {
    entry.headers = cache_representation_headers(&entry.headers);
    let mut replaced = std::collections::BTreeSet::new();
    for (name, value) in headers {
        // A 304 has no representation body; its framing never replaces cached bytes.
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "content-length" | "transfer-encoding" | "connection"
        ) {
            continue;
        }
        if replaced.insert(name.to_ascii_lowercase()) {
            entry
                .headers
                .retain(|(old, _)| !old.eq_ignore_ascii_case(name));
        }
        entry.headers.push((name.clone(), value.clone()));
    }
    entry.headers.retain(|(name, value)| {
        !name.eq_ignore_ascii_case("warning") || !value.trim_start().starts_with('1')
    });
    NetworkResponse {
        response_policy: None,
        timing: validated_cache_timing(Some(oryn_net::TransportTiming {
            encoded_body_size: entry.encoded_body_size,
            ..Default::default()
        })),
        request_id,
        status: entry.status,
        headers: entry.headers,
        body: entry.body,
    }
}
fn revalidation_operation(
    mut operation: NetworkOperation,
    entry: Option<CacheEntry>,
) -> NetworkOperation {
    let Some(entry) = entry else { return operation };
    let request_id = operation.request_id;
    let (sender, events) = tokio::sync::mpsc::channel(8);
    let (demand, mut requests) = tokio::sync::mpsc::channel(1);
    operation_runtime().spawn(async move{
        let Some(first)=operation.next_event().await else{return};
        match first{
            NetworkEvent::Headers{timing,status:304,headers,..}=>{
                // Consume the terminal receipt before replacing the empty 304
                // operation. Dropping it at headers would cancel shared work.
                loop{operation.request_body_chunk();let event=tokio::select!{event=operation.next_event()=>event,_=sender.closed()=>return};match event{
                    Some(NetworkEvent::Complete{encoded_body_size:_,request_id:id})if id==request_id=>break,
                    Some(NetworkEvent::Chunk{request_id:id,bytes})if id==request_id&&bytes.is_empty()=>{},
                    Some(NetworkEvent::Failed{request_id,error})=>{let _=sender.send(NetworkEvent::Failed{request_id,error}).await;return},
                    _=>{let _=sender.send(NetworkEvent::Failed{request_id,error:"invalid conditional response body".into()}).await;return},
                }}
                let mut response=revalidated_response(request_id,entry,&headers);response.timing=validated_cache_timing_with_size(timing.map(|timing|*timing),response.timing.as_ref().and_then(|timing|timing.encoded_body_size));operation=operation_from_response(response);
            }
            other=>{if sender.send(other).await.is_err(){return}}
        }
        loop{tokio::select!{
            request=requests.recv()=>{if request.is_none(){break}operation.request_body_chunk();}
            event=operation.next_event()=>{let Some(event)=event else{break};let done=matches!(event,NetworkEvent::Complete{..}|NetworkEvent::Failed{..});if sender.send(event).await.is_err()||done{break}}
        }}
    });
    NetworkOperation {
        body_demand_pending: Default::default(),
        prefetched: Default::default(),
        request_id,
        events,
        demand: Some(demand),
    }
}
fn cache_request_no_store(request: &NetworkRequest) -> bool {
    request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cache-control")
            && value
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("no-store"))
    })
}
fn cache_lookup_allowed(request: &NetworkRequest) -> bool {
    if request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("range"))
    {
        return false;
    }
    use oryn_js::platform::FetchCacheMode as Mode;
    matches!(request.method.as_str(), "GET" | "HEAD")
        && !cache_request_no_store(request)
        && !cache_has_condition(request)
        && !matches!(
            request.security.fetch_cache,
            Mode::NoStore | Mode::Reload | Mode::NoCache
        )
        && (matches!(
            request.security.fetch_cache,
            Mode::ForceCache | Mode::OnlyIfCached
        ) || !cache_requires_network(&request.headers))
}
fn cache_accepts_stale(request: &NetworkRequest) -> bool {
    matches!(
        request.security.fetch_cache,
        oryn_js::platform::FetchCacheMode::ForceCache
            | oryn_js::platform::FetchCacheMode::OnlyIfCached
    )
}
fn cache_miss_requires_error(request: &NetworkRequest) -> Result<(), NetworkError> {
    if request.security.fetch_cache == oryn_js::platform::FetchCacheMode::OnlyIfCached {
        return Err(NetworkError::Transport("only-if-cached cache miss".into()));
    }
    Ok(())
}

fn cache_requires_network(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        (name.eq_ignore_ascii_case("cache-control")
            && value.split(',').any(|directive| {
                let directive = directive.trim();
                directive.eq_ignore_ascii_case("no-cache")
                    || directive.eq_ignore_ascii_case("no-store")
                    || directive.split_once('=').is_some_and(|(name, value)| {
                        name.trim().eq_ignore_ascii_case("max-age")
                            && value.trim().trim_matches('"') == "0"
                    })
            }))
            || (name.eq_ignore_ascii_case("pragma")
                && value
                    .split(',')
                    .any(|value| value.trim().eq_ignore_ascii_case("no-cache")))
    })
}

fn cache_authorized(request: &NetworkRequest, headers: &[(String, String)]) -> bool {
    if request.method != "GET" {
        return false;
    }
    if request.security.fetch_cache == oryn_js::platform::FetchCacheMode::NoStore {
        return false;
    }
    if request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cache-control")
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("no-store"))
    }) {
        return false;
    }

    let has_authorization = request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
    !has_authorization
        || response_header(headers, "cache-control").is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("public"))
        })
}

fn cache_vary(
    response_headers: &[(String, String)],
    request_headers: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    let mut values = std::collections::BTreeMap::new();
    for name in response_headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("vary"))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
    {
        if name == "*" {
            return None;
        }
        if name.is_empty() {
            continue;
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return None;
        }
        let value = request_headers
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        values.insert(name.to_ascii_lowercase(), value);
    }
    Some(values.into_iter().collect())
}

/// Parent-owned exact host/scheme/port routes. Socket destinations change without
/// rewriting web origins or TLS names. No worker-supplied resolved address is used.
#[derive(Clone, Debug, Default)]
pub struct EndpointRoutes {
    routes: BTreeMap<(String, String, u16), IpAddr>,
}

impl EndpointRoutes {
    pub fn new(
        entries: impl IntoIterator<Item = (String, String, u16, IpAddr)>,
    ) -> Result<Self, NetworkError> {
        let mut routes = BTreeMap::new();
        for (scheme, host, port, ip) in entries {
            if routes.len() >= 4096
                || !matches!(scheme.as_str(), "http" | "https" | "ws" | "wss")
                || port == 0
                || ip.is_unspecified()
                || ip.is_multicast()
            {
                return Err(NetworkError::InvalidUrl("invalid endpoint route".into()));
            }
            let url = Url::parse(&format!("{scheme}://{host}:{port}/"))
                .map_err(|_| NetworkError::InvalidUrl("invalid endpoint hostname".into()))?;
            if url.host_str() != Some(host.as_str())
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || host.parse::<IpAddr>().is_ok()
                || host.len() > 253
                || host.split('.').any(|label| {
                    label.is_empty()
                        || label.len() > 63
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
            {
                return Err(NetworkError::InvalidUrl(
                    "endpoint route requires canonical DNS hostname".into(),
                ));
            }
            if routes.insert((scheme, host, port), ip).is_some() {
                return Err(NetworkError::InvalidUrl("duplicate endpoint route".into()));
            }
        }
        Ok(Self { routes })
    }

    pub fn resolve(
        &self,
        policy: &NetworkPolicy,
        url: &Url,
    ) -> Result<Vec<SocketAddr>, NetworkError> {
        policy.check_url(url)?;
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| NetworkError::InvalidUrl("missing endpoint port".into()))?;
        if let Some(ip) = self
            .routes
            .get(&(url.scheme().to_string(), host.clone(), port))
        {
            policy.check_ip(*ip)?;
            return Ok(vec![SocketAddr::new(*ip, port)]);
        }
        if !self.routes.is_empty() {
            return Err(NetworkError::Resolution {
                host,
                detail: "fixture endpoint has no configured scheme/port route".into(),
            });
        }
        resolve_and_authorize(policy, url)
    }
}

impl<B> PolicyBroker<B> {
    pub fn new(policy: NetworkPolicy, inner: B) -> Self {
        Self {
            policy,
            inner,
            endpoint_routes: None,
        }
    }

    pub fn with_endpoint_routes(mut self, routes: EndpointRoutes) -> Self {
        self.endpoint_routes = Some(routes);
        self
    }

    fn authorize(&self, request: &NetworkRequest) -> Result<Vec<SocketAddr>, NetworkError> {
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        match &self.endpoint_routes {
            Some(routes) => routes.resolve(&self.policy, &url),
            None => resolve_and_authorize(&self.policy, &url),
        }
    }
}

pub(crate) fn resolve_and_authorize(
    policy: &NetworkPolicy,
    url: &Url,
) -> Result<Vec<SocketAddr>, NetworkError> {
    policy.check_url(url)?;
    let mut vetted = Vec::new();
    if let Some(host) = url.host_str() {
        if (host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost"))
            && !policy.allow_loopback
        {
            return Err(NetworkError::Policy(PolicyDenied::Loopback));
        }
        let port = url.port_or_known_default().ok_or_else(|| {
            NetworkError::InvalidUrl("URL has no explicit or scheme-default port".into())
        })?;
        // Resolve the reserved localhost namespace locally, without depending
        // on OS wildcard DNS. Keep the URL/Host intact: separate localhost
        // names remain separate origins despite sharing a loopback address.
        let local_name = host.trim_end_matches('.').to_ascii_lowercase();
        if local_name == "localhost" || local_name.ends_with(".localhost") {
            let address = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
            policy.check_ip(address)?;
            vetted.push(SocketAddr::new(address, port));
        } else if let Ok(address) = host.parse::<IpAddr>() {
            policy.check_ip(address)?;
            vetted.push(SocketAddr::new(address, port));
        } else {
            let addresses =
                (host, port)
                    .to_socket_addrs()
                    .map_err(|error| NetworkError::Resolution {
                        host: host.to_string(),
                        detail: error.to_string(),
                    })?;
            let mut resolved = false;
            for address in addresses {
                resolved = true;
                policy.check_ip(address.ip())?;
                vetted.push(address);
            }
            if !resolved {
                return Err(NetworkError::Resolution {
                    host: host.to_string(),
                    detail: "resolver returned no addresses".into(),
                });
            }
        }
    }
    Ok(vetted)
}

/// A single-request rustls transport. Redirects are intentionally disabled so
/// callers can send every hop back through `PolicyBroker`.
pub struct RustlsTransport {
    runtime: Arc<tokio::runtime::Runtime>,
    transport: Arc<oryn_net::RustlsTransport>,
    max_response_bytes: usize,
}

struct AsyncBodyReader {
    encoded_bytes: Arc<std::sync::atomic::AtomicU64>,
    runtime: Arc<tokio::runtime::Runtime>,
    stream: oryn_net::BodyStream,
    current: bytes::Bytes,
    offset: usize,
}

impl Read for AsyncBodyReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        while self.offset >= self.current.len() {
            let next = self.runtime.block_on(self.stream.next());
            match next {
                Some(Ok(chunk)) => {
                    self.encoded_bytes
                        .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    self.current = chunk;
                    self.offset = 0;
                }
                Some(Err(error)) => return Err(std::io::Error::other(error.to_string())),
                None => return Ok(0),
            }
        }
        let count = output.len().min(self.current.len() - self.offset);
        output[..count].copy_from_slice(&self.current[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

struct CompressedLimitReader {
    inner: Box<dyn Read + Send>,
    read: usize,
    limit: usize,
}

impl Read for CompressedLimitReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.limit.saturating_add(1).saturating_sub(self.read);
        let requested = output.len().min(remaining);
        let read = self.inner.read(&mut output[..requested])?;
        self.read = self.read.saturating_add(read);
        if self.read > self.limit {
            return Err(std::io::Error::other(format!(
                "compressed response exceeded {} bytes",
                self.limit
            )));
        }
        Ok(read)
    }
}

fn decoded_body_reader(
    body: Box<dyn Read + Send>,
    encoding: Option<&str>,
    compressed_limit: usize,
) -> Result<Box<dyn Read + Send>, NetworkError> {
    let body = CompressedLimitReader {
        inner: body,
        read: 0,
        limit: compressed_limit,
    };
    match encoding
        .unwrap_or("identity")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "identity" => Ok(Box::new(body)),
        "gzip" | "x-gzip" => Ok(Box::new(flate2::read::GzDecoder::new(body))),
        "deflate" => Ok(Box::new(flate2::read::ZlibDecoder::new(body))),
        "br" => Ok(Box::new(brotli::Decompressor::new(body, 8 * 1024))),
        "zstd" => zstd::stream::read::Decoder::new(body)
            .map(|decoder| Box::new(decoder) as Box<dyn Read + Send>)
            .map_err(|error| NetworkError::Transport(error.to_string())),
        encoding => Err(NetworkError::UnsupportedContentEncoding(encoding.into())),
    }
}

impl RustlsTransport {
    pub fn new(timeout: Duration, max_response_bytes: usize) -> Result<Self, NetworkError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("oryn-http")
            .build()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let transport = oryn_net::RustlsTransport::new(timeout, max_response_bytes)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        Ok(Self {
            runtime: Arc::new(runtime),
            transport: Arc::new(transport),
            max_response_bytes,
        })
    }

    /// Configure fixture CA bytes in the parent-owned transport. The worker
    /// receives neither certificate paths nor authority to alter this trust set.
    pub fn with_trusted_roots(mut self, roots: &[Vec<u8>]) -> Result<Self, NetworkError> {
        let transport = Arc::try_unwrap(self.transport)
            .map_err(|_| {
                NetworkError::Transport("cannot reconfigure shared transport trust".into())
            })?
            .with_trusted_roots(roots)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        self.transport = Arc::new(transport);
        Ok(self)
    }

    #[cfg(test)]
    fn with_test_http2_prior_knowledge(mut self) -> Self {
        let transport = Arc::try_unwrap(self.transport)
            .ok()
            .expect("test transport has one owner")
            .with_http2_prior_knowledge();
        self.transport = Arc::new(transport);
        self
    }
}

// Validate framing independently from decoded length. Compressed bodies expose no
// decoded expectation; HEAD/304 lengths describe the selected representation.
fn decoded_response_length(
    headers: &[(String, String)],
    bodyless: bool,
    identity: bool,
) -> Result<Option<usize>, NetworkError> {
    let mut expected = None;
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    {
        for value in value.split(',') {
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(NetworkError::Transport("invalid Content-Length".into()));
            }
            let length = value
                .parse::<usize>()
                .map_err(|_| NetworkError::Transport("Content-Length overflow".into()))?;
            if expected.is_some_and(|prior| prior != length) {
                return Err(NetworkError::Transport("conflicting Content-Length".into()));
            }
            expected = Some(length);
        }
    }
    Ok(if bodyless {
        Some(0)
    } else if identity {
        expected
    } else {
        None
    })
}

impl NetworkBroker for RustlsTransport {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let max_response_bytes = self.max_response_bytes;
        let mut response = self.start_stream(request)?;
        let headers_received = std::time::Instant::now();
        let mut body = Vec::new();
        response
            .body
            .by_ref()
            .take(max_response_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if body.len() > max_response_bytes {
            return Err(NetworkError::ResponseTooLarge {
                limit: max_response_bytes,
            });
        }
        if let Some(timing) = &mut response.timing {
            timing.encoded_body_size = response
                .encoded_bytes
                .as_ref()
                .map(|count| count.load(Ordering::Relaxed));
            timing.response_end = Some(timing.response_start.saturating_add(
                headers_received.elapsed().as_micros().min(u64::MAX as u128) as u64,
            ));
        }
        Ok(NetworkResponse {
            response_policy: response.response_policy,
            timing: response.timing,
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            body,
        })
    }

    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        crate::fetch_metadata::apply(&mut request)?;
        cache_miss_requires_error(&request)?;
        apply_fetch_cache_headers(&mut request);
        let head = request.method.eq_ignore_ascii_case("HEAD");
        let response = self
            .runtime
            .block_on(self.transport.start(oryn_net::HttpRequest {
                dns_timing: request.security.dns_timing,
                operation_id: request.request_id,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                resolved_addrs: request.resolved_addrs,
            }))
            .map_err(|error| NetworkError::from(NetworkFailure::from(error)))?;
        let status = response.status;
        let headers = response.headers;
        let encoding = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
            .map(|(_, value)| value.as_str());
        let content_length = decoded_response_length(
            &headers,
            head || matches!(status, 204 | 304),
            encoding.is_none_or(|encoding| encoding.eq_ignore_ascii_case("identity")),
        )?;
        let encoded_bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let body = decoded_body_reader(
            Box::new(AsyncBodyReader {
                encoded_bytes: encoded_bytes.clone(),
                runtime: self.runtime.clone(),
                stream: response.body,
                current: bytes::Bytes::new(),
                offset: 0,
            }),
            encoding,
            self.max_response_bytes,
        )?;
        Ok(NetworkStreamResponse {
            response_policy: None,
            encoded_bytes: Some(encoded_bytes),
            timing: Some(response.timing),
            request_id: response.operation_id,
            status,
            headers,
            content_length,
            upload: response.upload,
            body,
        })
    }

    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        crate::fetch_metadata::apply(&mut request)?;
        cache_miss_requires_error(&request)?;
        apply_fetch_cache_headers(&mut request);
        let request_id = request.request_id;
        let head = request.method.eq_ignore_ascii_case("HEAD");
        let transport = self.transport.clone();
        let decoded_limit = self.max_response_bytes;
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let (demand, mut requested) = tokio::sync::mpsc::channel(1);
        self.runtime.spawn(async move {
            let response = transport
                .start(oryn_net::HttpRequest {
                    dns_timing: request.security.dns_timing,
                    operation_id: request_id,
                    method: request.method,
                    url: request.url,
                    headers: request.headers,
                    body: request.body,
                    resolved_addrs: request.resolved_addrs,
                })
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    let _ = sender
                        .send(NetworkEvent::Failed {
                            request_id,
                            error: error.into(),
                        })
                        .await;
                    return;
                }
            };
            let encoding = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
                .map(|(_, value)| value.as_str());
            let encoding = match oryn_net::ContentEncoding::parse(encoding) {
                Ok(encoding) => encoding,
                Err(error) => {
                    let _ = sender
                        .send(NetworkEvent::Failed {
                            request_id,
                            error: error.to_string().into(),
                        })
                        .await;
                    return;
                }
            };
            let content_length = match decoded_response_length(
                &response.headers,
                head || matches!(response.status, 204 | 304),
                encoding == oryn_net::ContentEncoding::Identity,
            ) {
                Ok(length) => length,
                Err(error) => {
                    let _ = transport.cancel(request_id).await;
                    let _ = sender
                        .send(NetworkEvent::Failed {
                            request_id,
                            error: error.to_string().into(),
                        })
                        .await;
                    return;
                }
            };
            if sender
                .send(NetworkEvent::Headers {
                    response_policy: None,
                    timing: Some(Box::new(response.timing)),
                    request_id,
                    status: response.status,
                    headers: response.headers,
                    content_length,
                    upload: response.upload,
                })
                .await
                .is_err()
            {
                let _ = transport.cancel(request_id).await;
                return;
            }
            let encoded_bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let counter = encoded_bytes.clone();
            let raw_body = response.body.map(move |chunk| {
                if let Ok(bytes) = &chunk {
                    counter.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                }
                chunk
            });
            let mut body = oryn_net::decode_content_stream(
                Box::pin(raw_body),
                encoding,
                decoded_limit,
                decoded_limit,
            );
            while requested.recv().await.is_some() {
                let Some(chunk) = body.next().await else {
                    let _ = sender
                        .send(NetworkEvent::Complete {
                            encoded_body_size: Some(encoded_bytes.load(Ordering::Relaxed)),
                            request_id,
                        })
                        .await;
                    return;
                };
                match chunk {
                    Ok(bytes) => {
                        if sender
                            .send(NetworkEvent::Chunk { request_id, bytes })
                            .await
                            .is_err()
                        {
                            let _ = transport.cancel(request_id).await;
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender
                            .send(NetworkEvent::Failed {
                                request_id,
                                error: error.to_string().into(),
                            })
                            .await;
                        return;
                    }
                }
            }
            // Dropping the consumer (navigation, page, or worker replacement)
            // closes demand. Translate that ownership loss into an explicit
            // transport cancellation so late completions cannot be reused.
            let _ = transport.cancel(request_id).await;
        });
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand: Some(demand),
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        let transport = self.transport.clone();
        self.runtime.spawn(async move {
            let _ = transport.cancel(request_id).await;
        });
        Ok(())
    }
}

/// The only privileged service exposed to a page worker. The connected Unix
/// socket is inherited from the parent, so the sandboxed child needs no
/// filesystem or network entitlement of its own.
pub struct IpcNetworkBroker {
    writer: Arc<Mutex<UnixStream>>,
    next_message_id: Arc<std::sync::atomic::AtomicU64>,
    pending: Arc<Mutex<PendingNetworkReplies>>,
    active_requests: Arc<Mutex<ActiveNetworkRequests>>,
}

type PendingNetworkReplies =
    BTreeMap<u64, (Option<WorkerRoute>, tokio::sync::mpsc::Sender<NetworkEvent>)>;
type ActiveNetworkRequests = BTreeMap<(Option<WorkerRoute>, u64), u64>;

#[derive(Debug, Serialize, Deserialize)]
struct BrokerRequestEnvelope {
    protocol_version: u32,
    route: Option<WorkerRoute>,
    message_id: u64,
    command: BrokerCommand,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum BrokerCommand {
    Start { request: Box<EncodedNetworkRequest> },
    Cancel { request_id: u64 },
    DiscardResponsePolicy { token: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerResponseEnvelope {
    protocol_version: u32,
    route: Option<WorkerRoute>,
    correlation_id: u64,
    #[serde(default)]
    response: Option<EncodedNetworkResponse>,
    #[serde(default)]
    event: Option<EncodedNetworkEvent>,
    #[serde(default)]
    error: Option<NetworkFailure>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EncodedNetworkEvent {
    Headers {
        #[serde(default)]
        response_policy: Option<String>,
        #[serde(default)]
        timing: Option<Box<oryn_net::TransportTiming>>,
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        content_length: Option<usize>,
        upload: Option<oryn_net::UploadProgress>,
    },
    Chunk {
        request_id: u64,
        body_base64: String,
    },
    Complete {
        #[serde(default)]
        encoded_body_size: Option<u64>,
        request_id: u64,
    },
    Failed {
        request_id: u64,
        error: NetworkFailure,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedNetworkRequest {
    request_id: u64,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body_base64: String,
    #[serde(default = "default_credentials")]
    credentials: bool,
    #[serde(default)]
    security: CookieRequestContext,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedNetworkResponse {
    #[serde(default)]
    response_policy: Option<String>,
    #[serde(default)]
    timing: Option<oryn_net::TransportTiming>,
    request_id: u64,
    status: u16,
    headers: Vec<(String, String)>,
    body_base64: String,
}

impl IpcNetworkBroker {
    pub fn new(stream: UnixStream) -> Self {
        let reader = stream.try_clone().expect("clone broker IPC stream");
        let writer = Arc::new(Mutex::new(stream));
        let next_message_id = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let reader_writer = writer.clone();
        let reader_message_id = next_message_id.clone();
        let pending: Arc<Mutex<PendingNetworkReplies>> = Arc::new(Mutex::new(BTreeMap::new()));
        let reader_pending = pending.clone();
        let active_requests = Arc::new(Mutex::new(BTreeMap::new()));
        let reader_active_requests = active_requests.clone();
        std::thread::Builder::new()
            .name("oryn-worker-broker-reader".into())
            .spawn(move || {
                broker_response_loop(
                    reader,
                    reader_pending,
                    reader_active_requests,
                    reader_writer,
                    reader_message_id,
                )
            })
            .expect("spawn broker response reader");
        Self {
            writer,
            next_message_id,
            pending,
            active_requests,
        }
    }
}

impl Drop for IpcNetworkBroker {
    fn drop(&mut self) {
        if let Ok(writer) = self.writer.lock() {
            let _ = writer.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl NetworkBroker for IpcNetworkBroker {
    fn discard_response_policy_routed(
        &self,
        route: Option<&WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        write_response_policy_discard(
            &self.writer,
            &self.next_message_id,
            route.cloned(),
            token.into(),
        )
    }
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let mut operation = self.start_operation(request)?;
        let mut response_policy = None;
        let mut transport_timing: Option<oryn_net::TransportTiming> = None;
        let mut timing_headers = None;
        let mut status = None;
        let mut headers = Vec::new();
        let mut body = Vec::new();
        while let Some(event) = operation.blocking_event() {
            match event {
                NetworkEvent::Headers {
                    response_policy: policy,
                    timing,
                    status: value,
                    headers: values,
                    ..
                } => {
                    response_policy = policy;
                    transport_timing = timing.map(|timing| *timing);
                    timing_headers = Some(std::time::Instant::now());
                    status = Some(value);
                    headers = values;
                }
                NetworkEvent::Chunk { bytes, .. } => {
                    body.extend_from_slice(&bytes);
                    if body.len() > MAX_DECODED_RESPONSE_BYTES {
                        return Err(NetworkError::ResponseTooLarge {
                            limit: MAX_DECODED_RESPONSE_BYTES,
                        });
                    }
                }
                NetworkEvent::Complete {
                    encoded_body_size,
                    request_id,
                } => {
                    return Ok(NetworkResponse {
                        response_policy,
                        timing: transport_timing.map(|mut timing| {
                            timing.encoded_body_size =
                                encoded_body_size.or(timing.encoded_body_size);
                            timing.response_end = Some(timing.response_start.saturating_add(
                                timing_headers.map_or(0, |headers: std::time::Instant| {
                                    headers.elapsed().as_micros().min(u64::MAX as u128) as u64
                                }),
                            ));
                            timing
                        }),
                        request_id,
                        status: status.ok_or_else(|| {
                            NetworkError::Transport("broker completed before headers".into())
                        })?,
                        headers,
                        body,
                    });
                }
                NetworkEvent::Failed { error, .. } => {
                    return Err(NetworkError::from(error));
                }
            }
        }
        Err(NetworkError::Transport(
            "broker operation ended before completion".into(),
        ))
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        // Allocate IDs under the same lock that orders frames on the wire.
        self.start_routed(None, request)
    }

    fn start_routed(
        &self,
        route: Option<&WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        // Allocate IDs under the same lock that orders frames on the wire.
        // Otherwise concurrent callers can publish N+1 before N and trip the
        // parent's replay protection on a legitimate request.
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| NetworkError::Transport("network IPC writer lock poisoned".into()))?;
        let message_id = self
            .next_message_id
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |id| id.checked_add(1),
            )
            .map_err(|_| NetworkError::Transport("network message identifiers exhausted".into()))?;
        let request_id = request.request_id;
        let (sender, events) = tokio::sync::mpsc::channel(8);
        self.pending
            .lock()
            .map_err(|_| NetworkError::Transport("network IPC pending lock poisoned".into()))?
            .insert(message_id, (route.cloned(), sender));
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            route: route.cloned(),
            message_id,
            command: BrokerCommand::Start {
                request: Box::new(EncodedNetworkRequest {
                    request_id,
                    method: request.method,
                    url: request.url,
                    headers: request.headers,
                    body_base64: BASE64.encode(request.body),
                    credentials: request.credentials,
                    security: request.security,
                }),
            },
        };
        // Publish correlation state before the request becomes visible to the
        // parent. A loopback or cached response may complete immediately.
        self.active_requests
            .lock()
            .map_err(|_| NetworkError::Transport("network IPC active lock poisoned".into()))?
            .insert((route.cloned(), request_id), message_id);
        let result = (|| {
            serde_json::to_writer(&mut *writer, &envelope)
                .map_err(|error| NetworkError::Transport(error.to_string()))?;
            writer
                .write_all(b"\n")
                .and_then(|_| writer.flush())
                .map_err(|error| NetworkError::Transport(error.to_string()))
        })();
        if let Err(error) = result {
            let _ = self
                .pending
                .lock()
                .map(|mut pending| pending.remove(&message_id));
            let _ = self
                .active_requests
                .lock()
                .map(|mut active| active.remove(&(route.cloned(), request_id)));
            return Err(error);
        }
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand: None,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.cancel_routed(None, request_id)
    }

    fn cancel_routed(
        &self,
        route: Option<&WorkerRoute>,
        request_id: u64,
    ) -> Result<(), NetworkError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| NetworkError::Transport("network IPC writer lock poisoned".into()))?;
        let message_id = self
            .next_message_id
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |id| id.checked_add(1),
            )
            .map_err(|_| NetworkError::Transport("network message identifiers exhausted".into()))?;
        if let Some(message_id) = self
            .active_requests
            .lock()
            .map_err(|_| NetworkError::Transport("network IPC active lock poisoned".into()))?
            .remove(&(route.cloned(), request_id))
        {
            self.pending
                .lock()
                .map_err(|_| NetworkError::Transport("network IPC pending lock poisoned".into()))?
                .remove(&message_id);
        }
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            route: route.cloned(),
            message_id,
            command: BrokerCommand::Cancel { request_id },
        };
        serde_json::to_writer(&mut *writer, &envelope)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| NetworkError::Transport(error.to_string()))
    }
}

fn broker_response_loop(
    stream: UnixStream,
    pending: Arc<Mutex<PendingNetworkReplies>>,
    active_requests: Arc<Mutex<ActiveNetworkRequests>>,
    writer: Arc<Mutex<UnixStream>>,
    next_message_id: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = Vec::new();
        let Ok(read) = read_bounded_line(&mut reader, &mut line, MAX_BROKER_FRAME_BYTES) else {
            break;
        };
        if read == 0 || line.len() > MAX_BROKER_FRAME_BYTES {
            break;
        }
        let Ok(mut response) = serde_json::from_slice::<BrokerResponseEnvelope>(&line) else {
            continue;
        };
        if response.protocol_version != BROKER_PROTOCOL_VERSION {
            continue;
        }
        // Acquire cleanup before validating correlation, route or transport
        // metadata. Rejected headers still carry a parent-owned reservation.
        let mut response_policy = match response.event.as_mut() {
            Some(EncodedNetworkEvent::Headers {
                response_policy, ..
            }) => response_policy.take(),
            _ => response
                .response
                .as_mut()
                .and_then(|response| response.response_policy.take()),
        }
        .map(ResponsePolicyToken::from);
        if let Some(token) = &response_policy {
            attach_response_policy_cleanup(
                token,
                response.route.clone(),
                &writer,
                &next_message_id,
            );
        }
        let sender = pending
            .lock()
            .ok()
            .and_then(|pending| pending.get(&response.correlation_id).cloned());
        let Some((route, sender)) = sender else {
            // Cancellation may have removed correlation before an admitted
            // response reaches this reader. Reclaim its token even though no
            // navigation consumer will ever see the headers.
            continue;
        };
        let mut event = if response.route != route {
            NetworkEvent::Failed {
                request_id: 0,
                error: "network response route mismatch".into(),
            }
        } else if let Some(error) = response.error {
            NetworkEvent::Failed {
                request_id: 0,
                error,
            }
        } else if let Some(event) = response.event {
            match decode_network_event(event) {
                Ok(event) => event,
                Err(error) => NetworkEvent::Failed {
                    request_id: 0,
                    error: error.to_string().into(),
                },
            }
        } else if let Some(response) = response.response {
            let body = BASE64.decode(response.body_base64).unwrap_or_default();
            let request_id = response.request_id;
            let encoded_body_size = response
                .timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size);
            let _ = sender.blocking_send(NetworkEvent::Headers {
                response_policy: response_policy.take(),
                timing: response.timing.map(Box::new),
                request_id,
                status: response.status,
                headers: sanitize_parent_response_headers(response.headers),
                content_length: Some(body.len()),
                upload: None,
            });
            let _ = sender.blocking_send(NetworkEvent::Chunk {
                request_id,
                bytes: bytes::Bytes::from(body),
            });
            NetworkEvent::Complete {
                encoded_body_size,
                request_id,
            }
        } else {
            NetworkEvent::Failed {
                request_id: 0,
                error: "broker returned no event".into(),
            }
        };
        if let NetworkEvent::Headers {
            response_policy: policy,
            ..
        } = &mut event
        {
            *policy = response_policy.take();
        }
        let terminal = matches!(
            event,
            NetworkEvent::Complete { .. } | NetworkEvent::Failed { .. }
        );
        let _ = sender.blocking_send(event);
        if terminal && let Ok(mut pending) = pending.lock() {
            pending.remove(&response.correlation_id);
            if let Ok(mut active) = active_requests.lock() {
                active.retain(|_, message_id| *message_id != response.correlation_id);
            }
        }
    }
    // A broken response channel cannot leave request-side authority alive.
    // Closing both halves also makes the parent revoke outstanding grants.
    let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
    if let Ok(mut pending) = pending.lock() {
        for (_, (_, sender)) in std::mem::take(&mut *pending) {
            let _ = sender.blocking_send(NetworkEvent::Failed {
                request_id: 0,
                error: "broker IPC closed".into(),
            });
        }
    }
}

fn write_response_policy_discard(
    writer: &Arc<Mutex<UnixStream>>,
    next_message_id: &Arc<std::sync::atomic::AtomicU64>,
    route: Option<WorkerRoute>,
    value: String,
) -> Result<(), NetworkError> {
    let mut writer = writer.lock().unwrap_or_else(|error| error.into_inner());
    let result = (|| -> Result<(), NetworkError> {
        let message_id = next_message_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| NetworkError::Transport("network message identifiers exhausted".into()))?;
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            route,
            message_id,
            command: BrokerCommand::DiscardResponsePolicy { token: value },
        };
        serde_json::to_writer(&mut *writer, &envelope)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| NetworkError::Transport(error.to_string()))
    })();
    if result.is_err() {
        // A failed cleanup frame must not leave a half-written stream
        // available for subsequent requests under the same route.
        let _ = writer.shutdown(std::net::Shutdown::Both);
    }
    result
}

fn attach_response_policy_cleanup(
    token: &ResponsePolicyToken,
    route: Option<WorkerRoute>,
    writer: &Arc<Mutex<UnixStream>>,
    next_message_id: &Arc<std::sync::atomic::AtomicU64>,
) {
    let value = token.as_str().to_owned();
    let writer = Arc::downgrade(writer);
    let next_message_id = next_message_id.clone();
    token.attach_cleanup(move || {
        // Dropping a queue item cannot block its consumer or an IPC reader.
        // The message is token-specific; it never cancels a successor's body.
        operation_runtime().spawn_blocking(move || {
            let Some(writer) = writer.upgrade() else {
                return;
            };
            let _ = write_response_policy_discard(&writer, &next_message_id, route, value);
        });
    });
}

fn decode_network_event(event: EncodedNetworkEvent) -> Result<NetworkEvent, NetworkError> {
    if matches!(&event,EncodedNetworkEvent::Headers{timing:Some(timing),..}if !timing.valid_snapshot())
    {
        return Err(NetworkError::Transport(
            "invalid native transport timing snapshot".into(),
        ));
    }
    Ok(match event {
        EncodedNetworkEvent::Headers {
            response_policy,
            timing,
            request_id,
            status,
            headers,
            content_length,
            upload,
        } => NetworkEvent::Headers {
            response_policy: response_policy.map(ResponsePolicyToken::from),
            timing,
            request_id,
            status,
            headers,
            content_length,
            upload,
        },
        EncodedNetworkEvent::Chunk {
            request_id,
            body_base64,
        } => NetworkEvent::Chunk {
            request_id,
            bytes: bytes::Bytes::from(
                BASE64
                    .decode(body_base64)
                    .map_err(|error| NetworkError::Transport(error.to_string()))?,
            ),
        },
        EncodedNetworkEvent::Complete {
            encoded_body_size,
            request_id,
        } => NetworkEvent::Complete {
            encoded_body_size,
            request_id,
        },
        EncodedNetworkEvent::Failed { request_id, error } => {
            NetworkEvent::Failed { request_id, error }
        }
    })
}

/// Serve a worker's inherited broker socket from the privileged parent.
pub fn serve_ipc_broker(
    stream: UnixStream,
    broker: impl NetworkBroker + 'static,
    max_frame_bytes: usize,
) -> Result<(), NetworkError> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let mut reader = BufReader::new(reader_stream);
    let writer = Arc::new(Mutex::new(stream));
    let broker: Arc<dyn NetworkBroker> = Arc::new(broker);
    let mut last_message_id = 0_u64;
    loop {
        let mut line = Vec::new();
        let read = read_bounded_line(&mut reader, &mut line, max_frame_bytes)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if read == 0 {
            return Ok(());
        }
        let response = if line.len() > max_frame_bytes {
            Some(BrokerResponseEnvelope {
                protocol_version: BROKER_PROTOCOL_VERSION,
                route: None,
                correlation_id: 0,
                response: None,
                event: None,
                error: Some("broker request exceeded the IPC frame limit".into()),
            })
        } else {
            match serde_json::from_slice::<BrokerRequestEnvelope>(&line) {
                Ok(envelope)
                    if envelope.protocol_version == BROKER_PROTOCOL_VERSION
                        && envelope.message_id > last_message_id =>
                {
                    let correlation_id = envelope.message_id;
                    let route = envelope.route;
                    last_message_id = envelope.message_id;
                    let request = match envelope.command {
                        BrokerCommand::Start { request } => *request,
                        BrokerCommand::Cancel { request_id } => {
                            let _ = broker.cancel_routed(route.as_ref(), request_id);
                            continue;
                        }
                        BrokerCommand::DiscardResponsePolicy { token } => {
                            let _ = broker.discard_response_policy_routed(route.as_ref(), &token);
                            continue;
                        }
                    };
                    let result = BASE64
                        .decode(request.body_base64)
                        .map_err(|error| error.to_string())
                        .and_then(|body| {
                            broker
                                .start_routed(
                                    route.as_ref(),
                                    NetworkRequest {
                                        request_id: request.request_id,
                                        method: request.method,
                                        url: request.url,
                                        headers: sanitize_worker_headers(request.headers),
                                        body,
                                        credentials: request.credentials,
                                        security: request.security,
                                        resolved_addrs: Vec::new(),
                                    },
                                )
                                .map_err(|error| error.to_string())
                        });
                    match result {
                        Ok(mut operation) => {
                            let writer = writer.clone();
                            operation_runtime().spawn(async move {
                                while let Some(event) = operation.next_event().await {
                                    match write_network_event(
                                        &writer,
                                        route.clone(),
                                        correlation_id,
                                        event,
                                    ) {
                                        Ok(false) => {}
                                        Ok(true) | Err(_) => break,
                                    }
                                    operation.request_body_chunk();
                                }
                            });
                            None
                        }
                        Err(error) => Some(BrokerResponseEnvelope {
                            protocol_version: BROKER_PROTOCOL_VERSION,
                            route,
                            correlation_id,
                            response: None,
                            event: None,
                            error: Some(error.into()),
                        }),
                    }
                }
                Ok(envelope) => Some(BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    route: envelope.route,
                    correlation_id: envelope.message_id,
                    response: None,
                    event: None,
                    error: Some(if envelope.protocol_version != BROKER_PROTOCOL_VERSION {
                        "unsupported broker protocol version".into()
                    } else {
                        "duplicate or out-of-order broker message".into()
                    }),
                }),
                Err(error) => Some(BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    route: None,
                    correlation_id: 0,
                    response: None,
                    event: None,
                    error: Some(format!("invalid broker request: {error}").into()),
                }),
            }
        };
        if let Some(response) = response {
            write_broker_response(&writer, &response)?;
        }
        if line.len() > max_frame_bytes {
            return Ok(());
        }
    }
}

// Retain parent cleanup across serialization and the complete frame write.
// A queued/dropped event or failed write still releases the original grant;
// only successful delivery transfers cleanup custody to the worker decoder.
fn write_network_event(
    writer: &Arc<Mutex<UnixStream>>,
    route: Option<WorkerRoute>,
    correlation_id: u64,
    event: NetworkEvent,
) -> Result<bool, NetworkError> {
    let response_policy = match &event {
        NetworkEvent::Headers {
            response_policy, ..
        } => response_policy.clone(),
        _ => None,
    };
    let event = encode_network_event(event);
    let terminal = matches!(
        event,
        EncodedNetworkEvent::Complete { .. } | EncodedNetworkEvent::Failed { .. }
    );
    write_broker_response(
        writer,
        &BrokerResponseEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            route,
            correlation_id,
            response: None,
            event: Some(event),
            error: None,
        },
    )?;
    if let Some(token) = response_policy {
        token.transferred_to_remote();
    }
    Ok(terminal)
}

fn encode_network_event(event: NetworkEvent) -> EncodedNetworkEvent {
    match event {
        NetworkEvent::Headers {
            response_policy,
            timing,
            request_id,
            status,
            headers,
            content_length,
            upload,
        } => EncodedNetworkEvent::Headers {
            response_policy: response_policy.map(|token| token.as_str().to_owned()),
            timing,
            request_id,
            status,
            headers: sanitize_parent_response_headers(headers),
            content_length,
            upload,
        },
        NetworkEvent::Chunk { request_id, bytes } => EncodedNetworkEvent::Chunk {
            request_id,
            body_base64: BASE64.encode(bytes),
        },
        NetworkEvent::Complete {
            encoded_body_size,
            request_id,
        } => EncodedNetworkEvent::Complete {
            encoded_body_size,
            request_id,
        },
        NetworkEvent::Failed { request_id, error } => {
            EncodedNetworkEvent::Failed { request_id, error }
        }
    }
}

fn write_broker_response(
    writer: &Arc<Mutex<UnixStream>>,
    response: &BrokerResponseEnvelope,
) -> Result<(), NetworkError> {
    let mut writer = writer
        .lock()
        .map_err(|_| NetworkError::Transport("broker response writer lock poisoned".into()))?;
    serde_json::to_writer(&mut *writer, response)
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| NetworkError::Transport(error.to_string()))
}

pub(crate) fn sanitize_worker_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(name, value)| {
            oryn_common::http_headers::valid_header_name(name)
                && oryn_common::http_headers::valid_header_value(value)
                && !oryn_common::http_headers::forbidden_request_header(name, value)
        })
        .collect()
}

pub(crate) fn sanitize_parent_response_headers(
    headers: Vec<(String, String)>,
) -> Vec<(String, String)> {
    const DENIED: &[&str] = &[
        "set-cookie",
        "set-cookie2",
        "proxy-authenticate",
        "proxy-authorization",
    ];
    headers
        .into_iter()
        .filter(|(name, _)| !DENIED.contains(&name.to_ascii_lowercase().as_str()))
        .collect()
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<usize> {
    let mut limited = reader.take(limit.saturating_add(1) as u64);
    limited.read_until(b'\n', output)
}

impl<B: NetworkBroker> NetworkBroker for PolicyBroker<B> {
    fn send(&self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let dns_start = std::time::Instant::now();
        request.resolved_addrs = self.authorize(&request)?;
        request.security.dns_timing = Some(oryn_net::DnsTiming {
            start: dns_start,
            end: std::time::Instant::now(),
        });
        self.inner.send(request)
    }

    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        let dns_start = std::time::Instant::now();
        request.resolved_addrs = self.authorize(&request)?;
        request.security.dns_timing = Some(oryn_net::DnsTiming {
            start: dns_start,
            end: std::time::Instant::now(),
        });
        self.inner.start_stream(request)
    }

    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        let dns_start = std::time::Instant::now();
        request.resolved_addrs = self.authorize(&request)?;
        request.security.dns_timing = Some(oryn_net::DnsTiming {
            start: dns_start,
            end: std::time::Instant::now(),
        });
        self.inner.start_operation(request)
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner.cancel_operation(request_id)
    }
}

#[derive(Default)]
pub struct UnavailableBroker;

impl NetworkBroker for UnavailableBroker {
    fn send(&self, _request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        Err(NetworkError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    #[test]
    fn parent_header_cleanup_is_released_only_after_successful_ipc_write() {
        for successful in [false, true] {
            let count = Arc::new(AtomicUsize::new(0));
            let cleanup_count = count.clone();
            let token = ResponsePolicyToken::from("parent-test-grant");
            token.attach_cleanup(move || {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
            });
            let (parent, worker) = UnixStream::pair().unwrap();
            let mut reader = BufReader::new(worker);
            if !successful {
                // Disable this endpoint's writer: shutting down the peer does
                // not portably guarantee that the next local write fails.
                parent.shutdown(std::net::Shutdown::Write).unwrap();
            }
            let writer = Arc::new(Mutex::new(parent));
            let result = write_network_event(
                &writer,
                None,
                7,
                NetworkEvent::Headers {
                    response_policy: Some(token),
                    timing: None,
                    request_id: 41,
                    status: 200,
                    headers: Vec::new(),
                    content_length: Some(1),
                    upload: None,
                },
            );
            if successful {
                assert!(!result.unwrap());
                assert_eq!(
                    count.load(Ordering::SeqCst),
                    0,
                    "successful handoff must not cancel the remote grant"
                );
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let envelope: BrokerResponseEnvelope = serde_json::from_str(&line).unwrap();
                assert!(
                    matches!(envelope.event, Some(EncodedNetworkEvent::Headers { response_policy: Some(token), .. }) if token == "parent-test-grant")
                );
                assert!(
                    write_network_event(
                        &writer,
                        None,
                        7,
                        NetworkEvent::Failed {
                            request_id: 41,
                            error: "body failed".into()
                        }
                    )
                    .unwrap()
                );
                line.clear();
                reader.read_line(&mut line).unwrap();
                let envelope: BrokerResponseEnvelope = serde_json::from_str(&line).unwrap();
                assert!(
                    matches!(envelope.event, Some(EncodedNetworkEvent::Failed { error, .. }) if error.to_string() == "body failed")
                );
                assert_eq!(count.load(Ordering::SeqCst), 0);
            } else {
                assert!(result.is_err());
                assert_eq!(
                    count.load(Ordering::SeqCst),
                    1,
                    "failed delivery must reclaim parent custody exactly once"
                );
            }
        }
    }

    fn policy_test_request() -> NetworkRequest {
        NetworkRequest {
            request_id: 71,
            method: "GET".into(),
            url: "https://policy.test/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: true,
            security: CookieRequestContext::default(),
            resolved_addrs: Vec::new(),
        }
    }

    fn policy_test_read(reader: &mut BufReader<UnixStream>) -> BrokerRequestEnvelope {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn policy_test_write(
        writer: &mut UnixStream,
        request: &BrokerRequestEnvelope,
        event: EncodedNetworkEvent,
    ) {
        serde_json::to_writer(
            &mut *writer,
            &BrokerResponseEnvelope {
                protocol_version: BROKER_PROTOCOL_VERSION,
                route: request.route.clone(),
                correlation_id: request.message_id,
                response: None,
                event: Some(event),
                error: None,
            },
        )
        .unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
    }

    fn policy_test_headers() -> EncodedNetworkEvent {
        EncodedNetworkEvent::Headers {
            response_policy: Some("exact-native-grant".into()),
            timing: None,
            request_id: 71,
            status: 200,
            headers: Vec::new(),
            content_length: Some(1),
            upload: None,
        }
    }

    #[test]
    fn unread_completed_ipc_headers_discard_the_exact_grant() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(parent.try_clone().unwrap());
            let start = policy_test_read(&mut reader);
            policy_test_write(&mut parent, &start, policy_test_headers());
            policy_test_write(
                &mut parent,
                &start,
                EncodedNetworkEvent::Complete {
                    encoded_body_size: Some(0),
                    request_id: 71,
                },
            );
            let discard = policy_test_read(&mut reader);
            assert_eq!(discard.route, start.route);
            assert!(discard.message_id > start.message_id);
            assert!(
                matches!(discard.command, BrokerCommand::DiscardResponsePolicy { token } if token == "exact-native-grant")
            );
        });
        let broker = IpcNetworkBroker::new(worker);
        let route = WorkerRoute::issue(1, 1, 0).unwrap();
        let operation = broker
            .start_routed(Some(&route), policy_test_request())
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while operation.events.len() < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "headers/completion did not reach the unread queue"
            );
            std::thread::park_timeout(Duration::from_millis(1));
        }
        drop(operation);
        server.join().unwrap();
    }

    #[test]
    fn late_ipc_headers_are_discarded_after_correlation_was_cancelled() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(parent.try_clone().unwrap());
            let start = policy_test_read(&mut reader);
            let cancel = policy_test_read(&mut reader);
            assert!(matches!(
                cancel.command,
                BrokerCommand::Cancel { request_id: 71 }
            ));
            policy_test_write(&mut parent, &start, policy_test_headers());
            let discard = policy_test_read(&mut reader);
            assert_eq!(discard.route, start.route);
            assert!(discard.message_id > cancel.message_id);
            assert!(
                matches!(discard.command, BrokerCommand::DiscardResponsePolicy { token } if token == "exact-native-grant")
            );
        });
        let broker = IpcNetworkBroker::new(worker);
        let route = WorkerRoute::issue(1, 1, 0).unwrap();
        let operation = broker
            .start_routed(Some(&route), policy_test_request())
            .unwrap();
        broker.cancel_routed(Some(&route), 71).unwrap();
        drop(operation);
        server.join().unwrap();
    }

    #[test]
    fn rejected_ipc_timing_releases_response_policy_before_publication() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(parent.try_clone().unwrap());
            let start = policy_test_read(&mut reader);
            let mut headers = policy_test_headers();
            if let EncodedNetworkEvent::Headers { timing, .. } = &mut headers {
                *timing = Some(Box::new(oryn_net::TransportTiming {
                    dns_start: Some(2),
                    dns_end: Some(1),
                    ..Default::default()
                }));
            }
            policy_test_write(&mut parent, &start, headers);
            let discard = policy_test_read(&mut reader);
            assert_eq!(discard.route, start.route);
            assert!(
                matches!(discard.command, BrokerCommand::DiscardResponsePolicy { token } if token == "exact-native-grant")
            );
        });
        let broker = IpcNetworkBroker::new(worker);
        let route = WorkerRoute::issue(1, 1, 0).unwrap();
        let mut operation = broker
            .start_routed(Some(&route), policy_test_request())
            .unwrap();
        assert!(
            matches!(operation.blocking_event(), Some(NetworkEvent::Failed { error, .. })
            if error.contains("invalid native transport timing snapshot"))
        );
        server.join().unwrap();
    }

    #[test]
    fn committed_ipc_grant_does_not_cancel_the_remaining_body() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (committed, ready) = std::sync::mpsc::channel();
        let (consumed, done) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(parent.try_clone().unwrap());
            let start = policy_test_read(&mut reader);
            policy_test_write(&mut parent, &start, policy_test_headers());
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            policy_test_write(
                &mut parent,
                &start,
                EncodedNetworkEvent::Chunk {
                    request_id: 71,
                    body_base64: BASE64.encode(b"x"),
                },
            );
            policy_test_write(
                &mut parent,
                &start,
                EncodedNetworkEvent::Complete {
                    request_id: 71,
                    encoded_body_size: Some(1),
                },
            );
            done.recv_timeout(Duration::from_secs(5)).unwrap();
            reader
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let mut line = String::new();
            let error = reader
                .read_line(&mut line)
                .expect_err("committed token emitted cleanup");
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ));
        });
        let broker = IpcNetworkBroker::new(worker);
        let route = WorkerRoute::issue(1, 1, 0).unwrap();
        let mut operation = broker
            .start_routed(Some(&route), policy_test_request())
            .unwrap();
        let token = match operation.blocking_event().unwrap() {
            NetworkEvent::Headers {
                response_policy: Some(token),
                ..
            } => token,
            event => panic!("expected owned headers: {event:?}"),
        };
        token.committed();
        drop(token);
        committed.send(()).unwrap();
        assert!(
            matches!(operation.blocking_event(), Some(NetworkEvent::Chunk { bytes, .. }) if bytes.as_ref() == b"x")
        );
        assert!(matches!(
            operation.blocking_event(),
            Some(NetworkEvent::Complete { .. })
        ));
        drop(operation);
        consumed.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn extracted_response_mime_aligns_document_decoder_and_timing() {
        for (values, bytes, expected_type, expected_text) in [
            (
                vec![
                    "application/octet-stream",
                    "text/html;charset=windows-1252",
                    "text/html, */*, invalid",
                ],
                &b"<p>\xe9</p>"[..],
                "text/html",
                "<p>é</p>",
            ),
            (
                vec![
                    "text/html",
                    "application/example+xml;charset=windows-1252",
                    "application/example+xml",
                ],
                &b"<r>\xe9</r>"[..],
                "application/example+xml",
                "<r>é</r>",
            ),
            (
                vec![
                    "application/octet-stream",
                    "text/plain;charset=windows-1252",
                ],
                &b"<meta charset=utf-8>\xe9"[..],
                "text/plain",
                "<meta charset=utf-8>é",
            ),
        ] {
            let response = NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: 1,
                status: 200,
                headers: values
                    .into_iter()
                    .map(|value| ("Content-Type".into(), value.into()))
                    .collect(),
                body: bytes.to_vec(),
            };
            let mime = oryn_common::mime_type::MimeType::from_headers(&response.headers)
                .unwrap()
                .serialize();
            assert_eq!(
                decode_navigation_text(&response.body, Some(&mime)).0,
                expected_text
            );
            let timing =
                resource_timing_metadata("https://mime.test", false, Vec::new(), &response, 1);
            assert_eq!(timing.content_type, expected_type);
            for split in 0..=bytes.len() {
                let mut decoder = oryn_net::IncrementalTextDecoder::new(
                    Some(&mime),
                    navigation_text_kind(Some(&mime)),
                );
                let mut text = decoder.push(&bytes[..split]);
                text.push_str(&decoder.push(&bytes[split..]));
                text.push_str(&decoder.finish());
                assert_eq!(text, expected_text, "split {split}, MIME {mime}");
            }
        }
    }

    #[test]
    fn buffered_and_streamed_documents_preserve_mime_encoding_rules() {
        let cases: [(&str, &[u8], &str); 3] = [
            ("text/plain", &[0xff, 0xfe, b'<', 0, 0xe9, 0], "<é"),
            (
                "application/example+xml",
                b"<?xml version='1.0' encoding='ISO-8859-1'?><r>\xe9</r>",
                "<?xml version='1.0' encoding='ISO-8859-1'?><r>é</r>",
            ),
            (
                "text/html",
                b"<meta charset=windows-1252><p>\x80</p>",
                "<meta charset=windows-1252><p>€</p>",
            ),
        ];
        for (mime, bytes, expected) in cases {
            let (buffered, encoding) = decode_navigation_text(bytes, Some(mime));
            assert_eq!(buffered, expected);
            for split in 0..=bytes.len() {
                let mut decoder = oryn_net::IncrementalTextDecoder::new(
                    Some(mime),
                    navigation_text_kind(Some(mime)),
                );
                let mut streamed = decoder.push(&bytes[..split]);
                streamed.push_str(&decoder.push(&[]));
                streamed.push_str(&decoder.push(&bytes[split..]));
                streamed.push_str(&decoder.finish());
                assert_eq!(streamed, expected, "{mime} split {split}");
                assert_eq!(decoder.encoding_name(), Some(encoding));
            }
        }
    }

    #[test]
    fn endpoint_routes_select_actual_ip_by_port_without_rewriting_origin() {
        let routes = EndpointRoutes::new([
            (
                "https".into(),
                "web-platform.test".into(),
                8443,
                "127.0.0.1".parse().unwrap(),
            ),
            (
                "https".into(),
                "web-platform.test".into(),
                8446,
                "10.0.0.2".parse().unwrap(),
            ),
        ])
        .unwrap();
        let policy = NetworkPolicy {
            allow_loopback: true,
            allow_private_networks: true,
            ..Default::default()
        };
        for (port, ip) in [(8443, "127.0.0.1"), (8446, "10.0.0.2")] {
            let original = format!("https://web-platform.test:{port}/resource");
            let url = Url::parse(&original).unwrap();
            assert_eq!(
                routes.resolve(&policy, &url).unwrap(),
                vec![format!("{ip}:{port}").parse::<SocketAddr>().unwrap()]
            );
            assert_eq!(url.as_str(), original);
            assert!(routes.resolve(&NetworkPolicy::default(), &url).is_err());
        }
        assert!(
            routes
                .resolve(
                    &policy,
                    &Url::parse("https://web-platform.test:9999/").unwrap()
                )
                .is_err()
        );
        assert!(
            routes
                .resolve(
                    &policy,
                    &Url::parse("http://web-platform.test:8443/").unwrap()
                )
                .is_err()
        );
    }

    #[test]
    fn configured_fixture_mapping_never_resolves_unlisted_hosts_or_uses_supplied_addresses() {
        let routes = EndpointRoutes::new([(
            "https".into(),
            "web-platform.test".into(),
            8443,
            "127.0.0.1".parse().unwrap(),
        )])
        .unwrap();
        let policy = NetworkPolicy {
            allow_loopback: true,
            allow_private_networks: true,
            ..Default::default()
        };
        let broker = PolicyBroker::new(policy, UnavailableBroker).with_endpoint_routes(routes);
        for url in [
            "https://nonexistent.web-platform.test:8443/",
            "https://localhost:8443/",
            "https://127.0.0.1:8443/",
            "https://web-platform.test:443/",
        ] {
            let mut request = request(url);
            request.resolved_addrs = vec!["127.0.0.1:8443".parse().unwrap()];
            assert!(
                matches!(broker.send(request),Err(NetworkError::Resolution{detail,..})
                if detail=="fixture endpoint has no configured scheme/port route")
            );
        }
    }

    #[test]
    fn endpoint_routes_reject_ambiguous_configuration_and_respect_host_denial() {
        let entry = (
            "https".to_string(),
            "web-platform.test".to_string(),
            8443,
            "127.0.0.1".parse::<IpAddr>().unwrap(),
        );
        assert!(EndpointRoutes::new([entry.clone(), entry.clone()]).is_err());
        for host in [
            "user@web-platform.test",
            "web-platform.test/path",
            "*.test",
            "127.0.0.1",
        ] {
            assert!(EndpointRoutes::new([("https".into(), host.into(), 8443, entry.3)]).is_err());
        }
        let routes = EndpointRoutes::new([entry]).unwrap();
        let policy = NetworkPolicy {
            allow_loopback: true,
            allow_private_networks: true,
            denied_hosts: ["web-platform.test".into()].into_iter().collect(),
            ..Default::default()
        };
        assert!(
            routes
                .resolve(
                    &policy,
                    &Url::parse("https://web-platform.test:8443/").unwrap()
                )
                .is_err()
        );
    }

    #[test]
    fn upload_measurements_survive_broker_wire_round_trip() {
        for upload in [
            None,
            Some(oryn_net::UploadProgress {
                bytes_consumed: 0,
                total_bytes: 0,
                body_complete: true,
            }),
            Some(oryn_net::UploadProgress {
                bytes_consumed: 65536,
                total_bytes: 100000,
                body_complete: false,
            }),
            Some(oryn_net::UploadProgress {
                bytes_consumed: 100000,
                total_bytes: 100000,
                body_complete: true,
            }),
        ] {
            let encoded = encode_network_event(NetworkEvent::Headers {
                response_policy: Some("parent-response-grant".into()),
                timing: Some(Box::new(oryn_net::TransportTiming {
                    measured_response: true,
                    dns_start: Some(2),
                    dns_end: Some(5),
                    connect_start: Some(5),
                    connect_end: Some(9),
                    secure_start: Some(9),
                    secure_end: Some(14),
                    request_start: 14,
                    response_start: 22,
                    next_hop_protocol: "h2".into(),
                    ..Default::default()
                })),
                request_id: 27,
                status: 200,
                headers: vec![
                    ("set-cookie".into(), "private=value".into()),
                    ("content-type".into(), "text/plain".into()),
                ],
                content_length: Some(2),
                upload,
            });
            let wire = serde_json::to_vec(&encoded).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&wire).unwrap();
            assert_eq!(json["kind"], "headers");
            assert_eq!(json["timing"]["dns_start"], 2);
            assert_eq!(json["timing"]["dns_end"], 5);
            assert_eq!(json["timing"]["next_hop_protocol"], "h2");
            let decoded = decode_network_event(serde_json::from_slice(&wire).unwrap()).unwrap();
            assert_eq!(
                decoded,
                NetworkEvent::Headers {
                    response_policy: Some("parent-response-grant".into()),
                    timing: Some(Box::new(oryn_net::TransportTiming {
                        measured_response: true,
                        dns_start: Some(2),
                        dns_end: Some(5),
                        connect_start: Some(5),
                        connect_end: Some(9),
                        secure_start: Some(9),
                        secure_end: Some(14),
                        request_start: 14,
                        response_start: 22,
                        next_hop_protocol: "h2".into(),
                        ..Default::default()
                    })),
                    request_id: 27,
                    status: 200,
                    headers: vec![("content-type".into(), "text/plain".into())],
                    content_length: Some(2),
                    upload,
                }
            );
        }
    }

    #[test]
    fn boxed_broker_preserves_owned_operation_activity() {
        struct Activity(std::time::Instant);
        impl NetworkBroker for Activity {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Err(NetworkError::Unavailable)
            }
            fn owned_operation_activity(&self) -> (usize, Option<std::time::Instant>) {
                (1, Some(self.0))
            }
        }
        let observed = std::time::Instant::now();
        let direct = Activity(observed);
        assert_eq!(direct.owned_operation_activity(), (1, Some(observed)));
        let boxed: Box<dyn NetworkBroker> = Box::new(direct);
        assert_eq!(boxed.owned_operation_activity(), (1, Some(observed)));
        let shared = SharedNetworkBroker::new(boxed);
        assert_eq!(
            shared.owned_operation_activity(),
            (0, None),
            "a fresh document identity owns no IO from its underlying transport"
        );
    }

    #[test]
    fn buffered_response_does_not_invent_upload_completion() {
        let mut operation = operation_from_response(NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: 41,
            status: 200,
            headers: Vec::new(),
            body: b"response".to_vec(),
        });
        assert!(matches!(
            operation.blocking_event(),
            Some(NetworkEvent::Headers {
                request_id: 41,
                upload: None,
                ..
            })
        ));
    }

    struct MalformedCompletionBroker {
        cancelled: Arc<AtomicUsize>,
        stale: bool,
    }

    impl NetworkBroker for MalformedCompletionBroker {
        fn send(&self, _request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!("bounded operation test uses start_operation")
        }

        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(2);
            if self.stale {
                sender
                    .blocking_send(NetworkEvent::Headers {
                        response_policy: None,
                        timing: None,
                        request_id: request.request_id + 1,
                        status: 200,
                        headers: Vec::new(),
                        content_length: Some(0),
                        upload: None,
                    })
                    .unwrap();
            } else {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(1));
                    drop(sender);
                });
            }
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
    fn bounded_collector_cancels_timeouts_and_stale_completions() {
        for stale in [false, true] {
            let cancelled = Arc::new(AtomicUsize::new(0));
            let broker = MalformedCompletionBroker {
                cancelled: cancelled.clone(),
                stale,
            };
            let error = collect_operation_bounded(
                &broker,
                request("https://example.test/bounded"),
                Duration::from_millis(10),
                1024,
            )
            .expect_err("malformed operation must fail closed");
            if stale {
                assert!(error.to_string().contains("stale or unmatched"));
            } else {
                assert!(matches!(error, NetworkError::TimedOut { .. }));
            }
            assert_eq!(cancelled.load(Ordering::SeqCst), 1);
        }
    }

    struct RecordingBroker {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
        body: Vec<u8>,
    }

    impl NetworkBroker for RecordingBroker {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/plain".into())],
                body: self.body.clone(),
            })
        }
    }

    fn request(url: &str) -> NetworkRequest {
        NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: true,
            security: CookieRequestContext::default(),
            resolved_addrs: Vec::new(),
        }
    }

    #[test]
    fn broker_enforces_policy_before_transport() {
        let broker = PolicyBroker::new(NetworkPolicy::default(), UnavailableBroker);
        assert!(matches!(
            broker.send(request("http://127.0.0.1/private")),
            Err(NetworkError::Policy(PolicyDenied::Loopback))
        ));
        assert!(matches!(
            broker.send(request("file:///etc/passwd")),
            Err(NetworkError::Policy(PolicyDenied::Scheme(_)))
        ));
        assert!(matches!(
            broker.send(request("http://localhost/private")),
            Err(NetworkError::Policy(PolicyDenied::Loopback))
        ));
        assert!(matches!(
            broker.send(request("https://1.1.1.1/")),
            Err(NetworkError::Unavailable)
        ));
    }

    #[test]
    fn native_fetch_metadata_reaches_transport_after_cookie_admission() {
        struct Inspect;
        impl NetworkBroker for Inspect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let values = |name: &str| {
                    request
                        .headers
                        .iter()
                        .filter(|(n, _)| n.eq_ignore_ascii_case(name))
                        .map(|(_, v)| v.as_str())
                        .collect::<Vec<_>>()
                };
                assert_eq!(values("sec-fetch-site"), vec!["cross-site"]);
                assert_eq!(values("sec-fetch-mode"), vec!["no-cors"]);
                assert_eq!(values("sec-fetch-dest"), vec!["audio"]);
                assert!(values("sec-fetch-user").is_empty());
                Ok(NetworkResponse {
                    response_policy: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                    timing: None,
                })
            }
        }
        let broker = CookieBroker::new(Inspect, ContextCookieStore::memory());
        let mut input = request("https://source.test/final");
        input.security.initiator_url = Some("https://source.test/".into());
        input.security.fetch_metadata =
            crate::fetch_metadata::Metadata::resource("no-cors", "audio", false);
        input.security.fetch_metadata.observe(
            Some("https://source.test/"),
            &Url::parse("https://other.test/redirect").unwrap(),
        );
        input.headers = vec![
            ("Sec-Fetch-Site".into(), "none".into()),
            ("Sec-Fetch-User".into(), "?1".into()),
        ];
        broker.send(input).unwrap();
    }

    #[test]
    fn cookie_broker_applies_same_site_before_transport_and_hides_set_cookie() {
        struct CookieFixture {
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
        }
        impl NetworkBroker for CookieFixture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: if request.request_id == 1 {
                        vec![
                            (
                                "set-cookie".into(),
                                "strict=1; SameSite=Strict; Secure; Path=/".into(),
                            ),
                            (
                                "set-cookie".into(),
                                "lax=1; SameSite=Lax; Secure; Path=/".into(),
                            ),
                            (
                                "set-cookie".into(),
                                "none=1; SameSite=None; Secure; Path=/".into(),
                            ),
                        ]
                    } else {
                        Vec::new()
                    },
                    body: Vec::new(),
                })
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        // Isolate SameSite filtering from the independently tested default
        // third-party blocking profile.
        let store = ContextCookieStore::memory();
        store.set_third_party_policy(crate::cookies::ThirdPartyCookiePolicy::Allow);
        let broker = CookieBroker::new(
            CookieFixture {
                requests: requests.clone(),
            },
            store,
        );
        let mut initial = request("https://auth.example.test/set");
        initial.security = CookieRequestContext::navigation(Some(initial.url.clone()));
        broker.send(initial).unwrap();

        let mut subresource = request("https://auth.example.test/data");
        subresource.request_id = 2;
        subresource.security = CookieRequestContext {
            dns_timing: None,
            kind: crate::cookies::CookieRequestKind::Fetch,
            initiator_url: Some("https://rp.other.test/".into()),
            top_level_url: Some("https://rp.other.test/".into()),
            ..Default::default()
        };
        broker.send(subresource).unwrap();

        let captured = requests.lock().unwrap();
        let cookie = captured[1]
            .headers
            .iter()
            .find(|(name, _)| name == "cookie")
            .map(|(_, value)| value.as_str());
        assert_eq!(cookie, Some("none=1"));
        let visible = sanitize_parent_response_headers(vec![
            ("content-type".into(), "text/plain".into()),
            ("set-cookie".into(), "secret=1".into()),
        ]);
        assert_eq!(visible, vec![("content-type".into(), "text/plain".into())]);
    }

    #[test]
    fn named_loopback_policy_allows_local_resolution() {
        let policy = NetworkPolicy {
            allow_loopback: true,
            ..NetworkPolicy::default()
        };
        let broker = PolicyBroker::new(policy, UnavailableBroker);
        assert!(matches!(
            broker.send(request("http://localhost:3000/")),
            Err(NetworkError::Unavailable)
        ));
    }

    #[test]
    fn localhost_subdomains_are_pinned_without_relaxing_policy() {
        for host in [
            "localhost",
            "www1.localhost",
            "www2.alt.localhost",
            "LOCALHOST.",
        ] {
            let url = Url::parse(&format!("http://{host}:8123/path")).unwrap();
            assert!(matches!(
                resolve_and_authorize(&NetworkPolicy::default(), &url),
                Err(NetworkError::Policy(PolicyDenied::Loopback))
            ));
            let policy = NetworkPolicy {
                allow_loopback: true,
                ..NetworkPolicy::default()
            };
            assert_eq!(
                resolve_and_authorize(&policy, &url).unwrap(),
                vec![SocketAddr::from(([127, 0, 0, 1], 8123))]
            );
            assert_eq!(url.host_str().unwrap(), host.to_ascii_lowercase());
        }
    }

    #[test]
    fn policy_broker_pins_every_vetted_resolution() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let inner = RecordingBroker {
            requests: requests.clone(),
            body: Vec::new(),
        };
        let broker = PolicyBroker::new(
            NetworkPolicy {
                allow_loopback: true,
                ..NetworkPolicy::default()
            },
            inner,
        );
        broker
            .send(request("http://localhost:34781/"))
            .expect("loopback test policy permits the vetted address");
        let recorded = requests.lock().unwrap();
        assert!(!recorded[0].resolved_addrs.is_empty());
        assert!(
            recorded[0]
                .resolved_addrs
                .iter()
                .all(|address| address.ip().is_loopback())
        );
    }

    #[test]
    fn worker_broker_is_typed_correlated_and_header_filtered() {
        let (worker, parent) = UnixStream::pair().expect("socket pair");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: captured,
                    body: b"brokered".to_vec(),
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let broker = IpcNetworkBroker::new(worker);
        let mut network_request = request("https://example.test/");
        // Fetch's credentials mode controls ambient credentials, not explicit
        // author headers. Pinned WPT authentication-basic.any.js covers this
        // Authorization distinction for include, same-origin and omit.
        let author_headers = vec![
            ("Accept".into(), "text/plain".into()),
            ("Authorization".into(), "secret".into()),
            ("X-Worker-Secret".into(), "secret".into()),
        ];
        network_request.headers = author_headers.clone();
        network_request.headers.extend([
            ("Cookie".into(), "session=forged".into()),
            ("Origin".into(), "https://forged.test".into()),
            ("Referer".into(), "https://forged.test/path".into()),
            ("Sec-Fetch-Site".into(), "same-origin".into()),
            ("Proxy-Authorization".into(), "forged".into()),
            ("Content-Length".into(), "999".into()),
            ("Invalid Name".into(), "invalid".into()),
            ("X-Injection".into(), "value\r\nCookie: forged".into()),
        ]);
        for (index, credentials) in [true, false].into_iter().enumerate() {
            network_request.request_id = 1 + index as u64;
            network_request.credentials = credentials;
            let response = broker
                .send(network_request.clone())
                .expect("broker response");
            assert_eq!(response.request_id, network_request.request_id);
            assert_eq!(response.body, b"brokered");
        }
        drop(broker);
        server.join().unwrap().expect("broker server");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for (request, credentials) in requests.iter().zip([true, false]) {
            assert_eq!(request.headers, author_headers);
            assert_eq!(request.credentials, credentials);
        }
    }

    #[test]
    fn concurrent_network_frames_keep_monotonic_wire_identifiers() {
        let (worker, parent) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(parent);
            for expected in 1..=64 {
                let mut line = Vec::new();
                reader.read_until(b'\n', &mut line).unwrap();
                let frame: BrokerRequestEnvelope = serde_json::from_slice(&line).unwrap();
                assert_eq!(frame.message_id, expected);
            }
        });
        let broker = Arc::new(IpcNetworkBroker::new(worker));
        let barrier = Arc::new(std::sync::Barrier::new(32));
        let callers = (1..=32)
            .map(|id| {
                let broker = broker.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut request = request("https://example.test/");
                    request.request_id = id;
                    let _operation = broker.start_operation(request).unwrap();
                    broker.cancel_operation(id).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for caller in callers {
            caller.join().unwrap();
        }
        server.join().unwrap();
    }

    #[test]
    fn network_routes_scope_cancellation_and_validate_response_authority() {
        for wrong_reply in [false, true] {
            let (worker, parent) = UnixStream::pair().unwrap();
            parent
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let server = std::thread::spawn(move || {
                let mut reader = BufReader::new(parent.try_clone().unwrap());
                let mut frames = Vec::new();
                for _ in 0..3 {
                    let mut line = Vec::new();
                    reader.read_until(b'\n', &mut line).unwrap();
                    frames.push(serde_json::from_slice::<BrokerRequestEnvelope>(&line).unwrap());
                }
                assert_ne!(frames[0].route, frames[1].route);
                assert_eq!(frames[0].route, frames[2].route);
                let native_ids = frames[..2]
                    .iter()
                    .map(|frame| match &frame.command {
                        BrokerCommand::Start { request } => request.request_id,
                        other => panic!("expected start, got {other:?}"),
                    })
                    .collect::<Vec<_>>();
                assert_ne!(
                    native_ids[0], native_ids[1],
                    "route-local IDs need distinct native cancellation authority"
                );
                assert!(
                    matches!(frames[2].command,BrokerCommand::Cancel{request_id} if request_id==native_ids[0])
                );
                let response = BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    route: frames[usize::from(!wrong_reply)].route.clone(),
                    correlation_id: frames[1].message_id,
                    response: None,
                    event: Some(EncodedNetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: native_ids[1],
                    }),
                    error: None,
                };
                write_broker_response(&Arc::new(Mutex::new(parent)), &response).unwrap();
            });
            let broker = SharedNetworkBroker::new(IpcNetworkBroker::new(worker));
            let first = broker
                .clone()
                .for_route(WorkerRoute::issue(1, 1, 0).unwrap());
            let second = broker.for_route(WorkerRoute::issue(1, 2, 0).unwrap());
            let mut req = request("https://example.test/");
            req.request_id = 5;
            let mut old = first.start_operation(req.clone()).unwrap();
            let mut live = second.start_operation(req).unwrap();
            first.cancel_operation(5).unwrap();
            assert!(old.blocking_event().is_none());
            let event = live.blocking_event().unwrap();
            if wrong_reply {
                assert!(matches!(event, NetworkEvent::Failed { .. }));
            } else {
                assert!(matches!(
                    event,
                    NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: 5
                    }
                ));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn exhausted_network_identifiers_never_wrap_or_publish_frames() {
        let (worker, mut parent) = UnixStream::pair().unwrap();
        parent.set_nonblocking(true).unwrap();
        let broker = IpcNetworkBroker::new(worker);
        broker
            .next_message_id
            .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
        assert!(
            broker
                .start_operation(request("https://example.test/"))
                .is_err()
        );
        assert!(broker.cancel_operation(1).is_err());
        assert_eq!(
            broker
                .next_message_id
                .load(std::sync::atomic::Ordering::Relaxed),
            u64::MAX
        );
        assert!(broker.pending.lock().unwrap().is_empty());
        assert!(broker.active_requests.lock().unwrap().is_empty());
        let mut byte = [0];
        assert_eq!(
            parent.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn worker_broker_forwards_explicit_operation_cancellation() {
        struct CancelRecording {
            cancelled: Arc<Mutex<Vec<u64>>>,
        }
        impl NetworkBroker for CancelRecording {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                })
            }
            fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
                self.cancelled.lock().unwrap().push(request_id);
                Ok(())
            }
        }

        let (worker, parent) = UnixStream::pair().unwrap();
        let cancelled = Arc::new(Mutex::new(Vec::new()));
        let captured = cancelled.clone();
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                CancelRecording {
                    cancelled: captured,
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let broker = IpcNetworkBroker::new(worker);
        broker.cancel_operation(77).unwrap();
        drop(broker);
        server.join().unwrap().unwrap();
        assert_eq!(*cancelled.lock().unwrap(), vec![77]);
    }

    #[test]
    fn worker_broker_multiplexes_independent_streams() {
        let (worker, parent) = UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: Arc::new(Mutex::new(Vec::new())),
                    body: b"chunked".to_vec(),
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let broker = IpcNetworkBroker::new(worker);
        let mut first = request("https://example.test/one");
        first.request_id = 41;
        let mut second = request("https://example.test/two");
        second.request_id = 42;
        let mut first = broker.start_operation(first).expect("first operation");
        let mut second = broker.start_operation(second).expect("second operation");

        fn collect(operation: &mut NetworkOperation) -> (u16, Vec<u8>) {
            let mut status = 0;
            let mut body = Vec::new();
            while let Some(event) = operation.blocking_event() {
                match event {
                    NetworkEvent::Headers { status: value, .. } => status = value,
                    NetworkEvent::Chunk { bytes, .. } => body.extend_from_slice(&bytes),
                    NetworkEvent::Complete { .. } => break,
                    NetworkEvent::Failed { error, .. } => panic!("operation failed: {error}"),
                }
            }
            (status, body)
        }

        assert_eq!(collect(&mut second), (200, b"chunked".to_vec()));
        assert_eq!(collect(&mut first), (200, b"chunked".to_vec()));
        drop(broker);
        server.join().unwrap().expect("broker server");
    }

    #[test]
    fn parent_cookie_headers_are_not_exposed_to_the_worker() {
        assert_eq!(
            sanitize_parent_response_headers(vec![
                ("Content-Type".into(), "text/plain".into()),
                ("Set-Cookie".into(), "session=secret".into()),
                ("Proxy-Authenticate".into(), "secret".into()),
            ]),
            vec![("Content-Type".into(), "text/plain".into())]
        );
    }

    struct CacheFixture {
        requests: Arc<Mutex<usize>>,
    }

    impl NetworkBroker for CacheFixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            *self.requests.lock().unwrap() += 1;
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("cache-control".into(), "public, max-age=60".into()),
                    ("vary".into(), "accept-language".into()),
                ],
                body: b"cached".to_vec(),
            })
        }
    }

    #[derive(Clone)]
    struct ValidatorFixture {
        seen: Arc<Mutex<Vec<NetworkRequest>>>,
    }
    impl NetworkBroker for ValidatorFixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.seen.lock().unwrap().push(request.clone());
            let conditional =
                response_header(&request.headers, "if-none-match") == Some("\"native\"");
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: if conditional { 304 } else { 200 },
                headers: vec![
                    (
                        "cache-control".into(),
                        if conditional {
                            "max-age=60"
                        } else {
                            "max-age=0"
                        }
                        .into(),
                    ),
                    ("etag".into(), "\"native\"".into()),
                    ("x-revalidated".into(), conditional.to_string()),
                ],
                body: if conditional {
                    Vec::new()
                } else {
                    b"representation".to_vec()
                },
            })
        }
    }
    #[test]
    fn conditional_cache_merge_preserves_repeated_policy_headers_and_cookie_custody() {
        let entry = CacheEntry {
            encoded_body_size: None,
            status: 200,
            headers: vec![
                ("set-cookie".into(), "old=1".into()),
                ("content-security-policy".into(), "old".into()),
                ("content-length".into(), "4".into()),
            ],
            body: b"body".to_vec(),
            stored_at_ms: 0,
            freshness_lifetime_ms: 0,
            vary: Vec::new(),
        };
        let response = revalidated_response(
            8,
            entry,
            &[
                ("content-security-policy".into(), "script-src 'none'".into()),
                ("content-security-policy".into(), "img-src 'none'".into()),
                ("set-cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
                ("content-length".into(), "0".into()),
            ],
        );
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|(n, _)| n == "content-security-policy")
                .count(),
            2
        );
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|(n, _)| n == "set-cookie")
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>(),
            ["a=1", "b=2"]
        );
        let stored = cache_representation_headers(&response.headers);
        assert!(!stored.iter().any(|(n, _)| n == "set-cookie"));
        assert_eq!(response_header(&stored, "content-length"), Some("4"));
        let vary = cache_vary(
            &[
                ("vary".into(), "Origin".into()),
                ("Vary".into(), "Accept-Language".into()),
            ],
            &[
                ("origin".into(), "https://a.test".into()),
                ("accept-language".into(), "en".into()),
                ("Accept-Language".into(), "fr".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            vary,
            vec![
                ("accept-language".into(), "en, fr".into()),
                ("origin".into(), "https://a.test".into())
            ]
        );
        assert_eq!(
            cache_freshness_ms(&[
                ("cache-control".into(), "max-age=600".into()),
                ("Cache-Control".into(), "no-store".into())
            ]),
            None
        );
    }
    #[test]
    fn native_fetch_cache_modes_revalidate_and_replay_body_with_backpressure() {
        use oryn_js::platform::FetchCacheMode as Mode;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let broker = MemoryCachingBroker::new(
            ValidatorFixture { seen: seen.clone() },
            "native-cache-modes",
            8,
            8192,
            1024,
        );
        let mut input = request("https://example.test/cache-mode");
        assert_eq!(broker.send(input.clone()).unwrap().body, b"representation");
        input.security.fetch_cache = Mode::ForceCache;
        let cached = broker.send(input.clone()).unwrap();
        assert_eq!(cached.body, b"representation");
        assert_eq!(
            cached.timing.unwrap().delivery,
            oryn_common::transport_timing::ResponseDelivery::LocalCache
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        input.security.fetch_cache = Mode::NoCache;
        let response =
            collect_operation_bounded(&broker, input.clone(), Duration::from_secs(2), 1024)
                .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"representation");
        assert_eq!(
            response_header(&response.headers, "x-revalidated"),
            Some("true")
        );
        assert_eq!(
            response.timing.as_ref().unwrap().delivery,
            oryn_common::transport_timing::ResponseDelivery::ValidatedCache
        );
        assert_eq!(
            response_header(&seen.lock().unwrap()[1].headers, "cache-control"),
            Some("max-age=0")
        );
        input.security.fetch_cache = Mode::OnlyIfCached;
        assert_eq!(broker.send(input.clone()).unwrap().body, b"representation");
        assert_eq!(seen.lock().unwrap().len(), 2);
        input.url.push_str("-miss");
        assert!(broker.start_operation(input.clone()).is_err());
        assert_eq!(seen.lock().unwrap().len(), 2);
        input.security.fetch_cache = Mode::NoStore;
        broker.send(input.clone()).unwrap();
        input.security.fetch_cache = Mode::OnlyIfCached;
        assert!(broker.send(input).is_err());
    }
    #[test]
    fn persistent_native_cache_revalidation_preserves_representation() {
        use oryn_js::platform::FetchCacheMode as Mode;
        let directory = std::env::temp_dir().join(format!(
            "oryn-native-cache-modes-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let broker = PersistentCachingBroker::open(
                ValidatorFixture { seen: seen.clone() },
                &directory,
                "native-modes",
                8,
                8192,
                1024,
            )
            .unwrap();
            let mut input = request("https://example.test/persistent-mode");
            broker.send(input.clone()).unwrap();
            input.security.fetch_cache = Mode::NoCache;
            let response =
                collect_operation_bounded(&broker, input.clone(), Duration::from_secs(2), 1024)
                    .unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.body, b"representation");
            input.security.fetch_cache = Mode::OnlyIfCached;
            assert_eq!(broker.send(input).unwrap().body, b"representation");
            assert_eq!(seen.lock().unwrap().len(), 2);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn request_cache_control_revalidates_without_disabling_response_storage() {
        let requests = Arc::new(Mutex::new(0));
        let broker = MemoryCachingBroker::new(
            CacheFixture {
                requests: requests.clone(),
            },
            "cache-request-control",
            8,
            1024,
            512,
        );
        let plain = request("https://example.test/data");
        broker.send(plain.clone()).unwrap();
        let mut refresh = plain.clone();
        refresh
            .headers
            .push(("cache-control".into(), "no-cache".into()));
        broker.send(refresh).unwrap();
        assert_eq!(*requests.lock().unwrap(), 2);
        broker.send(plain).unwrap();
        assert_eq!(
            *requests.lock().unwrap(),
            2,
            "revalidation response may replace cached bytes"
        );
        let mut uncached = request("https://example.test/private");
        uncached
            .headers
            .push(("cache-control".into(), "no-store".into()));
        broker.send(uncached).unwrap();
        broker
            .send(request("https://example.test/private"))
            .unwrap();
        assert_eq!(*requests.lock().unwrap(), 4);
    }

    #[test]
    fn context_cache_respects_freshness_and_vary() {
        let requests = Arc::new(Mutex::new(0));
        let broker = MemoryCachingBroker::new(
            CacheFixture {
                requests: requests.clone(),
            },
            "context-7",
            8,
            1024,
            512,
        );
        let mut first = request("https://example.test/data");
        first.headers.push(("accept-language".into(), "en".into()));
        assert_eq!(broker.send(first.clone()).unwrap().body, b"cached");
        first.request_id = 2;
        assert_eq!(broker.send(first.clone()).unwrap().request_id, 2);
        assert_eq!(*requests.lock().unwrap(), 1);

        first.request_id = 3;
        first.headers[0].1 = "fr".into();
        broker.send(first).unwrap();
        assert_eq!(*requests.lock().unwrap(), 2);
    }

    #[test]
    fn production_http_transport_exchanges_http2_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let server_seen = seen.clone();
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut connection = h2::server::handshake(stream).await.unwrap();
                    let (request, mut respond) = connection.accept().await.unwrap().unwrap();
                    server_seen
                        .lock()
                        .unwrap()
                        .push(request.uri().path().to_string());
                    let response = http::Response::builder()
                        .status(200)
                        .header("content-type", "text/plain")
                        .body(())
                        .unwrap();
                    let mut body = respond.send_response(response, false).unwrap();
                    body.send_data(bytes::Bytes::from_static(b"h2"), true)
                        .unwrap();
                    let _ =
                        tokio::time::timeout(Duration::from_millis(100), connection.accept()).await;
                });
        });

        let transport = RustlsTransport::new(Duration::from_secs(5), 1024)
            .unwrap()
            .with_test_http2_prior_knowledge();
        let url = format!("http://localhost:{}/", address.port());
        let mut first = request(&format!("{url}one"));
        first.resolved_addrs = vec![address];
        let mut first_response = transport.start_stream(first).unwrap();
        let mut body = String::new();
        first_response.body.read_to_string(&mut body).unwrap();
        assert_eq!(body, "h2");
        server.join().unwrap();
        assert_eq!(&*seen.lock().unwrap(), &["/one"]);
    }

    #[test]
    fn browser_content_encodings_decode_incrementally() {
        let input = b"Oryn streamed response body".repeat(256);

        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&input).unwrap();
        let gzip = gzip.finish().unwrap();

        let mut deflate =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(&input).unwrap();
        let deflate = deflate.finish().unwrap();

        let mut brotli = Vec::new();
        {
            let mut encoder = brotli::CompressorWriter::new(&mut brotli, 4096, 5, 22);
            encoder.write_all(&input).unwrap();
        }

        let zstd = zstd::stream::encode_all(Cursor::new(&input), 3).unwrap();

        for (encoding, compressed) in [
            ("gzip", gzip),
            ("deflate", deflate),
            ("br", brotli),
            ("zstd", zstd),
        ] {
            let mut decoder =
                decoded_body_reader(Box::new(Cursor::new(compressed)), Some(encoding), 64 * 1024)
                    .unwrap();
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded).unwrap();
            assert_eq!(decoded, input, "{encoding}");
        }
    }

    #[test]
    fn compressed_and_decoded_response_limits_are_independent() {
        let compressed = vec![0; 33];
        let mut reader =
            decoded_body_reader(Box::new(Cursor::new(compressed)), Some("identity"), 32).unwrap();
        let mut decoded = Vec::new();
        assert!(reader.read_to_end(&mut decoded).is_err());

        assert!(matches!(
            decoded_body_reader(Box::new(Cursor::new(Vec::new())), Some("compress"), 32),
            Err(NetworkError::UnsupportedContentEncoding(encoding)) if encoding == "compress"
        ));
    }

    #[test]
    fn persistent_context_cache_survives_broker_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "oryn-http-cache-test-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let requests = Arc::new(Mutex::new(0));
        {
            let broker = PersistentCachingBroker::open(
                CacheFixture {
                    requests: requests.clone(),
                },
                &directory,
                "profile",
                8,
                1024,
                512,
            )
            .unwrap();
            broker
                .send(request("https://example.test/persist"))
                .unwrap();
        }
        {
            let broker = PersistentCachingBroker::open(
                CacheFixture {
                    requests: requests.clone(),
                },
                &directory,
                "profile",
                8,
                1024,
                512,
            )
            .unwrap();
            let mut replayed = request("https://example.test/persist");
            replayed.request_id = 99;
            assert_eq!(broker.send(replayed).unwrap().request_id, 99);
        }
        assert_eq!(*requests.lock().unwrap(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn streaming_cache_commits_after_the_declared_body_is_consumed() {
        let requests = Arc::new(Mutex::new(0));
        let broker = MemoryCachingBroker::new(
            CacheFixture {
                requests: requests.clone(),
            },
            "context-9",
            8,
            1024,
            512,
        );
        let request = request("https://example.test/stream-cache");
        let mut response = broker.start_stream(request.clone()).unwrap();
        let mut body = Vec::new();
        response.body.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"cached");
        assert_eq!(broker.send(request).unwrap().body, b"cached");
        assert_eq!(*requests.lock().unwrap(), 1);
    }

    #[test]
    fn parent_broker_rejects_duplicate_message_ids() {
        let (mut worker, parent) = UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: Arc::new(Mutex::new(Vec::new())),
                    body: b"ok".to_vec(),
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let reader_stream = worker.try_clone().expect("clone worker socket");
        let mut reader = BufReader::new(reader_stream);
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            message_id: 1,
            route: None,
            command: BrokerCommand::Start {
                request: Box::new(EncodedNetworkRequest {
                    request_id: 7,
                    method: "GET".into(),
                    url: "https://example.test/".into(),
                    headers: Vec::new(),
                    body_base64: String::new(),
                    credentials: true,
                    security: CookieRequestContext::default(),
                }),
            },
        };
        for attempt in 0..2 {
            serde_json::to_writer(&mut worker, &envelope).expect("write request");
            worker.write_all(b"\n").expect("finish frame");
            worker.flush().expect("flush request");
            if attempt == 0 {
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("read response event");
                    let response: BrokerResponseEnvelope =
                        serde_json::from_str(&line).expect("valid response event");
                    assert!(response.error.is_none());
                    if matches!(response.event, Some(EncodedNetworkEvent::Complete { .. })) {
                        break;
                    }
                }
            } else {
                let mut line = String::new();
                reader.read_line(&mut line).expect("read rejection");
                let response: BrokerResponseEnvelope =
                    serde_json::from_str(&line).expect("valid rejection");
                assert_eq!(
                    response.error.as_deref(),
                    Some("duplicate or out-of-order broker message")
                );
            }
        }
        drop(reader);
        drop(worker);
        server.join().unwrap().expect("broker server");
    }

    #[test]
    fn worker_rejects_decoded_response_over_sixteen_mib() {
        let (worker, parent) = UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: Arc::new(Mutex::new(Vec::new())),
                    body: vec![0; MAX_DECODED_RESPONSE_BYTES + 1],
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let broker = IpcNetworkBroker::new(worker);
        assert!(matches!(
            broker.send(request("https://example.test/")),
            Err(NetworkError::ResponseTooLarge {
                limit: MAX_DECODED_RESPONSE_BYTES
            })
        ));
        drop(broker);
        server.join().unwrap().expect("broker server");
    }

    #[test]
    fn worker_cannot_supply_parent_owned_request_headers() {
        let headers = sanitize_worker_headers(vec![
            ("Origin".into(), "https://forged.example".into()),
            ("Referer".into(), "https://forged.example/path".into()),
            ("Cookie".into(), "session=forged".into()),
            ("Authorization".into(), "Bearer author".into()),
            ("Accept".into(), "application/json".into()),
        ]);
        assert_eq!(
            headers,
            vec![
                ("Authorization".into(), "Bearer author".into()),
                ("Accept".into(), "application/json".into())
            ]
        );
    }

    #[test]
    fn data_urls_preserve_bytes_and_media_type() {
        let url = Url::parse("data:text/css;charset=utf-8,a%20%7Bcolor%3Ared%7D")
            .expect("valid data URL");
        let (media_type, body) = decode_data_url(&url).expect("decoded data URL");
        assert_eq!(media_type.as_deref(), Some("text/css;charset=utf-8"));
        assert_eq!(body, b"a {color:red}");

        let url =
            Url::parse("data:application/octet-stream;base64,AP8H").expect("valid base64 data URL");
        let (media_type, body) = decode_data_url(&url).expect("decoded base64 data URL");
        assert_eq!(media_type.as_deref(), Some("application/octet-stream"));
        assert_eq!(body, [0, 255, 7]);
    }

    #[test]
    fn data_url_processing_preserves_mime_and_uses_forgiving_decoding() {
        for (url, mime, bytes) in [
            (
                "data:,a%2Gb%#excluded",
                "text/plain;charset=US-ASCII",
                b"a%2Gb%".as_slice(),
            ),
            (
                "data:;charset=windows-1252,%80",
                "text/plain;charset=windows-1252",
                &[128],
            ),
            ("data:text/plain; BASE64,YQ", "text/plain", b"a"),
            ("data:text/plain;base64,YR==", "text/plain", b"a"),
            ("data:text/plain;base64,Y%20Q%0A==", "text/plain", b"a"),
            ("data:text/plain;base64;x=y,YQ==", "text/plain;x=y", b"YQ=="),
            ("data:invalid,a", "text/plain;charset=US-ASCII", b"a"),
        ] {
            let result = decode_data_url(&Url::parse(url).unwrap()).unwrap();
            assert_eq!(result.0.as_deref(), Some(mime), "{url}");
            assert_eq!(result.1, bytes, "{url}");
        }
        for payload in ["Y", "Y===", "YQ===", "YQ-_"] {
            assert!(
                decode_data_url(&Url::parse(&format!("data:;base64,{payload}")).unwrap()).is_err()
            );
        }
    }

    #[test]
    fn malformed_data_urls_are_rejected() {
        let malformed = Url::parse("data:text/plain,%Q0").expect("URL parser accepts payload");
        assert_eq!(decode_data_url(&malformed).unwrap().1, b"%Q0");
        let malformed =
            Url::parse("data:text/plain;base64,not-base64").expect("URL parser accepts payload");
        assert!(matches!(
            decode_data_url(&malformed),
            Err(NetworkError::Transport(message)) if message.starts_with("invalid data URL base64:")
        ));
    }
}

#[cfg(test)]
mod integrity_body_source_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn integrity_buffering_replays_the_original_terminal_encoded_count() {
        for encoded in [None, Some(0), Some(17)] {
            let (tx, rx) = tokio::sync::mpsc::channel(2);
            tx.try_send(NetworkEvent::Chunk {
                request_id: 7,
                bytes: bytes::Bytes::from_static(b"hello"),
            })
            .unwrap();
            tx.try_send(NetworkEvent::Complete {
                request_id: 7,
                encoded_body_size: encoded,
            })
            .unwrap();
            drop(tx);
            let mut source = NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: 7,
                events: rx,
                demand: None,
            };
            let collected = collect_started_body_with_metadata(
                &mut source,
                std::time::Instant::now() + Duration::from_secs(1),
                5,
                &AtomicBool::new(false),
                None,
            )
            .unwrap();
            crate::integrity::verify(
                "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=",
                &collected.bytes,
                true,
            )
            .unwrap();
            let mut replay = buffered_body_operation(7, collected).unwrap();
            assert!(
                matches!(replay.try_event().unwrap(), NetworkEvent::Chunk { request_id: 7, bytes } if bytes.as_ref() == b"hello")
            );
            assert!(
                matches!(replay.try_event().unwrap(), NetworkEvent::Complete { request_id: 7, encoded_body_size } if encoded_body_size == encoded)
            );
            assert!(
                replay.try_event().is_err(),
                "one terminal metadata publication only"
            );
        }
    }

    #[test]
    fn verification_body_rejects_truncation_wrong_identity_limits_and_cancellation() {
        let collect = |events: Vec<NetworkEvent>, limit, cancelled| {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            for event in events {
                tx.try_send(event).unwrap();
            }
            drop(tx);
            let mut operation = NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: 7,
                events: rx,
                demand: None,
            };
            collect_started_body(
                &mut operation,
                std::time::Instant::now() + Duration::from_secs(1),
                limit,
                &AtomicBool::new(cancelled),
            )
        };
        assert_eq!(
            collect(
                vec![
                    NetworkEvent::Chunk {
                        request_id: 7,
                        bytes: bytes::Bytes::from_static(b"hello")
                    },
                    NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: 7
                    }
                ],
                5,
                false
            )
            .unwrap(),
            b"hello"
        );
        assert!(
            collect(
                vec![NetworkEvent::Chunk {
                    request_id: 7,
                    bytes: bytes::Bytes::from_static(b"hello")
                }],
                5,
                false
            )
            .is_err()
        );
        assert!(
            collect(
                vec![NetworkEvent::Complete {
                    encoded_body_size: None,
                    request_id: 8
                }],
                5,
                false
            )
            .is_err()
        );
        assert!(matches!(
            collect(
                vec![NetworkEvent::Chunk {
                    request_id: 7,
                    bytes: bytes::Bytes::from_static(b"hello")
                }],
                4,
                false
            ),
            Err(NetworkError::ResponseTooLarge { limit: 4 })
        ));
        assert!(matches!(
            collect(
                vec![NetworkEvent::Complete {
                    encoded_body_size: None,
                    request_id: 7
                }],
                5,
                true
            ),
            Err(NetworkError::Cancelled)
        ));
    }
}

#[cfg(test)]
mod transport_cache_header_tests {
    use super::*;
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<NetworkRequest>>>);
    impl NetworkBroker for Capture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let id = request.request_id;
            self.0.lock().unwrap().push(request);
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: id,
                status: 304,
                headers: Vec::new(),
                body: Vec::new(),
            })
        }
    }
    #[test]
    fn uncached_authored_condition_is_no_store_and_cache_only_never_dispatches() {
        use oryn_js::platform::FetchCacheMode as Mode;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let broker = UncachedBroker(Capture(seen.clone()));
        let mut request = NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: "https://cache.test/".into(),
            headers: vec![("if-none-match".into(), "authored".into())],
            body: Vec::new(),
            credentials: false,
            security: CookieRequestContext::default(),
            resolved_addrs: Vec::new(),
        };
        assert_eq!(broker.send(request.clone()).unwrap().status, 304);
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen[0].security.fetch_cache, Mode::NoStore);
            assert_eq!(
                response_header(&seen[0].headers, "cache-control"),
                Some("no-cache")
            );
        }
        request.security.fetch_cache = Mode::OnlyIfCached;
        assert!(broker.send(request.clone()).is_err());
        assert!(broker.start_stream(request.clone()).is_err());
        assert!(broker.start_operation(request).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn explicit_modes_are_idempotent_and_preserve_native_validators() {
        use oryn_js::platform::FetchCacheMode as Mode;
        for (mode, control) in [
            (Mode::Reload, "no-cache"),
            (Mode::NoStore, "no-cache"),
            (Mode::NoCache, "max-age=0"),
        ] {
            let mut request = NetworkRequest {
                request_id: 1,
                method: "GET".into(),
                url: "https://cache.test/".into(),
                headers: Vec::new(),
                body: Vec::new(),
                credentials: false,
                security: CookieRequestContext {
                    dns_timing: None,
                    fetch_cache: mode,
                    ..Default::default()
                },
                resolved_addrs: Vec::new(),
            };
            apply_fetch_cache_headers(&mut request);
            apply_fetch_cache_headers(&mut request);
            assert_eq!(
                request
                    .headers
                    .iter()
                    .filter(|(name, _)| name == "cache-control")
                    .count(),
                1
            );
            assert_eq!(
                response_header(&request.headers, "cache-control"),
                Some(control)
            );
            request.security.fetch_cache = Mode::Default;
            request.headers = vec![("if-none-match".into(), "native-validator".into())];
            apply_fetch_cache_headers(&mut request);
            assert_eq!(request.security.fetch_cache, Mode::Default);
            assert_eq!(request.headers.len(), 1);
        }
    }
}

#[cfg(test)]
mod stale_revalidation_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Gated {
        validator: bool,
        calls: Arc<AtomicUsize>,
        started: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl NetworkBroker for Gated {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let count = self.calls.fetch_add(1, Ordering::SeqCst);
            if count > 0 {
                assert_eq!(
                    request
                        .headers
                        .iter()
                        .any(|(name, value)| name == "if-none-match" && value == "first"),
                    self.validator
                );
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: {
                    let mut headers = vec![(
                        "cache-control".into(),
                        if count == 0 {
                            "private, max-age=0, stale-while-revalidate=600"
                        } else {
                            "max-age=600"
                        }
                        .into(),
                    )];
                    if self.validator {
                        headers.push(("etag".into(), "first".into()));
                    }
                    headers
                },
                body: if count == 0 {
                    b"first".to_vec()
                } else {
                    b"updated".to_vec()
                },
            })
        }
    }
    fn request(id: u64) -> NetworkRequest {
        NetworkRequest {
            request_id: id,
            method: "GET".into(),
            url: "https://swr.test/script.js".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: false,
            security: Default::default(),
            resolved_addrs: Vec::new(),
        }
    }
    #[test]
    fn stale_memory_and_persistent_revalidation_retains_only_initiating_document() {
        for (persistent, validator) in [(false, false), (false, true), (true, false), (true, true)]
        {
            let (started, waiting) = std::sync::mpsc::channel();
            let (release, gate) = std::sync::mpsc::channel();
            let calls = Arc::new(AtomicUsize::new(0));
            let inner = Gated {
                validator,
                calls: calls.clone(),
                started,
                release: Mutex::new(gate),
            };
            let directory = std::env::temp_dir().join(format!(
                "oryn-swr-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let cache: Box<dyn NetworkBroker> = if persistent {
                std::fs::create_dir_all(&directory).unwrap();
                Box::new(
                    PersistentCachingBroker::open(inner, &directory, "swr", 16, 65536, 4096)
                        .unwrap(),
                )
            } else {
                Box::new(MemoryCachingBroker::new(inner, "swr", 16, 65536, 4096))
            };
            let shared = SharedNetworkBroker::new(cache);
            let first = shared.clone().new_request_scope();
            let other = shared.clone().new_request_scope();
            assert_eq!(first.send(request(1)).unwrap().body, b"first");
            assert_eq!(first.send(request(2)).unwrap().body, b"first");
            waiting.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(
                first.owned_operation_activity().0 > 0,
                "foreground completion must retain background owner"
            );
            assert_eq!(other.owned_operation_activity().0, 0);
            assert_eq!(other.send(request(3)).unwrap().body, b"first");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "matching concurrent stale load shares one refresh"
            );
            assert_eq!(
                other.owned_operation_activity().0,
                0,
                "shared cache counts cannot make another document busy"
            );
            release.send(()).unwrap();
            first
                .drain_owned_operations(Duration::from_secs(2))
                .unwrap();
            assert_eq!(first.send(request(4)).unwrap().body, b"updated");
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            drop((first, other, shared));
            if persistent {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }
    #[test]
    fn partial_representation_is_not_reused_as_complete_cached_script() {
        struct Partial(Arc<AtomicUsize>);
        impl NetworkBroker for Partial {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let call = self.0.fetch_add(1, Ordering::SeqCst);
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if call == 0 { 206 } else { 200 },
                    headers: vec![("cache-control".into(), "max-age=600".into())],
                    body: if call == 0 {
                        b"partial".to_vec()
                    } else {
                        b"complete".to_vec()
                    },
                })
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let cache = MemoryCachingBroker::new(Partial(calls.clone()), "partial", 8, 8192, 1024);
        assert_eq!(cache.send(request(1)).unwrap().status, 206);
        assert_eq!(cache.send(request(2)).unwrap().body, b"complete");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let mut ranged = request(3);
        ranged.headers.push(("range".into(), "bytes=0-2".into()));
        cache.send(ranged).unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "range request reaches range-capable origin without whole-response substitution"
        );
    }
    #[test]
    fn stale_window_obeys_age_and_forbids_mandatory_revalidation() {
        let now = unix_time_ms();
        let mut entry = CacheEntry {
            encoded_body_size: None,
            status: 200,
            headers: vec![
                (
                    "cache-control".into(),
                    "max-age=10, stale-while-revalidate=5".into(),
                ),
                ("age".into(), "20".into()),
            ],
            body: Vec::new(),
            stored_at_ms: now,
            freshness_lifetime_ms: 0,
            vary: Vec::new(),
        };
        assert!(!cache_swr_allowed(&request(1), &entry));
        entry.headers[1].1 = "12".into();
        assert!(cache_swr_allowed(&request(1), &entry));
        entry
            .headers
            .push(("cache-control".into(), "must-revalidate".into()));
        assert!(!cache_swr_allowed(&request(1), &entry));
    }
}

#[cfg(test)]
mod response_length_tests {
    use super::decoded_response_length;
    #[test]
    fn malformed_and_conflicting_lengths_fail_closed() {
        for value in [
            "",
            "-1",
            "+1",
            "1x",
            "1, 2",
            "999999999999999999999999999999",
        ] {
            assert!(
                decoded_response_length(&[("Content-Length".into(), value.into())], false, true)
                    .is_err()
            );
        }
        assert!(
            decoded_response_length(
                &[
                    ("content-length".into(), "1".into()),
                    ("Content-Length".into(), "2".into())
                ],
                false,
                true
            )
            .is_err()
        );
    }
    #[test]
    fn framing_is_validated_without_confusing_decoded_and_representation_lengths() {
        let headers = vec![("content-length".into(), "12, 12".into())];
        assert_eq!(
            decoded_response_length(&headers, false, true).unwrap(),
            Some(12)
        );
        assert_eq!(
            decoded_response_length(&headers, false, false).unwrap(),
            None
        );
        assert_eq!(
            decoded_response_length(&headers, true, true).unwrap(),
            Some(0)
        );
        assert!(
            decoded_response_length(&[("content-length".into(), "bad".into())], false, false)
                .is_err()
        );
    }
}

/// Resource Timing TAO is independent from Fetch's CORS response filtering.
pub(crate) fn timing_allow_origin(
    headers: &[(String, String)],
    origin: &str,
    same_origin: bool,
) -> bool {
    same_origin
        || headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("timing-allow-origin"))
            .flat_map(|(_, value)| value.split(','))
            .any(|value| {
                let value = value.trim();
                value == "*" || value == origin
            })
}
#[cfg(test)]
mod timing_allow_origin_tests {
    use super::timing_allow_origin;
    #[test]
    fn tao_is_exact_and_independent_of_cors() {
        assert!(timing_allow_origin(&[], "https://a.test", true));
        assert!(!timing_allow_origin(&[], "https://a.test", false));
        for value in ["*", "https://b.test, https://a.test"] {
            assert!(timing_allow_origin(
                &[("Timing-Allow-Origin".into(), value.into())],
                "https://a.test",
                false
            ));
        }
        assert!(!timing_allow_origin(
            &[("timing-allow-origin".into(), "https://a.test.evil".into())],
            "https://a.test",
            false
        ));
    }
}

pub(crate) fn timing_hop(url: &Url, headers: &[(String, String)]) -> oryn_js::platform::TimingHop {
    oryn_js::platform::TimingHop {
        url: url.to_string(),
        allow_origin: headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("timing-allow-origin"))
            .map(|(_, value)| value.clone())
            .collect(),
    }
}
pub(crate) fn timing_metadata_allows(
    metadata: &oryn_js::platform::ResourceTimingMetadata,
    origin: &str,
    opaque: bool,
) -> bool {
    if !metadata.valid_native_snapshot() {
        return false;
    }
    if metadata.hops.is_empty() {
        return metadata.allow_details && metadata.origin == origin;
    }
    metadata.hops.len() <= 21
        && metadata.hops.iter().all(|hop| {
            let same = !opaque
                && Url::parse(&hop.url)
                    .is_ok_and(|url| url.origin().ascii_serialization() == origin);
            let headers = hop
                .allow_origin
                .iter()
                .map(|value| ("timing-allow-origin".into(), value.clone()))
                .collect::<Vec<_>>();
            timing_allow_origin(&headers, origin, same)
        })
}

/// Same-host native IPC alignment. This clock uses Instant's platform elapsed
/// domain; wall-clock adjustments never enter phase duration calculations.
pub(crate) fn timing_clock_micros() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        #[cfg(target_os = "macos")]
        let clock = libc::CLOCK_UPTIME_RAW;
        #[cfg(not(target_os = "macos"))]
        let clock = libc::CLOCK_MONOTONIC;
        if unsafe { libc::clock_gettime(clock, &mut value) } != 0
            || value.tv_sec < 0
            || value.tv_nsec < 0
        {
            return None;
        }
        (value.tv_sec as u64)
            .checked_mul(1_000_000)?
            .checked_add(value.tv_nsec as u64 / 1000)
    }
    #[cfg(not(unix))]
    {
        None
    }
}
pub(crate) fn timing_clock_origin(elapsed: u64) -> Option<u64> {
    timing_clock_micros()?.checked_sub(elapsed)
}
#[cfg(any(feature = "v8-host", test))]
pub(crate) fn timing_instant(
    metadata: &oryn_js::platform::ResourceTimingMetadata,
) -> Option<std::time::Instant> {
    let now = std::time::Instant::now();
    let elapsed = timing_clock_micros()?.checked_sub(metadata.producer_clock_micros?)?;
    now.checked_sub(std::time::Duration::from_micros(elapsed))
}

pub(crate) fn resource_timing_metadata(
    origin: &str,
    opaque: bool,
    hops: Vec<oryn_js::platform::TimingHop>,
    response: &NetworkResponse,
    elapsed: u64,
) -> oryn_js::platform::ResourceTimingMetadata {
    let exposure = response
        .timing
        .as_ref()
        .and_then(|timing| timing.exposure.as_ref());
    let local_allowed = exposure.is_some_and(|exposure| {
        exposure.hops.is_empty() && exposure.allowed && exposure.origin == origin
    });
    let hops = exposure.map_or(hops, |exposure| exposure.hops.clone());
    let transport = response.timing.clone().map(|timing| {
        let offset = elapsed.saturating_sub(timing.response_end.unwrap_or(timing.response_start));
        timing.shifted(offset)
    });
    let response_start_micros = transport
        .as_ref()
        .filter(|timing| timing.measured_response)
        .map_or(elapsed, |timing| timing.response_start);
    let mut metadata = oryn_js::platform::ResourceTimingMetadata {
        server_timing: oryn_js::server_timing::parse(&response.headers),
        response_end_micros: Some(elapsed),
        navigation_phases: None,
        producer_clock_micros: timing_clock_origin(elapsed),
        decoded_body_size: Some(response.body.len() as u64),
        origin: origin.into(),
        hops,
        transport,
        allow_details: local_allowed,
        response_status: response.status,
        response_start_micros,
        encoded_body_size: if response
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
            .all(|(_, value)| value.trim().eq_ignore_ascii_case("identity"))
        {
            Some(response.body.len() as u64)
        } else {
            response
                .timing
                .as_ref()
                .and_then(|timing| timing.encoded_body_size)
                .or_else(|| {
                    response
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.parse().ok())
                })
        },
        content_type: oryn_common::mime_type::MimeType::from_headers(&response.headers)
            .map(|mime| mime.essence().to_owned())
            .unwrap_or_default(),
    };
    metadata.allow_details = timing_metadata_allows(&metadata, origin, opaque);
    metadata
}

/// Service response reuse carries timing permission provenance, never timestamps
/// from the earlier fetch into the new request's transport phases.
pub(crate) fn service_timing_provenance(
    metadata: Option<&oryn_js::platform::ResourceTimingMetadata>,
) -> Option<oryn_net::TransportTiming> {
    metadata.map(|metadata| oryn_net::TransportTiming {
        encoded_body_size: metadata.encoded_body_size,
        exposure: Some(oryn_common::transport_timing::TimingExposure {
            origin: metadata.origin.clone(),
            hops: metadata.hops.clone(),
            allowed: metadata.allow_details,
        }),
        ..Default::default()
    })
}

#[cfg(test)]
mod timing_provenance_tests {
    use super::*;
    fn metadata() -> oryn_js::platform::ResourceTimingMetadata {
        let response = NetworkResponse {
            response_policy: None,
            timing: Some(oryn_net::TransportTiming {
                measured_response: true,
                connect_start: Some(1),
                connect_end: Some(2),
                request_start: 2,
                response_start: 4,
                response_end: Some(8),
                next_hop_protocol: "h2".into(),
                ..Default::default()
            }),
            request_id: 1,
            status: 200,
            headers: vec![("content-type".into(), "text/javascript".into())],
            body: b"void 0".to_vec(),
        };
        resource_timing_metadata(
            "https://a.test",
            false,
            vec![
                oryn_js::platform::TimingHop {
                    url: "https://a.test/start".into(),
                    allow_origin: Vec::new(),
                },
                oryn_js::platform::TimingHop {
                    url: "https://cdn.test/end".into(),
                    allow_origin: vec!["*".into()],
                },
            ],
            &response,
            12,
        )
    }
    #[test]
    fn completed_identity_response_uses_observed_size_not_service_header() {
        for encoding in [None, Some("identity")] {
            let mut response = NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: 1,
                status: 200,
                headers: vec![("Content-Length".into(), "99999".into())],
                body: b"actual".to_vec(),
            };
            if let Some(encoding) = encoding {
                response
                    .headers
                    .push(("Content-Encoding".into(), encoding.into()));
            }
            let details = resource_timing_metadata(
                "https://a.test",
                false,
                vec![oryn_js::platform::TimingHop {
                    url: "https://a.test/".into(),
                    allow_origin: Vec::new(),
                }],
                &response,
                5,
            );
            assert_eq!(details.encoded_body_size, Some(6));
            assert_eq!(details.decoded_body_size, Some(6));
        }
    }
    #[test]
    fn external_metadata_uses_raw_bytes_and_current_hop_measurements() {
        let metadata = metadata();
        assert!(metadata.allow_details);
        assert_eq!(metadata.decoded_body_size, Some(6));
        assert_eq!(metadata.encoded_body_size, Some(6));
        assert_eq!(metadata.transport.as_ref().unwrap().connect_start, Some(5));
    }
    #[test]
    fn redirected_service_reuse_rechecks_every_hop_for_new_origin() {
        let metadata = metadata();
        assert!(timing_metadata_allows(&metadata, "https://a.test", false));
        assert!(!timing_metadata_allows(&metadata, "https://b.test", false));
        let mut wildcard = metadata.clone();
        wildcard.hops[0].allow_origin = vec!["*".into()];
        assert!(timing_metadata_allows(&wildcard, "https://b.test", false));
        assert!(!timing_metadata_allows(
            &metadata,
            "https://a.test.evil",
            false
        ));
    }
    #[test]
    fn local_service_timing_permission_is_bound_to_original_origin() {
        let metadata = local_resource_timing_metadata("https://a.test", b"abc", None, 1);
        let response = NetworkResponse {
            response_policy: None,
            request_id: 1,
            status: 200,
            headers: Vec::new(),
            body: b"abc".to_vec(),
            timing: service_timing_provenance(Some(&metadata)),
        };
        assert!(
            resource_timing_metadata("https://a.test", false, Vec::new(), &response, 3)
                .allow_details
        );
        assert!(
            !resource_timing_metadata("https://b.test", false, Vec::new(), &response, 3)
                .allow_details
        );
    }
    #[test]
    fn service_reuse_does_not_replay_old_network_phase_timestamps() {
        let metadata = metadata();
        let carried = service_timing_provenance(Some(&metadata)).unwrap();
        assert!(carried.connect_start.is_none());
        assert!(carried.next_hop_protocol.is_empty());
        assert_eq!(carried.exposure.unwrap().hops, metadata.hops);
    }
}

pub(crate) fn local_resource_timing_metadata(
    origin: &str,
    body: &[u8],
    content_type: Option<&str>,
    elapsed: u64,
) -> oryn_js::platform::ResourceTimingMetadata {
    oryn_js::platform::ResourceTimingMetadata {
        server_timing: Vec::new(),
        response_end_micros: Some(elapsed),
        navigation_phases: None,
        producer_clock_micros: timing_clock_origin(elapsed),
        decoded_body_size: Some(body.len() as u64),
        origin: origin.into(),
        hops: Vec::new(),
        transport: None,
        allow_details: true,
        response_status: 200,
        response_start_micros: elapsed,
        encoded_body_size: Some(body.len() as u64),
        content_type: content_type
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .into(),
    }
}

#[cfg(test)]
mod document_mime_prefix_tests {
    use super::*;
    fn make_operation(events: Vec<NetworkEvent>) -> NetworkOperation {
        let (sender, receiver) = tokio::sync::mpsc::channel(events.len().max(1));
        for event in events {
            sender.try_send(event).unwrap();
        }
        drop(sender);
        NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: 7,
            events: receiver,
            demand: None,
        }
    }
    #[test]
    fn pending_prefix_is_nonblocking_and_preserves_one_outstanding_demand() {
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let (demand, mut requests) = tokio::sync::mpsc::channel(8);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 7,
                bytes: bytes::Bytes::from_static(b"abc"),
            })
            .unwrap();
        let operation = NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: 7,
            events,
            demand: Some(demand),
        };
        let mut sniff = DocumentMimeSniff::new(operation, None, false);
        assert!(sniff.poll().is_pending());
        assert_eq!(requests.try_recv(), Ok(()));
        assert!(sniff.poll().is_pending());
        assert!(requests.try_recv().is_err());
        sniff.state.deadline = std::time::Instant::now();
        let std::task::Poll::Ready(Ok((mut operation, mime))) = sniff.poll() else {
            panic!("sniff deadline")
        };
        assert_eq!(mime, "text/plain");
        assert!(operation.body_demand_pending.get());
        assert!(matches!(
            operation.try_event(),
            Ok(NetworkEvent::Chunk { .. })
        ));
        operation.request_body_chunk();
        assert!(
            requests.try_recv().is_err(),
            "replay cannot reset in-flight demand"
        );
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 7,
                bytes: bytes::Bytes::from_static(b"def"),
            })
            .unwrap();
        assert!(matches!(
            operation.try_event(),
            Ok(NetworkEvent::Chunk { .. })
        ));
        operation.request_body_chunk();
        assert_eq!(requests.try_recv(), Ok(()));
        let mut again = DocumentMimeSniff::new(operation, None, false);
        assert!(again.poll().is_pending());
        assert!(requests.try_recv().is_err());
        drop(again);
        assert!(
            sender.is_closed(),
            "abandoned prefix closes original custody"
        );
    }
    #[test]
    fn sniff_replays_all_bytes_and_actual_completion_in_order() {
        let chunk = NetworkEvent::Chunk {
            request_id: 7,
            bytes: bytes::Bytes::from_static(b"<!--comment--><p>value</p>"),
        };
        let terminal = NetworkEvent::Complete {
            encoded_body_size: None,
            request_id: 7,
        };
        let mut operation = make_operation(vec![chunk.clone(), terminal.clone()]);
        assert_eq!(
            sniff_document_operation(&mut operation, None, false, || Ok(())).unwrap(),
            "text/html"
        );
        assert_eq!(operation.try_event().unwrap(), chunk);
        assert_eq!(operation.try_event().unwrap(), terminal);
        assert!(operation.try_event().is_err());
    }
    #[test]
    fn sniff_does_not_treat_prefix_or_channel_close_as_terminal() {
        let bytes = bytes::Bytes::from(vec![b'x'; oryn_net::mime_sniff::PREFIX_LIMIT + 100]);
        let chunk = NetworkEvent::Chunk {
            request_id: 7,
            bytes,
        };
        let mut operation = make_operation(vec![chunk.clone()]);
        assert_eq!(
            sniff_document_operation(&mut operation, None, false, || Ok(())).unwrap(),
            "text/plain"
        );
        assert_eq!(operation.try_event().unwrap(), chunk);
        assert!(operation.try_event().is_err());
        let mut short = make_operation(vec![NetworkEvent::Chunk {
            request_id: 7,
            bytes: bytes::Bytes::from_static(b"abc"),
        }]);
        assert!(
            sniff_document_operation(&mut short, None, false, || Ok(()))
                .unwrap_err()
                .contains("terminal")
        );
        assert!(matches!(short.try_event(), Ok(NetworkEvent::Chunk { .. })));
    }
    #[test]
    fn sniff_rejects_wrong_owner_and_honors_cancellation() {
        let mut wrong = make_operation(vec![NetworkEvent::Complete {
            encoded_body_size: None,
            request_id: 8,
        }]);
        assert!(
            sniff_document_operation(&mut wrong, None, false, || Ok(()))
                .unwrap_err()
                .contains("identity")
        );
        let mut cancelled = make_operation(vec![NetworkEvent::Complete {
            encoded_body_size: None,
            request_id: 7,
        }]);
        assert_eq!(
            sniff_document_operation(&mut cancelled, None, false, || Err("cancelled".into()))
                .unwrap_err(),
            "cancelled"
        );
        assert!(matches!(
            cancelled.try_event(),
            Ok(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: 7
            })
        ));
    }
}

/// Project a measured native response onto the new Document's original clock.
/// An absent IPC anchor stays unavailable; it is never replaced by receipt time.
#[cfg(any(feature = "v8-host", test))]
pub(crate) fn install_frame_navigation_timing(
    record: &oryn_js::navigation_timing::NavigationTiming,
    url: &str,
    headers: &[(String, String)],
    metadata: Option<&oryn_js::platform::ResourceTimingMetadata>,
    referrer_policy: &str,
    complete: bool,
) -> Result<(), String> {
    let Some(metadata) = metadata.filter(|metadata| metadata.valid_native_snapshot()) else {
        return record.set_name(url.into());
    };
    let Some(fetch) = timing_instant(metadata) else {
        return record.set_name(url.into());
    };
    let received = fetch
        .checked_add(std::time::Duration::from_micros(
            metadata.response_start_micros,
        ))
        .unwrap_or(fetch);
    let mut details = metadata.clone();
    // Final transport belongs to the destination Document. Redirect disclosure is
    // separately checked against each hop; predecessor resource TAO is not reused.
    details.allow_details = true;
    if let Some(start) = native_worker_start(metadata.transport.as_ref()) {
        record.worker_start(start);
    }
    record.redirect_provenance(&metadata.hops)?;
    record.response(oryn_js::navigation_timing::NavigationResponse {
        url: url.into(),
        status: metadata.response_status,
        headers,
        fetch,
        received,
        details: Some(details),
        no_referrer: referrer_policy == "no-referrer",
    })?;
    if complete
        && let Some(end) = metadata
            .response_end_micros
            .or_else(|| {
                metadata
                    .transport
                    .as_ref()
                    .and_then(|timing| timing.response_end)
            })
            .and_then(|end| fetch.checked_add(std::time::Duration::from_micros(end)))
    {
        record.finish(end, metadata.decoded_body_size.unwrap_or(0));
    }
    Ok(())
}

/// Measured request-chain intervals. One collector belongs to one fetch; cached
/// response provenance never transfers an older collector's timestamps.
pub(crate) struct NavigationPhaseClock {
    start: std::time::Instant,
    fetch: u64,
    redirect_start: Option<u64>,
    redirect_end: Option<u64>,
}
impl NavigationPhaseClock {
    pub(crate) fn new(start: std::time::Instant) -> Self {
        Self {
            start,
            fetch: 0,
            redirect_start: None,
            redirect_end: None,
        }
    }
    fn elapsed(&self) -> u64 {
        self.start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }
    pub(crate) fn begin_hop(&mut self) {
        self.fetch = self.elapsed();
    }
    pub(crate) fn redirected(&mut self) {
        self.redirect_start.get_or_insert(self.fetch);
        self.redirect_end = Some(self.elapsed());
    }
    pub(crate) fn snapshot(&self) -> oryn_js::platform::NavigationTimingPhases {
        oryn_js::platform::NavigationTimingPhases {
            fetch_start_micros: self.fetch,
            redirect_start_micros: self.redirect_start,
            redirect_end_micros: self.redirect_end,
        }
    }
}

#[cfg(test)]
mod navigation_phase_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn buffered_navigation_finishes_without_inventing_network_transport_phases() {
        use oryn_js::navigation_timing::{Kind, NavigationTiming};
        let record = NavigationTiming::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
            Kind::Navigate,
        );
        let mut metadata = local_resource_timing_metadata(
            "https://timing.test",
            b"hello",
            Some("text/html"),
            50_000,
        );
        assert!(metadata.transport.is_none());
        assert_eq!(metadata.response_end_micros, Some(50_000));
        install_frame_navigation_timing(
            &record,
            "https://timing.test/final",
            &[],
            Some(&metadata),
            "",
            false,
        )
        .unwrap();
        assert_eq!(record.snapshot().values["responseEnd"], 0.);
        install_frame_navigation_timing(
            &record,
            "https://timing.test/final",
            &[],
            Some(&metadata),
            "",
            true,
        )
        .unwrap();
        let snapshot = record.snapshot();
        assert!(snapshot.values["responseEnd"] > 0.);
        assert_eq!(snapshot.values["decodedBodySize"], 5.);
        assert!(snapshot.protocol.is_empty());
        metadata.response_end_micros = Some(metadata.response_start_micros - 1);
        assert!(!metadata.valid_native_snapshot());
    }
    #[test]
    fn buffered_completion_provenance_is_not_reused_by_service_response() {
        let metadata = local_resource_timing_metadata("https://timing.test", b"old", None, 900);
        let reused = NetworkResponse {
            response_policy: None,
            request_id: 1,
            status: 200,
            headers: Vec::new(),
            body: b"new".to_vec(),
            timing: service_timing_provenance(Some(&metadata)),
        };
        let current =
            resource_timing_metadata("https://timing.test", false, Vec::new(), &reused, 20);
        assert_eq!(current.response_end_micros, Some(20));
        assert_eq!(current.response_start_micros, 20);
        assert!(current.valid_native_snapshot());
    }
    #[test]
    fn absent_or_future_monotonic_anchor_cannot_create_document_phases() {
        let mut metadata =
            local_resource_timing_metadata("https://timing.test", b"hello", Some("text/html"), 10);
        metadata.producer_clock_micros = None;
        assert!(timing_instant(&metadata).is_none());
        metadata.producer_clock_micros = Some(u64::MAX);
        assert!(timing_instant(&metadata).is_none());
    }
    #[test]
    fn redirect_phase_schema_rejects_impossible_or_partial_intervals() {
        use oryn_js::platform::NavigationTimingPhases as Phases;
        assert!(
            Phases {
                fetch_start_micros: 8,
                redirect_start_micros: Some(1),
                redirect_end_micros: Some(6)
            }
            .valid(10)
        );
        assert!(
            !Phases {
                fetch_start_micros: 8,
                redirect_start_micros: Some(1),
                redirect_end_micros: None
            }
            .valid(10)
        );
        assert!(
            !Phases {
                fetch_start_micros: 8,
                redirect_start_micros: Some(1),
                redirect_end_micros: Some(9)
            }
            .valid(10)
        );
        assert!(
            !Phases {
                fetch_start_micros: 11,
                redirect_start_micros: None,
                redirect_end_micros: None
            }
            .valid(10)
        );
    }
    #[test]
    fn measured_chain_does_not_reuse_final_fetch_offset_as_transport_origin() {
        use oryn_js::navigation_timing::{Kind, NavigationTiming};
        let start = std::time::Instant::now() - std::time::Duration::from_millis(100);
        let record = NavigationTiming::new(start, Kind::Navigate);
        let mut metadata =
            local_resource_timing_metadata("https://a.test", b"hello", Some("text/html"), 90_000);
        metadata.producer_clock_micros = None;
        metadata.navigation_phases = Some(oryn_js::platform::NavigationTimingPhases {
            fetch_start_micros: 30_000,
            redirect_start_micros: Some(10_000),
            redirect_end_micros: Some(25_000),
        });
        metadata.hops = vec![
            timing_hop(&Url::parse("https://a.test/redirect").unwrap(), &[]),
            timing_hop(&Url::parse("https://a.test/final").unwrap(), &[]),
        ];
        metadata.response_start_micros = 50_000;
        record.redirect_provenance(&metadata.hops).unwrap();
        record
            .response(oryn_js::navigation_timing::NavigationResponse {
                url: "https://a.test/final".into(),
                status: 200,
                headers: &[],
                fetch: start,
                received: start + std::time::Duration::from_millis(50),
                details: Some(metadata),
                no_referrer: false,
            })
            .unwrap();
        let snapshot = record.snapshot();
        assert_eq!(snapshot.values["fetchStart"], 30.);
        assert_eq!(snapshot.values["responseStart"], 50.);
        assert_eq!(snapshot.values["redirectStart"], 10.);
        assert_eq!(snapshot.values["redirectEnd"], 25.);
    }
}

/// One current service interception owns this clock, including a fetch fallback.
/// It is never reconstructed from an authored or cached Response.
pub(crate) struct ServiceWorkerTiming(std::cell::Cell<Option<u64>>, std::time::Instant);
impl Default for ServiceWorkerTiming {
    fn default() -> Self {
        Self(std::cell::Cell::new(None), std::time::Instant::now())
    }
}
impl ServiceWorkerTiming {
    pub(crate) fn selected(&self) {
        if self.0.get().is_none() {
            self.0.set(timing_clock_micros());
        }
    }
    fn apply(&self, timing: &mut Option<oryn_net::TransportTiming>) {
        if let Some(clock) = self.0.get() {
            self.stamp(timing.get_or_insert_with(Default::default), clock);
        }
    }
    fn apply_event(&self, timing: &mut Option<Box<oryn_net::TransportTiming>>) {
        if let Some(clock) = self.0.get() {
            self.stamp(timing.get_or_insert_with(Default::default).as_mut(), clock);
        }
    }
    fn stamp(&self, timing: &mut oryn_net::TransportTiming, clock: u64) {
        timing.worker_start_clock_micros = Some(clock);
        if !timing.measured_response {
            timing.response_start = self.1.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            timing.measured_response = true;
        }
    }
    pub(crate) fn response(&self, mut response: NetworkResponse) -> NetworkResponse {
        self.apply(&mut response.timing);
        response
    }
    pub(crate) fn operation(self, mut operation: NetworkOperation) -> NetworkOperation {
        if self.0.get().is_none() {
            return operation;
        }
        let request_id = operation.request_id;
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let (demand, mut credits) = tokio::sync::mpsc::channel(1);
        operation_runtime().spawn(async move{loop{tokio::select!{
   _=sender.closed()=>return,
   credit=credits.recv()=>{if credit.is_none(){return}operation.request_body_chunk();},
   event=operation.next_event()=>{let Some(mut event)=event else{return};if let NetworkEvent::Headers{timing,..}=&mut event{self.apply_event(timing);}let terminal=matches!(event,NetworkEvent::Complete{..}|NetworkEvent::Failed{..});if sender.send(event).await.is_err()||terminal{return}},
  }}});
        NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id,
            events,
            demand: Some(demand),
        }
    }
}
#[cfg(any(feature = "v8-host", test))]
pub(crate) fn native_worker_start(
    timing: Option<&oryn_net::TransportTiming>,
) -> Option<std::time::Instant> {
    let now = std::time::Instant::now();
    let elapsed = timing_clock_micros()?.checked_sub(timing?.worker_start_clock_micros?)?;
    now.checked_sub(std::time::Duration::from_micros(elapsed))
}

#[cfg(all(test, unix))]
mod service_worker_timing_tests {
    use super::*;
    #[test]
    fn selected_worker_stamp_survives_network_fallback_without_reusing_cached_response_clock() {
        let timing = ServiceWorkerTiming::default();
        let mut response = NetworkResponse {
            response_policy: None,
            request_id: 1,
            status: 200,
            headers: Vec::new(),
            body: b"fallback".to_vec(),
            timing: Some(oryn_net::TransportTiming {
                delivery: oryn_common::transport_timing::ResponseDelivery::Network,
                ..Default::default()
            }),
        };
        assert!(
            timing
                .response(response.clone())
                .timing
                .unwrap()
                .worker_start_clock_micros
                .is_none()
        );
        timing.selected();
        let clock = timing.0.get().unwrap();
        timing.selected();
        assert_eq!(timing.0.get(), Some(clock));
        response = timing.response(response);
        assert_eq!(
            response.timing.as_ref().unwrap().worker_start_clock_micros,
            Some(clock)
        );
        assert_eq!(
            response.timing.as_ref().unwrap().delivery,
            oryn_common::transport_timing::ResponseDelivery::Network
        );
        assert!(native_worker_start(response.timing.as_ref()).is_some());
        response.timing.as_mut().unwrap().worker_start_clock_micros = Some(u64::MAX);
        assert!(native_worker_start(response.timing.as_ref()).is_none());
    }
    #[test]
    fn worker_event_stamp_preserves_measured_transport_and_absent_selection() {
        let worker = ServiceWorkerTiming::default();
        let mut absent = None;
        worker.apply_event(&mut absent);
        assert!(absent.is_none());

        let mut measured = Some(Box::new(oryn_net::TransportTiming {
            measured_response: true,
            response_start: 123,
            dns_start: Some(2),
            dns_end: Some(5),
            next_hop_protocol: "h2".into(),
            ..Default::default()
        }));
        let allocation = measured.as_deref().unwrap() as *const oryn_net::TransportTiming;
        worker.selected();
        let clock = worker.0.get().unwrap();
        worker.apply_event(&mut measured);
        let snapshot = measured.as_deref().unwrap();
        assert_eq!(snapshot as *const _, allocation);
        assert_eq!(snapshot.worker_start_clock_micros, Some(clock));
        assert_eq!(snapshot.response_start, 123);
        assert_eq!(snapshot.dns_start, Some(2));
        assert_eq!(snapshot.dns_end, Some(5));
        assert_eq!(snapshot.next_hop_protocol, "h2");
        worker.apply_event(&mut absent);
        let snapshot = absent.as_deref().unwrap();
        assert!(snapshot.measured_response);
        assert_eq!(snapshot.worker_start_clock_micros, Some(clock));
    }

    #[test]
    fn timed_fallback_operation_preserves_body_demand_and_terminal_completion() {
        struct Fallback;
        impl NetworkBroker for Fallback {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                unreachable!()
            }
            fn start_operation(
                &self,
                request: NetworkRequest,
            ) -> Result<NetworkOperation, NetworkError> {
                let timing = ServiceWorkerTiming::default();
                timing.selected();
                Ok(timing.operation(operation_from_response(NetworkResponse {
                    response_policy: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: Vec::new(),
                    body: b"owned fallback".to_vec(),
                    timing: None,
                })))
            }
        }
        let request = NetworkRequest {
            request_id: 55,
            method: "GET".into(),
            url: "https://timing.test/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            credentials: false,
            security: Default::default(),
            resolved_addrs: Vec::new(),
        };
        let response =
            collect_operation_bounded(&Fallback, request, Duration::from_secs(2), 1024).unwrap();
        assert_eq!(response.body, b"owned fallback");
        assert!(native_worker_start(response.timing.as_ref()).is_some());
    }
}

#[cfg(test)]
mod encoded_timing_tests {
    use super::*;
    #[test]
    fn terminal_encoded_count_survives_broker_wire() {
        for size in [None, Some(0), Some(137)] {
            let event = NetworkEvent::Complete {
                request_id: 17,
                encoded_body_size: size,
            };
            let encoded = encode_network_event(event.clone());
            let wire = serde_json::to_vec(&encoded).unwrap();
            assert_eq!(
                decode_network_event(serde_json::from_slice(&wire).unwrap()).unwrap(),
                event
            );
        }
    }
    #[test]
    fn raw_counter_precedes_decompression_and_metadata_uses_it() {
        use std::io::Write;
        let source = b"navigation body".repeat(4096);
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&source).unwrap();
        let gzip = gzip.finish().unwrap();
        let expected = gzip.len() as u64;
        let runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        );
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let raw = AsyncBodyReader {
            runtime,
            stream: Box::pin(futures_util::stream::iter(vec![Ok(bytes::Bytes::from(
                gzip,
            ))])),
            current: bytes::Bytes::new(),
            offset: 0,
            encoded_bytes: counter.clone(),
        };
        let mut body = decoded_body_reader(Box::new(raw), Some("gzip"), 128 * 1024).unwrap();
        let mut decoded = Vec::new();
        body.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, source);
        assert_eq!(counter.load(Ordering::Relaxed), expected);
        let response = NetworkResponse {
            response_policy: None,
            request_id: 17,
            status: 200,
            headers: vec![
                ("content-encoding".into(), "gzip".into()),
                ("content-length".into(), "99999".into()),
            ],
            body: decoded,
            timing: Some(oryn_net::TransportTiming {
                encoded_body_size: Some(expected),
                ..Default::default()
            }),
        };
        let metadata =
            resource_timing_metadata("https://timing.test", false, Vec::new(), &response, 10);
        assert_eq!(metadata.encoded_body_size, Some(expected));
        assert_eq!(metadata.decoded_body_size, Some(source.len() as u64));
    }
    #[test]
    fn cache_reader_failure_cannot_be_retried_into_a_successful_cache_entry() {
        struct Failed(bool);
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                if !self.0 {
                    self.0 = true;
                    Err(std::io::Error::other("decoder checksum"))
                } else {
                    Ok(0)
                }
            }
        }
        let cache = Arc::new(Mutex::new(MemoryCache::new(2, 1024, 1024)));
        let key = CacheKey {
            partition: "test".into(),
            method: "GET".into(),
            url: "https://timing.test/failure".into(),
            credential_partition: "omit".into(),
        };
        let entry = CacheEntry {
            encoded_body_size: None,
            status: 200,
            headers: Vec::new(),
            body: b"partial".to_vec(),
            stored_at_ms: 0,
            freshness_lifetime_ms: 1000,
            vary: Vec::new(),
        };
        let mut reader = CachingReader {
            encoded_bytes: Some(Arc::new(std::sync::atomic::AtomicU64::new(27))),
            inner: Box::new(Failed(false)),
            cache: cache.clone(),
            key: Some(key.clone()),
            entry: Some(entry),
            max_entry_bytes: 1024,
            expected_length: None,
            complete: false,
        };
        assert!(reader.read(&mut [0; 8]).is_err());
        assert_eq!(reader.read(&mut [0; 8]).unwrap(), 0);
        assert!(
            cache
                .lock()
                .unwrap()
                .get_matching(&key, &[], None)
                .is_none()
        );
    }
    #[test]
    fn cache_reader_requires_eof_before_publishing_size_or_representation() {
        let cache = Arc::new(Mutex::new(MemoryCache::new(2, 1024, 1024)));
        let key = CacheKey {
            partition: "test".into(),
            method: "GET".into(),
            url: "https://timing.test/".into(),
            credential_partition: "omit".into(),
        };
        let entry = CacheEntry {
            encoded_body_size: None,
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            stored_at_ms: 0,
            freshness_lifetime_ms: 1000,
            vary: Vec::new(),
        };
        let count = Arc::new(std::sync::atomic::AtomicU64::new(27));
        let mut reader = CachingReader {
            encoded_bytes: Some(count),
            inner: Box::new(std::io::Cursor::new(b"body".to_vec())),
            cache: cache.clone(),
            key: Some(key.clone()),
            entry: Some(entry),
            max_entry_bytes: 1024,
            expected_length: Some(4),
            complete: false,
        };
        let mut buffer = [0; 4];
        assert_eq!(reader.read(&mut buffer).unwrap(), 4);
        assert!(
            cache
                .lock()
                .unwrap()
                .get_matching(&key, &[], None)
                .is_none()
        );
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert!(
            cache
                .lock()
                .unwrap()
                .get_matching(&key, &[], None)
                .is_none()
        );
        assert_eq!(reader.read(&mut buffer).unwrap(), 0);
        assert_eq!(
            cache
                .lock()
                .unwrap()
                .get_matching(&key, &[], None)
                .unwrap()
                .encoded_body_size,
            Some(27)
        );
    }
}

#[cfg(test)]
mod retained_body_owner_tests {
    use super::*;
    #[test]
    fn true_owner_shutdown_discards_an_integrity_buffered_prefix_before_eof() {
        let (sender, events) = tokio::sync::mpsc::channel(2);
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: 99,
                bytes: bytes::Bytes::from_static(b"prefix"),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                request_id: 99,
                encoded_body_size: Some(6),
            })
            .unwrap();
        let mut operation = NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: 99,
            events,
            demand: None,
        };
        let checks = std::cell::Cell::new(0);
        let closed = || {
            let count = checks.get();
            checks.set(count + 1);
            count != 0
        };
        assert!(matches!(
            collect_started_body_with_check(
                &mut operation,
                std::time::Instant::now() + Duration::from_secs(1),
                1024,
                &std::sync::atomic::AtomicBool::new(false),
                Some(&closed)
            ),
            Err(NetworkError::Cancelled)
        ));
        assert!(
            matches!(operation.try_event(), Ok(NetworkEvent::Complete { .. })),
            "shutdown stops before treating the buffered prefix as a completed response"
        );
    }
}

#[cfg(test)]
mod certificate_failure_wire_tests {
    use super::*;
    #[test]
    fn certificate_failure_survives_broker_event_wire_without_reclassifying_strings() {
        for error in [
            NetworkFailure::from(oryn_net::TransportError::CertificateRejected(
                "UnknownIssuer".into(),
            )),
            NetworkFailure::from(oryn_net::TransportError::Failed(
                "invalid certificate: UnknownIssuer".into(),
            )),
        ] {
            let event = NetworkEvent::Failed {
                request_id: 47,
                error: error.clone(),
            };
            let encoded = serde_json::to_vec(&encode_network_event(event.clone())).unwrap();
            let decoded = decode_network_event(serde_json::from_slice(&encoded).unwrap()).unwrap();
            assert_eq!(decoded, event);
            let runtime = crate::RuntimeError::from(error.clone());
            assert_eq!(
                matches!(
                    runtime.navigation_stop(),
                    crate::navigation_control::NavigationStop::CertificateRejected(_)
                ),
                matches!(error, NetworkFailure::Certificate { .. })
            );
        }
        let legacy: EncodedNetworkEvent = serde_json::from_str(
            r#"{"kind":"failed","request_id":47,"error":"server certificate rejected"}"#,
        )
        .unwrap();
        assert!(matches!(
            decode_network_event(legacy).unwrap(),
            NetworkEvent::Failed {
                error: NetworkFailure::Message(_),
                ..
            }
        ));
    }
}
