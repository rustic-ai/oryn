//! Protocol-neutral host operations used by generated web-platform bindings.
//!
//! This module contains no V8 handles and grants no direct I/O authority. A
//! page host forwards these bounded requests to parent-owned services while
//! generated bindings retain JavaScript object state and correlations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HostOperationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchResponseType {
    Opaque,
    OpaqueRedirect,
}
impl FetchResponseType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::OpaqueRedirect => "opaqueredirect",
        }
    }
}

/// Native Fetch cache mode. This metadata is never encoded as an authored header.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FetchCacheMode {
    #[default]
    Default,
    NoStore,
    Reload,
    NoCache,
    ForceCache,
    OnlyIfCached,
}
impl FetchCacheMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::NoStore => "no-store",
            Self::Reload => "reload",
            Self::NoCache => "no-cache",
            Self::ForceCache => "force-cache",
            Self::OnlyIfCached => "only-if-cached",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "default" => Self::Default,
            "no-store" => Self::NoStore,
            "reload" => Self::Reload,
            "no-cache" => Self::NoCache,
            "force-cache" => Self::ForceCache,
            "only-if-cached" => Self::OnlyIfCached,
            _ => return None,
        })
    }
}

/// Fetch-only semantics; image/XHR producers leave this absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchOptions {
    #[serde(default)]
    pub keepalive: bool,
    /// Native privileged Range copied from a media-originated Request.
    #[serde(default)]
    pub privileged_range: bool,
    #[serde(default)]
    pub cache: FetchCacheMode,
    pub mode: String,
    pub redirect: String,
    pub integrity: String,
}
impl FetchOptions {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !matches!(
            self.mode.as_str(),
            "cors" | "no-cors" | "same-origin" | "navigate"
        ) {
            return Err("invalid native fetch mode");
        }
        if !matches!(self.redirect.as_str(), "follow" | "error" | "manual") {
            return Err("invalid native fetch redirect mode");
        }
        if self.cache == FetchCacheMode::OnlyIfCached && self.mode != "same-origin" {
            return Err("only-if-cached requires same-origin");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformHttpRequest {
    #[serde(default)]
    pub upgrade_mixed_image: bool,
    #[serde(default)]
    pub fetch: Option<FetchOptions>,
    #[serde(default)]
    pub use_cors_preflight: bool,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub credentials: String,
    #[serde(default)]
    pub initiator_url: Option<String>,
    #[serde(default)]
    pub referrer_policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbObjectStoreDefinition {
    pub name: String,
    pub key_path: Option<String>,
    pub auto_increment: bool,
    #[serde(default)]
    pub indexes: Vec<IdbIndexDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbIndexDefinition {
    pub name: String,
    pub key_path: String,
    pub unique: bool,
    #[serde(default)]
    pub multi_entry: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbWrite {
    pub store: String,
    pub key: Option<serde_json::Value>,
    pub value: serde_json::Value,
    pub add_only: bool,
}

/// A bounded cursor request. Position pairs use index key then primary key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbCursorStep {
    pub after: Option<(serde_json::Value, serde_json::Value)>,
    pub target: Option<(serde_json::Value, Option<serde_json::Value>)>,
    pub advance: u32,
    pub unique: bool,
    pub with_value: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IdbHostCommand {
    BeginTransaction {
        token: String,
        database: String,
        scope: Vec<String>,
        mode: String,
        #[serde(default)]
        connection: Option<String>,
    },
    OpenConnection {
        token: String,
        name: String,
        version: u64,
    },
    UpgradeConnection {
        token: String,
        name: String,
        inner: Box<IdbHostCommand>,
    },
    DeleteConnection {
        token: String,
        name: String,
    },
    CloseConnection {
        token: String,
        name: String,
    },
    RetireConnection {
        token: String,
        name: String,
    },
    PollConnection {
        token: String,
        name: String,
    },
    AcknowledgeConnection {
        token: String,
        name: String,
        event: u64,
    },
    Transaction {
        token: String,
        database: String,
        inner: Box<IdbHostCommand>,
    },
    CommitTransaction {
        token: String,
        database: String,
    },
    AbortTransaction {
        token: String,
        database: String,
    },
    ListDatabases,
    Open {
        name: String,
        version: u64,
    },
    DeleteDatabase {
        name: String,
    },
    AbortUpgrade {
        name: String,
        old_version: u64,
    },
    CommitUpgrade {
        name: String,
    },
    CreateObjectStore {
        database: String,
        definition: IdbObjectStoreDefinition,
    },
    DeleteObjectStore {
        database: String,
        store: String,
    },
    CreateIndex {
        database: String,
        store: String,
        definition: IdbIndexDefinition,
    },
    DeleteIndex {
        database: String,
        store: String,
        index: String,
    },
    RenameObjectStore {
        database: String,
        store: String,
        new_name: String,
    },
    RenameIndex {
        database: String,
        store: String,
        index: String,
        new_name: String,
    },
    Batch {
        database: String,
        writes: Vec<IdbWrite>,
    },
    Get {
        database: String,
        store: String,
        key: serde_json::Value,
    },
    GetAll {
        database: String,
        store: String,
    },
    Delete {
        database: String,
        store: String,
        key: serde_json::Value,
    },
    Clear {
        database: String,
        store: String,
    },
    Count {
        database: String,
        store: String,
    },
    Cursor {
        database: String,
        store: String,
        index: Option<String>,
        query: Option<serde_json::Value>,
        reverse: bool,
    },
    CursorStep {
        database: String,
        store: String,
        index: Option<String>,
        query: Option<serde_json::Value>,
        reverse: bool,
        step: IdbCursorStep,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbHostRecord {
    pub key: serde_json::Value,
    pub primary_key: serde_json::Value,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdbDatabaseInfo {
    pub name: String,
    pub version: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IdbHostResponse {
    #[serde(default)]
    pub transaction_blocked: bool,
    #[serde(default)]
    pub connection_blocked: bool,
    #[serde(default)]
    pub connection_managed: bool,
    #[serde(default)]
    pub connection_events: Vec<IdbConnectionEvent>,
    pub databases: Vec<IdbDatabaseInfo>,
    pub version: Option<u64>,
    pub old_version: Option<u64>,
    pub upgrade_needed: bool,
    pub object_stores: Vec<String>,
    /// Authoritative object-store metadata returned when a database is opened.
    /// Keeping this in the parent-owned store prevents a reopened transaction
    /// from losing key-path and auto-increment semantics.
    pub object_store_definitions: Vec<IdbObjectStoreDefinition>,
    pub key: Option<serde_json::Value>,
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub value_present: bool,
    pub values: Vec<serde_json::Value>,
    pub records: Vec<IdbHostRecord>,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PlatformHostRequest {
    XsltTransform {
        request: XsltRequest,
    },
    /// Native service FetchEvent task; rejected by window and non-service routes.
    NavigationPreload {
        task: u64,
    },
    HttpStart {
        request: PlatformHttpRequest,
    },
    /// Window-owned, same-origin XSLT stylesheet request.
    XsltStart {
        request: PlatformHttpRequest,
    },
    ImageStart {
        request: PlatformHttpRequest,
        #[serde(default)]
        cors_enabled: bool,
    },
    /// Native link prefetch: empty Fetch destination and resource-hint policy.
    PrefetchStart {
        request: PlatformHttpRequest,
    },
    ImagePreloadStart {
        request: PlatformHttpRequest,
        cors_enabled: bool,
    },
    /// Native HTML media fetch; destination is validated as audio or video.
    MediaStart {
        request: PlatformHttpRequest,
        cors_enabled: bool,
        destination: String,
    },
    HttpRead {
        stream_id: u64,
        max_bytes: usize,
    },
    HttpAbort {
        stream_id: u64,
    },
    WebSocketOpen {
        url: String,
        protocols: Vec<String>,
    },
    WebSocketSend {
        socket_id: u64,
        frame: PlatformWebSocketFrame,
    },
    WebSocketPoll {
        socket_id: u64,
        max_frames: usize,
    },
    WebSocketClose {
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    },
    IndexedDb {
        origin: String,
        command: IdbHostCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformWebSocketFrame {
    Text { value: String },
    Binary { bytes: Vec<u8> },
    Close { code: Option<u16>, reason: String },
}

/// Transport consumption of the original request body, independent of download size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformUploadProgress {
    pub bytes_consumed: u64,
    pub total_bytes: u64,
    pub body_complete: bool,
}

pub use oryn_common::transport_timing::TimingHop;

/// Native timing visibility, kept outside authored response headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTimingMetadata {
    #[serde(default)]
    pub server_timing: Vec<crate::server_timing::Metric>,
    #[serde(default)]
    pub response_end_micros: Option<u64>,
    #[serde(default)]
    pub navigation_phases: Option<NavigationTimingPhases>,
    #[serde(default)]
    pub producer_clock_micros: Option<u64>,
    #[serde(default)]
    pub decoded_body_size: Option<u64>,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub hops: Vec<TimingHop>,
    #[serde(default)]
    pub transport: Option<oryn_common::transport_timing::TransportTiming>,
    pub allow_details: bool,
    pub response_status: u16,
    pub response_start_micros: u64,
    pub encoded_body_size: Option<u64>,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationTimingPhases {
    pub fetch_start_micros: u64,
    pub redirect_start_micros: Option<u64>,
    pub redirect_end_micros: Option<u64>,
}
impl NavigationTimingPhases {
    pub fn valid(&self, response_start: u64) -> bool {
        self.fetch_start_micros <= response_start
            && match (self.redirect_start_micros, self.redirect_end_micros) {
                (None, None) => true,
                (Some(start), Some(end)) => start <= end && end <= self.fetch_start_micros,
                _ => false,
            }
    }
}
impl ResourceTimingMetadata {
    pub fn valid_native_snapshot(&self) -> bool {
        crate::server_timing::valid(&self.server_timing)
            && self
                .response_end_micros
                .is_none_or(|end| end >= self.response_start_micros)
            && self
                .navigation_phases
                .as_ref()
                .is_none_or(|phases| phases.valid(self.response_start_micros))
            && self.origin.len() <= 65536
            && self.hops.len() <= 21
            && self.content_type.len() <= 8192
            && self.hops.iter().all(|hop| {
                hop.url.len() <= 65536
                    && hop.allow_origin.len() <= 64
                    && hop
                        .allow_origin
                        .iter()
                        .all(|value| value.len() <= 65536 && !value.contains(['\r', '\n']))
            })
            && self
                .hops
                .iter()
                .map(|hop| hop.url.len() + hop.allow_origin.iter().map(String::len).sum::<usize>())
                .sum::<usize>()
                <= 256 * 1024
            && self
                .transport
                .as_ref()
                .is_none_or(|timing| timing.valid_snapshot())
    }
}

/// Private native response backing. Never expose these fields through authored
/// Response getters when the accompanying response type is filtered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalResponseMetadata {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub url: String,
    pub redirected: bool,
    pub range_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PlatformHostResult {
    XsltTransformed {
        transform: XsltResult,
    },
    HttpStarted {
        #[serde(default)]
        timing: Option<Box<ResourceTimingMetadata>>,
        #[serde(default)]
        internal_response: Option<InternalResponseMetadata>,
        #[serde(default)]
        response_type: Option<FetchResponseType>,
        upload: Option<PlatformUploadProgress>,
        cors_filtered: bool,
        redirected: bool,
        stream_id: u64,
        status: u16,
        url: String,
        headers: Vec<(String, String)>,
    },
    HttpChunk {
        encoded_body_size: Option<u64>,
        stream_id: u64,
        bytes: Vec<u8>,
        done: bool,
    },
    HttpAborted,
    WebSocketOpened {
        socket_id: u64,
        protocol: String,
    },
    WebSocketSent,
    WebSocketEvents {
        frames: Vec<PlatformWebSocketFrame>,
    },
    WebSocketClosed {
        code: u16,
        reason: String,
        was_clean: bool,
    },
    IndexedDb {
        response: IdbHostResponse,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformHostCompletion {
    pub operation_id: HostOperationId,
    pub result: Result<PlatformHostResult, String>,
}

/// Internal owned-submit outcome; Busy retains producer retry ownership.
#[derive(Debug)]
pub enum OwnedIdbSubmitError {
    Busy,
    Failure(String),
}

/// Non-blocking authority boundary used by generated bindings.
/// Each accepted operation produces one terminal completion (including after
/// cancellation). A failed HttpRead must close its stream before completing.
/// Retirement revokes the endpoint and may discard outstanding completions.
pub trait PlatformHost {
    /// Internal owned admission; unimplemented endpoints fail closed rather
    /// than cloning native scope storage through the ordinary DTO path.
    fn submit_owned_idb_admission(
        &self,
        _operation_id: HostOperationId,
        _origin: String,
        _admission: crate::idb_owned_admission::PreparedAdmission,
    ) -> Result<(), OwnedIdbSubmitError> {
        Err(OwnedIdbSubmitError::Failure(
            "owned IndexedDB admission is unavailable".into(),
        ))
    }

    /// Whether this endpoint provides an image resource backend. This reports
    /// availability only; individual requests still pass admission and policy.
    /// Transform-only endpoints must not be selected for CSS image demand.
    fn supports_image_requests(&self) -> bool {
        true
    }
    fn submit(
        &self,
        operation_id: HostOperationId,
        request: PlatformHostRequest,
    ) -> Result<(), String>;
    fn cancel(&self, operation_id: HostOperationId);
    /// Abandon an already owned HTTP stream when a read/abort cannot be admitted.
    /// Returning transfers the stream out of realm delivery ownership. Native
    /// implementations must retain a separately bounded, pre-reserved cleanup
    /// owner until physical termination is acknowledged; this is not a release
    /// receipt. Implementations must not silently discard failed cleanup.
    /// Native implementations use retained cleanup custody, never a new IO task.
    /// The default revokes the endpoint so an implementation cannot silently leak.
    fn abandon_http_stream(&self, _stream_id: u64) {
        self.retire();
    }
    /// Transfer an accepted socket to pre-reserved native cleanup custody.
    fn abandon_websocket(&self, _socket_id: u64) {
        self.retire();
    }
    /// Revoke this environment's endpoint. Implementations with external
    /// authority must reject retained clones and queued work after retirement.
    fn retire(&self);
    fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion>;
    /// Native resource custody remains with the producer until the binding
    /// consumes a queued completion. Rejection must release only that result.
    fn acknowledge_completion(&self, _id: HostOperationId, _accepted: bool) {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdbConnectionEvent {
    pub event: u64,
    pub old_version: u64,
    pub new_version: Option<u64>,
}

/// Native navigation provenance; authored RequestInit cannot set these flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NavigationRequestFlags {
    #[serde(default)]
    pub reload: bool,
    #[serde(default)]
    pub history: bool,
}

/// Immutable input to a single owned native XSLT helper. Resources are already
/// authorized browser responses; the helper has no network or file authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XsltRequest {
    pub style: String,
    pub source: Option<String>,
    pub parameters: Vec<(String, String, String)>,
    pub url: String,
    pub resources: Vec<(String, String, String)>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XsltResult {
    pub valid: bool,
    pub output: Option<(String, i32)>,
}
