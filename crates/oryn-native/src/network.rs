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
use url::Url;

use oryn_net::cache::{CacheEntry, CacheKey, MemoryCache};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::{
    cookies::{ContextCookieStore, CookieRequestContext},
    security::{NetworkPolicy, PolicyDenied},
};

const BROKER_PROTOCOL_VERSION: u32 = 5;
use crate::worker_route::WorkerRoute;
const MAX_BROKER_FRAME_BYTES: usize = 24 * 1024 * 1024;
const MAX_DECODED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const PERSISTENT_HTTP_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("http_cache_v1");

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
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A policy-approved response whose body remains owned by the parent and is
/// consumed only when the worker asks for another bounded chunk.
pub struct NetworkStreamResponse {
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub content_length: Option<usize>,
    pub body: Box<dyn Read + Send>,
}

/// Events emitted by one independently cancellable browser-network operation.
/// Headers are always emitted before body chunks and completion.  Keeping this
/// contract in `oryn-native` lets policy/cache/cookie adapters decorate a live
/// body without turning it back into a blocking `Read` implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkEvent {
    Headers {
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        content_length: Option<usize>,
    },
    Chunk {
        request_id: u64,
        bytes: bytes::Bytes,
    },
    Complete {
        request_id: u64,
    },
    Failed {
        request_id: u64,
        error: String,
    },
}

pub struct NetworkOperation {
    pub request_id: u64,
    pub events: tokio::sync::mpsc::Receiver<NetworkEvent>,
    pub(crate) demand: Option<tokio::sync::mpsc::Sender<()>>,
}

impl NetworkOperation {
    pub fn request_body_chunk(&self) {
        if let Some(demand) = &self.demand {
            let _ = demand.try_send(());
        }
    }
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
            match operation.events.try_recv() {
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
            | NetworkEvent::Complete { request_id }
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
                status: value,
                headers: values,
                ..
            } => {
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
            NetworkEvent::Complete { request_id } => {
                return Ok(NetworkResponse {
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
                return Err(NetworkError::Transport(error));
            }
        }
    }
}

fn operation_from_response(response: NetworkResponse) -> NetworkOperation {
    let request_id = response.request_id;
    let (sender, events) = tokio::sync::mpsc::channel(3);
    let (demand, mut requested) = tokio::sync::mpsc::channel(1);
    operation_runtime().spawn(async move {
        let length = response.body.len();
        if sender
            .send(NetworkEvent::Headers {
                request_id,
                status: response.status,
                headers: response.headers,
                content_length: Some(length),
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
        let _ = sender.send(NetworkEvent::Complete { request_id }).await;
    });
    NetworkOperation {
        request_id,
        events,
        demand: Some(demand),
    }
}

pub trait NetworkBroker: Send + Sync {
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
        Ok(NetworkStreamResponse {
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            content_length,
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
        std::thread::spawn(move || {
            let _ = sender.blocking_send(NetworkEvent::Headers {
                request_id,
                status: response.status,
                headers: response.headers,
                content_length: response.content_length,
            });
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                if requested.blocking_recv().is_none() {
                    break;
                }
                match response.body.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.blocking_send(NetworkEvent::Complete { request_id });
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
                            error: error.to_string(),
                        });
                        break;
                    }
                }
            }
        });
        Ok(NetworkOperation {
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
            while let Some(event) = operation.events.recv().await {
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
            request_id,
            events,
            demand,
        })
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner.cancel_operation(request_id)
    }
}

pub struct SharedNetworkBroker {
    inner: Option<Arc<Box<dyn NetworkBroker>>>,
    audit: crate::resource_audit::ResourceAudit,
    audit_page_id: Option<u64>,
    audit_worker_pid: Option<u32>,
}

/// Context-owned browser memory cache. Policy remains outside this adapter so
/// every hit and miss is still authorized against the current request URL.
pub struct MemoryCachingBroker<B> {
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
        if request.method != "GET" && request.method != "HEAD" {
            return None;
        }
        let key = self.key(request);
        self.cache
            .lock()
            .ok()?
            .get(&key, &request.headers, unix_time_ms())
            .cloned()
            .map(|entry| NetworkResponse {
                request_id: request.request_id,
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
            })
    }

    fn entry(
        &self,
        request: &NetworkRequest,
        status: u16,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Option<(CacheKey, CacheEntry)> {
        if body.len() > self.max_entry_bytes || !cache_authorized(request, headers) {
            return None;
        }
        let freshness_lifetime_ms = cache_freshness_ms(headers)?;
        let vary = cache_vary(headers, &request.headers)?;
        Some((
            self.key(request),
            CacheEntry {
                status,
                headers: headers.to_vec(),
                body,
                stored_at_ms: unix_time_ms(),
                freshness_lifetime_ms,
                vary,
            },
        ))
    }

    fn store(&self, request: &NetworkRequest, response: &NetworkResponse) {
        if let Some((key, entry)) = self.entry(
            request,
            response.status,
            &response.headers,
            response.body.clone(),
        ) && let Ok(mut cache) = self.cache.lock()
        {
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
        let read = self.inner.read(output)?;
        if read == 0 {
            if !self.complete {
                self.complete = true;
                if let (Some(key), Some(entry)) = (self.key.take(), self.entry.take())
                    && let Ok(mut cache) = self.cache.lock()
                {
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
        if !self.complete
            && self.expected_length.is_some_and(|length| {
                self.entry
                    .as_ref()
                    .is_some_and(|entry| entry.body.len() >= length)
            })
        {
            self.complete = true;
            if let (Some(key), Some(entry)) = (self.key.take(), self.entry.take())
                && let Ok(mut cache) = self.cache.lock()
            {
                cache.insert(key, entry);
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
            (record.entry.is_fresh(unix_time_ms()) && record.entry.matches_vary(request_headers))
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
        if request.method != "GET" && request.method != "HEAD" {
            return Ok(None);
        }
        Ok(self
            .store
            .get(&self.key(request), &request.headers)?
            .map(|entry| NetworkResponse {
                request_id: request.request_id,
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
            }))
    }

    fn entry(
        &self,
        request: &NetworkRequest,
        status: u16,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Option<(CacheKey, CacheEntry)> {
        if body.len() > self.store.max_entry_bytes || !cache_authorized(request, headers) {
            return None;
        }
        Some((
            self.key(request),
            CacheEntry {
                status,
                headers: headers.to_vec(),
                body,
                stored_at_ms: unix_time_ms(),
                freshness_lifetime_ms: cache_freshness_ms(headers)?,
                vary: cache_vary(headers, &request.headers)?,
            },
        ))
    }
}

struct PersistentCachingReader {
    inner: Box<dyn Read + Send>,
    store: Arc<PersistentCacheStore>,
    key: Option<CacheKey>,
    entry: Option<CacheEntry>,
    expected_length: Option<usize>,
    complete: bool,
}

impl Read for PersistentCachingReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(output)?;
        if read == 0 {
            if !self.complete {
                self.complete = true;
                if let (Some(key), Some(entry)) = (self.key.take(), self.entry.take()) {
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
        if !self.complete
            && self.expected_length.is_some_and(|length| {
                self.entry
                    .as_ref()
                    .is_some_and(|entry| entry.body.len() >= length)
            })
        {
            self.complete = true;
            if let (Some(key), Some(entry)) = (self.key.take(), self.entry.take()) {
                self.store
                    .insert(key, entry)
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
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
            inner: Some(Arc::new(Box::new(broker))),
            audit: Default::default(),
            audit_page_id: None,
            audit_worker_pid: None,
        }
    }
    pub fn resource_audit(&self) -> crate::resource_audit::ResourceAudit {
        self.audit.clone()
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
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
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

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
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

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
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
        std::thread::spawn(move || {
            while let Some(event) = operation.events.blocking_recv() {
                match &event {
                    NetworkEvent::Headers {
                        status, headers, ..
                    } => observation.headers(*status, headers),
                    NetworkEvent::Chunk { bytes, .. } => observation.chunk(bytes),
                    NetworkEvent::Complete { .. } => observation.finish("complete"),
                    NetworkEvent::Failed { .. } => observation.finish("failed"),
                }
                if sender.blocking_send(event).is_err() {
                    break;
                }
            }
        });
        Ok(NetworkOperation {
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
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        if let Some(response) = self.cached(&request) {
            return Ok(response);
        }
        self.invalidate_after_mutation(&request);
        let response = self.inner.send(request.clone())?;
        self.store(&request, &response);
        Ok(response)
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        if let Some(response) = self.cached(&request) {
            let content_length = Some(response.body.len());
            return Ok(NetworkStreamResponse {
                request_id: response.request_id,
                status: response.status,
                headers: response.headers,
                content_length,
                body: Box::new(std::io::Cursor::new(response.body)),
            });
        }
        self.invalidate_after_mutation(&request);
        let response = self.inner.start_stream(request.clone())?;
        let candidate = self
            .entry(&request, response.status, &response.headers, Vec::new())
            .filter(|_| {
                response
                    .content_length
                    .is_none_or(|length| length <= self.max_entry_bytes)
            });
        let (key, entry) = candidate.unzip();
        Ok(NetworkStreamResponse {
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            content_length: response.content_length,
            body: Box::new(CachingReader {
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

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        if let Some(response) = self.cached(&request) {
            return Ok(operation_from_response(response));
        }
        self.invalidate_after_mutation(&request);
        let request_id = request.request_id;
        let key = self.key(&request);
        let request_headers = request.headers.clone();
        let cacheable_request = request.clone();
        let eligible = matches!(request.method.as_str(), "GET" | "HEAD");
        let cache = self.cache.clone();
        let max_entry_bytes = self.max_entry_bytes;
        let mut operation = self.inner.start_operation(request)?;
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        operation_runtime().spawn(async move {
            let mut candidate: Option<CacheEntry> = None;
            while let Some(event) = operation.events.recv().await {
                match &event {
                    NetworkEvent::Headers {
                        status,
                        headers,
                        content_length,
                        ..
                    } => {
                        if eligible
                            && content_length.is_none_or(|length| length <= max_entry_bytes)
                            && cache_authorized(&cacheable_request, headers)
                        {
                            candidate =
                                cache_freshness_ms(headers).and_then(|freshness_lifetime_ms| {
                                    cache_vary(headers, &request_headers).map(|vary| CacheEntry {
                                        status: *status,
                                        headers: headers.clone(),
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
                    NetworkEvent::Complete { .. } => {
                        if let Some(entry) = candidate.take()
                            && let Ok(mut cache) = cache.lock()
                        {
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
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        if let Some(response) = self.cached(&request)? {
            return Ok(response);
        }
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            self.store.invalidate_url(&self.partition, &request.url)?;
        }
        let response = self.inner.send(request.clone())?;
        if let Some((key, entry)) = self.entry(
            &request,
            response.status,
            &response.headers,
            response.body.clone(),
        ) {
            self.store.insert(key, entry)?;
        }
        Ok(response)
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        if let Some(response) = self.cached(&request)? {
            let content_length = Some(response.body.len());
            return Ok(NetworkStreamResponse {
                request_id: response.request_id,
                status: response.status,
                headers: response.headers,
                content_length,
                body: Box::new(std::io::Cursor::new(response.body)),
            });
        }
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            self.store.invalidate_url(&self.partition, &request.url)?;
        }
        let response = self.inner.start_stream(request.clone())?;
        let candidate = self
            .entry(&request, response.status, &response.headers, Vec::new())
            .filter(|_| {
                response
                    .content_length
                    .is_none_or(|length| length <= self.store.max_entry_bytes)
            });
        let (key, entry) = candidate.unzip();
        Ok(NetworkStreamResponse {
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            content_length: response.content_length,
            body: Box::new(PersistentCachingReader {
                inner: response.body,
                store: self.store.clone(),
                key,
                entry,
                expected_length: response.content_length,
                complete: false,
            }),
        })
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        if let Some(response) = self.cached(&request)? {
            return Ok(operation_from_response(response));
        }
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            self.store.invalidate_url(&self.partition, &request.url)?;
        }
        let request_id = request.request_id;
        let key = self.key(&request);
        let request_headers = request.headers.clone();
        let cacheable_request = request.clone();
        let max_entry_bytes = self.store.max_entry_bytes;
        let store = self.store.clone();
        let mut operation = self.inner.start_operation(request)?;
        let demand = operation.demand.take();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        operation_runtime().spawn(async move {
            let mut candidate: Option<CacheEntry> = None;
            while let Some(event) = operation.events.recv().await {
                match &event {
                    NetworkEvent::Headers {
                        status,
                        headers,
                        content_length,
                        ..
                    } => {
                        if content_length.is_none_or(|length| length <= max_entry_bytes)
                            && cache_authorized(&cacheable_request, headers)
                        {
                            candidate =
                                cache_freshness_ms(headers).and_then(|freshness_lifetime_ms| {
                                    cache_vary(headers, &request_headers).map(|vary| CacheEntry {
                                        status: *status,
                                        headers: headers.clone(),
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
                    NetworkEvent::Complete { .. } => {
                        if let Some(entry) = candidate.take() {
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

fn cache_freshness_ms(headers: &[(String, String)]) -> Option<u64> {
    let control = response_header(headers, "cache-control")?;
    if control.split(',').any(|part| {
        matches!(
            part.trim().to_ascii_lowercase().as_str(),
            "no-store" | "no-cache" | "private"
        )
    }) {
        return None;
    }
    control.split(',').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.eq_ignore_ascii_case("max-age")
            .then(|| value.trim_matches('"').parse::<u64>().ok())
            .flatten()
            .map(|seconds| seconds.saturating_mul(1_000))
    })
}

fn cache_authorized(request: &NetworkRequest, headers: &[(String, String)]) -> bool {
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
    let Some(vary) = response_header(response_headers, "vary") else {
        return Some(Vec::new());
    };
    let mut values = Vec::new();
    for name in vary.split(',').map(str::trim) {
        if name == "*" {
            return None;
        }
        let value = request_headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        values.push((name.to_ascii_lowercase(), value));
    }
    Some(values)
}

impl<B> PolicyBroker<B> {
    pub fn new(policy: NetworkPolicy, inner: B) -> Self {
        Self { policy, inner }
    }

    fn authorize(&self, request: &NetworkRequest) -> Result<Vec<SocketAddr>, NetworkError> {
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        resolve_and_authorize(&self.policy, &url)
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

impl NetworkBroker for RustlsTransport {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let max_response_bytes = self.max_response_bytes;
        let mut response = self.start_stream(request)?;
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
        Ok(NetworkResponse {
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            body,
        })
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        let response = self
            .runtime
            .block_on(self.transport.start(oryn_net::HttpRequest {
                operation_id: request.request_id,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                resolved_addrs: request.resolved_addrs,
            }))
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let status = response.status;
        let headers = response.headers;
        let encoding = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
            .map(|(_, value)| value.as_str());
        let content_length = encoding
            .is_none_or(|encoding| encoding.eq_ignore_ascii_case("identity"))
            .then(|| {
                headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.parse::<usize>().ok())
            })
            .flatten();
        let body = decoded_body_reader(
            Box::new(AsyncBodyReader {
                runtime: self.runtime.clone(),
                stream: response.body,
                current: bytes::Bytes::new(),
                offset: 0,
            }),
            encoding,
            self.max_response_bytes,
        )?;
        Ok(NetworkStreamResponse {
            request_id: response.operation_id,
            status,
            headers,
            content_length,
            body,
        })
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let request_id = request.request_id;
        let transport = self.transport.clone();
        let decoded_limit = self.max_response_bytes;
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let (demand, mut requested) = tokio::sync::mpsc::channel(1);
        self.runtime.spawn(async move {
            let response = transport
                .start(oryn_net::HttpRequest {
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
                            error: error.to_string(),
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
                            error: error.to_string(),
                        })
                        .await;
                    return;
                }
            };
            let content_length = (encoding == oryn_net::ContentEncoding::Identity)
                .then(|| {
                    response
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.parse::<usize>().ok())
                })
                .flatten();
            if sender
                .send(NetworkEvent::Headers {
                    request_id,
                    status: response.status,
                    headers: response.headers,
                    content_length,
                })
                .await
                .is_err()
            {
                let _ = transport.cancel(request_id).await;
                return;
            }
            let mut body = oryn_net::decode_content_stream(
                response.body,
                encoding,
                decoded_limit,
                decoded_limit,
            );
            while requested.recv().await.is_some() {
                let Some(chunk) = body.next().await else {
                    let _ = sender.send(NetworkEvent::Complete { request_id }).await;
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
                                error: error.to_string(),
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
    writer: Mutex<UnixStream>,
    next_message_id: std::sync::atomic::AtomicU64,
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
    Start { request: EncodedNetworkRequest },
    Cancel { request_id: u64 },
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
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EncodedNetworkEvent {
    Headers {
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        content_length: Option<usize>,
    },
    Chunk {
        request_id: u64,
        body_base64: String,
    },
    Complete {
        request_id: u64,
    },
    Failed {
        request_id: u64,
        error: String,
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
    request_id: u64,
    status: u16,
    headers: Vec<(String, String)>,
    body_base64: String,
}

impl IpcNetworkBroker {
    pub fn new(stream: UnixStream) -> Self {
        let reader = stream.try_clone().expect("clone broker IPC stream");
        let pending: Arc<Mutex<PendingNetworkReplies>> = Arc::new(Mutex::new(BTreeMap::new()));
        let reader_pending = pending.clone();
        let active_requests = Arc::new(Mutex::new(BTreeMap::new()));
        let reader_active_requests = active_requests.clone();
        std::thread::Builder::new()
            .name("oryn-worker-broker-reader".into())
            .spawn(move || broker_response_loop(reader, reader_pending, reader_active_requests))
            .expect("spawn broker response reader");
        Self {
            writer: Mutex::new(stream),
            next_message_id: std::sync::atomic::AtomicU64::new(1),
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
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let mut operation = self.start_operation(request)?;
        let mut status = None;
        let mut headers = Vec::new();
        let mut body = Vec::new();
        while let Some(event) = operation.events.blocking_recv() {
            match event {
                NetworkEvent::Headers {
                    status: value,
                    headers: values,
                    ..
                } => {
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
                NetworkEvent::Complete { request_id } => {
                    return Ok(NetworkResponse {
                        request_id,
                        status: status.ok_or_else(|| {
                            NetworkError::Transport("broker completed before headers".into())
                        })?,
                        headers,
                        body,
                    });
                }
                NetworkEvent::Failed { error, .. } => {
                    return Err(NetworkError::Transport(error));
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
                request: EncodedNetworkRequest {
                    request_id,
                    method: request.method,
                    url: request.url,
                    headers: request.headers,
                    body_base64: BASE64.encode(request.body),
                    credentials: request.credentials,
                    security: request.security,
                },
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
        let Ok(response) = serde_json::from_slice::<BrokerResponseEnvelope>(&line) else {
            continue;
        };
        if response.protocol_version != BROKER_PROTOCOL_VERSION {
            continue;
        }
        let sender = pending
            .lock()
            .ok()
            .and_then(|pending| pending.get(&response.correlation_id).cloned());
        let Some((route, sender)) = sender else {
            continue;
        };
        let event = if response.route != route {
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
                    error: error.to_string(),
                },
            }
        } else if let Some(response) = response.response {
            let body = BASE64.decode(response.body_base64).unwrap_or_default();
            let request_id = response.request_id;
            let _ = sender.blocking_send(NetworkEvent::Headers {
                request_id,
                status: response.status,
                headers: response.headers,
                content_length: Some(body.len()),
            });
            let _ = sender.blocking_send(NetworkEvent::Chunk {
                request_id,
                bytes: bytes::Bytes::from(body),
            });
            NetworkEvent::Complete { request_id }
        } else {
            NetworkEvent::Failed {
                request_id: 0,
                error: "broker returned no event".into(),
            }
        };
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
    if let Ok(mut pending) = pending.lock() {
        for (_, (_, sender)) in std::mem::take(&mut *pending) {
            let _ = sender.blocking_send(NetworkEvent::Failed {
                request_id: 0,
                error: "broker IPC closed".into(),
            });
        }
    }
}

fn decode_network_event(event: EncodedNetworkEvent) -> Result<NetworkEvent, NetworkError> {
    Ok(match event {
        EncodedNetworkEvent::Headers {
            request_id,
            status,
            headers,
            content_length,
        } => NetworkEvent::Headers {
            request_id,
            status,
            headers,
            content_length,
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
        EncodedNetworkEvent::Complete { request_id } => NetworkEvent::Complete { request_id },
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
                        BrokerCommand::Start { request } => request,
                        BrokerCommand::Cancel { request_id } => {
                            let _ = broker.cancel_routed(route.as_ref(), request_id);
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
                                while let Some(event) = operation.events.recv().await {
                                    let event = encode_network_event(event);
                                    let terminal = matches!(
                                        event,
                                        EncodedNetworkEvent::Complete { .. }
                                            | EncodedNetworkEvent::Failed { .. }
                                    );
                                    let envelope = BrokerResponseEnvelope {
                                        protocol_version: BROKER_PROTOCOL_VERSION,
                                        route: route.clone(),
                                        correlation_id,
                                        response: None,
                                        event: Some(event),
                                        error: None,
                                    };
                                    if write_broker_response(&writer, &envelope).is_err()
                                        || terminal
                                    {
                                        break;
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
                            error: Some(error.to_string()),
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
                    error: Some(format!("invalid broker request: {error}")),
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

fn encode_network_event(event: NetworkEvent) -> EncodedNetworkEvent {
    match event {
        NetworkEvent::Headers {
            request_id,
            status,
            headers,
            content_length,
        } => EncodedNetworkEvent::Headers {
            request_id,
            status,
            headers: sanitize_parent_response_headers(headers),
            content_length,
        },
        NetworkEvent::Chunk { request_id, bytes } => EncodedNetworkEvent::Chunk {
            request_id,
            body_base64: BASE64.encode(bytes),
        },
        NetworkEvent::Complete { request_id } => EncodedNetworkEvent::Complete { request_id },
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
    const ALLOWED: &[&str] = &[
        "accept",
        "accept-language",
        "cache-control",
        "content-type",
        "if-match",
        "if-modified-since",
        "if-none-match",
        "if-unmodified-since",
        "pragma",
        "range",
        "user-agent",
    ];
    headers
        .into_iter()
        .filter(|(name, _)| ALLOWED.contains(&name.to_ascii_lowercase().as_str()))
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
        request.resolved_addrs = self.authorize(&request)?;
        self.inner.send(request)
    }

    fn start_stream(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkStreamResponse, NetworkError> {
        request.resolved_addrs = self.authorize(&request)?;
        self.inner.start_stream(request)
    }

    fn start_operation(
        &self,
        mut request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        request.resolved_addrs = self.authorize(&request)?;
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
    fn cookie_broker_applies_same_site_before_transport_and_hides_set_cookie() {
        struct CookieFixture {
            requests: Arc<Mutex<Vec<NetworkRequest>>>,
        }
        impl NetworkBroker for CookieFixture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
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
        let broker = CookieBroker::new(
            CookieFixture {
                requests: requests.clone(),
            },
            ContextCookieStore::memory(),
        );
        let mut initial = request("https://auth.example.test/set");
        initial.security = CookieRequestContext::navigation(Some(initial.url.clone()));
        broker.send(initial).unwrap();

        let mut subresource = request("https://auth.example.test/data");
        subresource.request_id = 2;
        subresource.security = CookieRequestContext {
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
        network_request.headers = vec![
            ("Accept".into(), "text/plain".into()),
            ("Authorization".into(), "secret".into()),
            ("X-Worker-Secret".into(), "secret".into()),
        ];
        let response = broker.send(network_request).expect("broker response");
        assert_eq!(response.body, b"brokered");
        drop(broker);
        server.join().unwrap().expect("broker server");
        assert_eq!(
            requests.lock().unwrap()[0].headers,
            vec![("Accept".into(), "text/plain".into())]
        );
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
                assert!(matches!(
                    frames[2].command,
                    BrokerCommand::Cancel { request_id: 5 }
                ));
                let response = BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    route: frames[usize::from(!wrong_reply)].route.clone(),
                    correlation_id: frames[1].message_id,
                    response: None,
                    event: Some(EncodedNetworkEvent::Complete { request_id: 5 }),
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
            assert!(old.events.blocking_recv().is_none());
            let event = live.events.blocking_recv().unwrap();
            if wrong_reply {
                assert!(matches!(event, NetworkEvent::Failed { .. }));
            } else {
                assert!(matches!(event, NetworkEvent::Complete { request_id: 5 }));
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
            while let Some(event) = operation.events.blocking_recv() {
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
                request: EncodedNetworkRequest {
                    request_id: 7,
                    method: "GET".into(),
                    url: "https://example.test/".into(),
                    headers: Vec::new(),
                    body_base64: String::new(),
                    credentials: true,
                    security: CookieRequestContext::default(),
                },
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
            ("Authorization".into(), "Bearer forged".into()),
            ("Accept".into(), "application/json".into()),
        ]);
        assert_eq!(headers, vec![("Accept".into(), "application/json".into())]);
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
