use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    os::unix::net::UnixStream,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use url::Url;

use oryn_net::cache::{CacheEntry, CacheKey, MemoryCache};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::{
    cookies::{ContextCookieStore, CookieRequestContext},
    security::{NetworkPolicy, PolicyDenied},
};

const BROKER_PROTOCOL_VERSION: u32 = 3;
const MAX_BROKER_FRAME_BYTES: usize = 24 * 1024 * 1024;
const MAX_DECODED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const PERSISTENT_HTTP_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("http_cache_v1");

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

pub trait NetworkBroker: Send + Sync {
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
}

impl NetworkBroker for Box<dyn NetworkBroker> {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        self.as_ref().send(request)
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        self.as_ref().start_stream(request)
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
}

pub struct SharedNetworkBroker {
    inner: Option<Arc<Box<dyn NetworkBroker>>>,
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
    pub fn new(broker: impl NetworkBroker + 'static) -> Self {
        Self {
            inner: Some(Arc::new(Box::new(broker))),
        }
    }
}

impl Clone for SharedNetworkBroker {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
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
        self.inner
            .as_ref()
            .expect("broker handle is live")
            .send(request)
    }

    fn start_stream(&self, request: NetworkRequest) -> Result<NetworkStreamResponse, NetworkError> {
        self.inner
            .as_ref()
            .expect("broker handle is live")
            .start_stream(request)
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
        if let Ok(address) = host.parse::<IpAddr>() {
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
    timeout: Duration,
    max_response_bytes: usize,
    clients: Mutex<BTreeMap<String, reqwest::blocking::Client>>,
    #[cfg(test)]
    test_force_http2: bool,
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
        Ok(Self {
            timeout,
            max_response_bytes,
            clients: Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            test_force_http2: false,
        })
    }

    #[cfg(test)]
    fn with_test_http2_prior_knowledge(mut self) -> Self {
        self.test_force_http2 = true;
        self
    }

    fn client_for(
        &self,
        url: &Url,
        request: &NetworkRequest,
    ) -> Result<reqwest::blocking::Client, NetworkError> {
        let key = format!(
            "{}|{}",
            url.host_str().unwrap_or_default().to_ascii_lowercase(),
            request
                .resolved_addrs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        if let Some(client) = self.clients.lock().expect("client pool lock").get(&key) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd();
        #[cfg(test)]
        if self.test_force_http2 {
            builder = builder.http2_prior_knowledge();
        }
        if let Some(host) = url.host_str()
            && !request.resolved_addrs.is_empty()
        {
            builder = builder.resolve_to_addrs(host, &request.resolved_addrs);
        }
        let client = builder
            .build()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        self.clients
            .lock()
            .expect("client pool lock")
            .insert(key, client.clone());
        Ok(client)
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
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        let client = self.client_for(&url, &request)?;
        let mut builder = client.request(method, url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let response = builder
            .send()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(NetworkError::ResponseTooLarge {
                limit: self.max_response_bytes,
            });
        }
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let encoding = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
            .map(|(_, value)| value.as_str());
        let content_length = encoding
            .is_none_or(|encoding| encoding.eq_ignore_ascii_case("identity"))
            .then(|| response.content_length())
            .flatten()
            .and_then(|length| usize::try_from(length).ok());
        let body = decoded_body_reader(Box::new(response), encoding, self.max_response_bytes)?;
        Ok(NetworkStreamResponse {
            request_id: request.request_id,
            status,
            headers,
            content_length,
            body,
        })
    }
}

/// The only privileged service exposed to a page worker. The connected Unix
/// socket is inherited from the parent, so the sandboxed child needs no
/// filesystem or network entitlement of its own.
pub struct IpcNetworkBroker {
    state: Mutex<IpcNetworkState>,
}

struct IpcNetworkState {
    stream: UnixStream,
    next_message_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerRequestEnvelope {
    protocol_version: u32,
    message_id: u64,
    request: EncodedNetworkRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerResponseEnvelope {
    protocol_version: u32,
    correlation_id: u64,
    response: Option<EncodedNetworkResponse>,
    error: Option<String>,
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
        Self {
            state: Mutex::new(IpcNetworkState {
                stream,
                next_message_id: 1,
            }),
        }
    }
}

impl NetworkBroker for IpcNetworkBroker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| NetworkError::Transport("network IPC lock poisoned".into()))?;
        let message_id = state.next_message_id;
        state.next_message_id += 1;
        let request_id = request.request_id;
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            message_id,
            request: EncodedNetworkRequest {
                request_id: request.request_id,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body_base64: BASE64.encode(request.body),
                credentials: request.credentials,
                security: request.security,
            },
        };
        serde_json::to_writer(&mut state.stream, &envelope)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        state
            .stream
            .write_all(b"\n")
            .and_then(|_| state.stream.flush())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;

        let mut line = Vec::new();
        let mut reader = BufReader::new(
            state
                .stream
                .try_clone()
                .map_err(|error| NetworkError::Transport(error.to_string()))?,
        );
        read_bounded_line(&mut reader, &mut line, MAX_BROKER_FRAME_BYTES)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if line.len() > MAX_BROKER_FRAME_BYTES {
            return Err(NetworkError::Transport(
                "broker response exceeded the IPC frame limit".into(),
            ));
        }
        let response: BrokerResponseEnvelope = serde_json::from_slice(&line)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if response.protocol_version != BROKER_PROTOCOL_VERSION
            || response.correlation_id != message_id
        {
            return Err(NetworkError::Transport(
                "broker response correlation or protocol mismatch".into(),
            ));
        }
        if let Some(error) = response.error {
            return Err(NetworkError::Transport(error));
        }
        let response = response
            .response
            .ok_or_else(|| NetworkError::Transport("broker returned no response".into()))?;
        if response.request_id != request_id {
            return Err(NetworkError::Transport(
                "broker response request identifier mismatch".into(),
            ));
        }
        let body = BASE64
            .decode(response.body_base64)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if body.len() > MAX_DECODED_RESPONSE_BYTES {
            return Err(NetworkError::ResponseTooLarge {
                limit: MAX_DECODED_RESPONSE_BYTES,
            });
        }
        Ok(NetworkResponse {
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            body,
        })
    }
}

/// Serve a worker's inherited broker socket from the privileged parent.
pub fn serve_ipc_broker(
    stream: UnixStream,
    broker: impl NetworkBroker,
    max_frame_bytes: usize,
) -> Result<(), NetworkError> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut last_message_id = 0_u64;
    loop {
        let mut line = Vec::new();
        let read = read_bounded_line(&mut reader, &mut line, max_frame_bytes)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if read == 0 {
            return Ok(());
        }
        let response = if line.len() > max_frame_bytes {
            BrokerResponseEnvelope {
                protocol_version: BROKER_PROTOCOL_VERSION,
                correlation_id: 0,
                response: None,
                error: Some("broker request exceeded the IPC frame limit".into()),
            }
        } else {
            match serde_json::from_slice::<BrokerRequestEnvelope>(&line) {
                Ok(envelope)
                    if envelope.protocol_version == BROKER_PROTOCOL_VERSION
                        && envelope.message_id > last_message_id =>
                {
                    let correlation_id = envelope.message_id;
                    last_message_id = envelope.message_id;
                    let request = envelope.request;
                    let result = BASE64
                        .decode(request.body_base64)
                        .map_err(|error| error.to_string())
                        .and_then(|body| {
                            broker
                                .send(NetworkRequest {
                                    request_id: request.request_id,
                                    method: request.method,
                                    url: request.url,
                                    headers: sanitize_worker_headers(request.headers),
                                    body,
                                    credentials: request.credentials,
                                    security: request.security,
                                    resolved_addrs: Vec::new(),
                                })
                                .map_err(|error| error.to_string())
                        });
                    match result {
                        Ok(response) => BrokerResponseEnvelope {
                            protocol_version: BROKER_PROTOCOL_VERSION,
                            correlation_id,
                            response: Some(EncodedNetworkResponse {
                                request_id: response.request_id,
                                status: response.status,
                                headers: sanitize_parent_response_headers(response.headers),
                                body_base64: BASE64.encode(response.body),
                            }),
                            error: None,
                        },
                        Err(error) => BrokerResponseEnvelope {
                            protocol_version: BROKER_PROTOCOL_VERSION,
                            correlation_id,
                            response: None,
                            error: Some(error.to_string()),
                        },
                    }
                }
                Ok(envelope) => BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    correlation_id: envelope.message_id,
                    response: None,
                    error: Some(if envelope.protocol_version != BROKER_PROTOCOL_VERSION {
                        "unsupported broker protocol version".into()
                    } else {
                        "duplicate or out-of-order broker message".into()
                    }),
                },
                Err(error) => BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    correlation_id: 0,
                    response: None,
                    error: Some(format!("invalid broker request: {error}")),
                },
            }
        };
        serde_json::to_writer(&mut writer, &response)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if line.len() > max_frame_bytes {
            return Ok(());
        }
    }
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
    use std::sync::{Arc, Mutex};

    use super::*;

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
    fn rustls_transport_reuses_clients_by_pinned_route() {
        let transport = RustlsTransport::new(Duration::from_secs(1), 1024).unwrap();
        let url = Url::parse("https://example.test/data").unwrap();
        let mut first = request(url.as_str());
        first.resolved_addrs = vec!["203.0.113.8:443".parse().unwrap()];
        transport.client_for(&url, &first).unwrap();
        first.request_id = 2;
        transport.client_for(&url, &first).unwrap();
        assert_eq!(transport.clients.lock().unwrap().len(), 1);
        first.credentials = false;
        transport.client_for(&url, &first).unwrap();
        // Cookie credentials are now selected by the parent-owned CookieBroker
        // and never alter transport state, so both requests safely reuse the
        // same pinned-route client.
        assert_eq!(transport.clients.lock().unwrap().len(), 1);
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
            request: EncodedNetworkRequest {
                request_id: 7,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: Vec::new(),
                body_base64: String::new(),
                credentials: true,
                security: CookieRequestContext::default(),
            },
        };
        for attempt in 0..2 {
            serde_json::to_writer(&mut worker, &envelope).expect("write request");
            worker.write_all(b"\n").expect("finish frame");
            worker.flush().expect("flush request");
            let mut line = String::new();
            reader.read_line(&mut line).expect("read response");
            let response: BrokerResponseEnvelope =
                serde_json::from_str(&line).expect("valid response");
            if attempt == 0 {
                assert!(response.error.is_none());
            } else {
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
}
