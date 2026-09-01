//! Generated V8 templates backed by the Rust-owned live DOM.
//!
//! Browser objects carry a numeric wrapper identifier only. All interface,
//! realm, document-generation and node-generation checks happen before a
//! generated callback reaches a Rust behavior implementation.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::{Rc, Weak as RcWeak},
    time::{Duration, Instant},
};

use oryn_dom::{
    BoundaryPoint, DomError, DomRuntime, LiveRange, LiveRangeId, Mutation, NodeId, NodeKind,
    ShadowMode,
};
use url::Url;

use crate::{
    bindings::{BindingError, InterfaceBrand, WrapperId, WrapperRegistry},
    generated::{
        GENERATED_INTERFACES, GeneratedMember, GeneratedMemberId, GeneratedMemberKind,
        generated_member_by_id,
    },
    platform::{
        HostOperationId, IdbHostCommand, IdbIndexDefinition, IdbObjectStoreDefinition, IdbWrite,
        PlatformHost, PlatformHostCompletion, PlatformHostRequest, PlatformHostResult,
        PlatformHttpRequest, PlatformWebSocketFrame,
    },
    realm::RealmId,
    trusted_types::{TrustedKind, TrustedTypesConfig, TrustedTypesRegistry, TrustedValue},
};

pub type SharedDom = oryn_dom::SharedDomRuntime;

/// The WebIDL global on which generated interface objects are installed.
/// Keeping exposure as generated data prevents Window-only APIs from leaking
/// into worker realms (and vice versa) as the worker bindings are migrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingGlobal {
    Window,
    Worker,
}

impl BindingGlobal {
    const fn webidl_name(self) -> &'static str {
        match self {
            Self::Window => "Window",
            Self::Worker => "Worker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParserInputCommand {
    Open,
    Write(String),
    Close,
}

#[derive(Debug, thiserror::Error)]
pub enum V8BindingError {
    #[error("generated interface {0} is unavailable")]
    MissingInterface(&'static str),
    #[error("V8 failed to allocate {0}")]
    Allocation(&'static str),
    #[error("event callback failed: {0}")]
    EventCallback(String),
    #[error(transparent)]
    Binding(#[from] BindingError),
    #[error(transparent)]
    Dom(#[from] DomError),
    #[error("generated binding {interface}.{member} is not implemented")]
    Unsupported { interface: String, member: String },
    #[error("document.write exceeded the configured {resource} limit")]
    DocumentWriteLimit { resource: &'static str },
}

struct WrapperObject {
    handle: v8::Weak<v8::Object>,
    connected_root: Option<v8::Global<v8::Object>>,
}

#[derive(Debug, Clone)]
struct EventRecord {
    event_type: String,
    target: Option<NodeId>,
    current_target: Option<NodeId>,
    bubbles: bool,
    cancelable: bool,
    default_prevented: bool,
    propagation_stopped: bool,
    immediate_stopped: bool,
}

struct EventListener {
    callback: v8::Global<v8::Function>,
}

type PlatformListenerMap = BTreeMap<(u64, String), Vec<v8::Global<v8::Value>>>;

#[derive(Debug, Clone, Copy, Default)]
struct MutationObserverOptions {
    child_list: bool,
    attributes: bool,
    character_data: bool,
    subtree: bool,
}

struct MutationObserverEntry {
    callback: v8::Global<v8::Function>,
    object: v8::Global<v8::Object>,
    target: Option<NodeId>,
    options: MutationObserverOptions,
    records: Vec<Mutation>,
}

#[derive(Debug, Clone)]
struct RangeRecord {
    start_container: NodeId,
    start_offset: u32,
    end_container: NodeId,
    end_offset: u32,
}

impl From<LiveRange> for RangeRecord {
    fn from(range: LiveRange) -> Self {
        Self {
            start_container: range.start.container,
            start_offset: range.start.offset,
            end_container: range.end.container,
            end_offset: range.end.offset,
        }
    }
}

impl From<&RangeRecord> for LiveRange {
    fn from(range: &RangeRecord) -> Self {
        Self {
            start: BoundaryPoint {
                container: range.start_container,
                offset: range.start_offset,
            },
            end: BoundaryPoint {
                container: range.end_container,
                offset: range.end_offset,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct TraversalRecord {
    root: NodeId,
    nodes: Vec<NodeId>,
    accepted: BTreeSet<NodeId>,
    index: usize,
    pointer_before: bool,
    what_to_show: u32,
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    url: String,
    state_json: Option<String>,
}

#[derive(Debug, Clone)]
struct HistoryRecord {
    entries: Vec<HistoryEntry>,
    index: usize,
    scroll_restoration: String,
}

#[derive(Debug, Clone)]
struct BlobRecord {
    bytes: Vec<u8>,
    media_type: String,
    file: Option<FileRecord>,
}

#[derive(Debug, Clone)]
struct FileRecord {
    name: String,
    last_modified: i64,
}

struct FormDataRecord {
    entries: Vec<(String, v8::Global<v8::Value>)>,
}

#[derive(Debug, Clone, Default)]
struct HeadersRecord {
    entries: Vec<(String, String)>,
}

struct AbortSignalRecord {
    aborted: bool,
    reason: Option<v8::Global<v8::Value>>,
}

struct RequestRecord {
    url: String,
    method: String,
    headers_id: u64,
    signal_id: u64,
    credentials: String,
    body: Vec<u8>,
    body_object_id: Option<u64>,
    body_used: bool,
}

struct ResponseRecord {
    status: u16,
    url: String,
    headers_id: u64,
    body_stream_id: Option<u64>,
    bytes: Vec<u8>,
    body_used: bool,
    signal_id: Option<u64>,
}

struct XhrRecord {
    ready_state: u16,
    timeout: u64,
    with_credentials: bool,
    response_type: String,
    status: u16,
    response_url: String,
    response_headers: Vec<(String, String)>,
    response_bytes: Vec<u8>,
    method: String,
    url: String,
    request_headers: Vec<(String, String)>,
    stream_id: Option<u64>,
    active_operation: Option<HostOperationId>,
    deadline: Option<Instant>,
}

struct WebSocketRecord {
    url: String,
    ready_state: u16,
    protocol: String,
    binary_type: String,
    socket_id: Option<u64>,
}

struct IdbRequestRecord {
    result: Option<v8::Global<v8::Value>>,
    error: Option<String>,
    transaction_id: Option<u64>,
    source_id: Option<u64>,
    done: bool,
}

struct IdbDatabaseRecord {
    name: String,
    version: u64,
    old_version: u64,
    object_stores: Vec<String>,
    object_store_definitions: BTreeMap<String, IdbObjectStoreDefinition>,
    upgrade_transaction_id: Option<u64>,
    closed: bool,
}

struct IdbTransactionRecord {
    database_id: u64,
    request_id: Option<u64>,
    mode: String,
    object_stores: Vec<String>,
    aborted: bool,
    error: Option<String>,
    pending_requests: usize,
    completion_requested: bool,
    completion_queued: bool,
    active: bool,
}

struct IdbObjectStoreRecord {
    transaction_id: u64,
    name: String,
    key_path: Option<String>,
    auto_increment: bool,
    indexes: BTreeMap<String, (String, bool)>,
    deleted: bool,
}

struct IdbIndexRecord {
    object_store_id: u64,
    name: String,
    key_path: String,
    unique: bool,
}

struct IdbCursorRecord {
    request_id: u64,
    source_id: u64,
    direction: String,
    records: Vec<crate::platform::IdbHostRecord>,
    index: usize,
    got_value: bool,
    iteration_pending: bool,
}

struct IdbKeyRangeRecord {
    lower: Option<serde_json::Value>,
    upper: Option<serde_json::Value>,
    lower_open: bool,
    upper_open: bool,
}

enum PlatformObjectRecord {
    Blob(BlobRecord),
    FormData(FormDataRecord),
    Headers(HeadersRecord),
    AbortSignal(AbortSignalRecord),
    AbortController { signal_id: u64 },
    Request(RequestRecord),
    Response(ResponseRecord),
    XMLHttpRequest(XhrRecord),
    WebSocket(WebSocketRecord),
    IdbFactory,
    IdbRequest(IdbRequestRecord),
    IdbDatabase(IdbDatabaseRecord),
    IdbTransaction(IdbTransactionRecord),
    IdbObjectStore(IdbObjectStoreRecord),
    IdbIndex(IdbIndexRecord),
    IdbCursor(IdbCursorRecord),
    IdbKeyRange(IdbKeyRangeRecord),
    ReadableStream(ReadableStreamRecord),
    ReadableController { stream_id: u64 },
    ReadableReader { stream_id: u64 },
    WritableStream(WritableStreamRecord),
    WritableController { stream_id: u64 },
    WritableWriter { stream_id: u64 },
    TransformStream { readable_id: u64, writable_id: u64 },
}

struct ReadableStreamRecord {
    queue: VecDeque<v8::Global<v8::Value>>,
    waiting: VecDeque<v8::Global<v8::PromiseResolver>>,
    closed: bool,
    locked: bool,
    pull: Option<v8::Global<v8::Function>>,
    cancel: Option<v8::Global<v8::Function>>,
    controller_id: u64,
    pipes: Vec<ReadablePipe>,
    host_stream_id: Option<u64>,
    host_read_pending: bool,
}

enum PendingPlatformOperation {
    FetchStart {
        resolver: v8::Global<v8::PromiseResolver>,
        signal_id: u64,
    },
    BodyRead {
        stream_object_id: u64,
    },
    ResponseRead {
        response_id: u64,
        resolver: v8::Global<v8::PromiseResolver>,
        member: String,
    },
    XhrStart {
        xhr_id: u64,
    },
    XhrRead {
        xhr_id: u64,
    },
    WebSocketOpen {
        websocket_id: u64,
    },
    WebSocketPoll {
        websocket_id: u64,
    },
    WebSocketClose {
        websocket_id: u64,
        code: Option<u16>,
        reason: String,
    },
    WebSocketCommand,
    IndexedDb {
        request_id: u64,
        action: IdbCompletionAction,
    },
    IdbDatabases {
        resolver: v8::Global<v8::PromiseResolver>,
    },
}

struct QueuedIdbOperation {
    request: PlatformHostRequest,
    pending: PendingPlatformOperation,
}

enum DeferredPlatformTask {
    Event {
        target_id: u64,
        event_type: &'static str,
    },
    FinalizeIdbOpen {
        request_id: u64,
        database_id: u64,
        transaction_id: u64,
    },
    CompleteBlobXhr {
        xhr_id: u64,
        url: String,
        blob: BlobRecord,
    },
    AdvanceIdbCursor {
        cursor_id: u64,
        request_id: u64,
    },
}

enum IdbCompletionAction {
    Open { name: String },
    DeleteDatabase,
    Value,
    Key,
    Values { count: Option<usize> },
    Keys { count: Option<usize> },
    Count,
    Cursor { source_id: u64, direction: String },
    Unit,
}

struct ReadablePipe {
    writable_id: u64,
    resolver: v8::Global<v8::PromiseResolver>,
}

enum WritableKind {
    Sink {
        write: Option<v8::Global<v8::Function>>,
        close: Option<v8::Global<v8::Function>>,
        abort: Option<v8::Global<v8::Function>>,
    },
    Transform {
        transform: Option<v8::Global<v8::Function>>,
        flush: Option<v8::Global<v8::Function>>,
        readable_controller_id: u64,
    },
}

struct WritableStreamRecord {
    locked: bool,
    closed: bool,
    kind: WritableKind,
}

struct TrustedPolicyCallbacks {
    name: String,
    create_html: Option<v8::Global<v8::Function>>,
    create_script: Option<v8::Global<v8::Function>>,
    create_script_url: Option<v8::Global<v8::Function>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BindingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub enum BindingGeometry {
    Available(BindingRect),
    Hidden,
    Unsupported(String),
}

#[derive(Debug, Clone, Default)]
pub struct BindingLayout {
    pub styles: BTreeMap<u64, BTreeMap<String, String>>,
    pub geometry: BTreeMap<u64, BindingGeometry>,
}

/// Per-context binding state. The DOM may be shared with the page orchestrator,
/// but V8 handles and wrapper identities remain realm-local and thread-affine.
pub struct V8BindingState {
    dom: SharedDom,
    realm: RealmId,
    document: NodeId,
    document_generation: u64,
    wrappers: Rc<RefCell<WrapperRegistry>>,
    objects: RefCell<BTreeMap<WrapperId, WrapperObject>>,
    instance_templates: RefCell<BTreeMap<&'static str, v8::Global<v8::ObjectTemplate>>>,
    next_event_id: Cell<u64>,
    events: RefCell<BTreeMap<u64, EventRecord>>,
    listeners: RefCell<BTreeMap<(NodeId, String), Vec<EventListener>>>,
    event_handlers: RefCell<BTreeMap<(NodeId, String), v8::Global<v8::Value>>>,
    next_mutation_observer_id: Cell<u64>,
    mutation_observers: RefCell<BTreeMap<u64, MutationObserverEntry>>,
    ranges: RefCell<BTreeMap<u64, RangeRecord>>,
    selection_range: Cell<Option<u64>>,
    selection_anchor: Cell<Option<BoundaryPoint>>,
    selection_focus: Cell<Option<BoundaryPoint>>,
    selection_object: RefCell<Option<v8::Global<v8::Object>>>,
    implementation_object: RefCell<Option<v8::Global<v8::Object>>>,
    history: RefCell<Option<HistoryRecord>>,
    storage_fallback: RefCell<[BTreeMap<String, String>; 2]>,
    next_platform_object_id: Cell<u64>,
    platform_objects: RefCell<BTreeMap<u64, PlatformObjectRecord>>,
    platform_wrappers: RefCell<BTreeMap<u64, v8::Weak<v8::Object>>>,
    platform_host: RefCell<Option<Rc<dyn PlatformHost>>>,
    next_platform_operation_id: Cell<u64>,
    pending_platform_operations: RefCell<BTreeMap<HostOperationId, PendingPlatformOperation>>,
    queued_idb_operations: RefCell<VecDeque<QueuedIdbOperation>>,
    deferred_platform_tasks: RefCell<VecDeque<DeferredPlatformTask>>,
    cancelled_platform_operations: RefCell<BTreeSet<HostOperationId>>,
    platform_listeners: RefCell<PlatformListenerMap>,
    platform_handlers: RefCell<BTreeMap<(u64, String), v8::Global<v8::Value>>>,
    blob_urls: RefCell<BTreeMap<String, u64>>,
    next_blob_url_id: Cell<u64>,
    idb_transactions_to_deactivate: RefCell<BTreeSet<u64>>,
    node_documents: RefCell<BTreeMap<NodeId, NodeId>>,
    next_traversal_id: Cell<u64>,
    traversals: RefCell<BTreeMap<u64, TraversalRecord>>,
    layout: RefCell<BindingLayout>,
    frame_roots: RefCell<BTreeMap<NodeId, NodeId>>,
    frame_windows: RefCell<BTreeMap<NodeId, v8::Global<v8::Object>>>,
    template_contents: RefCell<BTreeMap<NodeId, NodeId>>,
    current_script: Cell<Option<NodeId>>,
    parser_input_active: Cell<bool>,
    parser_input_commands: RefCell<VecDeque<ParserInputCommand>>,
    document_open: Cell<bool>,
    design_mode: Cell<bool>,
    style_with_css: Cell<bool>,
    document_write_buffer: RefCell<String>,
    document_write_depth: Cell<u32>,
    pending_dynamic_scripts: RefCell<VecDeque<NodeId>>,
    prepared_scripts: RefCell<BTreeSet<NodeId>>,
    pending_dynamic_frames: RefCell<VecDeque<NodeId>>,
    prepared_frames: RefCell<BTreeSet<NodeId>>,
    trusted_registry: RefCell<TrustedTypesRegistry>,
    trusted_policies: RefCell<BTreeMap<u64, TrustedPolicyCallbacks>>,
    trusted_values: RefCell<BTreeMap<u64, TrustedValue>>,
    next_trusted_id: Cell<u64>,
}

#[derive(Default)]
struct BindingStateRegistry {
    realms: RefCell<BTreeMap<RealmId, RcWeak<V8BindingState>>>,
}

impl V8BindingState {
    pub fn set_platform_host(&self, host: Rc<dyn PlatformHost>) {
        *self.platform_host.borrow_mut() = Some(host);
    }

    /// Deliver bounded parent-service completions at a page-scheduler
    /// checkpoint. V8 handles never cross the host thread boundary.
    pub fn drain_platform_completions(
        self: &Rc<Self>,
        scope: &mut v8::PinScope,
        limit: usize,
    ) -> Result<usize, V8BindingError> {
        self.finish_platform_task_checkpoint()?;
        self.expire_xhr_timeouts(scope)?;
        let completions = self
            .platform_host
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, |host| host.drain_completions(limit));
        let mut count = completions.len();
        for completion in completions {
            self.handle_platform_completion(scope, completion)?;
            self.flush_queued_idb_operations()?;
        }
        while count < limit {
            let Some(task) = self.deferred_platform_tasks.borrow_mut().pop_front() else {
                break;
            };
            match task {
                DeferredPlatformTask::Event {
                    target_id,
                    event_type,
                } => dispatch_platform_event(scope, self, target_id, event_type, None)?,
                DeferredPlatformTask::FinalizeIdbOpen {
                    request_id,
                    database_id,
                    transaction_id,
                } => self.finalize_idb_open(scope, request_id, database_id, transaction_id)?,
                DeferredPlatformTask::CompleteBlobXhr { xhr_id, url, blob } => {
                    self.complete_blob_xhr(scope, xhr_id, url, blob)?
                }
                DeferredPlatformTask::AdvanceIdbCursor {
                    cursor_id,
                    request_id,
                } => {
                    let value = {
                        let mut record = platform_object_mut(self, cursor_id)?;
                        let PlatformObjectRecord::IdbCursor(cursor) = &mut *record else {
                            return Err(V8BindingError::Allocation("IDBCursor record"));
                        };
                        cursor.iteration_pending = false;
                        cursor.got_value = cursor.index < cursor.records.len();
                        cursor.got_value.then_some(cursor_id)
                    };
                    let value = match value {
                        Some(cursor_id) => {
                            new_platform_object(scope, self, "IDBCursor", cursor_id)?.into()
                        }
                        None => v8::null(scope).into(),
                    };
                    set_idb_request_result(scope, self, request_id, value)?;
                    self.activate_idb_request_transaction(request_id)?;
                    dispatch_platform_event(scope, self, request_id, "success", None)?;
                    finish_idb_request(self, request_id)?;
                }
            }
            count += 1;
        }
        self.expire_xhr_timeouts(scope)?;
        Ok(count)
    }

    fn expire_xhr_timeouts(
        self: &Rc<Self>,
        scope: &mut v8::PinScope,
    ) -> Result<(), V8BindingError> {
        let now = Instant::now();
        let expired = self
            .platform_objects
            .borrow()
            .iter()
            .filter_map(|(id, record)| match record {
                PlatformObjectRecord::XMLHttpRequest(xhr)
                    if xhr.deadline.is_some_and(|deadline| deadline <= now)
                        && xhr.active_operation.is_some() =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for xhr_id in expired {
            let (operation, stream_id) = {
                let mut record = platform_object_mut(self, xhr_id)?;
                let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                    continue;
                };
                xhr.deadline = None;
                xhr.ready_state = 4;
                xhr.status = 0;
                (xhr.active_operation.take(), xhr.stream_id.take())
            };
            if let Some(operation) = operation {
                if let Some(host) = self.platform_host.borrow().as_ref() {
                    host.cancel(operation);
                }
                self.pending_platform_operations
                    .borrow_mut()
                    .remove(&operation);
                self.cancelled_platform_operations
                    .borrow_mut()
                    .insert(operation);
            }
            if let Some(stream_id) = stream_id {
                let _ = submit_platform_operation(
                    self,
                    PlatformHostRequest::HttpAbort { stream_id },
                    PendingPlatformOperation::WebSocketCommand,
                );
            }
            dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
            dispatch_platform_event(scope, self, xhr_id, "timeout", None)?;
            dispatch_platform_event(scope, self, xhr_id, "loadend", None)?;
        }
        Ok(())
    }

    fn complete_blob_xhr(
        self: &Rc<Self>,
        scope: &mut v8::PinScope,
        xhr_id: u64,
        url: String,
        blob: BlobRecord,
    ) -> Result<(), V8BindingError> {
        {
            let mut record = platform_object_mut(self, xhr_id)?;
            let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                return Err(V8BindingError::Allocation("XHR record"));
            };
            xhr.status = 200;
            xhr.response_url = url;
            xhr.response_headers = if blob.media_type.is_empty() {
                Vec::new()
            } else {
                vec![("content-type".into(), blob.media_type)]
            };
            xhr.ready_state = 2;
        }
        dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
        {
            let mut record = platform_object_mut(self, xhr_id)?;
            let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                return Err(V8BindingError::Allocation("XHR record"));
            };
            xhr.response_bytes = blob.bytes;
            xhr.ready_state = 3;
        }
        dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
        dispatch_platform_event(scope, self, xhr_id, "progress", None)?;
        if let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *platform_object_mut(self, xhr_id)?
        {
            xhr.ready_state = 4;
        }
        dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
        dispatch_platform_event(scope, self, xhr_id, "load", None)?;
        dispatch_platform_event(scope, self, xhr_id, "loadend", None)?;
        Ok(())
    }

    pub fn has_pending_platform_work(&self) -> bool {
        !self.queued_idb_operations.borrow().is_empty()
            || !self.deferred_platform_tasks.borrow().is_empty()
            || self
                .pending_platform_operations
                .borrow()
                .values()
                .any(|pending| !matches!(pending, PendingPlatformOperation::WebSocketPoll { .. }))
    }

    fn flush_queued_idb_operations(&self) -> Result<(), V8BindingError> {
        while self.pending_platform_operations.borrow().len() < 32 {
            let Some(queued) = self.queued_idb_operations.borrow_mut().pop_front() else {
                break;
            };
            submit_platform_operation(self, queued.request, queued.pending)?;
        }
        Ok(())
    }

    fn finalize_idb_open(
        &self,
        scope: &mut v8::PinScope,
        request_id: u64,
        database_id: u64,
        _transaction_id: u64,
    ) -> Result<(), V8BindingError> {
        if let PlatformObjectRecord::IdbRequest(request) =
            &mut *platform_object_mut(self, request_id)?
        {
            request.transaction_id = None;
        }
        let closed = {
            let mut record = platform_object_mut(self, database_id)?;
            let PlatformObjectRecord::IdbDatabase(database) = &mut *record else {
                return Err(V8BindingError::Allocation("IDBDatabase record"));
            };
            database.upgrade_transaction_id = None;
            database.closed
        };
        if closed {
            if let PlatformObjectRecord::IdbRequest(request) =
                &mut *platform_object_mut(self, request_id)?
            {
                request.result = None;
                request.error = Some("AbortError".into());
            }
            dispatch_platform_event(scope, self, request_id, "error", None)
        } else {
            dispatch_platform_event(scope, self, request_id, "success", None)
        }
    }

    fn handle_platform_completion(
        self: &Rc<Self>,
        scope: &mut v8::PinScope,
        completion: PlatformHostCompletion,
    ) -> Result<(), V8BindingError> {
        let pending = self
            .pending_platform_operations
            .borrow_mut()
            .remove(&completion.operation_id);
        let Some(pending) = pending else {
            if self
                .cancelled_platform_operations
                .borrow_mut()
                .remove(&completion.operation_id)
            {
                return Ok(());
            }
            return Err(V8BindingError::Unsupported {
                interface: "Host".into(),
                member: "stale or unmatched completion".into(),
            });
        };
        let result = completion
            .result
            .map_err(|message| V8BindingError::Unsupported {
                interface: "Host".into(),
                member: message,
            });
        match (pending, result) {
            (
                PendingPlatformOperation::FetchStart {
                    resolver,
                    signal_id,
                },
                Ok(PlatformHostResult::HttpStarted {
                    stream_id,
                    status,
                    url,
                    headers,
                }),
            ) => {
                record_platform_trace(
                    scope,
                    "response_received",
                    serde_json::json!({"url":url,"status":status,"streaming":true}),
                );
                let headers_id = register_platform_object(
                    self,
                    PlatformObjectRecord::Headers(HeadersRecord { entries: headers }),
                );
                let stream_object_id = reserve_platform_object_id(self);
                let controller_id = reserve_platform_object_id(self);
                insert_platform_object(
                    self,
                    controller_id,
                    PlatformObjectRecord::ReadableController {
                        stream_id: stream_object_id,
                    },
                );
                insert_platform_object(
                    self,
                    stream_object_id,
                    PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
                        queue: VecDeque::new(),
                        waiting: VecDeque::new(),
                        closed: false,
                        locked: false,
                        pull: None,
                        cancel: None,
                        controller_id,
                        pipes: Vec::new(),
                        host_stream_id: Some(stream_id),
                        host_read_pending: false,
                    }),
                );
                let response_id = register_platform_object(
                    self,
                    PlatformObjectRecord::Response(ResponseRecord {
                        status,
                        url,
                        headers_id,
                        body_stream_id: Some(stream_object_id),
                        bytes: Vec::new(),
                        body_used: false,
                        signal_id: Some(signal_id),
                    }),
                );
                let response = new_platform_object(scope, self, "Response", response_id)?;
                v8::Local::new(scope, resolver).resolve(scope, response.into());
            }
            (PendingPlatformOperation::FetchStart { resolver, .. }, Err(error)) => {
                let message = v8_string(scope, &error.to_string())?;
                let exception = v8::Exception::type_error(
                    scope,
                    v8::Local::<v8::String>::try_from(message)
                        .map_err(|_| V8BindingError::Allocation("fetch exception"))?,
                );
                v8::Local::new(scope, resolver).reject(scope, exception);
            }
            (
                PendingPlatformOperation::BodyRead { stream_object_id },
                Ok(PlatformHostResult::HttpChunk { bytes, done, .. }),
            ) => {
                if !bytes.is_empty() {
                    record_platform_trace(
                        scope,
                        "stream_chunk",
                        serde_json::json!({"direction":"response","bytes":bytes.len()}),
                    );
                }
                let controller_id = {
                    let mut record = platform_object_mut(self, stream_object_id)?;
                    let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                        return Err(V8BindingError::Allocation("host ReadableStream"));
                    };
                    stream.host_read_pending = false;
                    stream.controller_id
                };
                if !bytes.is_empty() {
                    let buffer = array_buffer_from_bytes(scope, &bytes);
                    let chunk = v8::Uint8Array::new(scope, buffer, 0, bytes.len())
                        .ok_or(V8BindingError::Allocation("HTTP stream chunk"))?;
                    readable_enqueue(scope, self, controller_id, chunk.into())?;
                    for record in self.platform_objects.borrow_mut().values_mut() {
                        if let PlatformObjectRecord::Response(response) = record
                            && response.body_stream_id == Some(stream_object_id)
                        {
                            response.bytes.extend_from_slice(&bytes);
                        }
                    }
                }
                if done {
                    readable_close(scope, self, controller_id)?;
                }
            }
            (
                PendingPlatformOperation::ResponseRead {
                    response_id,
                    resolver,
                    member,
                },
                Ok(PlatformHostResult::HttpChunk {
                    stream_id,
                    bytes,
                    done,
                }),
            ) => {
                if !bytes.is_empty() {
                    record_platform_trace(
                        scope,
                        "stream_chunk",
                        serde_json::json!({"direction":"response","bytes":bytes.len()}),
                    );
                }
                {
                    let mut record = platform_object_mut(self, response_id)?;
                    let PlatformObjectRecord::Response(response) = &mut *record else {
                        return Err(V8BindingError::Allocation("Response record"));
                    };
                    response.bytes.extend_from_slice(&bytes);
                }
                if done {
                    let value = response_consumed_value(scope, self, response_id, &member)?;
                    v8::Local::new(scope, resolver).resolve(scope, value);
                } else {
                    submit_platform_operation(
                        self,
                        PlatformHostRequest::HttpRead {
                            stream_id,
                            max_bytes: 64 * 1024,
                        },
                        PendingPlatformOperation::ResponseRead {
                            response_id,
                            resolver,
                            member,
                        },
                    )?;
                }
            }
            (
                PendingPlatformOperation::XhrStart { xhr_id },
                Ok(PlatformHostResult::HttpStarted {
                    stream_id,
                    status,
                    url,
                    headers,
                }),
            ) => {
                {
                    let mut record = platform_object_mut(self, xhr_id)?;
                    let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                        return Err(V8BindingError::Allocation("XHR record"));
                    };
                    xhr.ready_state = 2;
                    xhr.status = status;
                    xhr.response_url = url;
                    xhr.response_headers = headers;
                    xhr.stream_id = Some(stream_id);
                }
                dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
                let operation = submit_platform_operation(
                    self,
                    PlatformHostRequest::HttpRead {
                        stream_id,
                        max_bytes: 64 * 1024,
                    },
                    PendingPlatformOperation::XhrRead { xhr_id },
                )?;
                if let PlatformObjectRecord::XMLHttpRequest(xhr) =
                    &mut *platform_object_mut(self, xhr_id)?
                {
                    xhr.active_operation = Some(operation);
                }
            }
            (
                PendingPlatformOperation::XhrRead { xhr_id },
                Ok(PlatformHostResult::HttpChunk {
                    stream_id,
                    bytes,
                    done,
                }),
            ) => {
                {
                    let mut record = platform_object_mut(self, xhr_id)?;
                    let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                        return Err(V8BindingError::Allocation("XHR record"));
                    };
                    xhr.response_bytes.extend_from_slice(&bytes);
                    xhr.ready_state = 3;
                    xhr.active_operation = None;
                }
                dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
                if !bytes.is_empty() {
                    dispatch_platform_event(scope, self, xhr_id, "progress", None)?;
                }
                if done {
                    if let PlatformObjectRecord::XMLHttpRequest(xhr) =
                        &mut *platform_object_mut(self, xhr_id)?
                    {
                        xhr.ready_state = 4;
                        xhr.deadline = None;
                    }
                    dispatch_platform_event(scope, self, xhr_id, "readystatechange", None)?;
                    dispatch_platform_event(scope, self, xhr_id, "load", None)?;
                    dispatch_platform_event(scope, self, xhr_id, "loadend", None)?;
                } else {
                    let operation = submit_platform_operation(
                        self,
                        PlatformHostRequest::HttpRead {
                            stream_id,
                            max_bytes: 64 * 1024,
                        },
                        PendingPlatformOperation::XhrRead { xhr_id },
                    )?;
                    if let PlatformObjectRecord::XMLHttpRequest(xhr) =
                        &mut *platform_object_mut(self, xhr_id)?
                    {
                        xhr.active_operation = Some(operation);
                    }
                }
            }
            (
                PendingPlatformOperation::WebSocketOpen { websocket_id },
                Ok(PlatformHostResult::WebSocketOpened {
                    socket_id,
                    protocol,
                }),
            ) => {
                {
                    let mut record = platform_object_mut(self, websocket_id)?;
                    let PlatformObjectRecord::WebSocket(socket) = &mut *record else {
                        return Err(V8BindingError::Allocation("WebSocket record"));
                    };
                    socket.socket_id = Some(socket_id);
                    socket.protocol = protocol;
                    socket.ready_state = 1;
                }
                dispatch_platform_event(scope, self, websocket_id, "open", None)?;
                submit_platform_operation(
                    self,
                    PlatformHostRequest::WebSocketPoll {
                        socket_id,
                        max_frames: 32,
                    },
                    PendingPlatformOperation::WebSocketPoll { websocket_id },
                )?;
            }
            (
                PendingPlatformOperation::WebSocketPoll { websocket_id },
                Ok(PlatformHostResult::WebSocketEvents { frames }),
            ) => {
                let (socket_id, binary_type) = {
                    let record = platform_object(self, websocket_id)?;
                    let PlatformObjectRecord::WebSocket(socket) = &*record else {
                        return Err(V8BindingError::Allocation("WebSocket record"));
                    };
                    (socket.socket_id, socket.binary_type.clone())
                };
                let mut closed = false;
                for frame in frames {
                    let (frame_type, frame_bytes) = match &frame {
                        PlatformWebSocketFrame::Text { value } => ("text", value.len()),
                        PlatformWebSocketFrame::Binary { bytes } => ("binary", bytes.len()),
                        PlatformWebSocketFrame::Close { reason, .. } => ("close", reason.len()),
                    };
                    record_platform_trace(
                        scope,
                        "web_socket_frame",
                        serde_json::json!({"socket_id":socket_id,"direction":"inbound","frame_type":frame_type,"bytes":frame_bytes}),
                    );
                    match frame {
                        PlatformWebSocketFrame::Text { value } => {
                            let data = v8_string(scope, &value)?;
                            dispatch_platform_message_event(scope, self, websocket_id, data)?;
                        }
                        PlatformWebSocketFrame::Binary { bytes } => {
                            let data = if binary_type == "arraybuffer" {
                                array_buffer_from_bytes(scope, &bytes).into()
                            } else {
                                let id = register_platform_object(
                                    self,
                                    PlatformObjectRecord::Blob(BlobRecord {
                                        bytes,
                                        media_type: String::new(),
                                        file: None,
                                    }),
                                );
                                new_platform_object(scope, self, "Blob", id)?.into()
                            };
                            dispatch_platform_message_event(scope, self, websocket_id, data)?;
                        }
                        PlatformWebSocketFrame::Close { code, reason } => {
                            closed = true;
                            if let PlatformObjectRecord::WebSocket(socket) =
                                &mut *platform_object_mut(self, websocket_id)?
                            {
                                socket.ready_state = 3;
                            }
                            let event = v8::Object::new(scope);
                            for (name, value) in [
                                (
                                    "code",
                                    v8::Integer::new_from_unsigned(
                                        scope,
                                        code.unwrap_or(1005).into(),
                                    )
                                    .into(),
                                ),
                                ("reason", v8_string(scope, &reason)?),
                                ("wasClean", v8::Boolean::new(scope, true).into()),
                            ] {
                                let key = v8::String::new(scope, name)
                                    .ok_or(V8BindingError::Allocation("close event field"))?;
                                event.create_data_property(scope, key.into(), value);
                            }
                            dispatch_platform_event(
                                scope,
                                self,
                                websocket_id,
                                "close",
                                Some(event),
                            )?;
                        }
                    }
                }
                if !closed && let Some(socket_id) = socket_id {
                    submit_platform_operation(
                        self,
                        PlatformHostRequest::WebSocketPoll {
                            socket_id,
                            max_frames: 32,
                        },
                        PendingPlatformOperation::WebSocketPoll { websocket_id },
                    )?;
                }
            }
            (
                PendingPlatformOperation::WebSocketClose {
                    websocket_id,
                    code,
                    reason,
                },
                Ok(_),
            ) => {
                if let PlatformObjectRecord::WebSocket(socket) =
                    &mut *platform_object_mut(self, websocket_id)?
                {
                    socket.ready_state = 3;
                }
                let event = v8::Object::new(scope);
                for (name, value) in [
                    (
                        "code",
                        v8::Integer::new_from_unsigned(scope, code.unwrap_or(1005).into()).into(),
                    ),
                    ("reason", v8_string(scope, &reason)?),
                    ("wasClean", v8::Boolean::new(scope, true).into()),
                ] {
                    let key = v8::String::new(scope, name)
                        .ok_or(V8BindingError::Allocation("close event field"))?;
                    event.create_data_property(scope, key.into(), value);
                }
                dispatch_platform_event(scope, self, websocket_id, "close", Some(event))?;
            }
            (PendingPlatformOperation::WebSocketCommand, Ok(_)) => {}
            (
                PendingPlatformOperation::IndexedDb { request_id, action },
                Ok(PlatformHostResult::IndexedDb { response }),
            ) => {
                let value = match action {
                    IdbCompletionAction::Open { name } => {
                        let database_id = register_platform_object(
                            self,
                            PlatformObjectRecord::IdbDatabase(IdbDatabaseRecord {
                                name,
                                version: response.version.unwrap_or(1),
                                old_version: response.old_version.unwrap_or(0),
                                object_stores: response.object_stores.clone(),
                                object_store_definitions: response
                                    .object_store_definitions
                                    .iter()
                                    .cloned()
                                    .map(|definition| (definition.name.clone(), definition))
                                    .collect(),
                                upgrade_transaction_id: None,
                                closed: false,
                            }),
                        );
                        let transaction_id = response.upgrade_needed.then(|| {
                            register_platform_object(
                                self,
                                PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                                    database_id,
                                    request_id: Some(request_id),
                                    mode: "versionchange".into(),
                                    object_stores: response.object_stores.clone(),
                                    aborted: false,
                                    error: None,
                                    pending_requests: 0,
                                    completion_requested: false,
                                    completion_queued: false,
                                    active: true,
                                }),
                            )
                        });
                        if let Some(transaction_id) = transaction_id {
                            if let PlatformObjectRecord::IdbDatabase(database) =
                                &mut *platform_object_mut(self, database_id)?
                            {
                                database.upgrade_transaction_id = Some(transaction_id);
                            }
                            if let PlatformObjectRecord::IdbRequest(request) =
                                &mut *platform_object_mut(self, request_id)?
                            {
                                request.transaction_id = Some(transaction_id);
                            }
                            self.idb_transactions_to_deactivate
                                .borrow_mut()
                                .insert(transaction_id);
                        }
                        let database =
                            new_platform_object(scope, self, "IDBDatabase", database_id)?;
                        if response.upgrade_needed {
                            set_idb_request_result(scope, self, request_id, database.into())?;
                            let threw = dispatch_platform_event_capturing(
                                scope,
                                self,
                                request_id,
                                "upgradeneeded",
                                None,
                            )?;
                            if threw
                                && let Some(transaction_id) = transaction_id
                                && let PlatformObjectRecord::IdbTransaction(transaction) =
                                    &mut *platform_object_mut(self, transaction_id)?
                            {
                                transaction.aborted = true;
                                transaction.error = Some("AbortError".into());
                            }
                        }
                        let (aborted, abort_event_pending) = transaction_id
                            .and_then(|transaction_id| {
                                let record = platform_object(self, transaction_id).ok()?;
                                let PlatformObjectRecord::IdbTransaction(transaction) = &*record
                                else {
                                    return None;
                                };
                                Some((transaction.aborted, transaction.error.is_some()))
                            })
                            .unwrap_or((false, false));
                        if aborted {
                            let (name, old_version) = {
                                let mut record = platform_object_mut(self, database_id)?;
                                let PlatformObjectRecord::IdbDatabase(database) = &mut *record
                                else {
                                    return Err(V8BindingError::Allocation("IDBDatabase record"));
                                };
                                database.version = 0;
                                database.upgrade_transaction_id = None;
                                (database.name.clone(), database.old_version)
                            };
                            let hidden = new_idb_request(self, None);
                            submit_idb(
                                scope,
                                self,
                                hidden,
                                IdbHostCommand::AbortUpgrade { name, old_version },
                                IdbCompletionAction::Unit,
                            )?;
                            if let PlatformObjectRecord::IdbRequest(request) =
                                &mut *platform_object_mut(self, request_id)?
                            {
                                request.result = None;
                                request.error = Some("AbortError".into());
                                request.transaction_id = None;
                            }
                            if abort_event_pending && let Some(transaction_id) = transaction_id {
                                dispatch_platform_event(
                                    scope,
                                    self,
                                    transaction_id,
                                    "abort",
                                    None,
                                )?;
                            }
                            dispatch_platform_event(scope, self, request_id, "error", None)?;
                            return Ok(());
                        }
                        if let Some(transaction_id) = transaction_id {
                            if let PlatformObjectRecord::IdbTransaction(transaction) =
                                &mut *platform_object_mut(self, transaction_id)?
                            {
                                transaction.completion_requested = true;
                            }
                            queue_idb_transaction_completion(self, transaction_id)?;
                            return Ok(());
                        }
                        database.into()
                    }
                    IdbCompletionAction::DeleteDatabase | IdbCompletionAction::Unit => {
                        v8::undefined(scope).into()
                    }
                    IdbCompletionAction::Value => response
                        .value
                        .as_ref()
                        .or_else(|| response.records.first().map(|record| &record.value))
                        .map(|value| idb_json_to_value(scope, self, value))
                        .transpose()?
                        .unwrap_or_else(|| v8::undefined(scope).into()),
                    IdbCompletionAction::Key => response
                        .key
                        .as_ref()
                        .or_else(|| response.records.first().map(|record| &record.primary_key))
                        .map(|value| idb_json_to_value(scope, self, value))
                        .transpose()?
                        .unwrap_or_else(|| v8::undefined(scope).into()),
                    IdbCompletionAction::Values { count } => {
                        let values = if response.values.is_empty() {
                            response
                                .records
                                .iter()
                                .map(|record| record.value.clone())
                                .collect()
                        } else {
                            response.values
                        };
                        let limit = count.unwrap_or(values.len()).min(values.len());
                        let array = v8::Array::new(scope, limit as i32);
                        for (index, value) in values.iter().take(limit).enumerate() {
                            let value = idb_json_to_value(scope, self, value)?;
                            array.set_index(scope, index as u32, value);
                        }
                        array.into()
                    }
                    IdbCompletionAction::Keys { count } => {
                        let values = response
                            .records
                            .iter()
                            .map(|record| record.primary_key.clone())
                            .collect::<Vec<_>>();
                        let limit = count.unwrap_or(values.len()).min(values.len());
                        let array = v8::Array::new(scope, limit as i32);
                        for (index, value) in values.iter().take(limit).enumerate() {
                            let value = idb_json_to_value(scope, self, value)?;
                            array.set_index(scope, index as u32, value);
                        }
                        array.into()
                    }
                    IdbCompletionAction::Count => v8::Number::new(
                        scope,
                        response.count.unwrap_or(response.records.len() as u64) as f64,
                    )
                    .into(),
                    IdbCompletionAction::Cursor {
                        source_id,
                        direction,
                    } => {
                        if response.records.is_empty() {
                            v8::null(scope).into()
                        } else {
                            let cursor_id = register_platform_object(
                                self,
                                PlatformObjectRecord::IdbCursor(IdbCursorRecord {
                                    request_id,
                                    source_id,
                                    direction,
                                    records: response.records,
                                    index: 0,
                                    got_value: true,
                                    iteration_pending: false,
                                }),
                            );
                            new_platform_object(scope, self, "IDBCursor", cursor_id)?.into()
                        }
                    }
                };
                set_idb_request_result(scope, self, request_id, value)?;
                self.activate_idb_request_transaction(request_id)?;
                let threw =
                    dispatch_platform_event_capturing(scope, self, request_id, "success", None)?;
                if threw {
                    let transaction_id = match &*platform_object(self, request_id)? {
                        PlatformObjectRecord::IdbRequest(request) => request.transaction_id,
                        _ => None,
                    };
                    if let Some(transaction_id) = transaction_id {
                        if let PlatformObjectRecord::IdbTransaction(transaction) =
                            &mut *platform_object_mut(self, transaction_id)?
                        {
                            transaction.aborted = true;
                            transaction.error = Some("AbortError".into());
                        }
                        dispatch_platform_event(scope, self, transaction_id, "abort", None)?;
                    }
                }
                finish_idb_request(self, request_id)?;
            }
            (
                PendingPlatformOperation::IdbDatabases { resolver },
                Ok(PlatformHostResult::IndexedDb { response }),
            ) => {
                let databases = v8::Array::new(scope, response.databases.len() as i32);
                for (index, database) in response.databases.iter().enumerate() {
                    let value = v8::Object::new(scope);
                    let name = v8_string(scope, &database.name)?;
                    set_object_property(scope, value, "name", name)?;
                    let version = v8::Number::new(scope, database.version as f64).into();
                    set_object_property(scope, value, "version", version)?;
                    databases.set_index(scope, index as u32, value.into());
                }
                v8::Local::new(scope, resolver).resolve(scope, databases.into());
            }
            (PendingPlatformOperation::IdbDatabases { resolver }, Err(error)) => {
                let exception = dom_exception_value(scope, &idb_error_name(&error.to_string()))?;
                v8::Local::new(scope, resolver).reject(scope, exception);
            }
            (pending, Err(error)) => {
                if let PendingPlatformOperation::XhrStart { xhr_id }
                | PendingPlatformOperation::XhrRead { xhr_id } = &pending
                {
                    if let PlatformObjectRecord::XMLHttpRequest(xhr) =
                        &mut *platform_object_mut(self, *xhr_id)?
                    {
                        xhr.ready_state = 4;
                        xhr.status = 0;
                        xhr.active_operation = None;
                        xhr.deadline = None;
                    }
                    dispatch_platform_event(scope, self, *xhr_id, "readystatechange", None)?;
                    dispatch_platform_event(scope, self, *xhr_id, "error", None)?;
                    dispatch_platform_event(scope, self, *xhr_id, "loadend", None)?;
                    return Ok(());
                }
                let target = match &pending {
                    PendingPlatformOperation::WebSocketOpen { websocket_id }
                    | PendingPlatformOperation::WebSocketPoll { websocket_id }
                    | PendingPlatformOperation::WebSocketClose { websocket_id, .. } => {
                        Some((*websocket_id, "error"))
                    }
                    _ => None,
                };
                if let Some((id, event)) = target {
                    dispatch_platform_event(scope, self, id, event, None)?;
                }
                if let PendingPlatformOperation::IndexedDb { request_id, .. } = pending {
                    let error_name = idb_error_name(&error.to_string());
                    let transaction_id = {
                        let mut record = platform_object_mut(self, request_id)?;
                        let PlatformObjectRecord::IdbRequest(request) = &mut *record else {
                            return Err(V8BindingError::Allocation("IDBRequest record"));
                        };
                        request.error = Some(error_name.clone());
                        request.transaction_id
                    };
                    self.activate_idb_request_transaction(request_id)?;
                    let event = v8::Object::new(scope);
                    let mut threw = dispatch_platform_event_capturing(
                        scope,
                        self,
                        request_id,
                        "error",
                        Some(event),
                    )?;
                    let mut database_id = None;
                    if let Some(transaction_id) = transaction_id {
                        database_id = match &*platform_object(self, transaction_id)? {
                            PlatformObjectRecord::IdbTransaction(transaction) => {
                                Some(transaction.database_id)
                            }
                            _ => None,
                        };
                        threw |= dispatch_platform_event_capturing(
                            scope,
                            self,
                            transaction_id,
                            "error",
                            Some(event),
                        )?;
                    }
                    if let Some(database_id) = database_id {
                        threw |= dispatch_platform_event_capturing(
                            scope,
                            self,
                            database_id,
                            "error",
                            Some(event),
                        )?;
                    }
                    let default_prevented = v8::String::new(scope, "defaultPrevented")
                        .and_then(|key| event.get(scope, key.into()))
                        .is_some_and(|value| value.boolean_value(scope));
                    if let Some(transaction_id) = transaction_id
                        && (threw || !default_prevented)
                    {
                        if let PlatformObjectRecord::IdbTransaction(transaction) =
                            &mut *platform_object_mut(self, transaction_id)?
                        {
                            transaction.aborted = true;
                            transaction.error = Some(if threw {
                                "AbortError".into()
                            } else {
                                error_name
                            });
                        }
                        dispatch_platform_event(scope, self, transaction_id, "abort", None)?;
                    }
                    finish_idb_request(self, request_id)?;
                }
            }
            _ => {
                return Err(V8BindingError::Unsupported {
                    interface: "Host".into(),
                    member: "completion type mismatch".into(),
                });
            }
        }
        Ok(())
    }

    pub fn dom(&self) -> SharedDom {
        self.dom.clone()
    }

    fn activate_idb_request_transaction(&self, request_id: u64) -> Result<(), V8BindingError> {
        let transaction_id = match &*platform_object(self, request_id)? {
            PlatformObjectRecord::IdbRequest(request) => request.transaction_id,
            _ => None,
        };
        if let Some(transaction_id) = transaction_id {
            if let PlatformObjectRecord::IdbTransaction(transaction) =
                &mut *platform_object_mut(self, transaction_id)?
            {
                if transaction.aborted || transaction.completion_requested {
                    return Ok(());
                }
                transaction.active = true;
            }
            self.idb_transactions_to_deactivate
                .borrow_mut()
                .insert(transaction_id);
        }
        Ok(())
    }

    /// End the task-scoped active window after V8 has drained promise jobs.
    pub fn finish_platform_task_checkpoint(&self) -> Result<(), V8BindingError> {
        let transactions = std::mem::take(&mut *self.idb_transactions_to_deactivate.borrow_mut());
        for transaction_id in transactions {
            if let Ok(mut record) = platform_object_mut(self, transaction_id)
                && let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record
            {
                transaction.active = false;
            }
        }
        Ok(())
    }

    pub fn document_root(&self) -> NodeId {
        self.document
    }

    pub fn realm(&self) -> RealmId {
        self.realm
    }

    pub fn process_weak_releases(&self) -> Result<usize, BindingError> {
        let live_wrappers = {
            let mut objects = self.objects.borrow_mut();
            objects.retain(|_, object| !object.handle.is_empty());
            objects.keys().copied().collect::<Vec<_>>()
        };
        let mut wrappers = self.wrappers.borrow_mut();
        for wrapper in live_wrappers {
            // A callback from an older weak handle can race with recreation
            // of the realm-local wrapper. Only an ID with no current V8
            // object is eligible to release its Rust DOM lease.
            wrappers.cancel_weak_release(wrapper);
        }
        wrappers.process_weak_releases(&mut self.dom.borrow_mut())
    }

    /// Keep wrappers for connected nodes alive and release that embedder root
    /// as soon as the node becomes detached. This mirrors browser wrapper
    /// tracing: connected custom elements retain their JavaScript identity and
    /// expando state, while unreachable detached subtrees remain collectable.
    /// Returns the number of roots released so the host can request GC only
    /// when it can reclaim detached wrapper graphs.
    pub fn refresh_connected_wrapper_roots(&self, scope: &mut v8::PinScope<'_, '_>) -> usize {
        let dom = self.dom.borrow();
        let wrappers = self.wrappers.borrow();
        let mut objects = self.objects.borrow_mut();
        let mut released = 0;
        for (wrapper, object) in objects.iter_mut() {
            let connected = wrappers
                .metadata(*wrapper)
                .is_some_and(|metadata| dom.is_connected(metadata.node).unwrap_or(false));
            if connected {
                if object.connected_root.is_none()
                    && let Some(local) = object.handle.to_local(scope)
                {
                    object.connected_root = Some(v8::Global::new(scope, local));
                }
            } else if object.connected_root.take().is_some() {
                released += 1;
            }
        }
        released
    }

    pub fn set_layout(&self, layout: BindingLayout) {
        *self.layout.borrow_mut() = layout;
    }

    pub fn set_frame_document(&self, frame: NodeId, document: NodeId) {
        self.frame_roots.borrow_mut().insert(frame, document);
    }

    pub fn set_frame_window(&self, frame: NodeId, window: v8::Global<v8::Object>) {
        self.frame_windows.borrow_mut().insert(frame, window);
    }

    pub fn set_current_script(&self, script: Option<NodeId>) {
        self.current_script.set(script);
    }

    pub fn set_parser_input_active(&self, active: bool) {
        self.parser_input_active.set(active);
    }

    pub fn take_parser_input_commands(&self) -> Vec<ParserInputCommand> {
        self.parser_input_commands.borrow_mut().drain(..).collect()
    }

    pub fn configure_trusted_types(&self, config: TrustedTypesConfig) {
        *self.trusted_registry.borrow_mut() = TrustedTypesRegistry::new(config);
    }

    /// Drain scripts whose preparation was triggered by a JavaScript DOM
    /// insertion or by assigning executable content to an already-connected
    /// script element. The page orchestrator performs resource policy and
    /// execution; bindings remain the authoritative mutation boundary.
    pub fn take_pending_dynamic_scripts(&self) -> Vec<NodeId> {
        self.pending_dynamic_scripts
            .borrow_mut()
            .drain(..)
            .collect()
    }

    pub fn take_pending_dynamic_frames(&self) -> Vec<NodeId> {
        self.pending_dynamic_frames.borrow_mut().drain(..).collect()
    }

    fn dynamic_script_candidates(&self, root: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
        let candidates = {
            let dom = self.dom.borrow();
            if !dom.is_connected(root)? {
                return Ok(Vec::new());
            }
            let mut candidates = Vec::new();
            if dom
                .node_name(root)
                .is_ok_and(|name| name.eq_ignore_ascii_case("script"))
            {
                candidates.push(root);
            }
            candidates.extend(dom.query_selector_all(root, "script")?);
            candidates
                .into_iter()
                .filter(|script| {
                    dom.attribute(*script, "src")
                        .ok()
                        .flatten()
                        .is_some_and(|src| !src.trim().is_empty())
                        || dom
                            .arena()
                            .text_content(*script)
                            .is_ok_and(|source| !source.trim().is_empty())
                })
                .collect::<Vec<_>>()
        };
        Ok(candidates)
    }

    fn mark_script_subtree_inert(&self, root: NodeId) -> Result<(), V8BindingError> {
        let scripts = {
            let dom = self.dom.borrow();
            let mut scripts = Vec::new();
            if dom
                .node_name(root)
                .is_ok_and(|name| name.eq_ignore_ascii_case("script"))
            {
                scripts.push(root);
            }
            scripts.extend(dom.query_selector_all(root, "script")?);
            scripts
        };
        self.prepared_scripts.borrow_mut().extend(scripts);
        Ok(())
    }
}

fn set_idb_request_result(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    request_id: u64,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let mut record = platform_object_mut(state, request_id)?;
    let PlatformObjectRecord::IdbRequest(request) = &mut *record else {
        return Err(V8BindingError::Allocation("IDBRequest record"));
    };
    request.result = Some(v8::Global::new(scope, value));
    request.error = None;
    Ok(())
}

fn idb_error_name(error: &str) -> String {
    const NAMES: &[&str] = &[
        "AbortError",
        "ConstraintError",
        "DataCloneError",
        "DataError",
        "InvalidAccessError",
        "InvalidStateError",
        "NotFoundError",
        "QuotaExceededError",
        "ReadOnlyError",
        "TransactionInactiveError",
        "UnknownError",
        "VersionError",
    ];
    if error.contains("key already exists") || error.contains("already exists") {
        return "ConstraintError".into();
    }
    if error.contains("key is required") || error.contains("invalid IndexedDB key") {
        return "DataError".into();
    }
    if error.contains("cannot be cloned") {
        return "DataCloneError".into();
    }
    if error.contains("does not exist") {
        return "NotFoundError".into();
    }
    if error.contains("quota exceeded") {
        return "QuotaExceededError".into();
    }
    NAMES
        .iter()
        .find(|name| error.contains(**name))
        .copied()
        .unwrap_or("UnknownError")
        .to_string()
}

fn dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let object = v8::Object::new(scope);
    let name_value = v8_string(scope, name)?;
    set_object_property(scope, object, "name", name_value)?;
    let message = v8_string(scope, name)?;
    set_object_property(scope, object, "message", message)?;
    let context = scope.get_current_context();
    let global = context.global(scope);
    if let Some(constructor_key) = v8::String::new(scope, "DOMException")
        && let Some(constructor) = global.get(scope, constructor_key.into())
        && let Ok(constructor) = v8::Local::<v8::Object>::try_from(constructor)
        && let Some(prototype_key) = v8::String::new(scope, "prototype")
        && let Some(prototype) = constructor.get(scope, prototype_key.into())
    {
        object.set_prototype(scope, prototype);
    }
    Ok(object.into())
}

impl Drop for V8BindingState {
    fn drop(&mut self) {
        let _ = self
            .wrappers
            .borrow_mut()
            .release_all(&mut self.dom.borrow_mut());
    }
}

pub fn install_generated_bindings(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let document = dom.borrow().document();
    install_generated_bindings_with_root(scope, context, dom, realm, document_generation, document)
}

pub fn install_generated_bindings_with_root(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
    document: NodeId,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    install_generated_bindings_with_root_for_global(
        scope,
        context,
        dom,
        realm,
        document_generation,
        document,
        BindingGlobal::Window,
    )
}

pub fn install_generated_bindings_with_root_for_global(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
    document: NodeId,
    binding_global: BindingGlobal,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let state = Rc::new(V8BindingState {
        dom,
        realm,
        document,
        document_generation,
        wrappers: Rc::new(RefCell::new(WrapperRegistry::default())),
        objects: RefCell::new(BTreeMap::new()),
        instance_templates: RefCell::new(BTreeMap::new()),
        next_event_id: Cell::new(1),
        events: RefCell::new(BTreeMap::new()),
        listeners: RefCell::new(BTreeMap::new()),
        event_handlers: RefCell::new(BTreeMap::new()),
        next_mutation_observer_id: Cell::new(1),
        mutation_observers: RefCell::new(BTreeMap::new()),
        ranges: RefCell::new(BTreeMap::new()),
        selection_range: Cell::new(None),
        selection_anchor: Cell::new(None),
        selection_focus: Cell::new(None),
        selection_object: RefCell::new(None),
        implementation_object: RefCell::new(None),
        history: RefCell::new(None),
        storage_fallback: RefCell::new([BTreeMap::new(), BTreeMap::new()]),
        next_platform_object_id: Cell::new(1),
        platform_objects: RefCell::new(BTreeMap::new()),
        platform_wrappers: RefCell::new(BTreeMap::new()),
        platform_host: RefCell::new(None),
        next_platform_operation_id: Cell::new(1),
        pending_platform_operations: RefCell::new(BTreeMap::new()),
        queued_idb_operations: RefCell::new(VecDeque::new()),
        deferred_platform_tasks: RefCell::new(VecDeque::new()),
        cancelled_platform_operations: RefCell::new(BTreeSet::new()),
        platform_listeners: RefCell::new(BTreeMap::new()),
        platform_handlers: RefCell::new(BTreeMap::new()),
        blob_urls: RefCell::new(BTreeMap::new()),
        next_blob_url_id: Cell::new(1),
        idb_transactions_to_deactivate: RefCell::new(BTreeSet::new()),
        node_documents: RefCell::new(BTreeMap::from([(document, document)])),
        next_traversal_id: Cell::new(1),
        traversals: RefCell::new(BTreeMap::new()),
        layout: RefCell::new(BindingLayout::default()),
        frame_roots: RefCell::new(BTreeMap::new()),
        frame_windows: RefCell::new(BTreeMap::new()),
        template_contents: RefCell::new(BTreeMap::new()),
        current_script: Cell::new(None),
        parser_input_active: Cell::new(false),
        parser_input_commands: RefCell::new(VecDeque::new()),
        document_open: Cell::new(false),
        design_mode: Cell::new(false),
        style_with_css: Cell::new(false),
        document_write_buffer: RefCell::new(String::new()),
        document_write_depth: Cell::new(0),
        pending_dynamic_scripts: RefCell::new(VecDeque::new()),
        prepared_scripts: RefCell::new(BTreeSet::new()),
        pending_dynamic_frames: RefCell::new(VecDeque::new()),
        prepared_frames: RefCell::new(BTreeSet::new()),
        trusted_registry: RefCell::new(TrustedTypesRegistry::new(TrustedTypesConfig {
            allow_any_policy: true,
            ..TrustedTypesConfig::default()
        })),
        trusted_policies: RefCell::new(BTreeMap::new()),
        trusted_values: RefCell::new(BTreeMap::new()),
        next_trusted_id: Cell::new(1),
    });
    context.set_slot(state.clone());
    if scope.get_slot::<BindingStateRegistry>().is_none() {
        scope.set_slot(BindingStateRegistry::default());
    }
    scope
        .get_slot::<BindingStateRegistry>()
        .expect("binding-state registry was installed")
        .realms
        .borrow_mut()
        .insert(realm, Rc::downgrade(&state));

    let mut templates = BTreeMap::<&'static str, v8::Global<v8::FunctionTemplate>>::new();
    let mut remaining = GENERATED_INTERFACES.iter().enumerate().collect::<Vec<_>>();
    while !remaining.is_empty() {
        let before = remaining.len();
        let mut index = 0;
        while index < remaining.len() {
            let (interface_index, interface) = remaining[index];
            if interface
                .parent
                .is_some_and(|parent| !templates.contains_key(parent))
            {
                index += 1;
                continue;
            }
            let constructor_data = v8::Integer::new(scope, interface_index as i32);
            let template = v8::FunctionTemplate::builder(generated_constructor)
                .data(constructor_data.into())
                .build(scope);
            template.set_class_name(
                v8::String::new(scope, interface.name)
                    .ok_or(V8BindingError::Allocation("interface name"))?,
            );
            if let Some(parent) = interface.parent {
                let parent = v8::Local::new(
                    scope,
                    templates
                        .get(parent)
                        .ok_or(V8BindingError::MissingInterface(parent))?,
                );
                template.inherit(parent);
            }
            let instance = template.instance_template(scope);
            // DOM wrappers carry the realm-local wrapper lease plus the
            // generation-checked Rust identity required for same-origin
            // cross-realm argument conversion. Stateful non-node interfaces
            // use only field zero.
            instance.set_internal_field_count(5);
            let prototype = template.prototype_template(scope);
            for (member_position, member) in interface.members.iter().enumerate() {
                if interface.members[..member_position]
                    .iter()
                    .any(|previous| previous.name == member.name && previous.kind == member.kind)
                {
                    continue;
                }
                let data = v8::Integer::new(scope, member.id.0 as i32);
                let name = v8::String::new(scope, member.name)
                    .ok_or(V8BindingError::Allocation("member name"))?;
                match member.kind {
                    GeneratedMemberKind::Attribute => {
                        let getter = v8::FunctionTemplate::builder(generated_attribute_getter)
                            .data(data.into())
                            .length(0)
                            .constructor_behavior(v8::ConstructorBehavior::Throw)
                            .build(scope);
                        let setter = (!member.readonly
                            || member.replaceable
                            || member.put_forwards.is_some())
                        .then(|| {
                            v8::FunctionTemplate::builder(generated_attribute_setter)
                                .data(data.into())
                                .length(1)
                                .constructor_behavior(v8::ConstructorBehavior::Throw)
                                .build(scope)
                        });
                        let target = if member.unforgeable {
                            instance
                        } else {
                            prototype
                        };
                        target.set_accessor_property(
                            name.into(),
                            Some(getter),
                            setter,
                            v8::PropertyAttribute::NONE,
                        );
                    }
                    GeneratedMemberKind::Operation => {
                        let arity = interface
                            .members
                            .iter()
                            .filter(|overload| {
                                overload.kind == GeneratedMemberKind::Operation
                                    && overload.name == member.name
                            })
                            .map(|overload| overload.arity)
                            .min()
                            .unwrap_or(member.arity);
                        let function = v8::FunctionTemplate::builder(generated_operation)
                            .data(data.into())
                            .length(arity.into())
                            .constructor_behavior(v8::ConstructorBehavior::Throw)
                            .build(scope);
                        prototype.set(name.into(), function.into());
                    }
                    GeneratedMemberKind::Constant => {
                        let value = generated_constant_value(
                            scope,
                            member
                                .constant_value
                                .ok_or(V8BindingError::Allocation("WebIDL constant value"))?,
                        )?;
                        prototype.set(name.into(), value.into());
                    }
                    GeneratedMemberKind::Iterable => {
                        let function = v8::FunctionTemplate::builder(generated_operation)
                            .data(data.into())
                            .length(0)
                            .constructor_behavior(v8::ConstructorBehavior::Throw)
                            .build(scope);
                        prototype.set(v8::Symbol::get_iterator(scope).into(), function.into());
                    }
                    GeneratedMemberKind::Constructor => {}
                }
            }
            let constructor = template
                .get_function(scope)
                .ok_or(V8BindingError::Allocation("interface constructor"))?;
            for member in interface
                .members
                .iter()
                .filter(|member| member.kind == GeneratedMemberKind::Constant)
            {
                let name = v8::String::new(scope, member.name)
                    .ok_or(V8BindingError::Allocation("WebIDL constant name"))?;
                let value = generated_constant_value(
                    scope,
                    member
                        .constant_value
                        .ok_or(V8BindingError::Allocation("WebIDL constant value"))?,
                )?;
                constructor.create_data_property(scope, name.into(), value);
            }
            if interface.exposed.contains(&binding_global.webidl_name()) {
                let global = context.global(scope);
                global.set(
                    scope,
                    v8::String::new(scope, interface.name).unwrap().into(),
                    constructor.into(),
                );
            }
            state
                .instance_templates
                .borrow_mut()
                .insert(interface.name, v8::Global::new(scope, instance));
            templates.insert(interface.name, v8::Global::new(scope, template));
            remaining.remove(index);
        }
        if before == remaining.len() {
            return Err(V8BindingError::MissingInterface(
                remaining[0].1.parent.unwrap_or(remaining[0].1.name),
            ));
        }
    }

    let wrapper = wrap_node(scope, &state, document)?;
    let name = v8::String::new(scope, "document").ok_or(V8BindingError::Allocation("document"))?;
    context
        .global(scope)
        .set(scope, name.into(), wrapper.into());
    publish_named_window_properties(scope, context, &state, document)?;
    if binding_global == BindingGlobal::Window {
        let history = new_stateful_object(scope, &state, "History", 1)?;
        let name =
            v8::String::new(scope, "history").ok_or(V8BindingError::Allocation("history name"))?;
        context
            .global(scope)
            .set(scope, name.into(), history.into());
        for (name, id) in [("localStorage", 1), ("sessionStorage", 2)] {
            let storage = new_stateful_object(scope, &state, "Storage", id)?;
            let name =
                v8::String::new(scope, name).ok_or(V8BindingError::Allocation("storage name"))?;
            context
                .global(scope)
                .set(scope, name.into(), storage.into());
        }
        let factory_id = register_platform_object(&state, PlatformObjectRecord::IdbFactory);
        let factory = new_platform_object(scope, &state, "IDBFactory", factory_id)?;
        let name = v8::String::new(scope, "indexedDB")
            .ok_or(V8BindingError::Allocation("indexedDB name"))?;
        context
            .global(scope)
            .set(scope, name.into(), factory.into());
        install_idb_key_range_statics(scope, context)?;
        let restore = v8::Function::new(scope, restore_storage_callback)
            .ok_or(V8BindingError::Allocation("storage restore function"))?;
        let name = v8::String::new(scope, "__oryn_restore_storage")
            .ok_or(V8BindingError::Allocation("storage restore name"))?;
        context
            .global(scope)
            .set(scope, name.into(), restore.into());
    }
    install_response_statics(scope, context)?;
    let get_computed_style = v8::Function::new(scope, get_computed_style_callback)
        .ok_or(V8BindingError::Allocation("getComputedStyle function"))?;
    let name = v8::String::new(scope, "getComputedStyle")
        .ok_or(V8BindingError::Allocation("getComputedStyle name"))?;
    context
        .global(scope)
        .set(scope, name.into(), get_computed_style.into());
    install_trusted_types(scope, context)?;
    let fetch = v8::Function::new(scope, platform_fetch_callback)
        .ok_or(V8BindingError::Allocation("fetch function"))?;
    let name = v8::String::new(scope, "fetch").ok_or(V8BindingError::Allocation("fetch name"))?;
    context.global(scope).set(scope, name.into(), fetch.into());
    let create_object_url = v8::Function::new(scope, blob_url_create_callback)
        .ok_or(V8BindingError::Allocation("Blob URL create function"))?;
    let name = v8::String::new(scope, "__oryn_blob_url_create")
        .ok_or(V8BindingError::Allocation("Blob URL create name"))?;
    context
        .global(scope)
        .set(scope, name.into(), create_object_url.into());
    let revoke_object_url = v8::Function::new(scope, blob_url_revoke_callback)
        .ok_or(V8BindingError::Allocation("Blob URL revoke function"))?;
    let name = v8::String::new(scope, "__oryn_blob_url_revoke")
        .ok_or(V8BindingError::Allocation("Blob URL revoke name"))?;
    context
        .global(scope)
        .set(scope, name.into(), revoke_object_url.into());
    Ok(state)
}

pub fn publish_named_window_properties(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<(), V8BindingError> {
    let nodes = state.dom.borrow().query_selector_all(document, "*")?;
    for node in nodes {
        let names = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            if !matches!(record.kind, NodeKind::Element { .. }) {
                continue;
            }
            [dom.attribute(node, "id")?, dom.attribute(node, "name")?]
                .into_iter()
                .flatten()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        };
        for name in names {
            let key = v8::String::new(scope, &name)
                .ok_or(V8BindingError::Allocation("named window property"))?;
            let global = context.global(scope);
            if global.has_own_property(scope, key.into()).unwrap_or(false) {
                continue;
            }
            let wrapper = wrap_node(scope, state, node)?;
            global.create_data_property(scope, key.into(), wrapper.into());
        }
    }
    Ok(())
}

fn generated_constant_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    raw: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if let Ok(value) = raw.parse::<i32>() {
        return Ok(v8::Integer::new(scope, value).into());
    }
    if let Ok(value) = raw.parse::<f64>() {
        return Ok(v8::Number::new(scope, value).into());
    }
    match raw {
        "true" => Ok(v8::Boolean::new(scope, true).into()),
        "false" => Ok(v8::Boolean::new(scope, false).into()),
        _ => {
            let value = raw
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(raw);
            Ok(v8::String::new(scope, value)
                .ok_or(V8BindingError::Allocation("WebIDL string constant"))?
                .into())
        }
    }
}

fn install_idb_key_range_statics(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, "IDBKeyRange")
        .ok_or(V8BindingError::Allocation("IDBKeyRange name"))?;
    let constructor = context
        .global(scope)
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or(V8BindingError::MissingInterface("IDBKeyRange"))?;
    for (index, member) in ["only", "lowerBound", "upperBound", "bound"]
        .into_iter()
        .enumerate()
    {
        let callback = v8::Function::builder(idb_key_range_static)
            .data(v8::Integer::new(scope, index as i32).into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("IDBKeyRange static"))?;
        let name = v8::String::new(scope, member)
            .ok_or(V8BindingError::Allocation("IDBKeyRange static name"))?;
        constructor.create_data_property(scope, name.into(), callback.into());
    }
    Ok(())
}

fn install_response_statics(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let name =
        v8::String::new(scope, "Response").ok_or(V8BindingError::Allocation("Response name"))?;
    let constructor = context
        .global(scope)
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or(V8BindingError::MissingInterface("Response"))?;
    let callback = v8::Function::new(scope, response_json_static)
        .ok_or(V8BindingError::Allocation("Response.json static"))?;
    let name =
        v8::String::new(scope, "json").ok_or(V8BindingError::Allocation("Response.json name"))?;
    constructor.create_data_property(scope, name.into(), callback.into());
    Ok(())
}

fn response_json_static<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut return_value: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("Response state"))?;
        let body = v8::json::stringify(scope, args.get(0))
            .ok_or(V8BindingError::Allocation("Response JSON serialization"))?
            .to_rust_string_lossy(scope)
            .into_bytes();
        let init = args.get(1);
        let status = option_value(scope, init, "status")
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(200) as u16;
        let headers_id = option_value(scope, init, "headers")
            .map(|value| headers_from_value(scope, &state, value))
            .transpose()?
            .unwrap_or_else(|| {
                register_platform_object(
                    &state,
                    PlatformObjectRecord::Headers(HeadersRecord::default()),
                )
            });
        {
            let mut record = platform_object_mut(&state, headers_id)?;
            let PlatformObjectRecord::Headers(headers) = &mut *record else {
                return Err(V8BindingError::Allocation("Response Headers"));
            };
            if !headers
                .entries
                .iter()
                .any(|(name, _)| name == "content-type")
            {
                append_header(headers, "content-type".into(), "application/json".into())?;
            }
        }
        let body_stream_id = Some(new_byte_stream_record(scope, &state, body.clone())?);
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::Response(ResponseRecord {
                status,
                url: String::new(),
                headers_id,
                body_stream_id,
                bytes: body,
                body_used: false,
                signal_id: None,
            }),
        );
        new_platform_object(scope, &state, "Response", id)
    })();
    match result {
        Ok(value) => return_value.set(value.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn idb_key_range_static(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("IDB state"))?;
        let mode = args.data().int32_value(scope).unwrap_or(0);
        let first = idb_key_from_value(scope, args.get(0)).map(idb_key_to_json)?;
        let (lower, upper, lower_open, upper_open) = match mode {
            0 => (Some(first.clone()), Some(first), false, false),
            1 => (Some(first), None, args.get(1).boolean_value(scope), false),
            2 => (None, Some(first), false, args.get(1).boolean_value(scope)),
            _ => {
                let second = idb_key_from_value(scope, args.get(1)).map(idb_key_to_json)?;
                (
                    Some(first),
                    Some(second),
                    args.get(2).boolean_value(scope),
                    args.get(3).boolean_value(scope),
                )
            }
        };
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::IdbKeyRange(IdbKeyRangeRecord {
                lower,
                upper,
                lower_open,
                upper_open,
            }),
        );
        new_platform_object(scope, &state, "IDBKeyRange", id)
    })();
    match result {
        Ok(value) => return_value.set(value.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn idb_value_to_json(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
) -> Result<serde_json::Value, V8BindingError> {
    idb_value_to_json_inner(scope, state, value, 0)
}

#[derive(Debug, Clone, PartialEq)]
enum IdbKey {
    Number(f64),
    Date(f64),
    String(String),
    Binary(Vec<u8>),
    Array(Vec<IdbKey>),
}

fn idb_key_from_value(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<IdbKey, V8BindingError> {
    let data_error = || V8BindingError::Unsupported {
        interface: "IndexedDB".into(),
        member: "DataError".into(),
    };
    if value.is_number() {
        let value = value.number_value(scope).ok_or_else(data_error)?;
        return value
            .is_finite()
            .then_some(IdbKey::Number(value))
            .ok_or_else(data_error);
    }
    if value.is_date() {
        let value = v8::Local::<v8::Date>::try_from(value)
            .map_err(|_| data_error())?
            .value_of();
        return value
            .is_finite()
            .then_some(IdbKey::Date(value))
            .ok_or_else(data_error);
    }
    if value.is_string() {
        return Ok(IdbKey::String(value.to_rust_string_lossy(scope)));
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        return Ok(IdbKey::Binary(bytes_from_array_buffer(buffer)));
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        view.copy_contents(&mut bytes);
        return Ok(IdbKey::Binary(bytes));
    }
    if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
        let mut keys = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            let item = array.get_index(scope, index).ok_or_else(data_error)?;
            keys.push(idb_key_from_value(scope, item)?);
        }
        return Ok(IdbKey::Array(keys));
    }
    Err(data_error())
}

fn compare_idb_keys(left: &IdbKey, right: &IdbKey) -> std::cmp::Ordering {
    fn rank(key: &IdbKey) -> u8 {
        match key {
            IdbKey::Number(_) => 0,
            IdbKey::Date(_) => 1,
            IdbKey::String(_) => 2,
            IdbKey::Binary(_) => 3,
            IdbKey::Array(_) => 4,
        }
    }
    let type_order = rank(left).cmp(&rank(right));
    if type_order != std::cmp::Ordering::Equal {
        return type_order;
    }
    match (left, right) {
        (IdbKey::Number(left), IdbKey::Number(right))
        | (IdbKey::Date(left), IdbKey::Date(right)) => {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        }
        (IdbKey::String(left), IdbKey::String(right)) => left.cmp(right),
        (IdbKey::Binary(left), IdbKey::Binary(right)) => left.cmp(right),
        (IdbKey::Array(left), IdbKey::Array(right)) => {
            for (left, right) in left.iter().zip(right) {
                let ordering = compare_idb_keys(left, right);
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        _ => std::cmp::Ordering::Equal,
    }
}

fn idb_key_to_json(key: IdbKey) -> serde_json::Value {
    match key {
        IdbKey::Number(value) | IdbKey::Date(value) => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        IdbKey::String(value) => serde_json::Value::String(value),
        IdbKey::Binary(bytes) => serde_json::json!({
            "__oryn_idb_key_type": "binary",
            "bytes": bytes,
        }),
        IdbKey::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(idb_key_to_json).collect())
        }
    }
}

fn idb_query_to_json(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
) -> Result<serde_json::Value, V8BindingError> {
    if let Some(object) = value.to_object(scope)
        && let Ok(id) = stateful_id(scope, object)
        && matches!(
            &*platform_object(state, id)?,
            PlatformObjectRecord::IdbKeyRange(_)
        )
    {
        return idb_value_to_json(scope, state, value);
    }
    idb_key_from_value(scope, value).map(idb_key_to_json)
}

fn compare_idb_json_keys(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> std::cmp::Ordering {
    fn rank(value: &serde_json::Value) -> u8 {
        match value {
            serde_json::Value::Number(_) => 0,
            serde_json::Value::String(_) => 2,
            serde_json::Value::Object(object)
                if object
                    .get("__oryn_idb_key_type")
                    .and_then(|value| value.as_str())
                    == Some("binary") =>
            {
                3
            }
            serde_json::Value::Array(_) => 4,
            _ => 5,
        }
    }
    let ranked = rank(left).cmp(&rank(right));
    if !ranked.is_eq() {
        return ranked;
    }
    match (left, right) {
        (serde_json::Value::Number(left), serde_json::Value::Number(right)) => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (serde_json::Value::String(left), serde_json::Value::String(right)) => left.cmp(right),
        (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
            for (left, right) in left.iter().zip(right) {
                let ordering = compare_idb_json_keys(left, right);
                if !ordering.is_eq() {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        _ => left.to_string().cmp(&right.to_string()),
    }
}

fn idb_json_key_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

#[cfg(test)]
fn is_valid_idb_json_key(value: &serde_json::Value) -> bool {
    normalize_idb_json_key(value).is_some()
}

fn normalize_idb_json_key(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Number(value) if value.as_f64().is_some_and(f64::is_finite) => {
            Some(serde_json::Value::Number(value.clone()))
        }
        serde_json::Value::String(value) => Some(serde_json::Value::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .map(normalize_idb_json_key)
            .collect::<Option<Vec<_>>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(object)
            if matches!(
                object
                    .get("__oryn_clone_type")
                    .and_then(serde_json::Value::as_str),
                Some("array_buffer" | "typed_array")
            ) =>
        {
            Some(serde_json::json!({
                "__oryn_idb_key_type":"binary",
                "bytes":object.get("bytes").cloned().unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
            }))
        }
        _ => None,
    }
}

fn idb_value_to_json_inner(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
    depth: u8,
) -> Result<serde_json::Value, V8BindingError> {
    if depth >= 64 {
        return Err(V8BindingError::Unsupported {
            interface: "IndexedDB".into(),
            member: "DataCloneError".into(),
        });
    }
    if value.is_undefined() {
        return Ok(serde_json::Value::Null);
    }
    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if value.is_boolean() {
        return Ok(serde_json::Value::Bool(value.boolean_value(scope)));
    }
    if value.is_number() {
        return value
            .number_value(scope)
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .ok_or(V8BindingError::Unsupported {
                interface: "IndexedDB".into(),
                member: "DataCloneError".into(),
            });
    }
    if value.is_string() {
        return Ok(serde_json::Value::String(value.to_rust_string_lossy(scope)));
    }
    if let Some(object) = value.to_object(scope)
        && let Ok(id) = stateful_id(scope, object)
        && let Ok(record) = platform_object(state, id)
    {
        if let PlatformObjectRecord::Blob(blob) = &*record {
            return Ok(
                serde_json::json!({"__oryn_clone_type":"blob","bytes":blob.bytes,"type":blob.media_type,"name":blob.file.as_ref().map(|file| &file.name),"lastModified":blob.file.as_ref().map(|file| file.last_modified)}),
            );
        }
        if let PlatformObjectRecord::IdbKeyRange(range) = &*record {
            return Ok(
                serde_json::json!({"__oryn_key_range":true,"lower":range.lower,"upper":range.upper,"lower_open":range.lower_open,"upper_open":range.upper_open}),
            );
        }
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        return Ok(serde_json::json!({
            "__oryn_clone_type":"array_buffer",
            "bytes":bytes_from_array_buffer(buffer),
        }));
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        view.copy_contents(&mut bytes);
        let object = value
            .to_object(scope)
            .ok_or(V8BindingError::Allocation("typed array object"))?;
        let constructor = option_value(scope, object.into(), "constructor")
            .and_then(|value| option_value(scope, value, "name"))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "Uint8Array".into());
        let length = option_value(scope, object.into(), "length")
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(view.byte_length() as u32);
        let values = (0..length)
            .map(|index| {
                object
                    .get_index(scope, index)
                    .ok_or(V8BindingError::Allocation("typed array element"))
                    .and_then(|value| idb_value_to_json_inner(scope, state, value, depth + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(serde_json::json!({
            "__oryn_clone_type":"typed_array",
            "name":constructor,
            "values":values,
            "bytes":bytes,
        }));
    }
    if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
        return (0..array.length())
            .map(|index| {
                array
                    .get_index(scope, index)
                    .ok_or(V8BindingError::Allocation("IndexedDB array element"))
                    .and_then(|value| idb_value_to_json_inner(scope, state, value, depth + 1))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array);
    }
    if let Some(object) = value.to_object(scope) {
        let names = object
            .get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
            .ok_or(V8BindingError::Unsupported {
                interface: "IndexedDB".into(),
                member: "DataCloneError".into(),
            })?;
        let mut result = serde_json::Map::new();
        for index in 0..names.length() {
            let key = names
                .get_index(scope, index)
                .ok_or(V8BindingError::Allocation("IndexedDB property name"))?;
            let key_string = key.to_rust_string_lossy(scope);
            let child = object
                .get(scope, key)
                .ok_or(V8BindingError::Allocation("IndexedDB property"))?;
            result.insert(
                key_string,
                idb_value_to_json_inner(scope, state, child, depth + 1)?,
            );
        }
        return Ok(serde_json::Value::Object(result));
    }
    Err(V8BindingError::Unsupported {
        interface: "IndexedDB".into(),
        member: "DataCloneError".into(),
    })
}

fn idb_json_to_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    value: &serde_json::Value,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if value
        .get("__oryn_clone_type")
        .and_then(serde_json::Value::as_str)
        == Some("blob")
    {
        let bytes = value
            .get("bytes")
            .and_then(serde_json::Value::as_array)
            .map(|bytes| {
                bytes
                    .iter()
                    .filter_map(serde_json::Value::as_u64)
                    .map(|byte| byte as u8)
                    .collect()
            })
            .unwrap_or_default();
        let file = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| FileRecord {
                name: name.into(),
                last_modified: value
                    .get("lastModified")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
            });
        let interface = if file.is_some() { "File" } else { "Blob" };
        let id = register_platform_object(
            state,
            PlatformObjectRecord::Blob(BlobRecord {
                bytes,
                media_type: value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .into(),
                file,
            }),
        );
        return Ok(new_platform_object(scope, state, interface, id)?.into());
    }
    if value
        .get("__oryn_clone_type")
        .and_then(serde_json::Value::as_str)
        == Some("array_buffer")
    {
        let bytes = json_bytes(value.get("bytes"));
        return Ok(array_buffer_from_bytes(scope, &bytes).into());
    }
    if value
        .get("__oryn_clone_type")
        .and_then(serde_json::Value::as_str)
        == Some("typed_array")
    {
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Uint8Array");
        let values = value
            .get("values")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let array = v8::Array::new(scope, values.len() as i32);
        for (index, value) in values.iter().enumerate() {
            let value = idb_json_to_value(scope, state, value)?;
            array.set_index(scope, index as u32, value);
        }
        let constructor = scope
            .get_current_context()
            .global(scope)
            .get(
                scope,
                v8::String::new(scope, name)
                    .ok_or(V8BindingError::Allocation("typed array name"))?
                    .into(),
            )
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or(V8BindingError::Allocation("typed array constructor"))?;
        if name == "DataView" {
            let bytes = json_bytes(value.get("bytes"));
            let buffer = array_buffer_from_bytes(scope, &bytes);
            return constructor
                .new_instance(scope, &[buffer.into()])
                .map(Into::into)
                .ok_or(V8BindingError::Allocation("DataView clone"));
        }
        return constructor
            .new_instance(scope, &[array.into()])
            .map(Into::into)
            .ok_or(V8BindingError::Allocation("typed array clone"));
    }
    if value
        .get("__oryn_idb_key_type")
        .and_then(serde_json::Value::as_str)
        == Some("binary")
    {
        let bytes = json_bytes(value.get("bytes"));
        return Ok(array_buffer_from_bytes(scope, &bytes).into());
    }
    match value {
        serde_json::Value::Null => Ok(v8::null(scope).into()),
        serde_json::Value::Bool(value) => Ok(v8::Boolean::new(scope, *value).into()),
        serde_json::Value::Number(value) => Ok(v8::Number::new(
            scope,
            value
                .as_f64()
                .ok_or(V8BindingError::Allocation("IndexedDB number"))?,
        )
        .into()),
        serde_json::Value::String(value) => v8_string(scope, value),
        serde_json::Value::Array(values) => {
            let array = v8::Array::new(scope, values.len() as i32);
            for (index, value) in values.iter().enumerate() {
                let value = idb_json_to_value(scope, state, value)?;
                let key = v8::String::new(scope, &index.to_string())
                    .ok_or(V8BindingError::Allocation("IndexedDB array index"))?;
                array.create_data_property(scope, key.into(), value);
            }
            Ok(array.into())
        }
        serde_json::Value::Object(values) => {
            let object = v8::Object::new(scope);
            for (name, value) in values {
                let name = v8::String::new(scope, name)
                    .ok_or(V8BindingError::Allocation("IndexedDB property name"))?;
                let value = idb_json_to_value(scope, state, value)?;
                object.create_data_property(scope, name.into(), value);
            }
            Ok(object.into())
        }
    }
}

fn json_bytes(value: Option<&serde_json::Value>) -> Vec<u8> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|bytes| {
            bytes
                .iter()
                .filter_map(serde_json::Value::as_u64)
                .map(|byte| byte as u8)
                .collect()
        })
        .unwrap_or_default()
}

fn current_platform_origin(scope: &mut v8::PinScope) -> String {
    let global = scope.get_current_context().global(scope);
    let location = v8::String::new(scope, "location")
        .and_then(|key| global.get(scope, key.into()))
        .and_then(|value| value.to_object(scope));
    location
        .and_then(|location| {
            v8::String::new(scope, "origin").and_then(|key| location.get(scope, key.into()))
        })
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "null".into())
}

fn new_idb_request(state: &V8BindingState, transaction_id: Option<u64>) -> u64 {
    new_idb_request_for_source(state, transaction_id, None)
}

fn new_idb_request_for_source(
    state: &V8BindingState,
    transaction_id: Option<u64>,
    source_id: Option<u64>,
) -> u64 {
    register_platform_object(
        state,
        PlatformObjectRecord::IdbRequest(IdbRequestRecord {
            result: None,
            error: None,
            transaction_id,
            source_id,
            done: false,
        }),
    )
}

fn submit_idb(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    request_id: u64,
    command: IdbHostCommand,
    action: IdbCompletionAction,
) -> Result<(), V8BindingError> {
    let request = PlatformHostRequest::IndexedDb {
        origin: current_platform_origin(scope),
        command,
    };
    let pending = PendingPlatformOperation::IndexedDb { request_id, action };
    if state.pending_platform_operations.borrow().len() >= 32 {
        let mut queue = state.queued_idb_operations.borrow_mut();
        if queue.len() >= 1024 {
            return Err(V8BindingError::Unsupported {
                interface: "IndexedDB".into(),
                member: "request queue limit exceeded".into(),
            });
        }
        queue.push_back(QueuedIdbOperation { request, pending });
    } else {
        submit_platform_operation(state, request, pending)?;
    }
    let transaction_id = match &*platform_object(state, request_id)? {
        PlatformObjectRecord::IdbRequest(request) => request.transaction_id,
        _ => None,
    };
    if let Some(transaction_id) = transaction_id
        && let PlatformObjectRecord::IdbTransaction(transaction) =
            &mut *platform_object_mut(state, transaction_id)?
    {
        transaction.pending_requests = transaction.pending_requests.saturating_add(1);
    }
    Ok(())
}

fn finish_idb_request(state: &V8BindingState, request_id: u64) -> Result<(), V8BindingError> {
    let transaction_id = match &*platform_object(state, request_id)? {
        PlatformObjectRecord::IdbRequest(request) => request.transaction_id,
        _ => None,
    };
    let Some(transaction_id) = transaction_id else {
        return Ok(());
    };
    if let PlatformObjectRecord::IdbTransaction(transaction) =
        &mut *platform_object_mut(state, transaction_id)?
    {
        transaction.pending_requests = transaction.pending_requests.saturating_sub(1);
        if transaction.mode != "versionchange" && transaction.pending_requests == 0 {
            transaction.completion_requested = true;
        }
    }
    queue_idb_transaction_completion(state, transaction_id)
}

fn queue_idb_transaction_completion(
    state: &V8BindingState,
    transaction_id: u64,
) -> Result<(), V8BindingError> {
    let completion = {
        let mut record = platform_object_mut(state, transaction_id)?;
        let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record else {
            return Err(V8BindingError::Allocation("IDBTransaction record"));
        };
        if transaction.aborted
            || !transaction.completion_requested
            || transaction.completion_queued
            || transaction.pending_requests != 0
        {
            None
        } else {
            transaction.completion_queued = true;
            Some((transaction.request_id, transaction.database_id))
        }
    };
    let Some((request_id, database_id)) = completion else {
        return Ok(());
    };
    let mut tasks = state.deferred_platform_tasks.borrow_mut();
    tasks.push_back(DeferredPlatformTask::Event {
        target_id: transaction_id,
        event_type: "complete",
    });
    if let Some(request_id) = request_id {
        tasks.push_back(DeferredPlatformTask::FinalizeIdbOpen {
            request_id,
            database_id,
            transaction_id,
        });
    }
    Ok(())
}

fn install_trusted_types(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let factory = v8::Object::new(scope);
    set_generated_prototype(scope, context, factory, "TrustedTypePolicyFactory")?;
    macro_rules! install {
        ($name:literal, $callback:ident) => {{
            let function = v8::Function::new(scope, $callback)
                .ok_or(V8BindingError::Allocation("Trusted Types function"))?;
            let name = v8::String::new(scope, $name)
                .ok_or(V8BindingError::Allocation("Trusted Types member name"))?;
            factory.create_data_property(scope, name.into(), function.into());
        }};
    }
    install!("createPolicy", trusted_create_policy);
    install!("isHTML", trusted_is_html);
    install!("isScript", trusted_is_script);
    install!("isScriptURL", trusted_is_script_url);
    install!("getAttributeType", trusted_get_attribute_type);
    install!("getPropertyType", trusted_get_property_type);
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return Err(V8BindingError::Allocation("Trusted Types state"));
    };
    let empty_html = make_trusted_value(
        scope,
        &state,
        TrustedKind::Html,
        String::new(),
        String::new(),
    )?;
    let empty_script = make_trusted_value(
        scope,
        &state,
        TrustedKind::Script,
        String::new(),
        String::new(),
    )?;
    for (name, value) in [("emptyHTML", empty_html), ("emptyScript", empty_script)] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("Trusted Types constant name"))?;
        factory.create_data_property(scope, key.into(), value.into());
    }
    let name = v8::String::new(scope, "trustedTypes")
        .ok_or(V8BindingError::Allocation("trustedTypes global"))?;
    context
        .global(scope)
        .create_data_property(scope, name.into(), factory.into());
    Ok(())
}

fn trusted_create_policy(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "Trusted Types state is unavailable");
        return;
    };
    let name = args.get(0).to_rust_string_lossy(scope);
    if let Err(error) = state.trusted_registry.borrow_mut().create_policy(&name) {
        throw_type_error(scope, &error.to_string());
        return;
    }
    let options = args.get(1).to_object(scope);
    let callback = |scope: &mut v8::PinScope, name: &str| {
        let options = options?;
        let key = v8::String::new(scope, name)?;
        v8::Local::<v8::Function>::try_from(options.get(scope, key.into())?)
            .ok()
            .map(|function| v8::Global::new(scope, function))
    };
    let policy_id = state.next_trusted_id.get();
    state.next_trusted_id.set(policy_id.saturating_add(1));
    state.trusted_policies.borrow_mut().insert(
        policy_id,
        TrustedPolicyCallbacks {
            name: name.clone(),
            create_html: callback(scope, "createHTML"),
            create_script: callback(scope, "createScript"),
            create_script_url: callback(scope, "createScriptURL"),
        },
    );
    let policy = v8::Object::new(scope);
    if let Err(error) = set_generated_prototype(
        scope,
        scope.get_current_context(),
        policy,
        "TrustedTypePolicy",
    ) {
        throw_type_error(scope, &error.to_string());
        return;
    }
    if let (Some(key), Some(value)) = (
        v8::String::new(scope, "name"),
        v8::String::new(scope, &name),
    ) {
        policy.create_data_property(scope, key.into(), value.into());
    }
    for (member, kind) in [
        ("createHTML", TrustedKind::Html),
        ("createScript", TrustedKind::Script),
        ("createScriptURL", TrustedKind::ScriptUrl),
    ] {
        let token = policy_id.saturating_mul(4).saturating_add(kind_token(kind));
        let data = v8::BigInt::new_from_u64(scope, token);
        let function = v8::Function::builder(trusted_policy_create)
            .data(data.into())
            .build(scope)
            .expect("allocate Trusted Types policy function");
        let key = v8::String::new(scope, member).expect("allocate Trusted Types method name");
        policy.create_data_property(scope, key.into(), function.into());
    }
    return_value.set(policy.into());
}

fn trusted_policy_create(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Some(token) = exact_u64(args.data()) else {
        throw_type_error(scope, "invalid Trusted Types policy token");
        return;
    };
    let policy_id = token / 4;
    let kind = match token % 4 {
        1 => TrustedKind::Html,
        2 => TrustedKind::Script,
        3 => TrustedKind::ScriptUrl,
        _ => return,
    };
    let (policy_name, callback) = {
        let policies = state.trusted_policies.borrow();
        let Some(policy) = policies.get(&policy_id) else {
            throw_type_error(scope, "unknown Trusted Types policy");
            return;
        };
        let callback = match kind {
            TrustedKind::Html => policy.create_html.clone(),
            TrustedKind::Script => policy.create_script.clone(),
            TrustedKind::ScriptUrl => policy.create_script_url.clone(),
        };
        (policy.name.clone(), callback)
    };
    let input = args.get(0);
    let value = if let Some(callback) = callback {
        let callback = v8::Local::new(scope, &callback);
        let undefined = v8::undefined(scope).into();
        match callback.call(scope, undefined, &[input]) {
            Some(value) => value.to_rust_string_lossy(scope),
            None => return,
        }
    } else {
        input.to_rust_string_lossy(scope)
    };
    match make_trusted_value(scope, &state, kind, value, policy_name) {
        Ok(value) => return_value.set(value.into()),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn make_trusted_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    kind: TrustedKind,
    value: String,
    policy_name: String,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let id = state.next_trusted_id.get();
    state.next_trusted_id.set(id.saturating_add(1));
    state
        .trusted_values
        .borrow_mut()
        .insert(id, TrustedValue::new(kind, value, policy_name));
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(2);
    let object = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("Trusted Type value"))?;
    set_generated_prototype(
        scope,
        scope.get_current_context(),
        object,
        kind.interface_name(),
    )?;
    object.set_internal_field(0, v8::Integer::new(scope, kind_token(kind) as i32).into());
    object.set_internal_field(1, v8::BigInt::new_from_u64(scope, id).into());
    let function = v8::Function::builder(trusted_value_to_string)
        .data(v8::BigInt::new_from_u64(scope, id).into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("Trusted Type toString"))?;
    let key =
        v8::String::new(scope, "toString").ok_or(V8BindingError::Allocation("toString name"))?;
    object.create_data_property(scope, key.into(), function.into());
    Ok(object)
}

fn set_generated_prototype(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, interface)
        .ok_or(V8BindingError::Allocation("generated interface name"))?;
    let constructor = context
        .global(scope)
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation(
            "generated interface constructor",
        ))?;
    let prototype_name =
        v8::String::new(scope, "prototype").ok_or(V8BindingError::Allocation("prototype name"))?;
    let prototype = constructor
        .get(scope, prototype_name.into())
        .ok_or(V8BindingError::Allocation("generated interface prototype"))?;
    object.set_prototype(scope, prototype);
    Ok(())
}

fn trusted_value_to_string(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Some(id) = exact_u64(args.data()) else {
        return;
    };
    if let Some(value) = state.trusted_values.borrow().get(&id)
        && let Some(value) = v8::String::new(scope, value.value())
    {
        return_value.set(value.into());
    }
}

fn trusted_is_html(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_is(scope, args, return_value, TrustedKind::Html);
}
fn trusted_is_script(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_is(scope, args, return_value, TrustedKind::Script);
}
fn trusted_is_script_url(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_is(scope, args, return_value, TrustedKind::ScriptUrl);
}

fn trusted_is(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
    kind: TrustedKind,
) {
    let matches = trusted_value_id(scope, args.get(0), kind).is_some();
    return_value.set(v8::Boolean::new(scope, matches).into());
}

fn trusted_get_attribute_type(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let tag = args.get(0).to_rust_string_lossy(scope).to_ascii_lowercase();
    let attribute = args.get(1).to_rust_string_lossy(scope).to_ascii_lowercase();
    let kind = if tag == "script" && attribute == "src" {
        Some(TrustedKind::ScriptUrl)
    } else if tag == "iframe" && attribute == "srcdoc" {
        Some(TrustedKind::Html)
    } else if attribute.starts_with("on") {
        Some(TrustedKind::Script)
    } else {
        None
    };
    if let Some(kind) = kind {
        return_value.set(
            v8::String::new(scope, kind.interface_name())
                .unwrap()
                .into(),
        );
    } else {
        return_value.set(v8::null(scope).into());
    }
}

fn trusted_get_property_type(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_get_attribute_type(scope, args, return_value)
}

fn trusted_value_id(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    kind: TrustedKind,
) -> Option<u64> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if object.internal_field_count() < 2 {
        return None;
    }
    let tag = object.get_internal_field(scope, 0)?;
    let tag = v8::Local::<v8::Value>::try_from(tag)
        .ok()?
        .int32_value(scope)? as u64;
    (tag == kind_token(kind))
        .then(|| {
            object
                .get_internal_field(scope, 1)
                .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
                .and_then(exact_u64)
        })
        .flatten()
}

fn exact_u64(value: v8::Local<v8::Value>) -> Option<u64> {
    let bigint = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = bigint.u64_value();
    lossless.then_some(value)
}

const fn kind_token(kind: TrustedKind) -> u64 {
    match kind {
        TrustedKind::Html => 1,
        TrustedKind::Script => 2,
        TrustedKind::ScriptUrl => 3,
    }
}

fn get_computed_style_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let node = argument_node(scope, &state, args.get(0))?;
        let key = node_key(node);
        let styles = state
            .layout
            .borrow()
            .styles
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let object = v8::Local::<v8::Object>::try_from(associated_wrapper(
            scope,
            &state,
            node,
            "CSSStyleDeclaration",
        )?)
        .map_err(|_| V8BindingError::Allocation("computed CSSStyleDeclaration"))?;
        for (name, value) in styles {
            let Some(name) = v8::String::new(scope, &name) else {
                continue;
            };
            let Some(value) = v8::String::new(scope, &value) else {
                continue;
            };
            object.create_data_property(scope, name.into(), value.into());
        }
        let getter = v8::Function::builder(computed_style_get_property_value)
            .data(v8::BigInt::new_from_u64(scope, node_key(node)).into())
            .build(scope)
            .ok_or(V8BindingError::Allocation(
                "computed CSSStyleDeclaration.getPropertyValue",
            ))?;
        let name = v8::String::new(scope, "getPropertyValue")
            .ok_or(V8BindingError::Allocation("getPropertyValue name"))?;
        object.create_data_property(scope, name.into(), getter.into());
        Ok(object)
    })();
    match result {
        Ok(object) => return_value.set(object.into()),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn computed_style_get_property_value(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    let Some(key) = exact_u64(args.data()) else {
        throw_type_error(scope, "computed style target is unavailable");
        return;
    };
    let Some(property) = args.get(0).to_string(scope) else {
        throw_type_error(scope, "property must be a string");
        return;
    };
    let property = property.to_rust_string_lossy(scope).to_ascii_lowercase();
    let value = state
        .layout
        .borrow()
        .styles
        .get(&key)
        .and_then(|styles| styles.get(&property))
        .cloned()
        .unwrap_or_default();
    if let Some(value) = v8::String::new(scope, &value) {
        return_value.set(value.into());
    }
}

fn node_key(node: NodeId) -> u64 {
    (u64::from(node.generation) << 32) | u64::from(node.slot)
}

fn dom_rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rect: BindingRect,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let object = v8::Object::new(scope);
    for (name, value) in [
        ("x", rect.x),
        ("y", rect.y),
        ("width", rect.width),
        ("height", rect.height),
        ("top", rect.y),
        ("right", rect.x + rect.width),
        ("bottom", rect.y + rect.height),
        ("left", rect.x),
    ] {
        let name =
            v8::String::new(scope, name).ok_or(V8BindingError::Allocation("DOMRect property"))?;
        let value = v8::Number::new(scope, value);
        object.create_data_property(scope, name.into(), value.into());
    }
    Ok(object)
}

fn member_from_token(token: i32) -> Option<(&'static str, &'static str)> {
    let (interface, member) = generated_member_by_id(GeneratedMemberId(token as u32))?;
    Some((interface.name, member.name))
}

fn member_metadata_from_token(token: i32) -> Option<(&'static str, &'static GeneratedMember)> {
    let (interface, member) = generated_member_by_id(GeneratedMemberId(token as u32))?;
    Some((interface.name, member))
}

fn is_platform_interface(interface: &str) -> bool {
    matches!(
        interface,
        "Blob"
            | "File"
            | "FormData"
            | "Headers"
            | "AbortSignal"
            | "AbortController"
            | "ReadableStream"
            | "ReadableStreamDefaultReader"
            | "ReadableStreamDefaultController"
            | "WritableStream"
            | "WritableStreamDefaultWriter"
            | "WritableStreamDefaultController"
            | "TransformStream"
            | "Request"
            | "Response"
            | "XMLHttpRequest"
            | "WebSocket"
            | "IDBFactory"
            | "IDBRequest"
            | "IDBOpenDBRequest"
            | "IDBDatabase"
            | "IDBTransaction"
            | "IDBKeyRange"
            | "IDBCursor"
            | "IDBIndex"
            | "IDBObjectStore"
    )
}

fn register_platform_object(state: &V8BindingState, record: PlatformObjectRecord) -> u64 {
    let id = state.next_platform_object_id.get();
    state
        .next_platform_object_id
        .set(id.saturating_add(1).max(1));
    state.platform_objects.borrow_mut().insert(id, record);
    id
}

fn reserve_platform_object_id(state: &V8BindingState) -> u64 {
    let id = state.next_platform_object_id.get();
    state
        .next_platform_object_id
        .set(id.saturating_add(1).max(1));
    id
}

fn insert_platform_object(state: &V8BindingState, id: u64, record: PlatformObjectRecord) {
    state.platform_objects.borrow_mut().insert(id, record);
}

fn bind_platform_object(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    object: v8::Local<v8::Object>,
) {
    object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
    state
        .platform_wrappers
        .borrow_mut()
        .insert(id, v8::Weak::new(scope, object));
}

fn new_platform_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    interface: &'static str,
    id: u64,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Some(object) = state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|object| object.to_local(scope))
    {
        return Ok(object);
    }
    let object = new_stateful_object(scope, state, interface, id)?;
    state
        .platform_wrappers
        .borrow_mut()
        .insert(id, v8::Weak::new(scope, object));
    Ok(object)
}

fn platform_object<'a>(
    state: &'a V8BindingState,
    id: u64,
) -> Result<std::cell::Ref<'a, PlatformObjectRecord>, V8BindingError> {
    std::cell::Ref::filter_map(state.platform_objects.borrow(), |objects| objects.get(&id))
        .map_err(|_| V8BindingError::Allocation("platform object record"))
}

fn platform_object_mut<'a>(
    state: &'a V8BindingState,
    id: u64,
) -> Result<std::cell::RefMut<'a, PlatformObjectRecord>, V8BindingError> {
    std::cell::RefMut::filter_map(state.platform_objects.borrow_mut(), |objects| {
        objects.get_mut(&id)
    })
    .map_err(|_| V8BindingError::Allocation("platform object record"))
}

fn option_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    if value.is_null_or_undefined() {
        return None;
    }
    let object = value.to_object(scope)?;
    let name = v8::String::new(scope, name)?;
    object.get(scope, name.into())
}

fn option_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
) -> Option<v8::Global<v8::Function>> {
    option_value(scope, value, name)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .map(|function| v8::Global::new(scope, function))
}

fn bytes_from_array_buffer(buffer: v8::Local<v8::ArrayBuffer>) -> Vec<u8> {
    buffer
        .get_backing_store()
        .iter()
        .map(std::cell::Cell::get)
        .collect()
}

fn bytes_from_value(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
) -> Result<Vec<u8>, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        return Ok(bytes_from_array_buffer(buffer));
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        view.copy_contents(&mut bytes);
        return Ok(bytes);
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Ok(id) = stateful_id(scope, object)
        && let Ok(record) = platform_object(state, id)
        && let PlatformObjectRecord::Blob(blob) = &*record
    {
        return Ok(blob.bytes.clone());
    }
    Ok(value.to_rust_string_lossy(scope).into_bytes())
}

fn blob_parts(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
) -> Result<Vec<u8>, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    if let Ok(parts) = v8::Local::<v8::Array>::try_from(value) {
        let mut bytes = Vec::new();
        for index in 0..parts.length() {
            if let Some(part) = parts.get_index(scope, index) {
                bytes.extend(bytes_from_value(scope, state, part)?);
            }
        }
        Ok(bytes)
    } else {
        bytes_from_value(scope, state, value)
    }
}

fn normalize_media_type(value: String) -> String {
    if value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        value.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn array_buffer_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
) -> v8::Local<'s, v8::ArrayBuffer> {
    let backing = v8::ArrayBuffer::new_backing_store(scope, bytes.len());
    for (target, value) in backing.iter().zip(bytes.iter().copied()) {
        target.set(value);
    }
    let backing = backing.make_shared();
    v8::ArrayBuffer::with_backing_store(scope, &backing)
}

fn resolved_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("resolved promise"))?;
    let promise = resolver.get_promise(scope);
    resolver.resolve(scope, value);
    Ok(promise)
}

fn read_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    done: bool,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let result = v8::Object::new(scope);
    let done_name =
        v8::String::new(scope, "done").ok_or(V8BindingError::Allocation("stream result done"))?;
    let value_name =
        v8::String::new(scope, "value").ok_or(V8BindingError::Allocation("stream result value"))?;
    let done_value = v8::Boolean::new(scope, done);
    result.create_data_property(scope, done_name.into(), done_value.into());
    result.create_data_property(scope, value_name.into(), value);
    Ok(result)
}

fn callback_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: Option<v8::Global<v8::Function>>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let value = if let Some(callback) = callback {
        let callback = v8::Local::new(scope, callback);
        let receiver = v8::undefined(scope).into();
        callback
            .call(scope, receiver, arguments)
            .ok_or_else(|| V8BindingError::EventCallback("stream callback failed".into()))?
    } else {
        v8::undefined(scope).into()
    };
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        Ok(promise)
    } else {
        resolved_promise(scope, value)
    }
}

fn writable_write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    stream_id: u64,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    enum Target {
        Sink(Option<v8::Global<v8::Function>>),
        Transform(Option<v8::Global<v8::Function>>, u64),
    }
    let target = {
        let record = platform_object(state, stream_id)?;
        let PlatformObjectRecord::WritableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if stream.closed {
            return Err(V8BindingError::Unsupported {
                interface: "WritableStream".into(),
                member: "write after close".into(),
            });
        }
        match &stream.kind {
            WritableKind::Sink { write, .. } => Target::Sink(write.clone()),
            WritableKind::Transform {
                transform,
                readable_controller_id,
                ..
            } => Target::Transform(transform.clone(), *readable_controller_id),
        }
    };
    match target {
        Target::Sink(callback) => callback_promise(scope, callback, &[chunk]),
        Target::Transform(callback, controller_id) => {
            let controller = new_stateful_object(
                scope,
                state,
                "ReadableStreamDefaultController",
                controller_id,
            )?;
            if callback.is_some() {
                callback_promise(scope, callback, &[chunk, controller.into()])
            } else {
                readable_enqueue(scope, state, controller_id, chunk)?;
                let undefined = v8::undefined(scope).into();
                resolved_promise(scope, undefined)
            }
        }
    }
}

fn readable_enqueue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    controller_id: u64,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    let stream_id = {
        let record = platform_object(state, controller_id)?;
        let PlatformObjectRecord::ReadableController { stream_id } = &*record else {
            return Err(V8BindingError::Allocation("ReadableStream controller"));
        };
        *stream_id
    };
    let (waiter, pipes) = {
        let mut record = platform_object_mut(state, stream_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        if stream.closed {
            return Err(V8BindingError::Unsupported {
                interface: "ReadableStreamDefaultController".into(),
                member: "enqueue after close".into(),
            });
        }
        let waiter = stream.waiting.pop_front();
        if waiter.is_none() {
            stream.queue.push_back(v8::Global::new(scope, value));
        }
        let pipes = stream
            .pipes
            .iter()
            .map(|pipe| pipe.writable_id)
            .collect::<Vec<_>>();
        (waiter, pipes)
    };
    if let Some(waiter) = waiter {
        let waiter = v8::Local::new(scope, waiter);
        let result = read_result(scope, false, value)?;
        waiter.resolve(scope, result.into());
    }
    for writable_id in pipes {
        let _ = writable_write(scope, state, writable_id, value)?;
    }
    Ok(())
}

fn writable_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    stream_id: u64,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    enum Target {
        Sink(Option<v8::Global<v8::Function>>),
        Transform(Option<v8::Global<v8::Function>>, u64),
    }
    let target = {
        let mut record = platform_object_mut(state, stream_id)?;
        let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        stream.closed = true;
        match &stream.kind {
            WritableKind::Sink { close, .. } => Target::Sink(close.clone()),
            WritableKind::Transform {
                flush,
                readable_controller_id,
                ..
            } => Target::Transform(flush.clone(), *readable_controller_id),
        }
    };
    match target {
        Target::Sink(callback) => callback_promise(scope, callback, &[]),
        Target::Transform(callback, controller_id) => {
            let controller = new_stateful_object(
                scope,
                state,
                "ReadableStreamDefaultController",
                controller_id,
            )?;
            let promise = callback_promise(scope, callback, &[controller.into()])?;
            readable_close(scope, state, controller_id)?;
            Ok(promise)
        }
    }
}

fn readable_close(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    controller_id: u64,
) -> Result<(), V8BindingError> {
    let stream_id = {
        let record = platform_object(state, controller_id)?;
        let PlatformObjectRecord::ReadableController { stream_id } = &*record else {
            return Err(V8BindingError::Allocation("ReadableStream controller"));
        };
        *stream_id
    };
    let (waiters, pipes) = {
        let mut record = platform_object_mut(state, stream_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        stream.closed = true;
        (
            stream.waiting.drain(..).collect::<Vec<_>>(),
            std::mem::take(&mut stream.pipes),
        )
    };
    for waiter in waiters {
        let waiter = v8::Local::new(scope, waiter);
        let undefined = v8::undefined(scope).into();
        let result = read_result(scope, true, undefined)?;
        waiter.resolve(scope, result.into());
    }
    for pipe in pipes {
        let _ = writable_close(scope, state, pipe.writable_id)?;
        let resolver = v8::Local::new(scope, pipe.resolver);
        let undefined = v8::undefined(scope).into();
        resolver.resolve(scope, undefined);
    }
    Ok(())
}

fn writer_stream_id(
    _scope: &mut v8::PinScope,
    state: &V8BindingState,
    writer_id: u64,
) -> Result<u64, V8BindingError> {
    let record = platform_object(state, writer_id)?;
    let PlatformObjectRecord::WritableWriter { stream_id } = &*record else {
        return Err(V8BindingError::Allocation("WritableStream writer"));
    };
    Ok(*stream_id)
}

fn pipe_readable_to<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    readable_id: u64,
    writable_id: u64,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("pipeTo promise"))?;
    let promise = resolver.get_promise(scope);
    let (queued, closed) = {
        let mut record = platform_object_mut(state, readable_id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("ReadableStream record"));
        };
        let queued = stream.queue.drain(..).collect::<Vec<_>>();
        let closed = stream.closed;
        if !closed {
            stream.pipes.push(ReadablePipe {
                writable_id,
                resolver: v8::Global::new(scope, resolver),
            });
        }
        (queued, closed)
    };
    for chunk in queued {
        let chunk = v8::Local::new(scope, chunk);
        let _ = writable_write(scope, state, writable_id, chunk)?;
    }
    if closed {
        let _ = writable_close(scope, state, writable_id)?;
        let undefined = v8::undefined(scope).into();
        resolver.resolve(scope, undefined);
    }
    Ok(promise)
}

fn initialize_platform_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    name: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<bool, V8BindingError> {
    if !args.is_construct_call() {
        return Ok(false);
    }
    if name == "AbortController" {
        let signal_id = register_platform_object(
            state,
            PlatformObjectRecord::AbortSignal(AbortSignalRecord {
                aborted: false,
                reason: None,
            }),
        );
        let id =
            register_platform_object(state, PlatformObjectRecord::AbortController { signal_id });
        bind_platform_object(scope, state, id, args.this());
        return Ok(true);
    }
    if name == "FormData" {
        let mut entries = Vec::new();
        if !args.get(0).is_null_or_undefined()
            && let Some(form) = args.get(0).to_object(scope)
            && let Ok(form) = argument_node(scope, state, form.into())
        {
            let controls = state
                .dom
                .borrow()
                .query_selector_all(form, "input,textarea,select")?;
            for control in controls {
                let dom = state.dom.borrow();
                let Some(field_name) = dom
                    .attribute(control, "name")?
                    .map(str::to_owned)
                    .filter(|name| !name.is_empty())
                else {
                    continue;
                };
                if dom.attribute(control, "disabled")?.is_some() {
                    continue;
                }
                let kind = dom
                    .attribute(control, "type")?
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(kind.as_str(), "checkbox" | "radio")
                    && dom.attribute(control, "checked")?.is_none()
                {
                    continue;
                }
                let value = dom.attribute(control, "value")?.unwrap_or("").to_owned();
                drop(dom);
                let value = v8_string(scope, &value)?;
                entries.push((field_name, v8::Global::new(scope, value)));
            }
        }
        let id = register_platform_object(
            state,
            PlatformObjectRecord::FormData(FormDataRecord { entries }),
        );
        bind_platform_object(scope, state, id, args.this());
        return Ok(true);
    }
    if name == "Request" {
        let input = args.get(0);
        let init = args.get(1);
        let inherited = input
            .to_object(scope)
            .and_then(|object| stateful_id(scope, object).ok())
            .and_then(|id| {
                let record = platform_object(state, id).ok()?;
                match &*record {
                    PlatformObjectRecord::Request(request) => Some((
                        request.url.clone(),
                        request.method.clone(),
                        request.headers_id,
                        request.signal_id,
                        request.credentials.clone(),
                        request.body.clone(),
                        request.body_object_id,
                    )),
                    _ => None,
                }
            });
        let raw_url = inherited
            .as_ref()
            .map(|value| value.0.clone())
            .unwrap_or_else(|| input.to_rust_string_lossy(scope));
        let url = resolve_platform_url(scope, &raw_url)?;
        let method = option_value(scope, init, "method")
            .filter(|value| !value.is_null_or_undefined())
            .map(|value| value.to_rust_string_lossy(scope).to_ascii_uppercase())
            .or_else(|| inherited.as_ref().map(|value| value.1.clone()))
            .unwrap_or_else(|| "GET".into());
        if !method
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(V8BindingError::Unsupported {
                interface: name.into(),
                member: "invalid method".into(),
            });
        }
        let headers_id = if let Some(headers) = option_value(scope, init, "headers") {
            headers_from_value(scope, state, headers)?
        } else if let Some(inherited) = &inherited {
            clone_headers(state, inherited.2)?
        } else {
            register_platform_object(
                state,
                PlatformObjectRecord::Headers(HeadersRecord::default()),
            )
        };
        let signal_id = option_value(scope, init, "signal")
            .and_then(|value| value.to_object(scope))
            .and_then(|object| stateful_id(scope, object).ok())
            .or_else(|| inherited.as_ref().map(|value| value.3))
            .unwrap_or_else(|| {
                register_platform_object(
                    state,
                    PlatformObjectRecord::AbortSignal(AbortSignalRecord {
                        aborted: false,
                        reason: None,
                    }),
                )
            });
        let credentials = option_value(scope, init, "credentials")
            .filter(|value| !value.is_null_or_undefined())
            .map(|value| value.to_rust_string_lossy(scope))
            .or_else(|| inherited.as_ref().map(|value| value.4.clone()))
            .unwrap_or_else(|| "same-origin".into());
        let body_value =
            option_value(scope, init, "body").filter(|value| !value.is_null_or_undefined());
        let body_object_id = body_value
            .and_then(|value| value.to_object(scope))
            .and_then(|object| stateful_id(scope, object).ok())
            .filter(|id| state.platform_objects.borrow().contains_key(id))
            .or_else(|| inherited.as_ref().and_then(|value| value.6));
        let body = if let Some(body) = body_value {
            if let Some((bytes, content_type)) = form_data_body(scope, state, body)? {
                let mut headers = platform_object_mut(state, headers_id)?;
                let PlatformObjectRecord::Headers(headers) = &mut *headers else {
                    return Err(V8BindingError::Allocation("Request Headers"));
                };
                if !headers
                    .entries
                    .iter()
                    .any(|(name, _)| name == "content-type")
                {
                    append_header(headers, "content-type".into(), content_type)?;
                }
                bytes
            } else {
                bytes_from_value(scope, state, body)?
            }
        } else {
            inherited.map_or_else(Vec::new, |value| value.5)
        };
        let id = register_platform_object(
            state,
            PlatformObjectRecord::Request(RequestRecord {
                url,
                method,
                headers_id,
                signal_id,
                credentials,
                body,
                body_object_id,
                body_used: false,
            }),
        );
        bind_platform_object(scope, state, id, args.this());
        return Ok(true);
    }
    if name == "Response" {
        let body = if args.get(0).is_null_or_undefined() {
            Vec::new()
        } else {
            bytes_from_value(scope, state, args.get(0))?
        };
        let init = args.get(1);
        let status = option_value(scope, init, "status")
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(200) as u16;
        let headers_id = option_value(scope, init, "headers")
            .map(|value| headers_from_value(scope, state, value))
            .transpose()?
            .unwrap_or_else(|| {
                register_platform_object(
                    state,
                    PlatformObjectRecord::Headers(HeadersRecord::default()),
                )
            });
        let body_stream_id = (!body.is_empty())
            .then(|| new_byte_stream_record(scope, state, body.clone()))
            .transpose()?;
        let id = register_platform_object(
            state,
            PlatformObjectRecord::Response(ResponseRecord {
                status,
                url: String::new(),
                headers_id,
                body_stream_id,
                bytes: body,
                body_used: false,
                signal_id: None,
            }),
        );
        bind_platform_object(scope, state, id, args.this());
        return Ok(true);
    }
    if name == "XMLHttpRequest" {
        let id = register_platform_object(
            state,
            PlatformObjectRecord::XMLHttpRequest(XhrRecord {
                ready_state: 0,
                timeout: 0,
                with_credentials: false,
                response_type: String::new(),
                status: 0,
                response_url: String::new(),
                response_headers: Vec::new(),
                response_bytes: Vec::new(),
                method: String::new(),
                url: String::new(),
                request_headers: Vec::new(),
                stream_id: None,
                active_operation: None,
                deadline: None,
            }),
        );
        bind_platform_object(scope, state, id, args.this());
        return Ok(true);
    }
    if name == "WebSocket" {
        let url = resolve_platform_url(scope, &args.get(0).to_rust_string_lossy(scope))?;
        let protocols = string_sequence(scope, args.get(1));
        let id = register_platform_object(
            state,
            PlatformObjectRecord::WebSocket(WebSocketRecord {
                url: url.clone(),
                ready_state: 0,
                protocol: String::new(),
                binary_type: "blob".into(),
                socket_id: None,
            }),
        );
        bind_platform_object(scope, state, id, args.this());
        let operation_id = submit_platform_operation(
            state,
            PlatformHostRequest::WebSocketOpen { url, protocols },
            PendingPlatformOperation::WebSocketOpen { websocket_id: id },
        )?;
        let _ = operation_id;
        return Ok(true);
    }
    if name == "ReadableStream" {
        let stream_id = reserve_platform_object_id(state);
        let controller_id = reserve_platform_object_id(state);
        let source = args.get(0);
        let start = option_function(scope, source, "start");
        let pull = option_function(scope, source, "pull");
        let cancel = option_function(scope, source, "cancel");
        insert_platform_object(
            state,
            controller_id,
            PlatformObjectRecord::ReadableController { stream_id },
        );
        insert_platform_object(
            state,
            stream_id,
            PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
                queue: VecDeque::new(),
                waiting: VecDeque::new(),
                closed: false,
                locked: false,
                pull,
                cancel,
                controller_id,
                pipes: Vec::new(),
                host_stream_id: None,
                host_read_pending: false,
            }),
        );
        bind_platform_object(scope, state, stream_id, args.this());
        if let Some(start) = start {
            let start = v8::Local::new(scope, start);
            let controller = new_stateful_object(
                scope,
                state,
                "ReadableStreamDefaultController",
                controller_id,
            )?;
            let receiver = v8::undefined(scope).into();
            if start.call(scope, receiver, &[controller.into()]).is_none() {
                return Err(V8BindingError::EventCallback(
                    "ReadableStream start callback failed".into(),
                ));
            }
        }
        return Ok(true);
    }
    if name == "WritableStream" {
        let stream_id = reserve_platform_object_id(state);
        let controller_id = reserve_platform_object_id(state);
        let sink = args.get(0);
        insert_platform_object(
            state,
            controller_id,
            PlatformObjectRecord::WritableController { stream_id },
        );
        insert_platform_object(
            state,
            stream_id,
            PlatformObjectRecord::WritableStream(WritableStreamRecord {
                locked: false,
                closed: false,
                kind: WritableKind::Sink {
                    write: option_function(scope, sink, "write"),
                    close: option_function(scope, sink, "close"),
                    abort: option_function(scope, sink, "abort"),
                },
            }),
        );
        bind_platform_object(scope, state, stream_id, args.this());
        return Ok(true);
    }
    if name == "TransformStream" {
        let transform_id = reserve_platform_object_id(state);
        let readable_id = reserve_platform_object_id(state);
        let readable_controller_id = reserve_platform_object_id(state);
        let writable_id = reserve_platform_object_id(state);
        let writable_controller_id = reserve_platform_object_id(state);
        let transformer = args.get(0);
        insert_platform_object(
            state,
            readable_controller_id,
            PlatformObjectRecord::ReadableController {
                stream_id: readable_id,
            },
        );
        insert_platform_object(
            state,
            readable_id,
            PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
                queue: VecDeque::new(),
                waiting: VecDeque::new(),
                closed: false,
                locked: false,
                pull: None,
                cancel: None,
                controller_id: readable_controller_id,
                pipes: Vec::new(),
                host_stream_id: None,
                host_read_pending: false,
            }),
        );
        insert_platform_object(
            state,
            writable_controller_id,
            PlatformObjectRecord::WritableController {
                stream_id: writable_id,
            },
        );
        insert_platform_object(
            state,
            writable_id,
            PlatformObjectRecord::WritableStream(WritableStreamRecord {
                locked: false,
                closed: false,
                kind: WritableKind::Transform {
                    transform: option_function(scope, transformer, "transform"),
                    flush: option_function(scope, transformer, "flush"),
                    readable_controller_id,
                },
            }),
        );
        insert_platform_object(
            state,
            transform_id,
            PlatformObjectRecord::TransformStream {
                readable_id,
                writable_id,
            },
        );
        bind_platform_object(scope, state, transform_id, args.this());
        return Ok(true);
    }
    let record = match name {
        "Blob" | "File" => {
            let bytes = blob_parts(scope, state, args.get(0))?;
            let options_index = if name == "Blob" { 1 } else { 2 };
            let media_type = option_value(scope, args.get(options_index), "type")
                .filter(|value| !value.is_null_or_undefined())
                .map(|value| normalize_media_type(value.to_rust_string_lossy(scope)))
                .unwrap_or_default();
            let file = (name == "File").then(|| FileRecord {
                name: args.get(1).to_rust_string_lossy(scope),
                last_modified: option_value(scope, args.get(2), "lastModified")
                    .and_then(|value| value.integer_value(scope))
                    .unwrap_or(0),
            });
            PlatformObjectRecord::Blob(BlobRecord {
                bytes,
                media_type,
                file,
            })
        }
        "Headers" => {
            let mut record = HeadersRecord::default();
            let init = args.get(0);
            if let Ok(object) = v8::Local::<v8::Object>::try_from(init)
                && let Ok(id) = stateful_id(scope, object)
                && let Ok(source) = platform_object(state, id)
                && let PlatformObjectRecord::Headers(source) = &*source
            {
                record = source.clone();
            } else if let Ok(entries) = v8::Local::<v8::Array>::try_from(init) {
                for index in 0..entries.length() {
                    let Some(pair) = entries
                        .get_index(scope, index)
                        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
                    else {
                        continue;
                    };
                    if pair.length() >= 2 {
                        let name = pair
                            .get_index(scope, 0)
                            .map(|value| value.to_rust_string_lossy(scope))
                            .unwrap_or_default();
                        let value = pair
                            .get_index(scope, 1)
                            .map(|value| value.to_rust_string_lossy(scope))
                            .unwrap_or_default();
                        append_header(&mut record, name, value)?;
                    }
                }
            } else if !init.is_null_or_undefined()
                && let Some(object) = init.to_object(scope)
                && let Some(names) =
                    object.get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
            {
                for index in 0..names.length() {
                    let Some(name) = names.get_index(scope, index) else {
                        continue;
                    };
                    let value = object
                        .get(scope, name)
                        .map(|value| value.to_rust_string_lossy(scope))
                        .unwrap_or_default();
                    append_header(&mut record, name.to_rust_string_lossy(scope), value)?;
                }
            }
            PlatformObjectRecord::Headers(record)
        }
        _ => return Ok(false),
    };
    let id = register_platform_object(state, record);
    bind_platform_object(scope, state, id, args.this());
    Ok(true)
}

fn normalize_header_name(name: String) -> Result<String, V8BindingError> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        return Err(V8BindingError::Unsupported {
            interface: "Headers".into(),
            member: "invalid header name".into(),
        });
    }
    Ok(name)
}

fn resolve_platform_url(scope: &mut v8::PinScope, input: &str) -> Result<String, V8BindingError> {
    if let Ok(url) = Url::parse(input) {
        return Ok(url.to_string());
    }
    let global = scope.get_current_context().global(scope);
    let location = v8::String::new(scope, "location")
        .and_then(|key| global.get(scope, key.into()))
        .and_then(|value| value.to_object(scope));
    let base = location
        .and_then(|location| {
            let key = v8::String::new(scope, "href")?;
            location.get(scope, key.into())
        })
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "about:blank".into());
    Url::parse(&base)
        .and_then(|base| base.join(input))
        .map(|url| url.to_string())
        .map_err(|_| V8BindingError::Unsupported {
            interface: "URL".into(),
            member: "invalid URL".into(),
        })
}

fn blob_url_create_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("Blob URL binding state"))?;
        let object = args
            .get(0)
            .to_object(scope)
            .ok_or(V8BindingError::Unsupported {
                interface: "URL".into(),
                member: "TypeError".into(),
            })?;
        let blob_id = stateful_id(scope, object)?;
        if !matches!(
            &*platform_object(&state, blob_id)?,
            PlatformObjectRecord::Blob(_)
        ) {
            return Err(V8BindingError::Unsupported {
                interface: "URL".into(),
                member: "TypeError".into(),
            });
        }
        let id = state.next_blob_url_id.get();
        state.next_blob_url_id.set(id.saturating_add(1));
        let url = format!(
            "blob:{}/{}-{}",
            current_platform_origin(scope),
            state.document_generation,
            id
        );
        state.blob_urls.borrow_mut().insert(url.clone(), blob_id);
        v8_string(scope, &url)
    })();
    match result {
        Ok(value) => return_value.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn blob_url_revoke_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    state
        .blob_urls
        .borrow_mut()
        .remove(&args.get(0).to_rust_string_lossy(scope));
}

fn blob_url_record(state: &V8BindingState, url: &str) -> Option<BlobRecord> {
    let id = state.blob_urls.borrow().get(url).copied()?;
    let record = platform_object(state, id).ok()?;
    let PlatformObjectRecord::Blob(blob) = &*record else {
        return None;
    };
    Some(blob.clone())
}

fn clone_headers(state: &V8BindingState, id: u64) -> Result<u64, V8BindingError> {
    let record = platform_object(state, id)?;
    let PlatformObjectRecord::Headers(headers) = &*record else {
        return Err(V8BindingError::Allocation("Headers record"));
    };
    let headers = headers.clone();
    drop(record);
    Ok(register_platform_object(
        state,
        PlatformObjectRecord::Headers(headers),
    ))
}

fn headers_from_value(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
) -> Result<u64, V8BindingError> {
    if let Some(object) = value.to_object(scope)
        && let Ok(id) = stateful_id(scope, object)
        && matches!(
            &*platform_object(state, id)?,
            PlatformObjectRecord::Headers(_)
        )
    {
        return clone_headers(state, id);
    }
    let mut headers = HeadersRecord::default();
    if let Ok(entries) = v8::Local::<v8::Array>::try_from(value) {
        for index in 0..entries.length() {
            let Some(pair) = entries
                .get_index(scope, index)
                .and_then(|item| v8::Local::<v8::Array>::try_from(item).ok())
            else {
                continue;
            };
            if pair.length() >= 2 {
                let name = pair
                    .get_index(scope, 0)
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_default();
                let value = pair
                    .get_index(scope, 1)
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_default();
                append_header(&mut headers, name, value)?;
            }
        }
    } else if let Some(object) = value.to_object(scope)
        && let Some(names) =
            object.get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
    {
        for index in 0..names.length() {
            let Some(name) = names.get_index(scope, index) else {
                continue;
            };
            let value = object
                .get(scope, name)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            append_header(&mut headers, name.to_rust_string_lossy(scope), value)?;
        }
    }
    Ok(register_platform_object(
        state,
        PlatformObjectRecord::Headers(headers),
    ))
}

fn string_sequence(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Vec<String> {
    if value.is_null_or_undefined() {
        return Vec::new();
    }
    if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
        return (0..array.length())
            .filter_map(|index| array.get_index(scope, index))
            .map(|value| value.to_rust_string_lossy(scope))
            .collect();
    }
    vec![value.to_rust_string_lossy(scope)]
}

fn new_byte_stream_record(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    bytes: Vec<u8>,
) -> Result<u64, V8BindingError> {
    let stream_id = reserve_platform_object_id(state);
    let controller_id = reserve_platform_object_id(state);
    let buffer = array_buffer_from_bytes(scope, &bytes);
    let array = v8::Uint8Array::new(scope, buffer, 0, bytes.len())
        .ok_or(V8BindingError::Allocation("stream chunk"))?;
    insert_platform_object(
        state,
        controller_id,
        PlatformObjectRecord::ReadableController { stream_id },
    );
    insert_platform_object(
        state,
        stream_id,
        PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
            queue: VecDeque::from([v8::Global::new(scope, v8::Local::<v8::Value>::from(array))]),
            waiting: VecDeque::new(),
            closed: true,
            locked: false,
            pull: None,
            cancel: None,
            controller_id,
            pipes: Vec::new(),
            host_stream_id: None,
            host_read_pending: false,
        }),
    );
    Ok(stream_id)
}

fn submit_platform_operation(
    state: &V8BindingState,
    request: PlatformHostRequest,
    pending: PendingPlatformOperation,
) -> Result<HostOperationId, V8BindingError> {
    let host = state
        .platform_host
        .borrow()
        .clone()
        .ok_or(V8BindingError::Unsupported {
            interface: "Host".into(),
            member: "platform service unavailable".into(),
        })?;
    if state.pending_platform_operations.borrow().len() >= 32 {
        return Err(V8BindingError::Unsupported {
            interface: "Host".into(),
            member: "pending operation limit exceeded".into(),
        });
    }
    let id = HostOperationId(state.next_platform_operation_id.get());
    state.next_platform_operation_id.set(id.0.saturating_add(1));
    host.submit(id, request)
        .map_err(|message| V8BindingError::Unsupported {
            interface: "Host".into(),
            member: message,
        })?;
    state
        .pending_platform_operations
        .borrow_mut()
        .insert(id, pending);
    Ok(id)
}

fn normalize_header_value(value: String) -> Result<String, V8BindingError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(V8BindingError::Unsupported {
            interface: "Headers".into(),
            member: "invalid header value".into(),
        });
    }
    Ok(value.trim().to_string())
}

fn append_header(
    record: &mut HeadersRecord,
    name: String,
    value: String,
) -> Result<(), V8BindingError> {
    let name = normalize_header_name(name)?;
    let value = normalize_header_value(value)?;
    if let Some((_, current)) = record.entries.iter_mut().find(|(key, _)| key == &name) {
        current.push_str(", ");
        current.push_str(&value);
    } else {
        record.entries.push((name, value));
        record.entries.sort_by(|left, right| left.0.cmp(&right.0));
    }
    Ok(())
}

fn generated_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _return_value: v8::ReturnValue,
) {
    let interface = args
        .data()
        .int32_value(scope)
        .and_then(|index| GENERATED_INTERFACES.get(index as usize));
    let name = interface
        .map(|interface| interface.name)
        .unwrap_or("interface");
    if let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() {
        match initialize_platform_constructor(scope, &state, name, &args) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                throw_binding_error(scope, &error);
                return;
            }
        }
    }
    if matches!(name, "Event" | "CustomEvent" | "IDBVersionChangeEvent") && args.is_construct_call()
    {
        let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
            throw_type_error(scope, "generated binding state is unavailable");
            return;
        };
        let event_type = args.get(0).to_rust_string_lossy(scope);
        let init_value = args.get(1);
        let init = (!init_value.is_null_or_undefined())
            .then(|| init_value.to_object(scope))
            .flatten();
        let bubbles = init
            .and_then(|init| {
                let key = v8::String::new(scope, "bubbles")?;
                init.get(scope, key.into())
            })
            .is_some_and(|value| value.boolean_value(scope));
        let cancelable = init
            .and_then(|init| {
                let key = v8::String::new(scope, "cancelable")?;
                init.get(scope, key.into())
            })
            .is_some_and(|value| value.boolean_value(scope));
        let event_id = state.next_event_id.get();
        state.next_event_id.set(event_id.saturating_add(1));
        state.events.borrow_mut().insert(
            event_id,
            EventRecord {
                event_type,
                target: None,
                current_target: None,
                bubbles,
                cancelable,
                default_prevented: false,
                propagation_stopped: false,
                immediate_stopped: false,
            },
        );
        let id = v8::Number::new(scope, event_id as f64);
        args.this().set_internal_field(0, id.into());
        if name == "CustomEvent" {
            let detail = init
                .and_then(|init| {
                    let key = v8::String::new(scope, "detail")?;
                    init.get(scope, key.into())
                })
                .unwrap_or_else(|| v8::null(scope).into());
            if let Some(key) = v8::String::new(scope, "detail") {
                args.this().create_data_property(scope, key.into(), detail);
            }
            let composed = init
                .and_then(|init| {
                    let key = v8::String::new(scope, "composed")?;
                    init.get(scope, key.into())
                })
                .is_some_and(|value| value.boolean_value(scope));
            if let Some(key) = v8::String::new(scope, "composed") {
                let value = v8::Boolean::new(scope, composed);
                args.this()
                    .create_data_property(scope, key.into(), value.into());
            }
        }
        if name == "IDBVersionChangeEvent" {
            let old_version = init
                .and_then(|init| {
                    let key = v8::String::new(scope, "oldVersion")?;
                    init.get(scope, key.into())
                })
                .and_then(|value| value.number_value(scope))
                .unwrap_or(0.0);
            let new_version = init
                .and_then(|init| {
                    let key = v8::String::new(scope, "newVersion")?;
                    init.get(scope, key.into())
                })
                .unwrap_or_else(|| v8::null(scope).into());
            if let Some(key) = v8::String::new(scope, "oldVersion") {
                args.this().create_data_property(
                    scope,
                    key.into(),
                    v8::Number::new(scope, old_version).into(),
                );
            }
            if let Some(key) = v8::String::new(scope, "newVersion") {
                args.this()
                    .create_data_property(scope, key.into(), new_version);
            }
        }
        return;
    }
    if name == "MutationObserver" && args.is_construct_call() {
        let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
            throw_type_error(scope, "generated binding state is unavailable");
            return;
        };
        let Ok(callback) = v8::Local::<v8::Function>::try_from(args.get(0)) else {
            throw_type_error(scope, "MutationObserver callback must be callable");
            return;
        };
        let observer_id = state.next_mutation_observer_id.get();
        state
            .next_mutation_observer_id
            .set(observer_id.saturating_add(1));
        state.mutation_observers.borrow_mut().insert(
            observer_id,
            MutationObserverEntry {
                callback: v8::Global::new(scope, callback),
                object: v8::Global::new(scope, args.this()),
                target: None,
                options: MutationObserverOptions::default(),
                records: Vec::new(),
            },
        );
        let id = v8::Number::new(scope, observer_id as f64);
        args.this().set_internal_field(0, id.into());
        return;
    }
    if name == "Range" && args.is_construct_call() {
        let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
            throw_type_error(scope, "generated binding state is unavailable");
            return;
        };
        let document = state.dom.borrow().document();
        match register_live_range(
            &state,
            RangeRecord {
                start_container: document,
                start_offset: 0,
                end_container: document,
                end_offset: 0,
            },
        ) {
            Ok(id) => {
                args.this()
                    .set_internal_field(0, v8::Number::new(scope, id as f64).into());
            }
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    if name == "Document" && args.is_construct_call() {
        let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
            throw_type_error(scope, "generated binding state is unavailable");
            return;
        };
        let document = state.dom.borrow_mut().create(NodeKind::Document);
        let wrapper = {
            let mut dom = state.dom.borrow_mut();
            match state.wrappers.borrow_mut().get_or_create(
                &mut dom,
                state.realm,
                state.document_generation,
                document,
                InterfaceBrand::Document,
            ) {
                Ok(wrapper) => wrapper,
                Err(error) => {
                    throw_type_error(scope, &error.to_string());
                    return;
                }
            }
        };
        state.node_documents.borrow_mut().insert(document, document);
        args.this()
            .set_internal_field(0, v8::BigInt::new_from_u64(scope, wrapper.0).into());
        set_dom_wrapper_identity(
            scope,
            args.this(),
            document,
            state.realm,
            state.document_generation,
        );
        let registry: RcWeak<RefCell<WrapperRegistry>> = Rc::downgrade(&state.wrappers);
        let weak = v8::Weak::with_finalizer(
            scope,
            args.this(),
            Box::new(move |_isolate| {
                if let Some(registry) = registry.upgrade() {
                    registry.borrow_mut().enqueue_weak_release(wrapper);
                }
            }),
        );
        state.objects.borrow_mut().insert(
            wrapper,
            WrapperObject {
                handle: weak,
                connected_root: None,
            },
        );
        return;
    }
    // Custom-element upgrade uses Reflect.construct(definition, [],
    // definition) to execute the user constructor and then transfers its own
    // initialized fields onto the existing Rust-backed element wrapper. Its
    // `super()` reaches this generated constructor; the temporary shell never
    // escapes and deliberately receives no Rust node identity.
    if name == "HTMLElement" && args.is_construct_call() {
        return;
    }
    throw_type_error(scope, &format!("Illegal constructor: {name}"));
}

fn binding_state_for_receiver(
    scope: &mut v8::PinScope,
    receiver: v8::Local<v8::Object>,
) -> Option<Rc<V8BindingState>> {
    if let Some((_, realm, _)) = embedded_node_identity(scope, receiver)
        && let Some(state) = scope
            .get_slot::<BindingStateRegistry>()
            .and_then(|registry| registry.realms.borrow().get(&realm).cloned())
            .and_then(|state| state.upgrade())
    {
        return Some(state);
    }
    receiver
        .get_creation_context(scope)
        .and_then(|context| context.get_slot::<V8BindingState>())
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
}

fn generated_attribute_getter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some((interface, member)) = args.data().int32_value(scope).and_then(member_from_token)
    else {
        throw_type_error(scope, "invalid generated attribute token");
        return;
    };
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    if is_platform_interface(interface) {
        match get_platform_attribute(scope, &state, args.this(), interface, member) {
            Ok(value) => return_value.set(value),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    if interface == "Event" {
        match get_event_attribute(scope, &state, args.this(), member) {
            Ok(value) => return_value.set(value),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    if matches!(
        interface,
        "AbstractRange"
            | "Range"
            | "Selection"
            | "NodeIterator"
            | "TreeWalker"
            | "History"
            | "Storage"
    ) {
        match get_stateful_attribute(scope, &state, args.this(), interface, member) {
            Ok(value) => return_value.set(value),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    let result = resolve_receiver(scope, &state, args.this(), interface)
        .and_then(|node| get_attribute_value(scope, &state, node, interface, member));
    match result {
        Ok(value) => return_value.set(value),
        Err(error) => throw_type_error(scope, &format!("{interface}.{member}: {error}")),
    }
}

fn generated_attribute_setter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    let Some((interface, metadata)) = args
        .data()
        .int32_value(scope)
        .and_then(member_metadata_from_token)
    else {
        throw_type_error(scope, "invalid generated attribute token");
        return;
    };
    let member = metadata.name;
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    if metadata.replaceable {
        let Some(name) = v8::String::new(scope, member) else {
            throw_type_error(scope, "replaceable attribute name allocation failed");
            return;
        };
        if !args
            .this()
            .create_data_property(scope, name.into(), args.get(0))
            .unwrap_or(false)
        {
            throw_type_error(scope, &format!("{interface}.{member}: replacement failed"));
        }
        return;
    }
    if is_platform_interface(interface) {
        if let Err(error) =
            set_platform_attribute(scope, &state, args.this(), interface, member, args.get(0))
        {
            throw_binding_error(scope, &error);
        }
        return;
    }
    if let Some(forwarded_name) = metadata.put_forwards {
        let Some(attribute_name) = v8::String::new(scope, member) else {
            throw_type_error(scope, "forwarding attribute name allocation failed");
            return;
        };
        let Some(target) = args.this().get(scope, attribute_name.into()) else {
            throw_type_error(
                scope,
                &format!("{interface}.{member}: target is unavailable"),
            );
            return;
        };
        let Some(target) = target.to_object(scope) else {
            throw_type_error(
                scope,
                &format!("{interface}.{member}: target is not an object"),
            );
            return;
        };
        let Some(forwarded_name) = v8::String::new(scope, forwarded_name) else {
            throw_type_error(scope, "forwarded attribute name allocation failed");
            return;
        };
        if !target
            .set(scope, forwarded_name.into(), args.get(0))
            .unwrap_or(false)
        {
            throw_type_error(scope, &format!("{interface}.{member}: forwarding failed"));
        }
        return;
    }
    if interface == "TreeWalker" && member == "currentNode" {
        let result = stateful_id(scope, args.this()).and_then(|id| {
            let node = argument_node(scope, &state, args.get(0))?;
            let mut traversals = state.traversals.borrow_mut();
            let traversal = traversals
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("TreeWalker record"))?;
            traversal.index = traversal
                .nodes
                .iter()
                .position(|candidate| *candidate == node)
                .ok_or(V8BindingError::Unsupported {
                    interface: interface.into(),
                    member: "currentNode outside root".into(),
                })?;
            Ok(())
        });
        if let Err(error) = result {
            throw_type_error(scope, &format!("{interface}.{member}: {error}"));
        }
        return;
    }
    if interface == "History" && member == "scrollRestoration" {
        let value = args
            .get(0)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope));
        match value.as_deref() {
            Some("auto" | "manual") => {
                if let Err(error) = ensure_history(scope, &state) {
                    throw_binding_error(scope, &error);
                    return;
                }
                if let Some(history) = state.history.borrow_mut().as_mut() {
                    history.scroll_restoration = value.unwrap();
                }
            }
            _ => throw_type_error(
                scope,
                "History.scrollRestoration must be 'auto' or 'manual'",
            ),
        }
        return;
    }
    let result = resolve_receiver(scope, &state, args.this(), interface)
        .and_then(|node| set_attribute_value(scope, &state, node, interface, member, args.get(0)));
    if let Err(error) = result {
        throw_type_error(scope, &format!("{interface}.{member}: {error}"));
    }
}

fn generated_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut return_value: v8::ReturnValue,
) {
    let Some((interface, member)) = args.data().int32_value(scope).and_then(member_from_token)
    else {
        throw_type_error(scope, "invalid generated operation token");
        return;
    };
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    if interface == "IDBFactory" && member == "cmp" && args.length() < 2 {
        throw_type_error(scope, "IDBFactory.cmp requires two keys");
        return;
    }
    if interface == "EventTarget" && is_platform_receiver(scope, &state, args.this()) {
        match call_platform_event_target(scope, &state, args.this(), member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    if is_platform_interface(interface) {
        match call_platform_operation(scope, &state, args.this(), interface, member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    if interface == "Event" {
        match call_event_operation(scope, &state, args.this(), member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    if interface == "MutationObserver" {
        match call_mutation_observer_operation(scope, &state, args.this(), member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    if matches!(
        interface,
        "Range"
            | "Selection"
            | "NodeIterator"
            | "TreeWalker"
            | "DOMImplementation"
            | "History"
            | "Storage"
    ) {
        match call_stateful_operation(scope, &state, args.this(), interface, member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    let result = resolve_receiver(scope, &state, args.this(), interface)
        .and_then(|node| call_operation(scope, &state, node, interface, member, &args));
    // A DOM operation can connect a wrapper that was created while detached.
    // Promote it to an embedder root before V8 reaches another allocation/GC
    // point; waiting for the page-level checkpoint leaves a narrow race where
    // a framework can receive a fresh wrapper whose predecessor finalizer
    // subsequently releases the shared registry identity.
    state.refresh_connected_wrapper_roots(scope);
    match result {
        Ok(Some(value)) => return_value.set(value),
        Ok(None) => return_value.set(v8::undefined(scope).into()),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn is_platform_receiver(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> bool {
    // DOM wrappers use a BigInt lease in field zero and carry a complete
    // generation-checked identity in fields one through four.  Never attempt
    // to coerce that BigInt through the numeric stateful-object path: V8
    // correctly throws for an implicit BigInt-to-Number conversion.
    if embedded_node_identity(scope, object).is_some() {
        return false;
    }
    stateful_id(scope, object).is_ok_and(|id| state.platform_objects.borrow().contains_key(&id))
}

fn call_platform_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let event_type = args.get(0).to_rust_string_lossy(scope);
    match member {
        "addEventListener" => {
            let callback = args.get(1);
            if !callback.is_null_or_undefined() && (callback.is_function() || callback.is_object())
            {
                state
                    .platform_listeners
                    .borrow_mut()
                    .entry((id, event_type))
                    .or_default()
                    .push(v8::Global::new(scope, callback));
            }
            Ok(None)
        }
        "removeEventListener" => {
            let callback = args.get(1);
            if let Some(listeners) = state
                .platform_listeners
                .borrow_mut()
                .get_mut(&(id, event_type))
            {
                listeners.retain(|candidate| {
                    !v8::Local::new(scope, candidate).strict_equals(callback.into())
                });
            }
            Ok(None)
        }
        "dispatchEvent" => {
            let event = args
                .get(0)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("Event object"))?;
            let type_key =
                v8::String::new(scope, "type").ok_or(V8BindingError::Allocation("Event.type"))?;
            let event_type = event
                .get(scope, type_key.into())
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            dispatch_platform_event(scope, state, id, &event_type, Some(event))?;
            Ok(Some(v8::Boolean::new(scope, true).into()))
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "EventTarget".into(),
            member: member.into(),
        }),
    }
}

fn dispatch_platform_event(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    event_type: &str,
    existing: Option<v8::Local<v8::Object>>,
) -> Result<(), V8BindingError> {
    dispatch_platform_event_capturing(scope, state, id, event_type, existing).map(|_| ())
}

fn dispatch_platform_event_capturing(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    event_type: &str,
    existing: Option<v8::Local<v8::Object>>,
) -> Result<bool, V8BindingError> {
    if matches!(event_type, "success" | "error" | "upgradeneeded")
        && let Ok(mut record) = platform_object_mut(state, id)
        && let PlatformObjectRecord::IdbRequest(request) = &mut *record
    {
        request.done = true;
    }
    let event = existing.unwrap_or_else(|| v8::Object::new(scope));
    let default_prevented = v8::String::new(scope, "defaultPrevented")
        .ok_or(V8BindingError::Allocation("event defaultPrevented"))?;
    if !event
        .has_own_property(scope, default_prevented.into())
        .unwrap_or(false)
    {
        event.create_data_property(
            scope,
            default_prevented.into(),
            v8::Boolean::new(scope, false).into(),
        );
    }
    macro_rules! install_event_method {
        ($name:literal, $callback:ident) => {{
            let key = v8::String::new(scope, $name)
                .ok_or(V8BindingError::Allocation("platform event method"))?;
            if !event.has_own_property(scope, key.into()).unwrap_or(false) {
                let function = v8::Function::new(scope, $callback)
                    .ok_or(V8BindingError::Allocation("platform event method"))?;
                event.create_data_property(scope, key.into(), function.into());
            }
        }};
    }
    install_event_method!("preventDefault", platform_event_prevent_default);
    install_event_method!("stopPropagation", platform_event_stop);
    install_event_method!("stopImmediatePropagation", platform_event_stop);
    let type_key =
        v8::String::new(scope, "type").ok_or(V8BindingError::Allocation("event type"))?;
    let type_value = v8_string(scope, event_type)?;
    event.create_data_property(scope, type_key.into(), type_value);
    for (name, value) in [
        ("bubbles", event_type == "error"),
        ("cancelable", event_type == "error"),
        ("isTrusted", true),
    ] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("platform event property"))?;
        if !event.has_own_property(scope, key.into()).unwrap_or(false) {
            event.create_data_property(scope, key.into(), v8::Boolean::new(scope, value).into());
        }
    }
    let target = platform_interface_for_id(state, id)
        .and_then(|interface| new_platform_object(scope, state, interface, id).ok());
    if let Some(target) = target {
        let key =
            v8::String::new(scope, "target").ok_or(V8BindingError::Allocation("event target"))?;
        event.create_data_property(scope, key.into(), target.into());
    }
    let callbacks = state
        .platform_listeners
        .borrow()
        .get(&(id, event_type.into()))
        .cloned()
        .unwrap_or_default();
    let receiver = target.map_or_else(|| v8::undefined(scope).into(), Into::into);
    let mut threw = false;
    for callback in callbacks {
        threw |= invoke_platform_listener(scope, &callback, receiver, event);
    }
    let handler = {
        state
            .platform_handlers
            .borrow()
            .get(&(id, event_type.into()))
            .cloned()
    };
    if let Some(handler) = handler {
        threw |= invoke_platform_listener(scope, &handler, receiver, event);
    }
    Ok(threw)
}

fn invoke_platform_listener(
    scope: &mut v8::PinScope,
    listener: &v8::Global<v8::Value>,
    target: v8::Local<v8::Value>,
    event: v8::Local<v8::Object>,
) -> bool {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut try_catch = try_catch.init();
    let listener = v8::Local::new(&mut try_catch, listener);
    let (callback, receiver) = if let Ok(callback) = v8::Local::<v8::Function>::try_from(listener) {
        (Some(callback), target)
    } else if let Ok(object) = v8::Local::<v8::Object>::try_from(listener) {
        let callback = v8::String::new(&mut try_catch, "handleEvent")
            .and_then(|key| object.get(&mut try_catch, key.into()))
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
        (callback, object.into())
    } else {
        (None, target)
    };
    let succeeded = callback.is_some_and(|callback| {
        callback
            .call(&mut try_catch, receiver, &[event.into()])
            .is_some()
    });
    let threw = !succeeded || try_catch.has_caught();
    if try_catch.has_caught() {
        try_catch.reset();
    }
    threw
}

fn platform_event_prevent_default(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let receiver = args.this();
    if let Some(key) = v8::String::new(scope, "defaultPrevented") {
        receiver.create_data_property(scope, key.into(), v8::Boolean::new(scope, true).into());
    }
}

fn platform_event_stop(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
}

fn dispatch_platform_message_event(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    data: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let event = v8::Object::new(scope);
    let key = v8::String::new(scope, "data").ok_or(V8BindingError::Allocation("message data"))?;
    event.create_data_property(scope, key.into(), data);
    dispatch_platform_event(scope, state, id, "message", Some(event))
}

fn record_platform_trace(scope: &mut v8::PinScope, kind: &str, detail: serde_json::Value) {
    let global = scope.get_current_context().global(scope);
    let Some(name) = v8::String::new(scope, "__oryn_context_trace") else {
        return;
    };
    let Some(value) = global.get(scope, name.into()) else {
        return;
    };
    let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
        return;
    };
    let Ok(encoded) = serde_json::to_string(&serde_json::json!({"kind":kind,"detail":detail}))
    else {
        return;
    };
    let Some(encoded) = v8::String::new(scope, &encoded) else {
        return;
    };
    let Some(entry) = v8::json::parse(scope, encoded) else {
        return;
    };
    array.set_index(scope, array.length(), entry);
}

fn platform_interface_for_id(state: &V8BindingState, id: u64) -> Option<&'static str> {
    Some(match state.platform_objects.borrow().get(&id)? {
        PlatformObjectRecord::AbortSignal(_) => "AbortSignal",
        PlatformObjectRecord::XMLHttpRequest(_) => "XMLHttpRequest",
        PlatformObjectRecord::WebSocket(_) => "WebSocket",
        PlatformObjectRecord::IdbRequest(_) => "IDBRequest",
        PlatformObjectRecord::IdbDatabase(_) => "IDBDatabase",
        PlatformObjectRecord::IdbTransaction(_) => "IDBTransaction",
        _ => return None,
    })
}

fn stateful_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<u64, V8BindingError> {
    object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| value.integer_value(scope))
        .filter(|value| *value >= 1)
        .map(|value| value as u64)
        .ok_or(V8BindingError::Allocation("stateful interface identity"))
}

fn new_stateful_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    interface: &'static str,
    id: u64,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .cloned()
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("stateful interface object"))?;
    object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
    Ok(object)
}

fn traversal_node_type(dom: &DomRuntime, node: NodeId) -> Result<u32, DomError> {
    Ok(match dom.arena().get(node)?.kind {
        NodeKind::Element { .. } => 1,
        NodeKind::Attribute { .. } => 2,
        NodeKind::Text { .. } => 3,
        NodeKind::CDataSection { .. } => 4,
        NodeKind::ProcessingInstruction { .. } => 7,
        NodeKind::Comment { .. } => 8,
        NodeKind::Document => 9,
        NodeKind::DocumentType { .. } => 10,
        NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => 11,
    })
}

fn traversal_filter_result(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    filter: v8::Local<v8::Value>,
    node: NodeId,
) -> Result<u32, V8BindingError> {
    if filter.is_null_or_undefined() {
        return Ok(1);
    }
    let receiver = filter
        .to_object(scope)
        .ok_or(V8BindingError::Allocation("NodeFilter object"))?;
    let function = if let Ok(function) = v8::Local::<v8::Function>::try_from(filter) {
        function
    } else {
        let name = v8::String::new(scope, "acceptNode")
            .ok_or(V8BindingError::Allocation("NodeFilter.acceptNode name"))?;
        v8::Local::<v8::Function>::try_from(
            receiver
                .get(scope, name.into())
                .ok_or(V8BindingError::Allocation("NodeFilter.acceptNode"))?,
        )
        .map_err(|_| V8BindingError::Allocation("NodeFilter.acceptNode function"))?
    };
    let wrapper = wrap_node(scope, state, node)?;
    let result = function
        .call(scope, receiver.into(), &[wrapper.into()])
        .ok_or(V8BindingError::Allocation("NodeFilter callback result"))?;
    Ok(result.uint32_value(scope).unwrap_or(1))
}

fn filtered_traversal_nodes(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    what_to_show: u32,
    filter: v8::Local<v8::Value>,
    prune_rejected: bool,
) -> Result<(Vec<NodeId>, BTreeSet<NodeId>), V8BindingError> {
    fn visit(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        node: NodeId,
        what_to_show: u32,
        filter: v8::Local<v8::Value>,
        prune_rejected: bool,
        output: &mut Vec<NodeId>,
    ) -> Result<(), V8BindingError> {
        let (shown, children) = {
            let dom = state.dom.borrow();
            let node_type = traversal_node_type(&dom, node)?;
            (
                what_to_show == u32::MAX || what_to_show & (1 << (node_type - 1)) != 0,
                light_children(&dom, node)?,
            )
        };
        let decision = if shown {
            traversal_filter_result(scope, state, filter, node)?
        } else {
            3
        };
        if decision == 1 {
            output.push(node);
        }
        if !(decision == 2 && prune_rejected) {
            for child in children {
                visit(
                    scope,
                    state,
                    child,
                    what_to_show,
                    filter,
                    prune_rejected,
                    output,
                )?;
            }
        }
        Ok(())
    }
    let mut nodes = Vec::new();
    visit(
        scope,
        state,
        root,
        what_to_show,
        filter,
        prune_rejected,
        &mut nodes,
    )?;
    let accepted = nodes.iter().copied().collect::<BTreeSet<_>>();
    if !accepted.contains(&root) {
        // TreeWalker.currentNode and NodeIterator.referenceNode start at root
        // even when root is not visible through the filter.
        nodes.insert(0, root);
    }
    Ok((nodes, accepted))
}

fn light_children(dom: &DomRuntime, node: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
    Ok(dom
        .children(node)?
        .iter()
        .copied()
        .filter(|child| {
            !matches!(
                dom.arena().get(*child).map(|record| &record.kind),
                Ok(NodeKind::ShadowRoot { .. })
            )
        })
        .collect())
}

fn traversal_nodes(dom: &DomRuntime, root: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
    fn visit(dom: &DomRuntime, node: NodeId, output: &mut Vec<NodeId>) -> Result<(), DomError> {
        output.push(node);
        for child in dom.children(node)? {
            if matches!(dom.arena().get(*child)?.kind, NodeKind::ShadowRoot { .. }) {
                continue;
            }
            visit(dom, *child, output)?;
        }
        Ok(())
    }
    let mut nodes = Vec::new();
    visit(dom, root, &mut nodes)?;
    Ok(nodes)
}

fn common_ancestor(
    dom: &DomRuntime,
    left: NodeId,
    right: NodeId,
) -> Result<NodeId, V8BindingError> {
    let mut left_ancestors = BTreeSet::new();
    let mut cursor = Some(left);
    while let Some(node) = cursor {
        left_ancestors.insert(node);
        if matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. }) {
            break;
        }
        cursor = dom.parent(node)?;
    }
    let mut cursor = Some(right);
    while let Some(node) = cursor {
        if left_ancestors.contains(&node) {
            return Ok(node);
        }
        if matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. }) {
            break;
        }
        cursor = dom.parent(node)?;
    }
    Err(V8BindingError::Unsupported {
        interface: "Range".into(),
        member: "WrongDocumentError".into(),
    })
}

fn relative_boundary(
    dom: &DomRuntime,
    node: NodeId,
    after: bool,
) -> Result<(NodeId, u32), V8BindingError> {
    if matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. }) {
        return Err(V8BindingError::Unsupported {
            interface: "Range".into(),
            member: "InvalidNodeTypeError".into(),
        });
    }
    let parent = dom.parent(node)?.ok_or(V8BindingError::Unsupported {
        interface: "Range".into(),
        member: "InvalidNodeTypeError".into(),
    })?;
    let offset = light_children(dom, parent)?
        .iter()
        .position(|candidate| *candidate == node)
        .ok_or(V8BindingError::Allocation("range boundary child"))?
        + usize::from(after);
    Ok((parent, offset as u32))
}

fn traversal_position(dom: &DomRuntime, node: NodeId) -> Result<usize, V8BindingError> {
    let root = tree_root(dom, node)?;
    traversal_nodes(dom, root)?
        .iter()
        .position(|candidate| *candidate == node)
        .ok_or(V8BindingError::Allocation("range traversal position"))
}

fn tree_root(dom: &DomRuntime, mut node: NodeId) -> Result<NodeId, V8BindingError> {
    while !matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. })
        && let Some(parent) = dom.parent(node)?
    {
        node = parent;
    }
    Ok(node)
}

fn direct_child_offset(
    dom: &DomRuntime,
    ancestor: NodeId,
    descendant: NodeId,
) -> Result<Option<u32>, V8BindingError> {
    let mut child = descendant;
    loop {
        let Some(parent) = dom.parent(child)? else {
            return Ok(None);
        };
        if parent == ancestor {
            return Ok(light_children(dom, ancestor)?
                .iter()
                .position(|candidate| *candidate == child)
                .map(|offset| offset as u32));
        }
        child = parent;
    }
}

fn boundary_length(dom: &DomRuntime, node: NodeId) -> Result<u32, V8BindingError> {
    Ok(match &dom.arena().get(node)?.kind {
        NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::ProcessingInstruction { data, .. }
        | NodeKind::Comment { data } => data.encode_utf16().count() as u32,
        _ => light_children(dom, node)?.len() as u32,
    })
}

fn validate_boundary_offset(
    dom: &DomRuntime,
    node: NodeId,
    offset: u32,
) -> Result<(), V8BindingError> {
    if offset > boundary_length(dom, node)? {
        return Err(V8BindingError::Unsupported {
            interface: "Range".into(),
            member: "IndexSizeError".into(),
        });
    }
    Ok(())
}

fn compare_boundary_points(
    dom: &DomRuntime,
    left: (NodeId, u32),
    right: (NodeId, u32),
) -> Result<i32, V8BindingError> {
    if left.0 == right.0 {
        return Ok(match left.1.cmp(&right.1) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        });
    }
    if let Some(offset) = direct_child_offset(dom, left.0, right.0)? {
        return Ok(if left.1 <= offset { -1 } else { 1 });
    }
    if let Some(offset) = direct_child_offset(dom, right.0, left.0)? {
        return Ok(if offset < right.1 { -1 } else { 1 });
    }
    let left_position = traversal_position(dom, left.0)?;
    let right_position = traversal_position(dom, right.0)?;
    Ok(match left_position.cmp(&right_position) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeContentsMode {
    Clone,
    Extract,
    Delete,
}

fn utf16_slice(value: &str, start: u32, end: u32) -> String {
    let units = value.encode_utf16().collect::<Vec<_>>();
    let start = (start as usize).min(units.len());
    let end = (end as usize).clamp(start, units.len());
    String::from_utf16_lossy(&units[start..end])
}

fn utf16_without_range(value: &str, start: u32, end: u32) -> String {
    let units = value.encode_utf16().collect::<Vec<_>>();
    let start = (start as usize).min(units.len());
    let end = (end as usize).clamp(start, units.len());
    let mut retained = Vec::with_capacity(units.len() - (end - start));
    retained.extend_from_slice(&units[..start]);
    retained.extend_from_slice(&units[end..]);
    String::from_utf16_lossy(&retained)
}

fn utf16_replace(value: &str, start: u32, count: u32, replacement: &str) -> String {
    let units = value.encode_utf16().collect::<Vec<_>>();
    let start = (start as usize).min(units.len());
    let end = start.saturating_add(count as usize).min(units.len());
    let mut output =
        Vec::with_capacity(units.len() - (end - start) + replacement.encode_utf16().count());
    output.extend_from_slice(&units[..start]);
    output.extend(replacement.encode_utf16());
    output.extend_from_slice(&units[end..]);
    String::from_utf16_lossy(&output)
}

fn create_character_data_like(
    dom: &mut DomRuntime,
    kind: &NodeKind,
    data: String,
) -> Result<NodeId, V8BindingError> {
    Ok(match kind {
        NodeKind::Text { .. } => dom.create_text(data),
        NodeKind::CDataSection { .. } => dom.create_cdata_section(data),
        NodeKind::ProcessingInstruction { target, .. } => {
            dom.create_processing_instruction(target.clone(), data)
        }
        NodeKind::Comment { .. } => dom.create_comment(data),
        _ => return Err(V8BindingError::Allocation("character data kind")),
    })
}

fn range_contents(
    dom: &mut DomRuntime,
    range: &RangeRecord,
    mode: RangeContentsMode,
) -> Result<NodeId, V8BindingError> {
    let fragment = dom.create_fragment();
    if compare_boundary_points(
        dom,
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? == 0
    {
        return Ok(fragment);
    }
    let container = range.start_container;
    if range.start_container == range.end_container {
        let kind = dom.arena().get(container)?.kind.clone();
        match &kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => {
                let selected = utf16_slice(data, range.start_offset, range.end_offset);
                if mode != RangeContentsMode::Delete && !selected.is_empty() {
                    let selected = create_character_data_like(dom, &kind, selected)?;
                    dom.append_child(fragment, selected)?;
                }
                if mode != RangeContentsMode::Clone {
                    let retained = utf16_without_range(data, range.start_offset, range.end_offset);
                    dom.set_text(container, retained)?;
                }
            }
            _ => {
                let children = light_children(dom, container)?;
                let start = (range.start_offset as usize).min(children.len());
                let end = (range.end_offset as usize).clamp(start, children.len());
                for child in children[start..end].iter().copied() {
                    match mode {
                        RangeContentsMode::Clone => {
                            let clone = dom.clone_node(child, true)?;
                            dom.append_child(fragment, clone)?;
                        }
                        RangeContentsMode::Extract => dom.append_child(fragment, child)?,
                        RangeContentsMode::Delete => dom.remove_child(container, child)?,
                    }
                }
            }
        }
        return Ok(fragment);
    }

    let common = common_ancestor(dom, range.start_container, range.end_container)?;
    for child in light_children(dom, common)? {
        if let Some(selected) = range_process_node(dom, child, range, mode)? {
            dom.append_child(fragment, selected)?;
        }
    }
    Ok(fragment)
}

fn range_process_node(
    dom: &mut DomRuntime,
    node: NodeId,
    range: &RangeRecord,
    mode: RangeContentsMode,
) -> Result<Option<NodeId>, V8BindingError> {
    let before = relative_boundary(dom, node, false)?;
    let after = relative_boundary(dom, node, true)?;
    let start = (range.start_container, range.start_offset);
    let end = (range.end_container, range.end_offset);
    if compare_boundary_points(dom, end, before)? <= 0
        || compare_boundary_points(dom, start, after)? >= 0
    {
        return Ok(None);
    }
    let fully_contained = compare_boundary_points(dom, start, before)? <= 0
        && compare_boundary_points(dom, after, end)? <= 0;
    if fully_contained {
        return match mode {
            RangeContentsMode::Clone => Ok(Some(dom.clone_node(node, true)?)),
            RangeContentsMode::Extract => Ok(Some(node)),
            RangeContentsMode::Delete => {
                let parent = dom.parent(node)?.ok_or(V8BindingError::Unsupported {
                    interface: "Range".into(),
                    member: "InvalidNodeTypeError".into(),
                })?;
                dom.remove_child(parent, node)?;
                Ok(None)
            }
        };
    }

    let kind = dom.arena().get(node)?.kind.clone();
    if let NodeKind::Text { data }
    | NodeKind::CDataSection { data }
    | NodeKind::ProcessingInstruction { data, .. }
    | NodeKind::Comment { data } = &kind
    {
        let start_offset = if node == range.start_container {
            range.start_offset
        } else {
            0
        };
        let end_offset = if node == range.end_container {
            range.end_offset
        } else {
            data.encode_utf16().count() as u32
        };
        let selected = utf16_slice(data, start_offset, end_offset);
        let output = if mode == RangeContentsMode::Delete || selected.is_empty() {
            None
        } else {
            Some(create_character_data_like(dom, &kind, selected)?)
        };
        if mode != RangeContentsMode::Clone {
            dom.set_text(node, utf16_without_range(data, start_offset, end_offset))?;
        }
        return Ok(output);
    }

    let shell = (mode != RangeContentsMode::Delete)
        .then(|| dom.clone_node(node, false))
        .transpose()?;
    if let Some(shell) = shell {
        let clonable_roots = dom
            .children(node)?
            .iter()
            .copied()
            .filter(|child| {
                matches!(
                    dom.arena().get(*child).map(|record| &record.kind),
                    Ok(NodeKind::ShadowRoot { clonable: true, .. })
                )
            })
            .collect::<Vec<_>>();
        for root in clonable_roots {
            let root = dom.clone_node(root, true)?;
            dom.append_child(shell, root)?;
        }
    }
    for child in light_children(dom, node)? {
        if let Some(selected) = range_process_node(dom, child, range, mode)?
            && let Some(shell) = shell
        {
            dom.append_child(shell, selected)?;
        }
    }
    Ok(shell)
}

fn range_partially_contains_non_text(
    dom: &DomRuntime,
    range: &RangeRecord,
) -> Result<bool, V8BindingError> {
    let common = common_ancestor(dom, range.start_container, range.end_container)?;
    let start = (range.start_container, range.start_offset);
    let end = (range.end_container, range.end_offset);
    for node in traversal_nodes(dom, common)?.into_iter().skip(1) {
        if matches!(dom.arena().get(node)?.kind, NodeKind::Text { .. }) {
            continue;
        }
        let before = relative_boundary(dom, node, false)?;
        let after = relative_boundary(dom, node, true)?;
        let intersects = compare_boundary_points(dom, end, before)? > 0
            && compare_boundary_points(dom, start, after)? < 0;
        let contained = compare_boundary_points(dom, start, before)? <= 0
            && compare_boundary_points(dom, after, end)? <= 0;
        if intersects && !contained {
            return Ok(true);
        }
    }
    Ok(false)
}

fn range_text(dom: &DomRuntime, range: &RangeRecord) -> Result<String, V8BindingError> {
    if range.start_container == range.end_container {
        let container = dom.arena().get(range.start_container)?;
        return match &container.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. } => {
                Ok(utf16_slice(data, range.start_offset, range.end_offset))
            }
            _ => {
                let children = light_children(dom, range.start_container)?;
                let start = (range.start_offset as usize).min(children.len());
                let end = (range.end_offset as usize).clamp(start, children.len());
                let mut text = String::new();
                for child in &children[start..end] {
                    text.push_str(&dom.arena().text_content(*child)?);
                }
                Ok(text)
            }
        };
    }
    let nodes = traversal_nodes(
        dom,
        common_ancestor(dom, range.start_container, range.end_container)?,
    )?;
    let start = nodes
        .iter()
        .position(|node| *node == range.start_container)
        .ok_or(V8BindingError::Allocation("Range start"))?;
    let end = nodes
        .iter()
        .position(|node| *node == range.end_container)
        .ok_or(V8BindingError::Allocation("Range end"))?;
    let mut text = String::new();
    for node in &nodes[start..=end] {
        if let NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::ProcessingInstruction { data, .. } = &dom.arena().get(*node)?.kind
        {
            let value = if *node == range.start_container {
                utf16_slice(data, range.start_offset, data.encode_utf16().count() as u32)
            } else if *node == range.end_container {
                utf16_slice(data, 0, range.end_offset)
            } else {
                data.clone()
            };
            text.push_str(&value);
        }
    }
    Ok(text)
}

fn insert_at_boundary(
    dom: &mut DomRuntime,
    container: NodeId,
    offset: u32,
    node: NodeId,
) -> Result<(), V8BindingError> {
    match dom.arena().get(container)?.kind.clone() {
        NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::ProcessingInstruction { data, .. }
        | NodeKind::Comment { data } => {
            let parent = dom.parent(container)?.ok_or(V8BindingError::Unsupported {
                interface: "Range".into(),
                member: "HierarchyRequestError".into(),
            })?;
            let units = data.encode_utf16().collect::<Vec<_>>();
            let split = (offset as usize).min(units.len());
            let left = String::from_utf16_lossy(&units[..split]);
            let right = String::from_utf16_lossy(&units[split..]);
            let is_comment = matches!(dom.arena().get(container)?.kind, NodeKind::Comment { .. });
            dom.set_text(container, left)?;
            let trailing = if is_comment {
                dom.create_comment(right)
            } else {
                dom.create_text(right)
            };
            let next = light_children(dom, parent)?
                .iter()
                .position(|child| *child == container)
                .and_then(|index| light_children(dom, parent).ok()?.get(index + 1).copied());
            dom.insert_before(parent, trailing, next)?;
            insert_range_node(dom, parent, node, Some(trailing))
        }
        _ => {
            let reference = light_children(dom, container)?
                .get(offset as usize)
                .copied();
            insert_range_node(dom, container, node, reference)
        }
    }
}

fn insert_range_node(
    dom: &mut DomRuntime,
    parent: NodeId,
    node: NodeId,
    reference: Option<NodeId>,
) -> Result<(), V8BindingError> {
    if matches!(dom.arena().get(node)?.kind, NodeKind::DocumentFragment) {
        for child in dom.children(node)?.to_vec() {
            dom.insert_before(parent, child, reference)?;
        }
    } else {
        dom.insert_before(parent, node, reference)?;
    }
    Ok(())
}

fn adjacent_insertion_point(
    dom: &DomRuntime,
    element: NodeId,
    position: &str,
) -> Result<(NodeId, Option<NodeId>), V8BindingError> {
    match position.to_ascii_lowercase().as_str() {
        "beforebegin" => Ok((
            dom.parent(element)?.ok_or(V8BindingError::Unsupported {
                interface: "Element".into(),
                member: "NoModificationAllowedError".into(),
            })?,
            Some(element),
        )),
        "afterbegin" => Ok((element, light_children(dom, element)?.first().copied())),
        "beforeend" => Ok((element, None)),
        "afterend" => {
            let parent = dom.parent(element)?.ok_or(V8BindingError::Unsupported {
                interface: "Element".into(),
                member: "NoModificationAllowedError".into(),
            })?;
            let children = light_children(dom, parent)?;
            let index = children
                .iter()
                .position(|child| *child == element)
                .ok_or(V8BindingError::Allocation("adjacent element index"))?;
            Ok((parent, children.get(index + 1).copied()))
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "Element".into(),
            member: "SyntaxError".into(),
        }),
    }
}

fn synchronize_live_ranges(state: &V8BindingState) -> Result<(), V8BindingError> {
    let ids = state.ranges.borrow().keys().copied().collect::<Vec<_>>();
    let selection_id = state.selection_range.get();
    let selection_was_backward = selection_id.is_some_and(|id| {
        let ranges = state.ranges.borrow();
        let Some(range) = ranges.get(&id) else {
            return false;
        };
        state.selection_anchor.get().is_some_and(|anchor| {
            anchor.container == range.end_container && anchor.offset == range.end_offset
        })
    });
    let dom = state.dom.borrow();
    let mut ranges = state.ranges.borrow_mut();
    for id in ids {
        ranges.insert(id, dom.live_range(LiveRangeId(id))?.into());
    }
    if let Some(id) = selection_id
        && let Some(range) = ranges.get(&id)
    {
        let start = BoundaryPoint {
            container: range.start_container,
            offset: range.start_offset,
        };
        let end = BoundaryPoint {
            container: range.end_container,
            offset: range.end_offset,
        };
        if selection_was_backward {
            state.selection_anchor.set(Some(end));
            state.selection_focus.set(Some(start));
        } else {
            state.selection_anchor.set(Some(start));
            state.selection_focus.set(Some(end));
        }
    }
    Ok(())
}

fn persist_live_range(state: &V8BindingState, id: u64) -> Result<(), V8BindingError> {
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let live = LiveRange::from(&range);
    state
        .dom
        .borrow_mut()
        .set_live_range(LiveRangeId(id), live.start, live.end)?;
    Ok(())
}

fn register_live_range(state: &V8BindingState, range: RangeRecord) -> Result<u64, V8BindingError> {
    let live = LiveRange::from(&range);
    let id = state
        .dom
        .borrow_mut()
        .create_live_range(live.start, live.end)?
        .0;
    state.ranges.borrow_mut().insert(id, range);
    Ok(id)
}

fn set_selection_boundaries(
    state: &V8BindingState,
    anchor: BoundaryPoint,
    focus: BoundaryPoint,
) -> Result<u64, V8BindingError> {
    validate_boundary_offset(&state.dom.borrow(), anchor.container, anchor.offset)?;
    validate_boundary_offset(&state.dom.borrow(), focus.container, focus.offset)?;
    let (start, end) = if tree_root(&state.dom.borrow(), anchor.container)?
        != tree_root(&state.dom.borrow(), focus.container)?
    {
        // A Selection may expose anchor/focus points in different composed
        // trees, but its script-visible Range cannot span tree roots.
        (focus, focus)
    } else if compare_boundary_points(
        &state.dom.borrow(),
        (anchor.container, anchor.offset),
        (focus.container, focus.offset),
    )? <= 0
    {
        (anchor, focus)
    } else {
        (focus, anchor)
    };
    let range = RangeRecord {
        start_container: start.container,
        start_offset: start.offset,
        end_container: end.container,
        end_offset: end.offset,
    };
    let id = if let Some(id) = state.selection_range.get() {
        state.ranges.borrow_mut().insert(id, range);
        persist_live_range(state, id)?;
        id
    } else {
        register_live_range(state, range)?
    };
    state.selection_range.set(Some(id));
    state.selection_anchor.set(Some(anchor));
    state.selection_focus.set(Some(focus));
    Ok(id)
}

fn clear_selection(state: &V8BindingState) {
    state.selection_range.set(None);
    state.selection_anchor.set(None);
    state.selection_focus.set(None);
}

fn edit_command_supported(command: &str) -> bool {
    matches!(
        command,
        "delete"
            | "forwarddelete"
            | "inserthtml"
            | "insertlinebreak"
            | "insertparagraph"
            | "inserttext"
            | "selectall"
            | "stylewithcss"
    )
}

fn document_is_editable(state: &V8BindingState) -> bool {
    if state.design_mode.get() {
        return true;
    }
    let dom = state.dom.borrow();
    let mut current = dom
        .focused()
        .or_else(|| state.selection_anchor.get().map(|point| point.container));
    while let Some(node) = current {
        let Ok(record) = dom.arena().get(node) else {
            return false;
        };
        if let Some(value) = record.attributes.get("contenteditable") {
            return !value.eq_ignore_ascii_case("false");
        }
        current = record.parent;
    }
    false
}

fn replace_selection_with_text(
    state: &V8BindingState,
    replacement: &str,
) -> Result<(), V8BindingError> {
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "no active selection".into(),
        })?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    if compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0
    {
        range_contents(
            &mut state.dom.borrow_mut(),
            &range,
            RangeContentsMode::Delete,
        )?;
    }
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    if !replacement.is_empty() {
        let kind = state
            .dom
            .borrow()
            .arena()
            .get(point.container)?
            .kind
            .clone();
        match kind {
            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                let updated = utf16_replace(&data, point.offset, 0, replacement);
                state.dom.borrow_mut().set_text(point.container, updated)?;
                point.offset = point
                    .offset
                    .saturating_add(replacement.encode_utf16().count() as u32);
            }
            _ => {
                let text = state.dom.borrow_mut().create_text(replacement);
                insert_at_boundary(
                    &mut state.dom.borrow_mut(),
                    point.container,
                    point.offset,
                    text,
                )?;
                let (container, offset) = relative_boundary(&state.dom.borrow(), text, true)?;
                point = BoundaryPoint { container, offset };
            }
        }
    }
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

fn delete_selection_direction(state: &V8BindingState, forward: bool) -> Result<(), V8BindingError> {
    let Some(id) = state.selection_range.get() else {
        return Ok(());
    };
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    if compare_boundary_points(
        &state.dom.borrow(),
        (range.start_container, range.start_offset),
        (range.end_container, range.end_offset),
    )? != 0
    {
        return replace_selection_with_text(state, "");
    }
    let kind = state
        .dom
        .borrow()
        .arena()
        .get(range.start_container)?
        .kind
        .clone();
    if let NodeKind::Text { data } | NodeKind::CDataSection { data } = kind {
        let length = data.encode_utf16().count() as u32;
        let (start, end) = if forward {
            (
                range.start_offset,
                range.start_offset.saturating_add(1).min(length),
            )
        } else {
            (range.start_offset.saturating_sub(1), range.start_offset)
        };
        state.dom.borrow_mut().set_text(
            range.start_container,
            utf16_without_range(&data, start, end),
        )?;
        let point = BoundaryPoint {
            container: range.start_container,
            offset: start,
        };
        set_selection_boundaries(state, point, point)?;
    } else {
        let adjacent = {
            let dom = state.dom.borrow();
            let children = dom.arena().get(range.start_container)?.children.clone();
            if forward {
                children.get(range.start_offset as usize).copied()
            } else {
                range
                    .start_offset
                    .checked_sub(1)
                    .and_then(|index| children.get(index as usize))
                    .copied()
            }
        };
        if let Some(adjacent) = adjacent {
            let adjacent_kind = state.dom.borrow().arena().get(adjacent)?.kind.clone();
            if let NodeKind::Text { data } | NodeKind::CDataSection { data } = adjacent_kind {
                let length = data.encode_utf16().count() as u32;
                let (start, end) = if forward {
                    (0, 1.min(length))
                } else {
                    (length.saturating_sub(1), length)
                };
                state
                    .dom
                    .borrow_mut()
                    .set_text(adjacent, utf16_without_range(&data, start, end))?;
                let point = if forward {
                    BoundaryPoint {
                        container: range.start_container,
                        offset: range.start_offset,
                    }
                } else {
                    BoundaryPoint {
                        container: adjacent,
                        offset: start,
                    }
                };
                set_selection_boundaries(state, point, point)?;
            }
        }
    }
    Ok(())
}

fn replace_selection_with_html(state: &V8BindingState, markup: &str) -> Result<(), V8BindingError> {
    replace_selection_with_text(state, "")?;
    let id = state
        .selection_range
        .get()
        .ok_or(V8BindingError::Allocation("selection range"))?;
    let range = state
        .ranges
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("Range record"))?;
    let parsed = oryn_dom::html::parse_fragment(markup, "div");
    let children = parsed.dom.get(parsed.document)?.children.clone();
    let mut point = BoundaryPoint {
        container: range.start_container,
        offset: range.start_offset,
    };
    for child in children {
        let imported = state.dom.borrow_mut().import_subtree(&parsed.dom, child)?;
        insert_at_boundary(
            &mut state.dom.borrow_mut(),
            point.container,
            point.offset,
            imported,
        )?;
        let (container, offset) = relative_boundary(&state.dom.borrow(), imported, true)?;
        point = BoundaryPoint { container, offset };
    }
    set_selection_boundaries(state, point, point)?;
    Ok(())
}

fn global_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("global key"))?;
    scope
        .get_current_context()
        .global(scope)
        .get(scope, key.into())
        .ok_or(V8BindingError::Allocation("global property"))
}

fn location_href(scope: &mut v8::PinScope) -> Result<String, V8BindingError> {
    let location = global_property(scope, "location")?
        .to_object(scope)
        .ok_or(V8BindingError::Allocation("location object"))?;
    let key = v8::String::new(scope, "href").ok_or(V8BindingError::Allocation("href key"))?;
    location
        .get(scope, key.into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .ok_or(V8BindingError::Allocation("location href"))
}

fn set_location_href(scope: &mut v8::PinScope, href: &str) -> Result<(), V8BindingError> {
    let location = global_property(scope, "location")?
        .to_object(scope)
        .ok_or(V8BindingError::Allocation("location object"))?;
    let key = v8::String::new(scope, "href").ok_or(V8BindingError::Allocation("href key"))?;
    let value = v8::String::new(scope, href).ok_or(V8BindingError::Allocation("history URL"))?;
    if !location
        .set(scope, key.into(), value.into())
        .unwrap_or(false)
    {
        return Err(V8BindingError::Allocation("location href assignment"));
    }
    Ok(())
}

fn ensure_history(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.history.borrow().is_none() {
        let url = location_href(scope)?;
        state.history.borrow_mut().replace(HistoryRecord {
            entries: vec![HistoryEntry {
                url,
                state_json: Some("null".into()),
            }],
            index: 0,
            scroll_restoration: "auto".into(),
        });
    }
    Ok(())
}

fn parse_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    encoded: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let encoded =
        v8::String::new(scope, encoded).ok_or(V8BindingError::Allocation("history state"))?;
    v8::json::parse(scope, encoded).ok_or(V8BindingError::Allocation("history state parse"))
}

fn clone_history_state(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<String>, V8BindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    v8::json::stringify(scope, value)
        .map(|value| Some(value.to_rust_string_lossy(scope)))
        .ok_or(V8BindingError::Unsupported {
            interface: "History".into(),
            member: "DataCloneError".into(),
        })
}

fn history_url(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<String, V8BindingError> {
    let current = location_href(scope)?;
    if value.is_null_or_undefined() {
        return Ok(current);
    }
    let input = value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .ok_or(V8BindingError::Allocation("history URL conversion"))?;
    if input.is_empty() {
        return Ok(current);
    }
    if current.starts_with("about:blank") {
        if input.starts_with('#') {
            return Ok(format!(
                "{}{}",
                current.split('#').next().unwrap_or("about:blank"),
                input
            ));
        }
        if input.starts_with('?') {
            return Ok(format!(
                "{}{}",
                current.split(['?', '#']).next().unwrap_or("about:blank"),
                input
            ));
        }
    }
    let base = Url::parse(&current).map_err(|_| V8BindingError::Unsupported {
        interface: "History".into(),
        member: "SecurityError".into(),
    })?;
    let target = base.join(&input).map_err(|_| V8BindingError::Unsupported {
        interface: "History".into(),
        member: "SecurityError".into(),
    })?;
    if base.origin() != target.origin() {
        return Err(V8BindingError::Unsupported {
            interface: "History".into(),
            member: "SecurityError".into(),
        });
    }
    Ok(target.into())
}

fn dispatch_global_history_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    event_type: &str,
    properties: &[(&str, v8::Local<v8::Value>)],
) -> Result<(), V8BindingError> {
    let event = create_event_object(scope, state, event_type, false, false)?;
    for (name, value) in properties {
        let name = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("history event property"))?;
        event.create_data_property(scope, name.into(), *value);
    }
    let dispatch = v8::Local::<v8::Function>::try_from(global_property(scope, "dispatchEvent")?)
        .map_err(|_| V8BindingError::Allocation("global dispatchEvent"))?;
    let receiver = scope.get_current_context().global(scope);
    dispatch
        .call(scope, receiver.into(), &[event.into()])
        .ok_or(V8BindingError::EventCallback(event_type.into()))?;
    Ok(())
}

fn call_history_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    ensure_history(scope, state)?;
    match member {
        "pushState" | "replaceState" => {
            let state_json = clone_history_state(scope, args.get(0))?;
            let url = history_url(scope, args.get(2))?;
            {
                let mut history = state.history.borrow_mut();
                let history = history
                    .as_mut()
                    .ok_or(V8BindingError::Allocation("History record"))?;
                let entry = HistoryEntry {
                    url: url.clone(),
                    state_json,
                };
                if member == "pushState" {
                    let insertion = history.index.saturating_add(1);
                    history.entries.truncate(insertion);
                    history.entries.push(entry);
                    history.index = history.entries.len() - 1;
                } else {
                    history.entries[history.index] = entry;
                }
            }
            set_location_href(scope, &url)?;
        }
        "back" | "forward" | "go" => {
            let delta = match member {
                "back" => -1,
                "forward" => 1,
                _ => args.get(0).integer_value(scope).unwrap_or(0),
            };
            if delta == 0 {
                return Ok(None);
            }
            let previous = location_href(scope)?;
            let selected = {
                let mut history = state.history.borrow_mut();
                let history = history
                    .as_mut()
                    .ok_or(V8BindingError::Allocation("History record"))?;
                let next = history.index as i64 + delta;
                if next < 0 || next >= history.entries.len() as i64 {
                    None
                } else {
                    history.index = next as usize;
                    Some(history.entries[history.index].clone())
                }
            };
            if let Some(entry) = selected {
                set_location_href(scope, &entry.url)?;
                let event_state = if let Some(encoded) = entry.state_json.as_deref() {
                    parse_json(scope, encoded)?
                } else {
                    v8::undefined(scope).into()
                };
                dispatch_global_history_event(scope, state, "popstate", &[("state", event_state)])?;
                let old_hash = Url::parse(&previous)
                    .ok()
                    .and_then(|url| url.fragment().map(str::to_owned));
                let new_hash = Url::parse(&entry.url)
                    .ok()
                    .and_then(|url| url.fragment().map(str::to_owned));
                if old_hash != new_hash {
                    let old_url = v8_string(scope, &previous)?;
                    let new_url = v8_string(scope, &entry.url)?;
                    dispatch_global_history_event(
                        scope,
                        state,
                        "hashchange",
                        &[("oldURL", old_url), ("newURL", new_url)],
                    )?;
                }
            }
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: "History".into(),
                member: member.into(),
            });
        }
    }
    Ok(None)
}

fn restore_storage_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    let mut fallback = state.storage_fallback.borrow_mut();
    for (index, destination) in fallback.iter_mut().enumerate() {
        destination.clear();
        let Ok(items) = v8::Local::<v8::Array>::try_from(args.get(index as i32)) else {
            continue;
        };
        for item_index in 0..items.length() {
            let Some(item) = items.get_index(scope, item_index) else {
                continue;
            };
            let Ok(pair) = v8::Local::<v8::Array>::try_from(item) else {
                continue;
            };
            let Some(key) = pair
                .get_index(scope, 0)
                .and_then(|value| value.to_string(scope))
            else {
                continue;
            };
            let Some(value) = pair
                .get_index(scope, 1)
                .and_then(|value| value.to_string(scope))
            else {
                continue;
            };
            destination.insert(
                key.to_rust_string_lossy(scope),
                value.to_rust_string_lossy(scope),
            );
        }
    }
}

fn storage_index(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<usize, V8BindingError> {
    match stateful_id(scope, object)? {
        1 => Ok(0),
        2 => Ok(1),
        _ => Err(V8BindingError::Allocation("Storage identity")),
    }
}

fn storage_area(index: usize) -> &'static str {
    if index == 0 { "local" } else { "session" }
}

fn storage_context_response(
    scope: &mut v8::PinScope,
    index: usize,
    operation: &str,
    key: Option<&str>,
    value: Option<&str>,
) -> Result<Option<serde_json::Value>, V8BindingError> {
    let Ok(call) =
        v8::Local::<v8::Function>::try_from(global_property(scope, "__oryn_context_call")?)
    else {
        return Ok(None);
    };
    let page_id = global_property(scope, "__oryn_page_id")?;
    if page_id.is_null_or_undefined() {
        return Ok(None);
    }
    let page_id = page_id
        .integer_value(scope)
        .ok_or(V8BindingError::Allocation("storage page id"))?;
    let location = global_property(scope, "location")?
        .to_object(scope)
        .ok_or(V8BindingError::Allocation("location object"))?;
    let origin_key =
        v8::String::new(scope, "origin").ok_or(V8BindingError::Allocation("origin key"))?;
    let origin = location
        .get(scope, origin_key.into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .ok_or(V8BindingError::Allocation("storage origin"))?;
    let mut request = serde_json::json!({
        "operation": "storage",
        "request": {
            "operation": operation,
            "area": storage_area(index),
            "scope": {"origin": origin, "top_level_page_id": page_id},
        }
    });
    if let Some(key) = key {
        request["request"]["key"] = serde_json::Value::String(key.into());
    }
    if let Some(value) = value {
        request["request"]["value"] = serde_json::Value::String(value.into());
    }
    let encoded = serde_json::to_string(&request)
        .map_err(|_| V8BindingError::Allocation("storage request serialization"))?;
    let encoded =
        v8::String::new(scope, &encoded).ok_or(V8BindingError::Allocation("storage request"))?;
    let receiver = scope.get_current_context().global(scope);
    let raw = call
        .call(scope, receiver.into(), &[encoded.into()])
        .ok_or_else(|| V8BindingError::EventCallback("parent storage operation failed".into()))?
        .to_string(scope)
        .ok_or(V8BindingError::Allocation("storage response"))?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|_| V8BindingError::Allocation("storage response JSON"))
}

fn storage_entries(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    index: usize,
) -> Result<Vec<(String, String)>, V8BindingError> {
    if let Some(response) = storage_context_response(scope, index, "entries", None, None)? {
        let entries = response
            .get("response")
            .and_then(|response| response.get("entries"))
            .and_then(serde_json::Value::as_array)
            .ok_or(V8BindingError::Allocation("storage entries response"))?;
        return entries
            .iter()
            .map(|entry| {
                let entry = entry
                    .as_array()
                    .filter(|entry| entry.len() == 2)
                    .ok_or(V8BindingError::Allocation("storage entry"))?;
                let key = entry[0]
                    .as_str()
                    .ok_or(V8BindingError::Allocation("storage key"))?;
                let value = entry[1]
                    .as_str()
                    .ok_or(V8BindingError::Allocation("storage value"))?;
                Ok((key.into(), value.into()))
            })
            .collect();
    }
    Ok(state.storage_fallback.borrow()[index]
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn call_storage_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let index = storage_index(scope, object)?;
    match member {
        "key" => {
            let requested = args.get(0).uint32_value(scope).unwrap_or(u32::MAX) as usize;
            let entries = storage_entries(scope, state, index)?;
            return if let Some((key, _)) = entries.get(requested) {
                Ok(Some(v8_string(scope, key)?))
            } else {
                Ok(Some(v8::null(scope).into()))
            };
        }
        "getItem" => {
            let key = required_string(scope, args, 0)?;
            if let Some(response) = storage_context_response(scope, index, "get", Some(&key), None)?
            {
                let value = response
                    .get("response")
                    .and_then(|response| response.get("value"));
                return match value {
                    Some(serde_json::Value::String(value)) => Ok(Some(v8_string(scope, value)?)),
                    _ => Ok(Some(v8::null(scope).into())),
                };
            }
            let fallback = state.storage_fallback.borrow()[index].get(&key).cloned();
            return if let Some(value) = fallback {
                Ok(Some(v8_string(scope, &value)?))
            } else {
                Ok(Some(v8::null(scope).into()))
            };
        }
        "setItem" => {
            let key = required_string(scope, args, 0)?;
            let value = required_string(scope, args, 1)?;
            if storage_context_response(scope, index, "set", Some(&key), Some(&value))?.is_none() {
                state.storage_fallback.borrow_mut()[index].insert(key, value);
            }
        }
        "removeItem" => {
            let key = required_string(scope, args, 0)?;
            if storage_context_response(scope, index, "remove", Some(&key), None)?.is_none() {
                state.storage_fallback.borrow_mut()[index].remove(&key);
            }
        }
        "clear" => {
            if storage_context_response(scope, index, "clear", None, None)?.is_none() {
                state.storage_fallback.borrow_mut()[index].clear();
            }
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: "Storage".into(),
                member: member.into(),
            });
        }
    }
    Ok(None)
}

fn get_stateful_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if interface == "Storage" {
        let index = storage_index(scope, object)?;
        return match member {
            "length" => {
                let length = storage_entries(scope, state, index)?
                    .len()
                    .try_into()
                    .unwrap_or(u32::MAX);
                Ok(v8::Integer::new_from_unsigned(scope, length).into())
            }
            _ => Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            }),
        };
    }
    if interface == "History" {
        ensure_history(scope, state)?;
        let history = state.history.borrow();
        let history = history
            .as_ref()
            .ok_or(V8BindingError::Allocation("History record"))?;
        return match member {
            "length" => Ok(v8::Integer::new_from_unsigned(
                scope,
                history.entries.len().try_into().unwrap_or(u32::MAX),
            )
            .into()),
            "state" => {
                if let Some(encoded) = history.entries[history.index].state_json.as_deref() {
                    parse_json(scope, encoded)
                } else {
                    Ok(v8::undefined(scope).into())
                }
            }
            "scrollRestoration" => v8_string(scope, &history.scroll_restoration),
            _ => Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            }),
        };
    }
    if matches!(interface, "AbstractRange" | "Range" | "Selection") {
        synchronize_live_ranges(state)?;
    }
    if matches!(interface, "AbstractRange" | "Range") {
        let id = stateful_id(scope, object)?;
        let ranges = state.ranges.borrow();
        let range = ranges
            .get(&id)
            .ok_or(V8BindingError::Allocation("Range record"))?;
        return match member {
            "startContainer" => Ok(wrap_node(scope, state, range.start_container)?.into()),
            "startOffset" => Ok(v8::Integer::new_from_unsigned(scope, range.start_offset).into()),
            "endContainer" => Ok(wrap_node(scope, state, range.end_container)?.into()),
            "endOffset" => Ok(v8::Integer::new_from_unsigned(scope, range.end_offset).into()),
            "collapsed" => Ok(v8::Boolean::new(
                scope,
                range.start_container == range.end_container
                    && range.start_offset == range.end_offset,
            )
            .into()),
            "commonAncestorContainer" => {
                let ancestor = {
                    let dom = state.dom.borrow();
                    common_ancestor(&dom, range.start_container, range.end_container)?
                };
                Ok(wrap_node(scope, state, ancestor)?.into())
            }
            _ => Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            }),
        };
    }
    if interface == "Selection" {
        let selected = state
            .selection_range
            .get()
            .and_then(|id| state.ranges.borrow().get(&id).cloned());
        let anchor = state.selection_anchor.get();
        let focus = state.selection_focus.get();
        return match member {
            "anchorNode" => optional_node(scope, state, anchor.map(|point| point.container)),
            "anchorOffset" => Ok(v8::Integer::new_from_unsigned(
                scope,
                anchor.map_or(0, |point| point.offset),
            )
            .into()),
            "focusNode" => optional_node(scope, state, focus.map(|point| point.container)),
            "focusOffset" => Ok(v8::Integer::new_from_unsigned(
                scope,
                focus.map_or(0, |point| point.offset),
            )
            .into()),
            "isCollapsed" => Ok(v8::Boolean::new(
                scope,
                selected.as_ref().is_none_or(|range| {
                    range.start_container == range.end_container
                        && range.start_offset == range.end_offset
                }),
            )
            .into()),
            "rangeCount" => {
                Ok(v8::Integer::new_from_unsigned(scope, u32::from(selected.is_some())).into())
            }
            "type" => v8_string(
                scope,
                if selected.is_none() {
                    "None"
                } else if selected.as_ref().is_some_and(|range| {
                    range.start_container == range.end_container
                        && range.start_offset == range.end_offset
                }) {
                    "Caret"
                } else {
                    "Range"
                },
            ),
            "direction" => v8_string(
                scope,
                match (anchor, focus) {
                    (Some(anchor), Some(focus))
                        if compare_boundary_points(
                            &state.dom.borrow(),
                            (anchor.container, anchor.offset),
                            (focus.container, focus.offset),
                        )? > 0 =>
                    {
                        "backward"
                    }
                    (Some(_), Some(_)) => "forward",
                    _ => "none",
                },
            ),
            _ => Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            }),
        };
    }
    let id = stateful_id(scope, object)?;
    let traversals = state.traversals.borrow();
    let traversal = traversals
        .get(&id)
        .ok_or(V8BindingError::Allocation("traversal record"))?;
    match (interface, member) {
        ("NodeIterator" | "TreeWalker", "root") => {
            Ok(wrap_node(scope, state, traversal.root)?.into())
        }
        ("NodeIterator", "referenceNode") | ("TreeWalker", "currentNode") => {
            Ok(wrap_node(scope, state, traversal.nodes[traversal.index])?.into())
        }
        ("NodeIterator", "pointerBeforeReferenceNode") => {
            Ok(v8::Boolean::new(scope, traversal.pointer_before).into())
        }
        ("NodeIterator" | "TreeWalker", "whatToShow") => {
            Ok(v8::Integer::new_from_unsigned(scope, traversal.what_to_show).into())
        }
        _ => Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: member.into(),
        }),
    }
}

fn get_platform_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    if matches!(
        interface,
        "ReadableStreamDefaultReader" | "WritableStreamDefaultWriter"
    ) {
        let stream_id = {
            let record = platform_object(state, id)?;
            match &*record {
                PlatformObjectRecord::ReadableReader { stream_id }
                | PlatformObjectRecord::WritableWriter { stream_id } => *stream_id,
                _ => return Err(V8BindingError::Allocation("stream reader/writer record")),
            }
        };
        if interface == "WritableStreamDefaultWriter" && member == "desiredSize" {
            return Ok(v8::Integer::new(scope, 1).into());
        }
        if matches!(member, "ready" | "closed") {
            let undefined = v8::undefined(scope).into();
            return Ok(resolved_promise(scope, undefined)?.into());
        }
        let record = platform_object(state, stream_id)?;
        return match (&*record, interface, member) {
            (PlatformObjectRecord::ReadableStream(stream), _, "closed") if stream.closed => {
                let undefined = v8::undefined(scope).into();
                Ok(resolved_promise(scope, undefined)?.into())
            }
            _ => Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            }),
        };
    }
    let record = platform_object(state, id)?;
    match (&*record, interface, member) {
        (PlatformObjectRecord::Blob(blob), "Blob", "size") => {
            Ok(v8::Number::new(scope, blob.bytes.len() as f64).into())
        }
        (PlatformObjectRecord::Blob(blob), "Blob", "type") => v8_string(scope, &blob.media_type),
        (PlatformObjectRecord::Blob(blob), "File", "name") => {
            v8_string(scope, blob.file.as_ref().map_or("", |file| &file.name))
        }
        (PlatformObjectRecord::Blob(blob), "File", "lastModified") => Ok(v8::Number::new(
            scope,
            blob.file.as_ref().map_or(0, |file| file.last_modified) as f64,
        )
        .into()),
        (PlatformObjectRecord::Blob(_), "File", "webkitRelativePath") => v8_string(scope, ""),
        (PlatformObjectRecord::AbortSignal(signal), "AbortSignal", "aborted") => {
            Ok(v8::Boolean::new(scope, signal.aborted).into())
        }
        (PlatformObjectRecord::AbortSignal(signal), "AbortSignal", "reason") => Ok(signal
            .reason
            .as_ref()
            .map(|value| v8::Local::new(scope, value))
            .unwrap_or_else(|| v8::undefined(scope).into())),
        (PlatformObjectRecord::AbortController { signal_id }, "AbortController", "signal") => {
            Ok(new_platform_object(scope, state, "AbortSignal", *signal_id)?.into())
        }
        (PlatformObjectRecord::Request(request), "Request", "url") => {
            v8_string(scope, &request.url)
        }
        (PlatformObjectRecord::Request(request), "Request", "method") => {
            v8_string(scope, &request.method)
        }
        (PlatformObjectRecord::Request(request), "Request", "headers") => {
            Ok(new_platform_object(scope, state, "Headers", request.headers_id)?.into())
        }
        (PlatformObjectRecord::Request(request), "Request", "signal") => {
            Ok(new_platform_object(scope, state, "AbortSignal", request.signal_id)?.into())
        }
        (PlatformObjectRecord::Request(request), "Request", "credentials") => {
            v8_string(scope, &request.credentials)
        }
        (PlatformObjectRecord::Request(request), "Request", "body") => match request.body_object_id
        {
            Some(id) => {
                let interface = match &*platform_object(state, id)? {
                    PlatformObjectRecord::FormData(_) => "FormData",
                    PlatformObjectRecord::Blob(blob) if blob.file.is_some() => "File",
                    PlatformObjectRecord::Blob(_) => "Blob",
                    PlatformObjectRecord::ReadableStream(_) => "ReadableStream",
                    _ => "Blob",
                };
                Ok(new_platform_object(scope, state, interface, id)?.into())
            }
            None => Ok(v8::null(scope).into()),
        },
        (PlatformObjectRecord::Request(request), "Request", "bodyUsed") => {
            Ok(v8::Boolean::new(scope, request.body_used).into())
        }
        (PlatformObjectRecord::Response(response), "Response", "status") => {
            Ok(v8::Integer::new_from_unsigned(scope, response.status.into()).into())
        }
        (PlatformObjectRecord::Response(response), "Response", "ok") => {
            Ok(v8::Boolean::new(scope, (200..300).contains(&response.status)).into())
        }
        (PlatformObjectRecord::Response(response), "Response", "url") => {
            v8_string(scope, &response.url)
        }
        (PlatformObjectRecord::Response(response), "Response", "headers") => {
            Ok(new_platform_object(scope, state, "Headers", response.headers_id)?.into())
        }
        (PlatformObjectRecord::Response(response), "Response", "body") => {
            match response.body_stream_id {
                Some(id) => Ok(new_platform_object(scope, state, "ReadableStream", id)?.into()),
                None => Ok(v8::null(scope).into()),
            }
        }
        (PlatformObjectRecord::Response(response), "Response", "bodyUsed") => {
            Ok(v8::Boolean::new(scope, response.body_used).into())
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "readyState") => {
            Ok(v8::Integer::new_from_unsigned(scope, xhr.ready_state.into()).into())
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "timeout") => {
            Ok(v8::Number::new(scope, xhr.timeout as f64).into())
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "withCredentials") => {
            Ok(v8::Boolean::new(scope, xhr.with_credentials).into())
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "responseType") => {
            v8_string(scope, &xhr.response_type)
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "status") => {
            Ok(v8::Integer::new_from_unsigned(scope, xhr.status.into()).into())
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "responseURL") => {
            v8_string(scope, &xhr.response_url)
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "responseText") => {
            v8_string(scope, &String::from_utf8_lossy(&xhr.response_bytes))
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "response") => {
            xhr_response_value(scope, state, xhr)
        }
        (PlatformObjectRecord::WebSocket(socket), "WebSocket", "url") => {
            v8_string(scope, &socket.url)
        }
        (PlatformObjectRecord::WebSocket(socket), "WebSocket", "readyState") => {
            Ok(v8::Integer::new_from_unsigned(scope, socket.ready_state.into()).into())
        }
        (PlatformObjectRecord::WebSocket(socket), "WebSocket", "protocol") => {
            v8_string(scope, &socket.protocol)
        }
        (PlatformObjectRecord::WebSocket(socket), "WebSocket", "binaryType") => {
            v8_string(scope, &socket.binary_type)
        }
        (
            PlatformObjectRecord::IdbRequest(request),
            "IDBRequest" | "IDBOpenDBRequest",
            "result",
        ) => Ok(request
            .result
            .as_ref()
            .map(|value| v8::Local::new(scope, value))
            .unwrap_or_else(|| v8::undefined(scope).into())),
        (PlatformObjectRecord::IdbRequest(request), "IDBRequest" | "IDBOpenDBRequest", "error") => {
            match &request.error {
                Some(error) => dom_exception_value(scope, error),
                None => Ok(v8::null(scope).into()),
            }
        }
        (
            PlatformObjectRecord::IdbRequest(request),
            "IDBRequest" | "IDBOpenDBRequest",
            "transaction",
        ) => match request.transaction_id {
            Some(id) => Ok(new_platform_object(scope, state, "IDBTransaction", id)?.into()),
            None => Ok(v8::null(scope).into()),
        },
        (
            PlatformObjectRecord::IdbRequest(request),
            "IDBRequest" | "IDBOpenDBRequest",
            "source",
        ) => match request.source_id {
            Some(id) => {
                let interface = match &*platform_object(state, id)? {
                    PlatformObjectRecord::IdbObjectStore(_) => "IDBObjectStore",
                    PlatformObjectRecord::IdbIndex(_) => "IDBIndex",
                    PlatformObjectRecord::IdbCursor(_) => "IDBCursor",
                    _ => return Err(V8BindingError::Allocation("IDBRequest source record")),
                };
                Ok(new_platform_object(scope, state, interface, id)?.into())
            }
            None => Ok(v8::null(scope).into()),
        },
        (
            PlatformObjectRecord::IdbRequest(request),
            "IDBRequest" | "IDBOpenDBRequest",
            "readyState",
        ) => v8_string(scope, if request.done { "done" } else { "pending" }),
        (PlatformObjectRecord::IdbDatabase(database), "IDBDatabase", "name") => {
            v8_string(scope, &database.name)
        }
        (PlatformObjectRecord::IdbDatabase(database), "IDBDatabase", "version") => {
            Ok(v8::Number::new(scope, database.version as f64).into())
        }
        (PlatformObjectRecord::IdbDatabase(database), "IDBDatabase", "objectStoreNames") => {
            string_array(scope, &database.object_stores)
        }
        (PlatformObjectRecord::IdbTransaction(transaction), "IDBTransaction", "mode") => {
            v8_string(scope, &transaction.mode)
        }
        (PlatformObjectRecord::IdbTransaction(transaction), "IDBTransaction", "db") => {
            Ok(new_platform_object(scope, state, "IDBDatabase", transaction.database_id)?.into())
        }
        (PlatformObjectRecord::IdbTransaction(_), "IDBTransaction", "durability") => {
            v8_string(scope, "default")
        }
        (PlatformObjectRecord::IdbTransaction(transaction), "IDBTransaction", "error") => {
            match &transaction.error {
                Some(error) => dom_exception_value(scope, error),
                None => Ok(v8::null(scope).into()),
            }
        }
        (
            PlatformObjectRecord::IdbTransaction(transaction),
            "IDBTransaction",
            "objectStoreNames",
        ) => string_array(scope, &transaction.object_stores),
        (PlatformObjectRecord::IdbObjectStore(store), "IDBObjectStore", "name") => {
            v8_string(scope, &store.name)
        }
        (PlatformObjectRecord::IdbObjectStore(store), "IDBObjectStore", "keyPath") => {
            match &store.key_path {
                Some(value) => v8_string(scope, value),
                None => Ok(v8::null(scope).into()),
            }
        }
        (PlatformObjectRecord::IdbObjectStore(store), "IDBObjectStore", "indexNames") => {
            string_array(scope, &store.indexes.keys().cloned().collect::<Vec<_>>())
        }
        (PlatformObjectRecord::IdbObjectStore(store), "IDBObjectStore", "transaction") => {
            Ok(new_platform_object(scope, state, "IDBTransaction", store.transaction_id)?.into())
        }
        (PlatformObjectRecord::IdbObjectStore(store), "IDBObjectStore", "autoIncrement") => {
            Ok(v8::Boolean::new(scope, store.auto_increment).into())
        }
        (PlatformObjectRecord::IdbIndex(index), "IDBIndex", "name") => {
            v8_string(scope, &index.name)
        }
        (PlatformObjectRecord::IdbIndex(index), "IDBIndex", "objectStore") => {
            Ok(new_platform_object(scope, state, "IDBObjectStore", index.object_store_id)?.into())
        }
        (PlatformObjectRecord::IdbIndex(index), "IDBIndex", "keyPath") => {
            v8_string(scope, &index.key_path)
        }
        (PlatformObjectRecord::IdbIndex(_), "IDBIndex", "multiEntry") => {
            Ok(v8::Boolean::new(scope, false).into())
        }
        (PlatformObjectRecord::IdbIndex(index), "IDBIndex", "unique") => {
            Ok(v8::Boolean::new(scope, index.unique).into())
        }
        (PlatformObjectRecord::IdbKeyRange(range), "IDBKeyRange", "lower") => range
            .lower
            .as_ref()
            .map(|value| idb_json_to_value(scope, state, value))
            .transpose()
            .map(|value| value.unwrap_or_else(|| v8::undefined(scope).into())),
        (PlatformObjectRecord::IdbKeyRange(range), "IDBKeyRange", "upper") => range
            .upper
            .as_ref()
            .map(|value| idb_json_to_value(scope, state, value))
            .transpose()
            .map(|value| value.unwrap_or_else(|| v8::undefined(scope).into())),
        (PlatformObjectRecord::IdbKeyRange(range), "IDBKeyRange", "lowerOpen") => {
            Ok(v8::Boolean::new(scope, range.lower_open).into())
        }
        (PlatformObjectRecord::IdbKeyRange(range), "IDBKeyRange", "upperOpen") => {
            Ok(v8::Boolean::new(scope, range.upper_open).into())
        }
        (PlatformObjectRecord::IdbCursor(cursor), "IDBCursor", "source") => {
            Ok(new_platform_object(
                scope,
                state,
                if matches!(
                    &*platform_object(state, cursor.source_id)?,
                    PlatformObjectRecord::IdbIndex(_)
                ) {
                    "IDBIndex"
                } else {
                    "IDBObjectStore"
                },
                cursor.source_id,
            )?
            .into())
        }
        (PlatformObjectRecord::IdbCursor(cursor), "IDBCursor", "direction") => {
            v8_string(scope, &cursor.direction)
        }
        (PlatformObjectRecord::IdbCursor(cursor), "IDBCursor", "request") => {
            Ok(new_platform_object(scope, state, "IDBRequest", cursor.request_id)?.into())
        }
        (PlatformObjectRecord::IdbCursor(cursor), "IDBCursor", "key" | "primaryKey" | "value") => {
            let record = cursor.records.get(cursor.index);
            let value = match (member, record) {
                ("key", Some(record)) => Some(&record.key),
                ("primaryKey", Some(record)) => Some(&record.primary_key),
                ("value", Some(record)) => Some(&record.value),
                _ => None,
            };
            value
                .map(|value| idb_json_to_value(scope, state, value))
                .transpose()
                .map(|value| value.unwrap_or_else(|| v8::undefined(scope).into()))
        }
        (_, _, member) if member.starts_with("on") => Ok(state
            .platform_handlers
            .borrow()
            .get(&(id, member[2..].to_string()))
            .map(|value| v8::Local::new(scope, value))
            .unwrap_or_else(|| v8::null(scope).into())),
        (PlatformObjectRecord::ReadableStream(stream), "ReadableStream", "locked") => {
            Ok(v8::Boolean::new(scope, stream.locked).into())
        }
        (PlatformObjectRecord::WritableStream(stream), "WritableStream", "locked") => {
            Ok(v8::Boolean::new(scope, stream.locked).into())
        }
        (
            PlatformObjectRecord::ReadableController { stream_id },
            "ReadableStreamDefaultController",
            "desiredSize",
        ) => {
            let stream = platform_object(state, *stream_id)?;
            let PlatformObjectRecord::ReadableStream(stream) = &*stream else {
                return Err(V8BindingError::Allocation("ReadableStream record"));
            };
            Ok(v8::Integer::new(scope, i32::from(!stream.closed)).into())
        }
        (
            PlatformObjectRecord::TransformStream { readable_id, .. },
            "TransformStream",
            "readable",
        ) => Ok(new_platform_object(scope, state, "ReadableStream", *readable_id)?.into()),
        (
            PlatformObjectRecord::TransformStream { writable_id, .. },
            "TransformStream",
            "writable",
        ) => Ok(new_platform_object(scope, state, "WritableStream", *writable_id)?.into()),
        _ => Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: member.into(),
        }),
    }
}

fn xhr_response_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    xhr: &XhrRecord,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match xhr.response_type.as_str() {
        "" | "text" => v8_string(scope, &String::from_utf8_lossy(&xhr.response_bytes)),
        "json" => {
            let text = v8_string(scope, &String::from_utf8_lossy(&xhr.response_bytes))?;
            Ok(v8::json::parse(
                scope,
                v8::Local::<v8::String>::try_from(text)
                    .map_err(|_| V8BindingError::Allocation("XHR JSON"))?,
            )
            .unwrap_or_else(|| v8::null(scope).into()))
        }
        "arraybuffer" => Ok(array_buffer_from_bytes(scope, &xhr.response_bytes).into()),
        "blob" => {
            let id = register_platform_object(
                state,
                PlatformObjectRecord::Blob(BlobRecord {
                    bytes: xhr.response_bytes.clone(),
                    media_type: String::new(),
                    file: None,
                }),
            );
            Ok(new_platform_object(scope, state, "Blob", id)?.into())
        }
        _ => Ok(v8::null(scope).into()),
    }
}

fn string_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[String],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let array = v8::Array::new(scope, values.len() as i32);
    for (index, value) in values.iter().enumerate() {
        let value = v8_string(scope, value)?;
        array.set_index(scope, index as u32, value);
    }
    macro_rules! install_string_list_method {
        ($name:literal, $callback:ident) => {{
            let key = v8::String::new(scope, $name)
                .ok_or(V8BindingError::Allocation("DOMStringList method"))?;
            let function = v8::Function::new(scope, $callback)
                .ok_or(V8BindingError::Allocation("DOMStringList method"))?;
            array.create_data_property(scope, key.into(), function.into());
        }};
    }
    install_string_list_method!("contains", dom_string_list_contains);
    install_string_list_method!("item", dom_string_list_item);
    Ok(array.into())
}

fn dom_string_list_contains(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let receiver = args.this();
    let sought = args.get(0).to_rust_string_lossy(scope);
    let length = receiver
        .get(scope, v8::String::new(scope, "length").unwrap().into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let found = (0..length).any(|index| {
        receiver
            .get_index(scope, index)
            .is_some_and(|value| value.to_rust_string_lossy(scope) == sought)
    });
    return_value.set(v8::Boolean::new(scope, found).into());
}

fn dom_string_list_item(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let receiver = args.this();
    let value = args
        .get(0)
        .uint32_value(scope)
        .and_then(|index| receiver.get_index(scope, index))
        .unwrap_or_else(|| v8::null(scope).into());
    return_value.set(value);
}

fn response_body_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (bytes, body_stream_id) = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::Response(response) = &mut *record else {
            return Err(V8BindingError::Allocation("Response record"));
        };
        if response.body_used {
            return Err(V8BindingError::Unsupported {
                interface: "Response".into(),
                member: "body already consumed".into(),
            });
        }
        response.body_used = true;
        (response.bytes.clone(), response.body_stream_id)
    };
    let host_stream_id = body_stream_id.and_then(|stream_id| {
        let record = platform_object(state, stream_id).ok()?;
        let PlatformObjectRecord::ReadableStream(stream) = &*record else {
            return None;
        };
        stream.host_stream_id
    });
    if let Some(stream_id) = host_stream_id {
        let resolver = v8::PromiseResolver::new(scope)
            .ok_or(V8BindingError::Allocation("Response body promise"))?;
        let promise = resolver.get_promise(scope);
        submit_platform_operation(
            state,
            PlatformHostRequest::HttpRead {
                stream_id,
                max_bytes: 64 * 1024,
            },
            PendingPlatformOperation::ResponseRead {
                response_id: id,
                resolver: v8::Global::new(scope, resolver),
                member: member.into(),
            },
        )?;
        return Ok(promise.into());
    }
    let value = response_value_from_bytes(scope, state, member, bytes)?;
    Ok(resolved_promise(scope, value)?.into())
}

fn response_consumed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    response_id: u64,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let bytes = {
        let record = platform_object(state, response_id)?;
        let PlatformObjectRecord::Response(response) = &*record else {
            return Err(V8BindingError::Allocation("Response record"));
        };
        response.bytes.clone()
    };
    response_value_from_bytes(scope, state, member, bytes)
}

fn response_value_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    member: &str,
    bytes: Vec<u8>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let value = match member {
        "text" => v8_string(scope, &String::from_utf8_lossy(&bytes))?,
        "json" => {
            let text = v8_string(scope, &String::from_utf8_lossy(&bytes))?;
            v8::json::parse(
                scope,
                v8::Local::<v8::String>::try_from(text)
                    .map_err(|_| V8BindingError::Allocation("Response JSON"))?,
            )
            .ok_or(V8BindingError::Allocation("Response JSON parse"))?
        }
        "arrayBuffer" => array_buffer_from_bytes(scope, &bytes).into(),
        "blob" => {
            let id = register_platform_object(
                state,
                PlatformObjectRecord::Blob(BlobRecord {
                    bytes,
                    media_type: String::new(),
                    file: None,
                }),
            );
            new_platform_object(scope, state, "Blob", id)?.into()
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: "Response".into(),
                member: member.into(),
            });
        }
    };
    Ok(value)
}

fn platform_fetch_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Promise>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("fetch binding state"))?;
        let request_id = if let Some(object) = args.get(0).to_object(scope)
            && let Ok(id) = stateful_id(scope, object)
            && matches!(
                &*platform_object(&state, id)?,
                PlatformObjectRecord::Request(_)
            ) {
            id
        } else {
            create_request_record(scope, &state, args.get(0), args.get(1))?
        };
        let (method, url, headers_id, body, credentials, signal_id) = {
            let mut record = platform_object_mut(&state, request_id)?;
            let PlatformObjectRecord::Request(request) = &mut *record else {
                return Err(V8BindingError::Allocation("Request record"));
            };
            request.body_used = !request.body.is_empty();
            (
                request.method.clone(),
                request.url.clone(),
                request.headers_id,
                request.body.clone(),
                request.credentials.clone(),
                request.signal_id,
            )
        };
        let signal = platform_object(&state, signal_id)?;
        if matches!(
            &*signal,
            PlatformObjectRecord::AbortSignal(AbortSignalRecord { aborted: true, .. })
        ) {
            return Err(V8BindingError::Unsupported {
                interface: "fetch".into(),
                member: "request aborted".into(),
            });
        }
        drop(signal);
        if let Some(blob) = blob_url_record(&state, &url) {
            let headers_id = register_platform_object(
                &state,
                PlatformObjectRecord::Headers(HeadersRecord {
                    entries: if blob.media_type.is_empty() {
                        Vec::new()
                    } else {
                        vec![("content-type".into(), blob.media_type)]
                    },
                }),
            );
            let body_stream_id = Some(new_byte_stream_record(scope, &state, blob.bytes.clone())?);
            let response_id = register_platform_object(
                &state,
                PlatformObjectRecord::Response(ResponseRecord {
                    status: 200,
                    url,
                    headers_id,
                    body_stream_id,
                    bytes: blob.bytes,
                    body_used: false,
                    signal_id: Some(signal_id),
                }),
            );
            let response = new_platform_object(scope, &state, "Response", response_id)?;
            return resolved_promise(scope, response.into());
        }
        let headers = platform_object(&state, headers_id)?;
        let PlatformObjectRecord::Headers(headers) = &*headers else {
            return Err(V8BindingError::Allocation("Request Headers"));
        };
        let request = PlatformHttpRequest {
            method,
            url,
            headers: headers.entries.clone(),
            body,
            credentials,
        };
        let resolver =
            v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("fetch promise"))?;
        let promise = resolver.get_promise(scope);
        record_platform_trace(
            scope,
            "request_started",
            serde_json::json!({"method":&request.method,"url":&request.url,"bytes":request.body.len()}),
        );
        submit_platform_operation(
            &state,
            PlatformHostRequest::HttpStart { request },
            PendingPlatformOperation::FetchStart {
                resolver: v8::Global::new(scope, resolver),
                signal_id,
            },
        )?;
        Ok(promise)
    })();
    match result {
        Ok(promise) => return_value.set(promise.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn create_request_record(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    input: v8::Local<v8::Value>,
    init: v8::Local<v8::Value>,
) -> Result<u64, V8BindingError> {
    let global = scope.get_current_context().global(scope);
    let name =
        v8::String::new(scope, "Request").ok_or(V8BindingError::Allocation("Request name"))?;
    let constructor = global
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or(V8BindingError::MissingInterface("Request"))?;
    let object = constructor
        .new_instance(scope, &[input, init])
        .ok_or(V8BindingError::Allocation("Request object"))?;
    let id = stateful_id(scope, object)?;
    if !matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::Request(_)
    ) {
        return Err(V8BindingError::Allocation("Request record"));
    }
    Ok(id)
}

fn set_platform_attribute(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let id = stateful_id(scope, object)?;
    if let Some(event_type) = member.strip_prefix("on") {
        state
            .platform_handlers
            .borrow_mut()
            .insert((id, event_type.into()), v8::Global::new(scope, value));
        return Ok(());
    }
    let mut record = platform_object_mut(state, id)?;
    match (&mut *record, interface, member) {
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "timeout") => {
            xhr.timeout = value.integer_value(scope).unwrap_or(0).max(0) as u64
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "withCredentials") => {
            xhr.with_credentials = value.boolean_value(scope)
        }
        (PlatformObjectRecord::XMLHttpRequest(xhr), "XMLHttpRequest", "responseType") => {
            xhr.response_type = value.to_rust_string_lossy(scope)
        }
        (PlatformObjectRecord::WebSocket(socket), "WebSocket", "binaryType") => {
            let kind = value.to_rust_string_lossy(scope);
            if !matches!(kind.as_str(), "blob" | "arraybuffer") {
                return Err(V8BindingError::Unsupported {
                    interface: interface.into(),
                    member: "binaryType".into(),
                });
            }
            socket.binary_type = kind;
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    }
    Ok(())
}

fn call_platform_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    match (interface, member) {
        ("FormData", "append" | "set") => {
            let name = required_string(scope, args, 0)?;
            let value = form_data_entry_value(scope, state, args.get(1), args.get(2))?;
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::FormData(form) = &mut *record else {
                return Err(V8BindingError::Allocation("FormData record"));
            };
            if member == "set" {
                form.entries.retain(|(key, _)| key != &name);
            }
            form.entries.push((name, v8::Global::new(scope, value)));
            return Ok(None);
        }
        ("FormData", "delete" | "get" | "getAll" | "has") => {
            let name = required_string(scope, args, 0)?;
            if member == "delete" {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::FormData(form) = &mut *record else {
                    return Err(V8BindingError::Allocation("FormData record"));
                };
                form.entries.retain(|(key, _)| key != &name);
                return Ok(None);
            }
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::FormData(form) = &*record else {
                return Err(V8BindingError::Allocation("FormData record"));
            };
            let values = form
                .entries
                .iter()
                .filter(|(key, _)| key == &name)
                .map(|(_, value)| v8::Local::new(scope, value))
                .collect::<Vec<_>>();
            return Ok(Some(match member {
                "has" => v8::Boolean::new(scope, !values.is_empty()).into(),
                "get" => values
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| v8::null(scope).into()),
                _ => {
                    let array = v8::Array::new(scope, values.len() as i32);
                    for (index, value) in values.into_iter().enumerate() {
                        array.set_index(scope, index as u32, value);
                    }
                    array.into()
                }
            }));
        }
        ("FormData", "@@iterator") => {
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::FormData(form) = &*record else {
                return Err(V8BindingError::Allocation("FormData record"));
            };
            let entries = v8::Array::new(scope, form.entries.len() as i32);
            for (index, (name, value)) in form.entries.iter().enumerate() {
                let pair = v8::Array::new(scope, 2);
                let name = v8_string(scope, name)?;
                pair.set_index(scope, 0, name);
                pair.set_index(scope, 1, v8::Local::new(scope, value));
                entries.set_index(scope, index as u32, pair.into());
            }
            let iterator = entries
                .get(scope, v8::Symbol::get_iterator(scope).into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .and_then(|function| function.call(scope, entries.into(), &[]))
                .ok_or(V8BindingError::Allocation("FormData iterator"))?;
            return Ok(Some(iterator));
        }
        ("IDBFactory", "open") => {
            let name = required_string(scope, args, 0)?;
            let version = args.get(1).integer_value(scope).unwrap_or(1).max(1) as u64;
            let request_id = new_idb_request(state, None);
            submit_idb(
                scope,
                state,
                request_id,
                IdbHostCommand::Open {
                    name: name.clone(),
                    version,
                },
                IdbCompletionAction::Open { name },
            )?;
            return Ok(Some(
                new_platform_object(scope, state, "IDBOpenDBRequest", request_id)?.into(),
            ));
        }
        ("IDBFactory", "deleteDatabase") => {
            let name = required_string(scope, args, 0)?;
            let request_id = new_idb_request(state, None);
            submit_idb(
                scope,
                state,
                request_id,
                IdbHostCommand::DeleteDatabase { name },
                IdbCompletionAction::DeleteDatabase,
            )?;
            return Ok(Some(
                new_platform_object(scope, state, "IDBOpenDBRequest", request_id)?.into(),
            ));
        }
        ("IDBFactory", "cmp") => {
            let first = idb_key_from_value(scope, args.get(0))?;
            let second = idb_key_from_value(scope, args.get(1))?;
            let ordering = compare_idb_keys(&first, &second);
            return Ok(Some(
                v8::Integer::new(
                    scope,
                    match ordering {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    },
                )
                .into(),
            ));
        }
        ("IDBFactory", "databases") => {
            let resolver = v8::PromiseResolver::new(scope)
                .ok_or(V8BindingError::Allocation("IDBFactory.databases resolver"))?;
            let promise = resolver.get_promise(scope);
            submit_platform_operation(
                state,
                PlatformHostRequest::IndexedDb {
                    origin: current_platform_origin(scope),
                    command: IdbHostCommand::ListDatabases,
                },
                PendingPlatformOperation::IdbDatabases {
                    resolver: v8::Global::new(scope, resolver),
                },
            )?;
            return Ok(Some(promise.into()));
        }
        ("IDBDatabase", "createObjectStore") => {
            let name = required_string(scope, args, 0)?;
            let (database, upgrade_transaction_id, exists) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbDatabase(database) = &*record else {
                    return Err(V8BindingError::Allocation("IDBDatabase record"));
                };
                (
                    database.name.clone(),
                    database.upgrade_transaction_id,
                    database.object_stores.contains(&name),
                )
            };
            let transaction_id = upgrade_transaction_id.ok_or(V8BindingError::Unsupported {
                interface: "IDBDatabase".into(),
                member: "TransactionInactiveError".into(),
            })?;
            let active = matches!(
                &*platform_object(state, transaction_id)?,
                PlatformObjectRecord::IdbTransaction(transaction) if transaction.active && !transaction.aborted
            );
            if !active {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "TransactionInactiveError".into(),
                });
            }
            if exists {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "ConstraintError".into(),
                });
            }
            let options = args.get(1);
            let key_path = option_value(scope, options, "keyPath")
                .filter(|value| !value.is_null_or_undefined())
                .map(|value| value.to_rust_string_lossy(scope));
            let auto_increment = option_value(scope, options, "autoIncrement")
                .is_some_and(|value| value.boolean_value(scope));
            let store_id = register_platform_object(
                state,
                PlatformObjectRecord::IdbObjectStore(IdbObjectStoreRecord {
                    transaction_id,
                    name: name.clone(),
                    key_path: key_path.clone(),
                    auto_increment,
                    indexes: BTreeMap::new(),
                    deleted: false,
                }),
            );
            if let PlatformObjectRecord::IdbDatabase(database) =
                &mut *platform_object_mut(state, id)?
            {
                database.object_stores.push(name.clone());
                database.object_store_definitions.insert(
                    name.clone(),
                    IdbObjectStoreDefinition {
                        name: name.clone(),
                        key_path: key_path.clone(),
                        auto_increment,
                        indexes: Vec::new(),
                    },
                );
            }
            if let PlatformObjectRecord::IdbTransaction(transaction) =
                &mut *platform_object_mut(state, transaction_id)?
            {
                transaction.object_stores.push(name.clone());
            }
            let hidden = new_idb_request(state, Some(transaction_id));
            submit_idb(
                scope,
                state,
                hidden,
                IdbHostCommand::CreateObjectStore {
                    database,
                    definition: IdbObjectStoreDefinition {
                        name,
                        key_path,
                        auto_increment,
                        indexes: Vec::new(),
                    },
                },
                IdbCompletionAction::Unit,
            )?;
            return Ok(Some(
                new_platform_object(scope, state, "IDBObjectStore", store_id)?.into(),
            ));
        }
        ("IDBDatabase", "deleteObjectStore") => {
            let store = required_string(scope, args, 0)?;
            let (database, transaction_id, exists) = {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::IdbDatabase(database) = &mut *record else {
                    return Err(V8BindingError::Allocation("IDBDatabase record"));
                };
                (
                    database.name.clone(),
                    database.upgrade_transaction_id,
                    database.object_stores.contains(&store),
                )
            };
            let transaction_id = transaction_id.ok_or(V8BindingError::Unsupported {
                interface: "IDBDatabase".into(),
                member: "TransactionInactiveError".into(),
            })?;
            let active = matches!(
                &*platform_object(state, transaction_id)?,
                PlatformObjectRecord::IdbTransaction(transaction) if transaction.active && !transaction.aborted
            );
            if !active {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "TransactionInactiveError".into(),
                });
            }
            if !exists {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "NotFoundError".into(),
                });
            }
            if let PlatformObjectRecord::IdbDatabase(database) =
                &mut *platform_object_mut(state, id)?
            {
                database.object_stores.retain(|name| name != &store);
                database.object_store_definitions.remove(&store);
            }
            if let PlatformObjectRecord::IdbTransaction(transaction) =
                &mut *platform_object_mut(state, transaction_id)?
            {
                transaction.object_stores.retain(|name| name != &store);
            }
            for record in state.platform_objects.borrow_mut().values_mut() {
                if let PlatformObjectRecord::IdbObjectStore(object_store) = record
                    && object_store.transaction_id == transaction_id
                    && object_store.name == store
                {
                    object_store.deleted = true;
                }
            }
            let hidden = new_idb_request(state, None);
            submit_idb(
                scope,
                state,
                hidden,
                IdbHostCommand::DeleteObjectStore { database, store },
                IdbCompletionAction::Unit,
            )?;
            return Ok(None);
        }
        ("IDBDatabase", "transaction") => {
            let names = string_sequence(scope, args.get(0));
            let mode = if args.get(1).is_undefined() {
                "readonly".into()
            } else {
                args.get(1).to_rust_string_lossy(scope)
            };
            let stores = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbDatabase(database) = &*record else {
                    return Err(V8BindingError::Allocation("IDBDatabase record"));
                };
                if database.closed {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBDatabase".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                database.object_stores.clone()
            };
            if names.is_empty() {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "InvalidAccessError".into(),
                });
            }
            if names.iter().any(|name| !stores.contains(name)) {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "NotFoundError".into(),
                });
            }
            if !matches!(mode.as_str(), "readonly" | "readwrite") {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBDatabase".into(),
                    member: "invalid transaction mode".into(),
                });
            }
            let transaction_id = register_platform_object(
                state,
                PlatformObjectRecord::IdbTransaction(IdbTransactionRecord {
                    database_id: id,
                    request_id: None,
                    mode,
                    object_stores: names,
                    aborted: false,
                    error: None,
                    pending_requests: 0,
                    completion_requested: false,
                    completion_queued: false,
                    active: true,
                }),
            );
            state
                .idb_transactions_to_deactivate
                .borrow_mut()
                .insert(transaction_id);
            return Ok(Some(
                new_platform_object(scope, state, "IDBTransaction", transaction_id)?.into(),
            ));
        }
        ("IDBDatabase", "close") => {
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::IdbDatabase(database) = &mut *record else {
                return Err(V8BindingError::Allocation("IDBDatabase record"));
            };
            database.closed = true;
            return Ok(None);
        }
        ("IDBTransaction", "objectStore") => {
            let name = required_string(scope, args, 0)?;
            let (active, in_scope, definition) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbTransaction(transaction) = &*record else {
                    return Err(V8BindingError::Allocation("IDBTransaction record"));
                };
                let database = platform_object(state, transaction.database_id)?;
                let PlatformObjectRecord::IdbDatabase(database) = &*database else {
                    return Err(V8BindingError::Allocation("IDBDatabase record"));
                };
                (
                    !transaction.aborted && transaction.active,
                    transaction.object_stores.contains(&name),
                    database.object_store_definitions.get(&name).cloned(),
                )
            };
            if !active {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBTransaction".into(),
                    member: "TransactionInactiveError".into(),
                });
            }
            if !in_scope {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBTransaction".into(),
                    member: "NotFoundError".into(),
                });
            }
            let store_id = register_platform_object(
                state,
                PlatformObjectRecord::IdbObjectStore(IdbObjectStoreRecord {
                    transaction_id: id,
                    name,
                    key_path: definition.as_ref().and_then(|value| value.key_path.clone()),
                    auto_increment: definition
                        .as_ref()
                        .is_some_and(|value| value.auto_increment),
                    indexes: definition
                        .map(|value| {
                            value
                                .indexes
                                .into_iter()
                                .map(|index| (index.name, (index.key_path, index.unique)))
                                .collect()
                        })
                        .unwrap_or_default(),
                    deleted: false,
                }),
            );
            return Ok(Some(
                new_platform_object(scope, state, "IDBObjectStore", store_id)?.into(),
            ));
        }
        ("IDBTransaction", "abort") => {
            let (database_id, request_id) = {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record else {
                    return Err(V8BindingError::Allocation("IDBTransaction record"));
                };
                if transaction.aborted
                    || transaction.completion_requested
                    || transaction.completion_queued
                    || !transaction.active
                {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBTransaction".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                transaction.aborted = true;
                transaction.active = false;
                (transaction.database_id, transaction.request_id)
            };
            if request_id.is_some() {
                let (name, old_version) = {
                    let mut record = platform_object_mut(state, database_id)?;
                    let PlatformObjectRecord::IdbDatabase(database) = &mut *record else {
                        return Err(V8BindingError::Allocation("IDBDatabase record"));
                    };
                    database.version = 0;
                    (database.name.clone(), database.old_version)
                };
                let hidden = new_idb_request(state, None);
                submit_idb(
                    scope,
                    state,
                    hidden,
                    IdbHostCommand::AbortUpgrade { name, old_version },
                    IdbCompletionAction::Unit,
                )?;
            }
            dispatch_platform_event(scope, state, id, "abort", None)?;
            return Ok(None);
        }
        ("IDBTransaction", "commit") => {
            {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::IdbTransaction(transaction) = &mut *record else {
                    return Err(V8BindingError::Allocation("IDBTransaction record"));
                };
                if transaction.aborted
                    || transaction.completion_requested
                    || transaction.completion_queued
                    || !transaction.active
                {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBTransaction".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                transaction.active = false;
                transaction.completion_requested = true;
            }
            queue_idb_transaction_completion(state, id)?;
            return Ok(None);
        }
        ("IDBObjectStore", "add" | "put") => {
            let (database, store, transaction_id) = idb_store_context(state, id)?;
            let value = idb_value_to_json(scope, state, args.get(0))?;
            let explicit_key = !args.get(1).is_undefined();
            let mut key = if !explicit_key {
                None
            } else {
                Some(idb_query_to_json(scope, state, args.get(1))?)
            };
            let (key_path, auto_increment) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbObjectStore(store) = &*record else {
                    return Err(V8BindingError::Allocation("IDBObjectStore record"));
                };
                (store.key_path.clone(), store.auto_increment)
            };
            if let Some(key_path) = key_path {
                if explicit_key {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBObjectStore".into(),
                        member: "DataError".into(),
                    });
                }
                match idb_json_key_path(&value, &key_path) {
                    Some(path_key) => {
                        key = Some(normalize_idb_json_key(path_key).ok_or(
                            V8BindingError::Unsupported {
                                interface: "IDBObjectStore".into(),
                                member: "DataError".into(),
                            },
                        )?);
                    }
                    None if !auto_increment || !value.is_object() => {
                        return Err(V8BindingError::Unsupported {
                            interface: "IDBObjectStore".into(),
                            member: "DataError".into(),
                        });
                    }
                    None => {}
                }
            } else if key.is_none() && !auto_increment {
                return Err(V8BindingError::Unsupported {
                    interface: "IDBObjectStore".into(),
                    member: "DataError".into(),
                });
            }
            let request_id = new_idb_request_for_source(state, Some(transaction_id), Some(id));
            submit_idb(
                scope,
                state,
                request_id,
                IdbHostCommand::Batch {
                    database,
                    writes: vec![IdbWrite {
                        store,
                        key,
                        value,
                        add_only: member == "add",
                    }],
                },
                IdbCompletionAction::Key,
            )?;
            return Ok(Some(
                new_platform_object(scope, state, "IDBRequest", request_id)?.into(),
            ));
        }
        (
            "IDBObjectStore" | "IDBIndex",
            "get" | "getKey" | "getAll" | "getAllKeys" | "count" | "openCursor" | "openKeyCursor",
        ) => {
            return idb_source_request(scope, state, id, interface, member, args);
        }
        ("IDBObjectStore", "delete" | "clear") => {
            let (database, store, transaction_id) = idb_store_context(state, id)?;
            let command = if member == "clear" {
                IdbHostCommand::Clear { database, store }
            } else {
                IdbHostCommand::Delete {
                    database,
                    store,
                    key: idb_query_to_json(scope, state, args.get(0))?,
                }
            };
            let request_id = new_idb_request_for_source(state, Some(transaction_id), Some(id));
            submit_idb(scope, state, request_id, command, IdbCompletionAction::Unit)?;
            return Ok(Some(
                new_platform_object(scope, state, "IDBRequest", request_id)?.into(),
            ));
        }
        ("IDBObjectStore", "createIndex") => {
            let index_name = required_string(scope, args, 0)?;
            let key_path = required_string(scope, args, 1)?;
            let unique = option_value(scope, args.get(2), "unique")
                .is_some_and(|value| value.boolean_value(scope));
            let (database, store, transaction_id) = idb_store_context(state, id)?;
            if let PlatformObjectRecord::IdbObjectStore(record) =
                &mut *platform_object_mut(state, id)?
            {
                record
                    .indexes
                    .insert(index_name.clone(), (key_path.clone(), unique));
            }
            let database_id = match &*platform_object(state, transaction_id)? {
                PlatformObjectRecord::IdbTransaction(transaction) => transaction.database_id,
                _ => return Err(V8BindingError::Allocation("IDBTransaction record")),
            };
            if let PlatformObjectRecord::IdbDatabase(database_record) =
                &mut *platform_object_mut(state, database_id)?
                && let Some(definition) = database_record.object_store_definitions.get_mut(&store)
            {
                definition.indexes.retain(|index| index.name != index_name);
                definition.indexes.push(IdbIndexDefinition {
                    name: index_name.clone(),
                    key_path: key_path.clone(),
                    unique,
                });
            }
            let index_id = register_platform_object(
                state,
                PlatformObjectRecord::IdbIndex(IdbIndexRecord {
                    object_store_id: id,
                    name: index_name.clone(),
                    key_path: key_path.clone(),
                    unique,
                }),
            );
            let hidden = new_idb_request(state, Some(transaction_id));
            submit_idb(
                scope,
                state,
                hidden,
                IdbHostCommand::CreateIndex {
                    database,
                    store,
                    definition: IdbIndexDefinition {
                        name: index_name,
                        key_path,
                        unique,
                    },
                },
                IdbCompletionAction::Unit,
            )?;
            return Ok(Some(
                new_platform_object(scope, state, "IDBIndex", index_id)?.into(),
            ));
        }
        ("IDBObjectStore", "index") => {
            let name = required_string(scope, args, 0)?;
            let (key_path, unique) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbObjectStore(store) = &*record else {
                    return Err(V8BindingError::Allocation("IDBObjectStore record"));
                };
                if store.deleted {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBObjectStore".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                store
                    .indexes
                    .get(&name)
                    .cloned()
                    .ok_or(V8BindingError::Unsupported {
                        interface: "IDBObjectStore".into(),
                        member: "NotFoundError".into(),
                    })?
            };
            let index_id = register_platform_object(
                state,
                PlatformObjectRecord::IdbIndex(IdbIndexRecord {
                    object_store_id: id,
                    name,
                    key_path,
                    unique,
                }),
            );
            return Ok(Some(
                new_platform_object(scope, state, "IDBIndex", index_id)?.into(),
            ));
        }
        ("IDBObjectStore", "deleteIndex") => {
            let name = required_string(scope, args, 0)?;
            let (transaction_id, store_name) = {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::IdbObjectStore(store) = &mut *record else {
                    return Err(V8BindingError::Allocation("IDBObjectStore record"));
                };
                if store.deleted {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBObjectStore".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                if store.indexes.remove(&name).is_none() {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBObjectStore".into(),
                        member: "NotFoundError".into(),
                    });
                }
                (store.transaction_id, store.name.clone())
            };
            let database_id = match &*platform_object(state, transaction_id)? {
                PlatformObjectRecord::IdbTransaction(transaction) => transaction.database_id,
                _ => return Err(V8BindingError::Allocation("IDBTransaction record")),
            };
            if let PlatformObjectRecord::IdbDatabase(database) =
                &mut *platform_object_mut(state, database_id)?
                && let Some(definition) = database.object_store_definitions.get_mut(&store_name)
            {
                definition.indexes.retain(|index| index.name != name);
            }
            return Ok(None);
        }
        ("IDBCursor", "continue" | "advance" | "continuePrimaryKey") => {
            let (request_id, source_id, direction, current_key, current_primary_key, current_index) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbCursor(cursor) = &*record else {
                    return Err(V8BindingError::Allocation("IDBCursor record"));
                };
                if cursor.iteration_pending || !cursor.got_value {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBCursor".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                let current =
                    cursor
                        .records
                        .get(cursor.index)
                        .ok_or(V8BindingError::Unsupported {
                            interface: "IDBCursor".into(),
                            member: "InvalidStateError".into(),
                        })?;
                (
                    cursor.request_id,
                    cursor.source_id,
                    cursor.direction.clone(),
                    current.key.clone(),
                    current.primary_key.clone(),
                    cursor.index,
                )
            };
            let store_id = match &*platform_object(state, source_id)? {
                PlatformObjectRecord::IdbIndex(index) => index.object_store_id,
                PlatformObjectRecord::IdbObjectStore(_) => source_id,
                _ => return Err(V8BindingError::Allocation("IDBCursor source record")),
            };
            idb_store_context(state, store_id)?;
            let reverse = direction.starts_with("prev");
            let requested_key = (!args.get(0).is_undefined() && member != "advance")
                .then(|| idb_key_from_value(scope, args.get(0)).map(idb_key_to_json))
                .transpose()?;
            let requested_primary = (member == "continuePrimaryKey")
                .then(|| idb_key_from_value(scope, args.get(1)).map(idb_key_to_json))
                .transpose()?;
            let next_index = if member == "advance" {
                let raw_count = args.get(0).number_value(scope).unwrap_or(f64::NAN);
                if !raw_count.is_finite()
                    || raw_count.fract() != 0.0
                    || raw_count < 1.0
                    || raw_count > u32::MAX as f64
                {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBCursor".into(),
                        member: "advance count must be greater than zero".into(),
                    });
                }
                let count = raw_count as u32;
                current_index.saturating_add(count as usize)
            } else if let Some(requested_key) = requested_key {
                let key_order = compare_idb_json_keys(&requested_key, &current_key);
                let primary_order = requested_primary
                    .as_ref()
                    .map(|key| compare_idb_json_keys(key, &current_primary_key));
                let progresses = if reverse {
                    key_order.is_lt()
                        || (key_order.is_eq() && primary_order.is_some_and(|order| order.is_lt()))
                } else {
                    key_order.is_gt()
                        || (key_order.is_eq() && primary_order.is_some_and(|order| order.is_gt()))
                };
                if !progresses {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBCursor".into(),
                        member: "DataError".into(),
                    });
                }
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbCursor(cursor) = &*record else {
                    return Err(V8BindingError::Allocation("IDBCursor record"));
                };
                cursor
                    .records
                    .iter()
                    .enumerate()
                    .skip(current_index + 1)
                    .find(|(_, record)| {
                        let order = compare_idb_json_keys(&record.key, &requested_key);
                        let key_matches = if reverse {
                            order.is_le()
                        } else {
                            order.is_ge()
                        };
                        if !key_matches {
                            return false;
                        }
                        requested_primary.as_ref().is_none_or(|primary| {
                            if !order.is_eq() {
                                true
                            } else {
                                let primary_order =
                                    compare_idb_json_keys(&record.primary_key, primary);
                                if reverse {
                                    primary_order.is_le()
                                } else {
                                    primary_order.is_ge()
                                }
                            }
                        })
                    })
                    .map_or(cursor.records.len(), |(index, _)| index)
            } else {
                current_index.saturating_add(1)
            };
            {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::IdbCursor(cursor) = &mut *record else {
                    return Err(V8BindingError::Allocation("IDBCursor record"));
                };
                cursor.index = next_index;
                cursor.got_value = false;
                cursor.iteration_pending = true;
            }
            let transaction_id = {
                let mut record = platform_object_mut(state, request_id)?;
                let PlatformObjectRecord::IdbRequest(request) = &mut *record else {
                    return Err(V8BindingError::Allocation("IDBRequest record"));
                };
                request.done = false;
                request.transaction_id
            };
            if let Some(transaction_id) = transaction_id
                && let PlatformObjectRecord::IdbTransaction(transaction) =
                    &mut *platform_object_mut(state, transaction_id)?
            {
                transaction.pending_requests = transaction.pending_requests.saturating_add(1);
                transaction.completion_requested = false;
                transaction.completion_queued = false;
            }
            state.deferred_platform_tasks.borrow_mut().push_back(
                DeferredPlatformTask::AdvanceIdbCursor {
                    cursor_id: id,
                    request_id,
                },
            );
            return Ok(None);
        }
        ("IDBCursor", "delete" | "update") => {
            let (source_id, primary_key, current_value) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::IdbCursor(cursor) = &*record else {
                    return Err(V8BindingError::Allocation("IDBCursor record"));
                };
                let current =
                    cursor
                        .records
                        .get(cursor.index)
                        .ok_or(V8BindingError::Unsupported {
                            interface: "IDBCursor".into(),
                            member: "InvalidStateError".into(),
                        })?;
                (
                    cursor.source_id,
                    current.primary_key.clone(),
                    current.value.clone(),
                )
            };
            let store_id = match &*platform_object(state, source_id)? {
                PlatformObjectRecord::IdbIndex(index) => index.object_store_id,
                PlatformObjectRecord::IdbObjectStore(_) => source_id,
                _ => return Err(V8BindingError::Allocation("IDBCursor source record")),
            };
            let (database, store, transaction_id) = idb_store_context(state, store_id)?;
            {
                let transaction = platform_object(state, transaction_id)?;
                let PlatformObjectRecord::IdbTransaction(transaction) = &*transaction else {
                    return Err(V8BindingError::Allocation("IDBTransaction record"));
                };
                if transaction.aborted || transaction.mode != "readwrite" {
                    return Err(V8BindingError::Unsupported {
                        interface: "IDBCursor".into(),
                        member: "ReadOnlyError".into(),
                    });
                }
            }
            let update_value = (member == "update")
                .then(|| idb_value_to_json(scope, state, args.get(0)))
                .transpose()?;
            let command = if member == "delete" {
                IdbHostCommand::Delete {
                    database,
                    store,
                    key: primary_key,
                }
            } else {
                IdbHostCommand::Batch {
                    database,
                    writes: vec![IdbWrite {
                        store,
                        key: Some(primary_key),
                        value: update_value.clone().unwrap_or(current_value.clone()),
                        add_only: false,
                    }],
                }
            };
            let request_id = new_idb_request_for_source(state, Some(transaction_id), Some(id));
            submit_idb(
                scope,
                state,
                request_id,
                command,
                if member == "delete" {
                    IdbCompletionAction::Unit
                } else {
                    IdbCompletionAction::Key
                },
            )?;
            if member == "update"
                && let PlatformObjectRecord::IdbCursor(cursor) =
                    &mut *platform_object_mut(state, id)?
                && let Some(record) = cursor.records.get_mut(cursor.index)
            {
                record.value = update_value.unwrap_or(current_value);
            }
            return Ok(Some(
                new_platform_object(scope, state, "IDBRequest", request_id)?.into(),
            ));
        }
        ("AbortController", "abort") => {
            let signal_id = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::AbortController { signal_id } = &*record else {
                    return Err(V8BindingError::Allocation("AbortController record"));
                };
                *signal_id
            };
            let changed = {
                let mut record = platform_object_mut(state, signal_id)?;
                let PlatformObjectRecord::AbortSignal(signal) = &mut *record else {
                    return Err(V8BindingError::Allocation("AbortSignal record"));
                };
                if signal.aborted {
                    false
                } else {
                    signal.aborted = true;
                    signal.reason = Some(v8::Global::new(scope, args.get(0)));
                    true
                }
            };
            if changed {
                let operations = state
                    .pending_platform_operations
                    .borrow()
                    .iter()
                    .filter_map(|(operation, pending)| matches!(pending, PendingPlatformOperation::FetchStart { signal_id: pending_signal, .. } if *pending_signal == signal_id).then_some(*operation))
                    .collect::<Vec<_>>();
                for operation in operations {
                    let pending = state
                        .pending_platform_operations
                        .borrow_mut()
                        .remove(&operation);
                    if let Some(PendingPlatformOperation::FetchStart { resolver, .. }) = pending {
                        let reason = args.get(0);
                        v8::Local::new(scope, resolver).reject(scope, reason);
                    }
                    if let Some(host) = state.platform_host.borrow().as_ref() {
                        host.cancel(operation);
                    }
                    state
                        .cancelled_platform_operations
                        .borrow_mut()
                        .insert(operation);
                }
                let body_streams = state
                    .platform_objects
                    .borrow()
                    .values()
                    .filter_map(|record| {
                        let PlatformObjectRecord::Response(response) = record else {
                            return None;
                        };
                        (response.signal_id == Some(signal_id))
                            .then_some(response.body_stream_id)
                            .flatten()
                    })
                    .collect::<Vec<_>>();
                let streams = {
                    let objects = state.platform_objects.borrow();
                    body_streams
                        .into_iter()
                        .filter_map(|stream_id| match objects.get(&stream_id) {
                            Some(PlatformObjectRecord::ReadableStream(stream)) => {
                                stream.host_stream_id
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                };
                for stream_id in streams {
                    let _ = submit_platform_operation(
                        state,
                        PlatformHostRequest::HttpAbort { stream_id },
                        PendingPlatformOperation::WebSocketCommand,
                    );
                }
                dispatch_platform_event(scope, state, signal_id, "abort", None)?;
            }
            return Ok(None);
        }
        ("AbortSignal", "throwIfAborted") => {
            let signal = platform_object(state, id)?;
            if let PlatformObjectRecord::AbortSignal(signal) = &*signal
                && signal.aborted
            {
                return Err(V8BindingError::Unsupported {
                    interface: "AbortSignal".into(),
                    member: "aborted".into(),
                });
            }
            return Ok(None);
        }
        ("Request", "clone") => {
            let (url, method, source_headers_id, signal_id, credentials, body, body_object_id) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::Request(request) = &*record else {
                    return Err(V8BindingError::Allocation("Request record"));
                };
                if request.body_used {
                    return Err(V8BindingError::Unsupported {
                        interface: "Request".into(),
                        member: "body already consumed".into(),
                    });
                }
                (
                    request.url.clone(),
                    request.method.clone(),
                    request.headers_id,
                    request.signal_id,
                    request.credentials.clone(),
                    request.body.clone(),
                    request.body_object_id,
                )
            };
            let clone = RequestRecord {
                url,
                method,
                headers_id: clone_headers(state, source_headers_id)?,
                signal_id,
                credentials,
                body,
                body_object_id,
                body_used: false,
            };
            let clone_id = register_platform_object(state, PlatformObjectRecord::Request(clone));
            return Ok(Some(
                new_platform_object(scope, state, "Request", clone_id)?.into(),
            ));
        }
        ("Request", "text" | "arrayBuffer") => {
            let bytes = {
                let mut request = platform_object_mut(state, id)?;
                let PlatformObjectRecord::Request(request) = &mut *request else {
                    return Err(V8BindingError::Allocation("Request record"));
                };
                if request.body_used {
                    return Err(V8BindingError::Unsupported {
                        interface: "Request".into(),
                        member: "body already consumed".into(),
                    });
                }
                request.body_used = true;
                request.body.clone()
            };
            let value = if member == "text" {
                v8_string(scope, &String::from_utf8_lossy(&bytes))?
            } else {
                array_buffer_from_bytes(scope, &bytes).into()
            };
            return Ok(Some(resolved_promise(scope, value)?.into()));
        }
        ("Response", "clone") => {
            let (bytes, status, url, source_headers_id, signal_id) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::Response(response) = &*record else {
                    return Err(V8BindingError::Allocation("Response record"));
                };
                if response.body_used {
                    return Err(V8BindingError::Unsupported {
                        interface: "Response".into(),
                        member: "body already consumed".into(),
                    });
                }
                (
                    response.bytes.clone(),
                    response.status,
                    response.url.clone(),
                    response.headers_id,
                    response.signal_id,
                )
            };
            let headers_id = clone_headers(state, source_headers_id)?;
            let body_stream_id = (!bytes.is_empty())
                .then(|| new_byte_stream_record(scope, state, bytes.clone()))
                .transpose()?;
            let clone_id = register_platform_object(
                state,
                PlatformObjectRecord::Response(ResponseRecord {
                    status,
                    url,
                    headers_id,
                    body_stream_id,
                    bytes,
                    body_used: false,
                    signal_id,
                }),
            );
            return Ok(Some(
                new_platform_object(scope, state, "Response", clone_id)?.into(),
            ));
        }
        ("Response", "text" | "json" | "arrayBuffer" | "blob") => {
            return Ok(Some(response_body_operation(scope, state, id, member)?));
        }
        ("XMLHttpRequest", "open") => {
            if !args.get(2).is_undefined() && !args.get(2).boolean_value(scope) {
                return Err(V8BindingError::Unsupported {
                    interface: "XMLHttpRequest".into(),
                    member: "synchronous requests".into(),
                });
            }
            let method = required_string(scope, args, 0)?.to_ascii_uppercase();
            let raw_url = required_string(scope, args, 1)?;
            let url = resolve_platform_url(scope, &raw_url)?;
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                return Err(V8BindingError::Allocation("XHR record"));
            };
            xhr.method = method;
            xhr.url = url;
            xhr.ready_state = 1;
            xhr.response_bytes.clear();
            xhr.status = 0;
            drop(record);
            dispatch_platform_event(scope, state, id, "readystatechange", None)?;
            return Ok(None);
        }
        ("XMLHttpRequest", "setRequestHeader") => {
            let name = normalize_header_name(required_string(scope, args, 0)?)?;
            let value = normalize_header_value(required_string(scope, args, 1)?)?;
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                return Err(V8BindingError::Allocation("XHR record"));
            };
            if xhr.ready_state != 1 {
                return Err(V8BindingError::Unsupported {
                    interface: "XMLHttpRequest".into(),
                    member: "invalid state".into(),
                });
            }
            xhr.request_headers.push((name, value));
            return Ok(None);
        }
        ("XMLHttpRequest", "getResponseHeader") => {
            let name = normalize_header_name(required_string(scope, args, 0)?)?;
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::XMLHttpRequest(xhr) = &*record else {
                return Err(V8BindingError::Allocation("XHR record"));
            };
            let value = xhr
                .response_headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(&name))
                .map(|(_, value)| value);
            return Ok(Some(match value {
                Some(value) => v8_string(scope, value)?,
                None => v8::null(scope).into(),
            }));
        }
        ("XMLHttpRequest", "getAllResponseHeaders") => {
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::XMLHttpRequest(xhr) = &*record else {
                return Err(V8BindingError::Allocation("XHR record"));
            };
            let value = xhr
                .response_headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            return Ok(Some(v8_string(scope, &value)?));
        }
        ("XMLHttpRequest", "send") => {
            let (request, timeout) = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::XMLHttpRequest(xhr) = &*record else {
                    return Err(V8BindingError::Allocation("XHR record"));
                };
                if xhr.ready_state != 1 {
                    return Err(V8BindingError::Unsupported {
                        interface: "XMLHttpRequest".into(),
                        member: "invalid state".into(),
                    });
                }
                let mut headers = xhr.request_headers.clone();
                let body = if args.get(0).is_null_or_undefined() {
                    Vec::new()
                } else {
                    if let Some(object) = args.get(0).to_object(scope)
                        && let Ok(blob_id) = stateful_id(scope, object)
                        && let Ok(blob) = platform_object(state, blob_id)
                        && let PlatformObjectRecord::Blob(blob) = &*blob
                        && !blob.media_type.is_empty()
                        && !headers
                            .iter()
                            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                    {
                        headers.push(("content-type".into(), blob.media_type.clone()));
                    }
                    bytes_from_value(scope, state, args.get(0))?
                };
                (
                    PlatformHttpRequest {
                        method: xhr.method.clone(),
                        url: xhr.url.clone(),
                        headers,
                        body,
                        credentials: if xhr.with_credentials {
                            "include".into()
                        } else {
                            "same-origin".into()
                        },
                    },
                    xhr.timeout,
                )
            };
            dispatch_platform_event(scope, state, id, "loadstart", None)?;
            if let Some(blob) = blob_url_record(state, &request.url) {
                state.deferred_platform_tasks.borrow_mut().push_back(
                    DeferredPlatformTask::CompleteBlobXhr {
                        xhr_id: id,
                        url: request.url,
                        blob,
                    },
                );
                return Ok(None);
            }
            let operation = submit_platform_operation(
                state,
                PlatformHostRequest::HttpStart { request },
                PendingPlatformOperation::XhrStart { xhr_id: id },
            )?;
            if let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *platform_object_mut(state, id)?
            {
                xhr.active_operation = Some(operation);
                xhr.deadline =
                    (timeout > 0).then(|| Instant::now() + Duration::from_millis(timeout));
            }
            return Ok(None);
        }
        ("XMLHttpRequest", "abort") => {
            let (operation, stream_id) = {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
                    return Err(V8BindingError::Allocation("XHR record"));
                };
                xhr.ready_state = 0;
                xhr.deadline = None;
                (xhr.active_operation.take(), xhr.stream_id.take())
            };
            if let Some(operation) = operation
                && let Some(host) = state.platform_host.borrow().as_ref()
            {
                host.cancel(operation);
                state
                    .pending_platform_operations
                    .borrow_mut()
                    .remove(&operation);
                state
                    .cancelled_platform_operations
                    .borrow_mut()
                    .insert(operation);
            }
            if let Some(stream_id) = stream_id {
                let _ = submit_platform_operation(
                    state,
                    PlatformHostRequest::HttpAbort { stream_id },
                    PendingPlatformOperation::WebSocketCommand,
                );
            }
            dispatch_platform_event(scope, state, id, "abort", None)?;
            dispatch_platform_event(scope, state, id, "loadend", None)?;
            return Ok(None);
        }
        ("WebSocket", "send") => {
            let socket_id = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::WebSocket(socket) = &*record else {
                    return Err(V8BindingError::Allocation("WebSocket record"));
                };
                socket.socket_id.ok_or(V8BindingError::Unsupported {
                    interface: "WebSocket".into(),
                    member: "not open".into(),
                })?
            };
            let frame = if args.get(0).is_string() {
                PlatformWebSocketFrame::Text {
                    value: args.get(0).to_rust_string_lossy(scope),
                }
            } else {
                PlatformWebSocketFrame::Binary {
                    bytes: bytes_from_value(scope, state, args.get(0))?,
                }
            };
            let (frame_type, frame_bytes) = match &frame {
                PlatformWebSocketFrame::Text { value } => ("text", value.len()),
                PlatformWebSocketFrame::Binary { bytes } => ("binary", bytes.len()),
                PlatformWebSocketFrame::Close { reason, .. } => ("close", reason.len()),
            };
            record_platform_trace(
                scope,
                "web_socket_frame",
                serde_json::json!({"socket_id":socket_id,"direction":"outbound","frame_type":frame_type,"bytes":frame_bytes}),
            );
            submit_platform_operation(
                state,
                PlatformHostRequest::WebSocketSend { socket_id, frame },
                PendingPlatformOperation::WebSocketCommand,
            )?;
            return Ok(None);
        }
        ("WebSocket", "close") => {
            let (socket_id, already_closed) = {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::WebSocket(socket) = &mut *record else {
                    return Err(V8BindingError::Allocation("WebSocket record"));
                };
                let closed = socket.ready_state == 3;
                socket.ready_state = 2;
                (socket.socket_id, closed)
            };
            if !already_closed && let Some(socket_id) = socket_id {
                let code = args.get(0).uint32_value(scope).map(|value| value as u16);
                let reason = args.get(1).to_rust_string_lossy(scope);
                submit_platform_operation(
                    state,
                    PlatformHostRequest::WebSocketClose {
                        socket_id,
                        code,
                        reason: reason.clone(),
                    },
                    PendingPlatformOperation::WebSocketClose {
                        websocket_id: id,
                        code,
                        reason,
                    },
                )?;
            }
            return Ok(None);
        }
        ("Blob", "slice") => {
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::Blob(blob) = &*record else {
                return Err(V8BindingError::Allocation("Blob record"));
            };
            let length = blob.bytes.len() as i64;
            let normalize = |value: i64| {
                if value < 0 {
                    (length + value).max(0)
                } else {
                    value.min(length)
                }
            };
            let start = normalize(args.get(0).integer_value(scope).unwrap_or(0));
            let end = if args.get(1).is_undefined() {
                length
            } else {
                normalize(args.get(1).integer_value(scope).unwrap_or(0))
            }
            .max(start);
            let media_type = if args.get(2).is_undefined() {
                String::new()
            } else {
                normalize_media_type(args.get(2).to_rust_string_lossy(scope))
            };
            let bytes = blob.bytes[start as usize..end as usize].to_vec();
            drop(record);
            let blob_id = register_platform_object(
                state,
                PlatformObjectRecord::Blob(BlobRecord {
                    bytes,
                    media_type,
                    file: None,
                }),
            );
            return Ok(Some(
                new_platform_object(scope, state, "Blob", blob_id)?.into(),
            ));
        }
        ("Blob", "arrayBuffer") => {
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::Blob(blob) = &*record else {
                return Err(V8BindingError::Allocation("Blob record"));
            };
            let buffer = array_buffer_from_bytes(scope, &blob.bytes);
            let promise = resolved_promise(scope, buffer.into())?;
            return Ok(Some(promise.into()));
        }
        ("Blob", "text") => {
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::Blob(blob) = &*record else {
                return Err(V8BindingError::Allocation("Blob record"));
            };
            let value = String::from_utf8_lossy(&blob.bytes);
            let value = v8_string(scope, &value)?;
            let promise = resolved_promise(scope, value)?;
            return Ok(Some(promise.into()));
        }
        ("Blob", "stream") => {
            let bytes = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::Blob(blob) = &*record else {
                    return Err(V8BindingError::Allocation("Blob record"));
                };
                blob.bytes.clone()
            };
            let stream_id = reserve_platform_object_id(state);
            let controller_id = reserve_platform_object_id(state);
            let buffer = array_buffer_from_bytes(scope, &bytes);
            let array = v8::Uint8Array::new(scope, buffer, 0, bytes.len())
                .ok_or(V8BindingError::Allocation("Blob stream chunk"))?;
            let array_value: v8::Local<v8::Value> = array.into();
            insert_platform_object(
                state,
                controller_id,
                PlatformObjectRecord::ReadableController { stream_id },
            );
            insert_platform_object(
                state,
                stream_id,
                PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
                    queue: VecDeque::from([v8::Global::new(scope, array_value)]),
                    waiting: VecDeque::new(),
                    closed: true,
                    locked: false,
                    pull: None,
                    cancel: None,
                    controller_id,
                    pipes: Vec::new(),
                    host_stream_id: None,
                    host_read_pending: false,
                }),
            );
            return Ok(Some(
                new_platform_object(scope, state, "ReadableStream", stream_id)?.into(),
            ));
        }
        ("Headers", "append" | "set" | "delete" | "get" | "has") => {
            let name = normalize_header_name(required_string(scope, args, 0)?)?;
            if member == "get" || member == "has" {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::Headers(headers) = &*record else {
                    return Err(V8BindingError::Allocation("Headers record"));
                };
                let value = headers
                    .entries
                    .iter()
                    .find(|(key, _)| key == &name)
                    .map(|(_, value)| value);
                return if member == "has" {
                    Ok(Some(v8::Boolean::new(scope, value.is_some()).into()))
                } else if let Some(value) = value {
                    Ok(Some(v8_string(scope, value)?))
                } else {
                    Ok(Some(v8::null(scope).into()))
                };
            }
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::Headers(headers) = &mut *record else {
                return Err(V8BindingError::Allocation("Headers record"));
            };
            if member == "delete" {
                headers.entries.retain(|(key, _)| key != &name);
            } else {
                let value = normalize_header_value(required_string(scope, args, 1)?)?;
                if member == "set" {
                    headers.entries.retain(|(key, _)| key != &name);
                }
                append_header(headers, name, value)?;
            }
            return Ok(None);
        }
        ("Headers", "@@iterator") => {
            let record = platform_object(state, id)?;
            let PlatformObjectRecord::Headers(headers) = &*record else {
                return Err(V8BindingError::Allocation("Headers record"));
            };
            let entries = v8::Array::new(scope, headers.entries.len() as i32);
            for (index, (name, value)) in headers.entries.iter().enumerate() {
                let pair = v8::Array::new(scope, 2);
                let name = v8_string(scope, name)?;
                let value = v8_string(scope, value)?;
                pair.set_index(scope, 0, name);
                pair.set_index(scope, 1, value);
                entries.set_index(scope, index as u32, pair.into());
            }
            let symbol = v8::Symbol::get_iterator(scope);
            let function = entries
                .get(scope, symbol.into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .ok_or(V8BindingError::Allocation("Headers iterator"))?;
            let iterator = function
                .call(scope, entries.into(), &[])
                .ok_or(V8BindingError::Allocation("Headers iterator result"))?;
            return Ok(Some(iterator));
        }
        ("ReadableStream", "getReader") => {
            {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                    return Err(V8BindingError::Allocation("ReadableStream record"));
                };
                if stream.locked {
                    return Err(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "stream is locked".into(),
                    });
                }
                stream.locked = true;
            }
            let reader_id = register_platform_object(
                state,
                PlatformObjectRecord::ReadableReader { stream_id: id },
            );
            return Ok(Some(
                new_platform_object(scope, state, "ReadableStreamDefaultReader", reader_id)?.into(),
            ));
        }
        ("ReadableStreamDefaultReader", "read") => {
            let stream_id = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::ReadableReader { stream_id } = &*record else {
                    return Err(V8BindingError::Allocation("ReadableStream reader"));
                };
                *stream_id
            };
            let (queued, closed, pull, controller_id, host_stream_id, host_read_pending) = {
                let mut record = platform_object_mut(state, stream_id)?;
                let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                    return Err(V8BindingError::Allocation("ReadableStream record"));
                };
                (
                    stream.queue.pop_front(),
                    stream.closed,
                    stream.pull.clone(),
                    stream.controller_id,
                    stream.host_stream_id,
                    stream.host_read_pending,
                )
            };
            if let Some(queued) = queued {
                let value = v8::Local::new(scope, queued);
                let result = read_result(scope, false, value)?;
                return Ok(Some(resolved_promise(scope, result.into())?.into()));
            }
            if closed {
                let undefined = v8::undefined(scope).into();
                let result = read_result(scope, true, undefined)?;
                return Ok(Some(resolved_promise(scope, result.into())?.into()));
            }
            let resolver = v8::PromiseResolver::new(scope)
                .ok_or(V8BindingError::Allocation("ReadableStream read promise"))?;
            let promise = resolver.get_promise(scope);
            {
                let mut record = platform_object_mut(state, stream_id)?;
                let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                    return Err(V8BindingError::Allocation("ReadableStream record"));
                };
                stream.waiting.push_back(v8::Global::new(scope, resolver));
                if host_stream_id.is_some() && !host_read_pending {
                    stream.host_read_pending = true;
                }
            }
            if let Some(host_stream_id) = host_stream_id.filter(|_| !host_read_pending) {
                submit_platform_operation(
                    state,
                    PlatformHostRequest::HttpRead {
                        stream_id: host_stream_id,
                        max_bytes: 64 * 1024,
                    },
                    PendingPlatformOperation::BodyRead {
                        stream_object_id: stream_id,
                    },
                )?;
            }
            if let Some(pull) = pull {
                let pull = v8::Local::new(scope, pull);
                let controller = new_stateful_object(
                    scope,
                    state,
                    "ReadableStreamDefaultController",
                    controller_id,
                )?;
                let receiver = v8::undefined(scope).into();
                pull.call(scope, receiver, &[controller.into()]);
            }
            return Ok(Some(promise.into()));
        }
        ("ReadableStream", "cancel") | ("ReadableStreamDefaultReader", "cancel") => {
            let stream_id = if interface == "ReadableStream" {
                id
            } else {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::ReadableReader { stream_id } = &*record else {
                    return Err(V8BindingError::Allocation("ReadableStream reader"));
                };
                *stream_id
            };
            let (cancel, controller_id, host_stream_id) = {
                let record = platform_object(state, stream_id)?;
                let PlatformObjectRecord::ReadableStream(stream) = &*record else {
                    return Err(V8BindingError::Allocation("ReadableStream record"));
                };
                (
                    stream.cancel.clone(),
                    stream.controller_id,
                    stream.host_stream_id,
                )
            };
            readable_close(scope, state, controller_id)?;
            if let Some(stream_id) = host_stream_id {
                submit_platform_operation(
                    state,
                    PlatformHostRequest::HttpAbort { stream_id },
                    PendingPlatformOperation::WebSocketCommand,
                )?;
            }
            return Ok(Some(
                callback_promise(scope, cancel, &[args.get(0)])?.into(),
            ));
        }
        ("ReadableStreamDefaultReader", "releaseLock") => {
            let stream_id = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::ReadableReader { stream_id } = &*record else {
                    return Err(V8BindingError::Allocation("ReadableStream reader"));
                };
                *stream_id
            };
            let mut record = platform_object_mut(state, stream_id)?;
            let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
                return Err(V8BindingError::Allocation("ReadableStream record"));
            };
            stream.locked = false;
            return Ok(None);
        }
        ("ReadableStreamDefaultController", "enqueue") => {
            readable_enqueue(scope, state, id, args.get(0))?;
            return Ok(None);
        }
        ("ReadableStreamDefaultController", "close") => {
            readable_close(scope, state, id)?;
            return Ok(None);
        }
        ("ReadableStream", "pipeThrough") => {
            let transform = args
                .get(0)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("TransformStream argument"))?;
            let transform_id = stateful_id(scope, transform)?;
            let (readable_id, writable_id) = {
                let record = platform_object(state, transform_id)?;
                let PlatformObjectRecord::TransformStream {
                    readable_id,
                    writable_id,
                } = &*record
                else {
                    return Err(V8BindingError::Allocation("TransformStream record"));
                };
                (*readable_id, *writable_id)
            };
            pipe_readable_to(scope, state, id, writable_id)?;
            return Ok(Some(
                new_platform_object(scope, state, "ReadableStream", readable_id)?.into(),
            ));
        }
        ("ReadableStream", "pipeTo") => {
            let destination = args
                .get(0)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("WritableStream argument"))?;
            let writable_id = stateful_id(scope, destination)?;
            return Ok(Some(
                pipe_readable_to(scope, state, id, writable_id)?.into(),
            ));
        }
        ("WritableStream", "getWriter") => {
            {
                let mut record = platform_object_mut(state, id)?;
                let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
                    return Err(V8BindingError::Allocation("WritableStream record"));
                };
                if stream.locked {
                    return Err(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "stream is locked".into(),
                    });
                }
                stream.locked = true;
            }
            let writer_id = register_platform_object(
                state,
                PlatformObjectRecord::WritableWriter { stream_id: id },
            );
            return Ok(Some(
                new_platform_object(scope, state, "WritableStreamDefaultWriter", writer_id)?.into(),
            ));
        }
        ("WritableStreamDefaultWriter", "write") => {
            let stream_id = writer_stream_id(scope, state, id)?;
            return Ok(Some(
                writable_write(scope, state, stream_id, args.get(0))?.into(),
            ));
        }
        ("WritableStream", "close") | ("WritableStreamDefaultWriter", "close") => {
            let stream_id = if interface == "WritableStream" {
                id
            } else {
                writer_stream_id(scope, state, id)?
            };
            return Ok(Some(writable_close(scope, state, stream_id)?.into()));
        }
        ("WritableStream", "abort") | ("WritableStreamDefaultWriter", "abort") => {
            let stream_id = if interface == "WritableStream" {
                id
            } else {
                writer_stream_id(scope, state, id)?
            };
            let callback = {
                let mut record = platform_object_mut(state, stream_id)?;
                let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
                    return Err(V8BindingError::Allocation("WritableStream record"));
                };
                stream.closed = true;
                match &stream.kind {
                    WritableKind::Sink { abort, .. } => abort.clone(),
                    WritableKind::Transform { .. } => None,
                }
            };
            return Ok(Some(
                callback_promise(scope, callback, &[args.get(0)])?.into(),
            ));
        }
        ("WritableStreamDefaultController", "error") => {
            let stream_id = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::WritableController { stream_id } = &*record else {
                    return Err(V8BindingError::Allocation("WritableStream controller"));
                };
                *stream_id
            };
            let mut record = platform_object_mut(state, stream_id)?;
            let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
                return Err(V8BindingError::Allocation("WritableStream record"));
            };
            stream.closed = true;
            return Ok(None);
        }
        ("WritableStreamDefaultWriter", "releaseLock") => {
            let stream_id = writer_stream_id(scope, state, id)?;
            let mut record = platform_object_mut(state, stream_id)?;
            let PlatformObjectRecord::WritableStream(stream) = &mut *record else {
                return Err(V8BindingError::Allocation("WritableStream record"));
            };
            stream.locked = false;
            return Ok(None);
        }
        _ => {}
    }
    Err(V8BindingError::Unsupported {
        interface: interface.into(),
        member: member.into(),
    })
}

fn idb_store_context(
    state: &V8BindingState,
    store_id: u64,
) -> Result<(String, String, u64), V8BindingError> {
    let (transaction_id, store) = {
        let record = platform_object(state, store_id)?;
        let PlatformObjectRecord::IdbObjectStore(store) = &*record else {
            return Err(V8BindingError::Allocation("IDBObjectStore record"));
        };
        if store.deleted {
            return Err(V8BindingError::Unsupported {
                interface: "IDBObjectStore".into(),
                member: "InvalidStateError".into(),
            });
        }
        (store.transaction_id, store.name.clone())
    };
    let database_id = {
        let record = platform_object(state, transaction_id)?;
        let PlatformObjectRecord::IdbTransaction(transaction) = &*record else {
            return Err(V8BindingError::Allocation("IDBTransaction record"));
        };
        if transaction.aborted || !transaction.active {
            return Err(V8BindingError::Unsupported {
                interface: "IDBTransaction".into(),
                member: "TransactionInactiveError".into(),
            });
        }
        transaction.database_id
    };
    let database = {
        let record = platform_object(state, database_id)?;
        let PlatformObjectRecord::IdbDatabase(database) = &*record else {
            return Err(V8BindingError::Allocation("IDBDatabase record"));
        };
        database.name.clone()
    };
    Ok((database, store, transaction_id))
}

fn form_data_entry_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    value: v8::Local<'s, v8::Value>,
    filename: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if let Some(object) = value.to_object(scope)
        && let Ok(id) = stateful_id(scope, object)
        && let Ok(record) = platform_object(state, id)
        && let PlatformObjectRecord::Blob(blob) = &*record
    {
        if !filename.is_undefined() {
            let bytes = blob.bytes.clone();
            let media_type = blob.media_type.clone();
            drop(record);
            let file_id = register_platform_object(
                state,
                PlatformObjectRecord::Blob(BlobRecord {
                    bytes,
                    media_type,
                    file: Some(FileRecord {
                        name: filename.to_rust_string_lossy(scope),
                        last_modified: 0,
                    }),
                }),
            );
            return Ok(new_platform_object(scope, state, "File", file_id)?.into());
        }
        return Ok(value);
    }
    v8_string(scope, &value.to_rust_string_lossy(scope))
}

fn form_data_body(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Value>,
) -> Result<Option<(Vec<u8>, String)>, V8BindingError> {
    let Some(object) = value.to_object(scope) else {
        return Ok(None);
    };
    let Ok(id) = stateful_id(scope, object) else {
        return Ok(None);
    };
    let entries = {
        let Ok(record) = platform_object(state, id) else {
            return Ok(None);
        };
        let PlatformObjectRecord::FormData(form) = &*record else {
            return Ok(None);
        };
        form.entries
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>()
    };
    const BOUNDARY: &str = "----oryn-generated-form-data";
    let mut output = Vec::new();
    for (name, value) in entries {
        output.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"",
                name.replace(['\r', '\n', '"'], "_")
            )
            .as_bytes(),
        );
        let value = v8::Local::new(scope, value);
        let blob = value
            .to_object(scope)
            .and_then(|object| stateful_id(scope, object).ok())
            .and_then(|id| {
                let record = platform_object(state, id).ok()?;
                let PlatformObjectRecord::Blob(blob) = &*record else {
                    return None;
                };
                Some((
                    blob.bytes.clone(),
                    blob.media_type.clone(),
                    blob.file.as_ref().map(|file| file.name.clone()),
                ))
            });
        if let Some((bytes, media_type, filename)) = blob {
            if let Some(filename) = filename {
                output.extend_from_slice(
                    format!(
                        "; filename=\"{}\"",
                        filename.replace(['\r', '\n', '"'], "_")
                    )
                    .as_bytes(),
                );
            }
            output.extend_from_slice(
                format!(
                    "\r\nContent-Type: {}\r\n\r\n",
                    if media_type.is_empty() {
                        "application/octet-stream"
                    } else {
                        &media_type
                    }
                )
                .as_bytes(),
            );
            output.extend_from_slice(&bytes);
        } else {
            output.extend_from_slice(b"\r\n\r\n");
            output.extend_from_slice(value.to_rust_string_lossy(scope).as_bytes());
        }
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    Ok(Some((
        output,
        format!("multipart/form-data; boundary={BOUNDARY}"),
    )))
}

fn idb_source_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    source_id: u64,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let (store_id, index) = if interface == "IDBIndex" {
        let record = platform_object(state, source_id)?;
        let PlatformObjectRecord::IdbIndex(index) = &*record else {
            return Err(V8BindingError::Allocation("IDBIndex record"));
        };
        (index.object_store_id, Some(index.name.clone()))
    } else {
        (source_id, None)
    };
    let (database, store, transaction_id) = idb_store_context(state, store_id)?;
    let query = if args.get(0).is_undefined() {
        None
    } else {
        Some(idb_query_to_json(scope, state, args.get(0))?)
    };
    let request_id = new_idb_request_for_source(state, Some(transaction_id), Some(source_id));
    let direction_index = if member.starts_with("open") { 1 } else { 0 };
    let direction = if args.get(direction_index).is_undefined() {
        "next".into()
    } else {
        args.get(direction_index).to_rust_string_lossy(scope)
    };
    let (command, action) = match member {
        "get" if index.is_none() => (
            IdbHostCommand::Get {
                database,
                store,
                key: query.unwrap_or(serde_json::Value::Null),
            },
            IdbCompletionAction::Value,
        ),
        "getAll" if index.is_none() && query.is_none() => (
            IdbHostCommand::GetAll { database, store },
            IdbCompletionAction::Values {
                count: args.get(1).uint32_value(scope).map(|value| value as usize),
            },
        ),
        "count" if index.is_none() && query.is_none() => (
            IdbHostCommand::Count { database, store },
            IdbCompletionAction::Count,
        ),
        "openCursor" | "openKeyCursor" => (
            IdbHostCommand::Cursor {
                database,
                store,
                index,
                query,
                reverse: direction.starts_with("prev"),
            },
            IdbCompletionAction::Cursor {
                source_id,
                direction,
            },
        ),
        "getAll" => (
            IdbHostCommand::Cursor {
                database,
                store,
                index,
                query,
                reverse: false,
            },
            IdbCompletionAction::Values {
                count: args.get(1).uint32_value(scope).map(|value| value as usize),
            },
        ),
        "getAllKeys" => (
            IdbHostCommand::Cursor {
                database,
                store,
                index,
                query,
                reverse: false,
            },
            IdbCompletionAction::Keys {
                count: args.get(1).uint32_value(scope).map(|value| value as usize),
            },
        ),
        "count" => (
            IdbHostCommand::Cursor {
                database,
                store,
                index,
                query,
                reverse: false,
            },
            IdbCompletionAction::Count,
        ),
        "get" => (
            IdbHostCommand::Cursor {
                database,
                store,
                index,
                query,
                reverse: false,
            },
            IdbCompletionAction::Value,
        ),
        "getKey" => (
            IdbHostCommand::Cursor {
                database,
                store,
                index,
                query,
                reverse: false,
            },
            IdbCompletionAction::Key,
        ),
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    };
    submit_idb(scope, state, request_id, command, action)?;
    Ok(Some(
        new_platform_object(scope, state, "IDBRequest", request_id)?.into(),
    ))
}

fn call_stateful_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    if interface == "Storage" {
        return call_storage_operation(scope, state, object, member, args);
    }
    if interface == "History" {
        return call_history_operation(scope, state, member, args);
    }
    if matches!(interface, "Range" | "Selection") {
        synchronize_live_ranges(state)?;
    }
    if interface == "DOMImplementation" {
        match member {
            "createDocument" => {
                let namespace = if args.get(0).is_null_or_undefined() {
                    String::new()
                } else {
                    required_string(scope, args, 0)?
                };
                let qualified_name = if args.get(1).is_null_or_undefined() {
                    String::new()
                } else {
                    required_string(scope, args, 1)?
                };
                let requested_doctype = if args.get(2).is_null_or_undefined() {
                    None
                } else {
                    Some(argument_node(scope, state, args.get(2))?)
                };
                let mut dom = state.dom.borrow_mut();
                let document = dom.create(NodeKind::Document);
                if let Some(doctype) = requested_doctype {
                    if !matches!(
                        dom.arena().get(doctype)?.kind,
                        NodeKind::DocumentType { .. }
                    ) {
                        return Err(V8BindingError::Unsupported {
                            interface: "DOMImplementation".into(),
                            member: "HierarchyRequestError".into(),
                        });
                    }
                    dom.append_child(document, doctype)?;
                }
                if !qualified_name.is_empty() {
                    let root = dom.create_element_ns(&namespace, &qualified_name);
                    dom.append_child(document, root)?;
                    state.node_documents.borrow_mut().insert(root, document);
                }
                drop(dom);
                state.node_documents.borrow_mut().insert(document, document);
                return Ok(Some(wrap_node(scope, state, document)?.into()));
            }
            "createHTMLDocument" => {
                let title = if args.get(0).is_undefined() {
                    String::new()
                } else {
                    required_string(scope, args, 0)?
                };
                let mut dom = state.dom.borrow_mut();
                let document = dom.create(NodeKind::Document);
                let doctype = dom.create(NodeKind::DocumentType {
                    name: "html".into(),
                    public_id: String::new(),
                    system_id: String::new(),
                });
                let html = dom.create_element("html");
                let head = dom.create_element("head");
                let title_element = dom.create_element("title");
                let title_text = dom.create_text(title);
                let body = dom.create_element("body");
                dom.append_child(document, doctype)?;
                dom.append_child(document, html)?;
                dom.append_child(html, head)?;
                dom.append_child(head, title_element)?;
                dom.append_child(title_element, title_text)?;
                dom.append_child(html, body)?;
                drop(dom);
                let mut owners = state.node_documents.borrow_mut();
                owners.insert(document, document);
                for owned in [doctype, html, head, title_element, title_text, body] {
                    owners.insert(owned, document);
                }
                return Ok(Some(wrap_node(scope, state, document)?.into()));
            }
            "createDocumentType" => {
                let qualified_name = required_string(scope, args, 0)?;
                let public_id = if args.get(1).is_undefined() {
                    String::new()
                } else {
                    required_string(scope, args, 1)?
                };
                let system_id = if args.get(2).is_undefined() {
                    String::new()
                } else {
                    required_string(scope, args, 2)?
                };
                if qualified_name.is_empty() || qualified_name.chars().any(char::is_whitespace) {
                    return Err(V8BindingError::Unsupported {
                        interface: "DOMImplementation".into(),
                        member: "InvalidCharacterError".into(),
                    });
                }
                let node = state.dom.borrow_mut().create(NodeKind::DocumentType {
                    name: qualified_name,
                    public_id,
                    system_id,
                });
                return Ok(Some(wrap_node(scope, state, node)?.into()));
            }
            _ => {}
        }
    }
    if interface == "Range" {
        let id = stateful_id(scope, object)?;
        match member {
            "setStart" | "setEnd" => {
                let node = argument_node(scope, state, args.get(0))?;
                let offset = args.get(1).uint32_value(scope).unwrap_or(0);
                if offset > boundary_length(&state.dom.borrow(), node)? {
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "IndexSizeError".into(),
                    });
                }
                let mut ranges = state.ranges.borrow_mut();
                let range = ranges
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                if member == "setStart" {
                    range.start_container = node;
                    range.start_offset = offset;
                    if tree_root(&state.dom.borrow(), range.start_container)?
                        != tree_root(&state.dom.borrow(), range.end_container)?
                        || compare_boundary_points(
                            &state.dom.borrow(),
                            (range.start_container, range.start_offset),
                            (range.end_container, range.end_offset),
                        )? > 0
                    {
                        range.end_container = node;
                        range.end_offset = offset;
                    }
                } else {
                    range.end_container = node;
                    range.end_offset = offset;
                    if tree_root(&state.dom.borrow(), range.start_container)?
                        != tree_root(&state.dom.borrow(), range.end_container)?
                        || compare_boundary_points(
                            &state.dom.borrow(),
                            (range.start_container, range.start_offset),
                            (range.end_container, range.end_offset),
                        )? > 0
                    {
                        range.start_container = node;
                        range.start_offset = offset;
                    }
                }
                drop(ranges);
                persist_live_range(state, id)?;
                return Ok(None);
            }
            "setStartBefore" | "setStartAfter" | "setEndBefore" | "setEndAfter" => {
                let node = argument_node(scope, state, args.get(0))?;
                let (container, offset) =
                    relative_boundary(&state.dom.borrow(), node, member.ends_with("After"))?;
                let mut ranges = state.ranges.borrow_mut();
                let range = ranges
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                if member.starts_with("setStart") {
                    range.start_container = container;
                    range.start_offset = offset;
                } else {
                    range.end_container = container;
                    range.end_offset = offset;
                }
                drop(ranges);
                persist_live_range(state, id)?;
                return Ok(None);
            }
            "collapse" => {
                let to_start = args.get(0).boolean_value(scope);
                let mut ranges = state.ranges.borrow_mut();
                let range = ranges
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                if to_start {
                    range.end_container = range.start_container;
                    range.end_offset = range.start_offset;
                } else {
                    range.start_container = range.end_container;
                    range.start_offset = range.end_offset;
                }
                drop(ranges);
                persist_live_range(state, id)?;
                return Ok(None);
            }
            "selectNode" | "selectNodeContents" => {
                let node = argument_node(scope, state, args.get(0))?;
                let (container, start, end) = if member == "selectNode" {
                    let (container, start) = relative_boundary(&state.dom.borrow(), node, false)?;
                    (container, start, start.saturating_add(1))
                } else {
                    let dom = state.dom.borrow();
                    let end = boundary_length(&dom, node)?;
                    (node, 0, end)
                };
                let mut ranges = state.ranges.borrow_mut();
                let range = ranges
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                range.start_container = container;
                range.start_offset = start;
                range.end_container = container;
                range.end_offset = end;
                drop(ranges);
                persist_live_range(state, id)?;
                return Ok(None);
            }
            "compareBoundaryPoints" => {
                let how = args.get(0).uint32_value(scope).unwrap_or(u32::MAX);
                let other_id = stateful_id(
                    scope,
                    v8::Local::<v8::Object>::try_from(args.get(1))
                        .map_err(|_| V8BindingError::Allocation("Range argument"))?,
                )?;
                let ranges = state.ranges.borrow();
                let range = ranges
                    .get(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let other = ranges
                    .get(&other_id)
                    .ok_or(V8BindingError::Allocation("Range argument"))?;
                if tree_root(&state.dom.borrow(), range.start_container)?
                    != tree_root(&state.dom.borrow(), other.start_container)?
                {
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "WrongDocumentError".into(),
                    });
                }
                let (left, right) = match how {
                    0 => (
                        (range.start_container, range.start_offset),
                        (other.start_container, other.start_offset),
                    ),
                    1 => (
                        (range.end_container, range.end_offset),
                        (other.start_container, other.start_offset),
                    ),
                    2 => (
                        (range.end_container, range.end_offset),
                        (other.end_container, other.end_offset),
                    ),
                    3 => (
                        (range.start_container, range.start_offset),
                        (other.end_container, other.end_offset),
                    ),
                    _ => {
                        return Err(V8BindingError::Unsupported {
                            interface: "Range".into(),
                            member: "NotSupportedError".into(),
                        });
                    }
                };
                let ordering = compare_boundary_points(&state.dom.borrow(), left, right)?;
                return Ok(Some(v8::Integer::new(scope, ordering).into()));
            }
            "deleteContents" | "extractContents" | "cloneContents" => {
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let mode = match member {
                    "deleteContents" => RangeContentsMode::Delete,
                    "extractContents" => RangeContentsMode::Extract,
                    _ => RangeContentsMode::Clone,
                };
                let fragment = range_contents(&mut state.dom.borrow_mut(), &range, mode)?;
                if mode != RangeContentsMode::Clone {
                    let mut ranges = state.ranges.borrow_mut();
                    let range = ranges
                        .get_mut(&id)
                        .ok_or(V8BindingError::Allocation("Range record"))?;
                    range.end_container = range.start_container;
                    range.end_offset = range.start_offset;
                    drop(ranges);
                    persist_live_range(state, id)?;
                }
                if member == "deleteContents" {
                    return Ok(None);
                }
                return Ok(Some(wrap_node(scope, state, fragment)?.into()));
            }
            "insertNode" => {
                let node = argument_node(scope, state, args.get(0))?;
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                if matches!(
                    state.dom.borrow().arena().get(range.start_container)?.kind,
                    NodeKind::Attribute { .. }
                ) || matches!(
                    state.dom.borrow().arena().get(node)?.kind,
                    NodeKind::Attribute { .. }
                ) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "HierarchyRequestError".into(),
                    });
                }
                insert_at_boundary(
                    &mut state.dom.borrow_mut(),
                    range.start_container,
                    range.start_offset,
                    node,
                )?;
                return Ok(None);
            }
            "surroundContents" => {
                let wrapper = argument_node(scope, state, args.get(0))?;
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let mut dom = state.dom.borrow_mut();
                if matches!(
                    dom.arena().get(range.start_container)?.kind,
                    NodeKind::Attribute { .. }
                ) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "HierarchyRequestError".into(),
                    });
                }
                if range_partially_contains_non_text(&dom, &range)? {
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "InvalidStateError".into(),
                    });
                }
                if matches!(
                    dom.arena().get(wrapper)?.kind,
                    NodeKind::Document | NodeKind::DocumentType { .. } | NodeKind::DocumentFragment
                ) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "InvalidNodeTypeError".into(),
                    });
                }
                for child in dom.children(wrapper)?.to_vec() {
                    dom.remove_child(wrapper, child)?;
                }
                let fragment = range_contents(&mut dom, &range, RangeContentsMode::Extract)?;
                insert_at_boundary(&mut dom, range.start_container, range.start_offset, wrapper)?;
                for child in dom.children(fragment)?.to_vec() {
                    dom.append_child(wrapper, child)?;
                }
                drop(dom);
                let mut ranges = state.ranges.borrow_mut();
                let range = ranges
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let (container, start) = relative_boundary(&state.dom.borrow(), wrapper, false)?;
                range.start_container = container;
                range.start_offset = start;
                range.end_container = container;
                range.end_offset = start.saturating_add(1);
                drop(ranges);
                persist_live_range(state, id)?;
                return Ok(None);
            }
            "cloneRange" => {
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let clone_id = register_live_range(state, range)?;
                return Ok(Some(
                    new_stateful_object(scope, state, "Range", clone_id)?.into(),
                ));
            }
            "detach" => return Ok(None),
            "isPointInRange" | "comparePoint" => {
                let node = argument_node(scope, state, args.get(0))?;
                let offset = args.get(1).uint32_value(scope).unwrap_or(0);
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let dom = state.dom.borrow();
                validate_boundary_offset(&dom, node, offset)?;
                if tree_root(&dom, node)? != tree_root(&dom, range.start_container)? {
                    if member == "isPointInRange" {
                        return Ok(Some(v8::Boolean::new(scope, false).into()));
                    }
                    return Err(V8BindingError::Unsupported {
                        interface: "Range".into(),
                        member: "WrongDocumentError".into(),
                    });
                }
                let point_position = traversal_position(&dom, node)?;
                let start_position = traversal_position(&dom, range.start_container)?;
                let end_position = traversal_position(&dom, range.end_container)?;
                let before_start = point_position < start_position
                    || (point_position == start_position && offset < range.start_offset);
                let after_end = point_position > end_position
                    || (point_position == end_position && offset > range.end_offset);
                if member == "isPointInRange" {
                    return Ok(Some(
                        v8::Boolean::new(scope, !before_start && !after_end).into(),
                    ));
                }
                let comparison = if before_start {
                    -1
                } else if after_end {
                    1
                } else {
                    0
                };
                return Ok(Some(v8::Integer::new(scope, comparison).into()));
            }
            "intersectsNode" => {
                let node = argument_node(scope, state, args.get(0))?;
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let dom = state.dom.borrow();
                let intersects = if range.start_container == range.end_container {
                    direct_child_offset(&dom, range.start_container, node)?.is_some_and(|offset| {
                        offset >= range.start_offset && offset < range.end_offset
                    }) || node == range.start_container
                } else {
                    let position = traversal_position(&dom, node)?;
                    let start = traversal_position(&dom, range.start_container)?;
                    let end = traversal_position(&dom, range.end_container)?;
                    position >= start && position <= end
                };
                return Ok(Some(v8::Boolean::new(scope, intersects).into()));
            }
            "toString" => {
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                let text = range_text(&state.dom.borrow(), &range)?;
                return Ok(Some(v8_string(scope, &text)?));
            }
            "createContextualFragment" => {
                let markup = required_string(scope, args, 0)?;
                let parsed = oryn_dom::html::parse_fragment(&markup, "div");
                let parsed_children = parsed.dom.get(parsed.document)?.children.clone();
                let mut dom = state.dom.borrow_mut();
                let fragment = dom.create_fragment();
                for child in parsed_children {
                    let child = dom.import_subtree(&parsed.dom, child)?;
                    dom.append_child(fragment, child)?;
                }
                drop(dom);
                return Ok(Some(wrap_node(scope, state, fragment)?.into()));
            }
            _ => {}
        }
    }
    if interface == "Selection" {
        match member {
            "getRangeAt" => {
                let index = args.get(0).uint32_value(scope).unwrap_or(u32::MAX);
                if index != 0 {
                    return Err(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "IndexSizeError".into(),
                    });
                }
                let id = state
                    .selection_range
                    .get()
                    .ok_or(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "IndexSizeError".into(),
                    })?;
                return Ok(Some(new_stateful_object(scope, state, "Range", id)?.into()));
            }
            "addRange" => {
                let id = stateful_id(
                    scope,
                    v8::Local::<v8::Object>::try_from(args.get(0))
                        .map_err(|_| V8BindingError::Allocation("Range argument"))?,
                )?;
                let range = state
                    .ranges
                    .borrow()
                    .get(&id)
                    .cloned()
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                state.selection_range.set(Some(id));
                state.selection_anchor.set(Some(BoundaryPoint {
                    container: range.start_container,
                    offset: range.start_offset,
                }));
                state.selection_focus.set(Some(BoundaryPoint {
                    container: range.end_container,
                    offset: range.end_offset,
                }));
                return Ok(None);
            }
            "removeRange" => {
                let range_id = stateful_id(
                    scope,
                    v8::Local::<v8::Object>::try_from(args.get(0))
                        .map_err(|_| V8BindingError::Allocation("Range argument"))?,
                )?;
                if state.selection_range.get() != Some(range_id) {
                    return Err(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "NotFoundError".into(),
                    });
                }
                clear_selection(state);
                return Ok(None);
            }
            "removeAllRanges" | "empty" => {
                clear_selection(state);
                return Ok(None);
            }
            "deleteFromDocument" => {
                if let Some(id) = state.selection_range.get() {
                    let range = state
                        .ranges
                        .borrow()
                        .get(&id)
                        .cloned()
                        .ok_or(V8BindingError::Allocation("Range record"))?;
                    range_contents(
                        &mut state.dom.borrow_mut(),
                        &range,
                        RangeContentsMode::Delete,
                    )?;
                    let mut ranges = state.ranges.borrow_mut();
                    let range = ranges
                        .get_mut(&id)
                        .ok_or(V8BindingError::Allocation("Range record"))?;
                    range.end_container = range.start_container;
                    range.end_offset = range.start_offset;
                    drop(ranges);
                    persist_live_range(state, id)?;
                    let collapsed = state.ranges.borrow()[&id].start_container;
                    let offset = state.ranges.borrow()[&id].start_offset;
                    let point = BoundaryPoint {
                        container: collapsed,
                        offset,
                    };
                    state.selection_anchor.set(Some(point));
                    state.selection_focus.set(Some(point));
                }
                return Ok(None);
            }
            "collapse" | "setPosition" | "selectAllChildren" => {
                if args.get(0).is_null_or_undefined() {
                    clear_selection(state);
                    return Ok(None);
                }
                let node = argument_node(scope, state, args.get(0))?;
                let offset = if member == "selectAllChildren" {
                    boundary_length(&state.dom.borrow(), node)?
                } else {
                    args.get(1).uint32_value(scope).unwrap_or(0)
                };
                validate_boundary_offset(&state.dom.borrow(), node, offset)?;
                let anchor = BoundaryPoint {
                    container: node,
                    offset: if member == "selectAllChildren" {
                        0
                    } else {
                        offset
                    },
                };
                let focus = BoundaryPoint {
                    container: node,
                    offset,
                };
                set_selection_boundaries(state, anchor, focus)?;
                return Ok(None);
            }
            "extend" => {
                let anchor = state
                    .selection_anchor
                    .get()
                    .ok_or(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "InvalidStateError".into(),
                    })?;
                let node = argument_node(scope, state, args.get(0))?;
                let focus = BoundaryPoint {
                    container: node,
                    offset: args.get(1).uint32_value(scope).unwrap_or(0),
                };
                validate_boundary_offset(&state.dom.borrow(), focus.container, focus.offset)?;
                set_selection_boundaries(state, anchor, focus)?;
                return Ok(None);
            }
            "setBaseAndExtent" => {
                let anchor = BoundaryPoint {
                    container: argument_node(scope, state, args.get(0))?,
                    offset: args.get(1).uint32_value(scope).unwrap_or(0),
                };
                let focus = BoundaryPoint {
                    container: argument_node(scope, state, args.get(2))?,
                    offset: args.get(3).uint32_value(scope).unwrap_or(0),
                };
                validate_boundary_offset(&state.dom.borrow(), anchor.container, anchor.offset)?;
                validate_boundary_offset(&state.dom.borrow(), focus.container, focus.offset)?;
                set_selection_boundaries(state, anchor, focus)?;
                return Ok(None);
            }
            "modify" => {
                let alter = if args.get(0).is_undefined() {
                    "move".into()
                } else {
                    required_string(scope, args, 0)?.to_ascii_lowercase()
                };
                let direction = if args.get(1).is_undefined() {
                    "forward".into()
                } else {
                    required_string(scope, args, 1)?.to_ascii_lowercase()
                };
                let granularity = if args.get(2).is_undefined() {
                    "character".into()
                } else {
                    required_string(scope, args, 2)?.to_ascii_lowercase()
                };
                if granularity != "character" {
                    return Err(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: format!("modify granularity {granularity}"),
                    });
                }
                let anchor = state
                    .selection_anchor
                    .get()
                    .ok_or(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "InvalidStateError".into(),
                    })?;
                let mut focus = state.selection_focus.get().unwrap_or(anchor);
                let maximum = boundary_length(&state.dom.borrow(), focus.container)?;
                if matches!(direction.as_str(), "backward" | "left") {
                    focus.offset = focus.offset.saturating_sub(1);
                } else {
                    focus.offset = focus.offset.saturating_add(1).min(maximum);
                }
                let anchor = if alter == "move" { focus } else { anchor };
                set_selection_boundaries(state, anchor, focus)?;
                return Ok(None);
            }
            "collapseToStart" | "collapseToEnd" => {
                let id = state
                    .selection_range
                    .get()
                    .ok_or(V8BindingError::Unsupported {
                        interface: interface.into(),
                        member: "InvalidStateError".into(),
                    })?;
                let mut ranges = state.ranges.borrow_mut();
                let range = ranges
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("Range record"))?;
                if member == "collapseToStart" {
                    range.end_container = range.start_container;
                    range.end_offset = range.start_offset;
                } else {
                    range.start_container = range.end_container;
                    range.start_offset = range.end_offset;
                }
                drop(ranges);
                persist_live_range(state, id)?;
                let point = state.ranges.borrow()[&id].start_container;
                let offset = state.ranges.borrow()[&id].start_offset;
                let point = BoundaryPoint {
                    container: point,
                    offset,
                };
                state.selection_anchor.set(Some(point));
                state.selection_focus.set(Some(point));
                return Ok(None);
            }
            "containsNode" => {
                let node = argument_node(scope, state, args.get(0))?;
                let Some(range) = state
                    .selection_range
                    .get()
                    .and_then(|id| state.ranges.borrow().get(&id).cloned())
                else {
                    return Ok(Some(v8::Boolean::new(scope, false).into()));
                };
                let dom = state.dom.borrow();
                if tree_root(&dom, node)? != tree_root(&dom, range.start_container)? {
                    return Ok(Some(v8::Boolean::new(scope, false).into()));
                }
                let allow_partial = args.get(1).boolean_value(scope);
                let before = relative_boundary(&dom, node, false).ok();
                let after = relative_boundary(&dom, node, true).ok();
                let contained = match (before, after) {
                    (Some(before), Some(after)) => {
                        let starts_before_end = compare_boundary_points(
                            &dom,
                            before,
                            (range.end_container, range.end_offset),
                        )? < 0;
                        let ends_after_start = compare_boundary_points(
                            &dom,
                            after,
                            (range.start_container, range.start_offset),
                        )? > 0;
                        if allow_partial {
                            starts_before_end && ends_after_start
                        } else {
                            compare_boundary_points(
                                &dom,
                                (range.start_container, range.start_offset),
                                before,
                            )? <= 0
                                && compare_boundary_points(
                                    &dom,
                                    after,
                                    (range.end_container, range.end_offset),
                                )? <= 0
                        }
                    }
                    _ => node == range.start_container || node == range.end_container,
                };
                return Ok(Some(v8::Boolean::new(scope, contained).into()));
            }
            "toString" => {
                let text = state
                    .selection_range
                    .get()
                    .and_then(|id| state.ranges.borrow().get(&id).cloned())
                    .map(|range| range_text(&state.dom.borrow(), &range).unwrap_or_default())
                    .unwrap_or_default();
                return Ok(Some(v8_string(scope, &text)?));
            }
            _ => {}
        }
    }
    if matches!(interface, "NodeIterator" | "TreeWalker") {
        let id = stateful_id(scope, object)?;
        let mut traversals = state.traversals.borrow_mut();
        let traversal = traversals
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("traversal record"))?;
        match member {
            "nextNode" => {
                if interface == "NodeIterator" && traversal.pointer_before {
                    traversal.pointer_before = false;
                    if traversal
                        .accepted
                        .contains(&traversal.nodes[traversal.index])
                    {
                        return Ok(Some(
                            wrap_node(scope, state, traversal.nodes[traversal.index])?.into(),
                        ));
                    }
                }
                let Some(index) = ((traversal.index + 1)..traversal.nodes.len())
                    .find(|index| traversal.accepted.contains(&traversal.nodes[*index]))
                else {
                    return Ok(Some(v8::null(scope).into()));
                };
                traversal.index = index;
                return Ok(Some(
                    wrap_node(scope, state, traversal.nodes[traversal.index])?.into(),
                ));
            }
            "previousNode" => {
                if interface == "NodeIterator" && traversal.pointer_before {
                    return Ok(Some(v8::null(scope).into()));
                }
                let Some(index) = (0..traversal.index)
                    .rev()
                    .find(|index| traversal.accepted.contains(&traversal.nodes[*index]))
                else {
                    traversal.pointer_before = interface == "NodeIterator";
                    return Ok(Some(v8::null(scope).into()));
                };
                traversal.index = index;
                return Ok(Some(
                    wrap_node(scope, state, traversal.nodes[traversal.index])?.into(),
                ));
            }
            "parentNode" if interface == "TreeWalker" => {
                let current = traversal.nodes[traversal.index];
                let parent = {
                    let dom = state.dom.borrow();
                    dom.parent(current)?
                };
                let Some(mut parent) = parent else {
                    return Ok(Some(v8::null(scope).into()));
                };
                while parent != traversal.root && !traversal.accepted.contains(&parent) {
                    let Some(next) = state.dom.borrow().parent(parent)? else {
                        return Ok(Some(v8::null(scope).into()));
                    };
                    parent = next;
                }
                if let Some(index) = traversal.nodes.iter().position(|node| *node == parent) {
                    traversal.index = index;
                } else {
                    return Ok(Some(v8::null(scope).into()));
                }
                return Ok(Some(wrap_node(scope, state, parent)?.into()));
            }
            "firstChild" | "lastChild" if interface == "TreeWalker" => {
                let current = traversal.nodes[traversal.index];
                let child = {
                    let dom = state.dom.borrow();
                    let children = dom.children(current)?;
                    if member == "firstChild" {
                        children.first().copied()
                    } else {
                        children.last().copied()
                    }
                };
                let Some(child) = child else {
                    return Ok(Some(v8::null(scope).into()));
                };
                if let Some(index) = traversal
                    .nodes
                    .iter()
                    .position(|node| *node == child && traversal.accepted.contains(node))
                {
                    traversal.index = index;
                } else {
                    return Ok(Some(v8::null(scope).into()));
                }
                return Ok(Some(wrap_node(scope, state, child)?.into()));
            }
            "previousSibling" | "nextSibling" if interface == "TreeWalker" => {
                let current = traversal.nodes[traversal.index];
                let sibling = {
                    let dom = state.dom.borrow();
                    dom.parent(current)?.and_then(|parent| {
                        let children = dom.children(parent).ok()?;
                        let position = children.iter().position(|node| *node == current)?;
                        if member == "previousSibling" {
                            position
                                .checked_sub(1)
                                .and_then(|index| children.get(index).copied())
                        } else {
                            children.get(position + 1).copied()
                        }
                    })
                };
                let Some(sibling) = sibling else {
                    return Ok(Some(v8::null(scope).into()));
                };
                if let Some(index) = traversal
                    .nodes
                    .iter()
                    .position(|node| *node == sibling && traversal.accepted.contains(node))
                {
                    traversal.index = index;
                } else {
                    return Ok(Some(v8::null(scope).into()));
                }
                return Ok(Some(wrap_node(scope, state, sibling)?.into()));
            }
            "detach" => return Ok(None),
            _ => {}
        }
    }
    Err(V8BindingError::Unsupported {
        interface: interface.into(),
        member: member.into(),
    })
}

fn resolve_receiver(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<NodeId, V8BindingError> {
    let wrapper = receiver_wrapper_id(scope, object)
        .ok_or(BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    let brand = brand_for_interface(interface);
    let node = state.wrappers.borrow().resolve(
        &state.dom.borrow(),
        wrapper,
        state.realm,
        state.document_generation,
        brand,
    )?;
    validate_interface_node(&state.dom.borrow(), node, interface)?;
    Ok(node)
}

fn brand_for_interface(interface: &str) -> InterfaceBrand {
    match interface {
        "Document" => InterfaceBrand::Document,
        "Element" | "SVGElement" => InterfaceBrand::Element,
        interface if interface == "HTMLElement" || interface.starts_with("HTML") => {
            InterfaceBrand::HtmlElement
        }
        _ => InterfaceBrand::Node,
    }
}

fn validate_interface_node(
    dom: &DomRuntime,
    node: NodeId,
    interface: &str,
) -> Result<(), V8BindingError> {
    let expected_tag = match interface {
        "HTMLInputElement" => Some("input"),
        "HTMLTextAreaElement" => Some("textarea"),
        "HTMLSelectElement" => Some("select"),
        "HTMLOptionElement" => Some("option"),
        "HTMLButtonElement" => Some("button"),
        "HTMLFormElement" => Some("form"),
        "HTMLAnchorElement" => Some("a"),
        "HTMLCanvasElement" => Some("canvas"),
        _ => None,
    };
    if let Some(expected_tag) = expected_tag {
        let NodeKind::Element { name } = &dom.arena().get(node)?.kind else {
            return Err(BindingError::WrongBrand {
                expected: InterfaceBrand::HtmlElement,
                actual: InterfaceBrand::Node,
            }
            .into());
        };
        if name.local.as_ref() != expected_tag {
            return Err(BindingError::WrongBrand {
                expected: InterfaceBrand::HtmlElement,
                actual: InterfaceBrand::HtmlElement,
            }
            .into());
        }
    }
    Ok(())
}

fn most_derived_brand(kind: &NodeKind) -> InterfaceBrand {
    match kind {
        NodeKind::Document => InterfaceBrand::Document,
        NodeKind::Element { .. } => InterfaceBrand::HtmlElement,
        _ => InterfaceBrand::Node,
    }
}

fn interface_for_node(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Document => "Document",
        NodeKind::Attribute { .. } => "Attr",
        NodeKind::Element { name }
            if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "a" =>
        {
            "SVGAElement"
        }
        NodeKind::Element { name } if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" => {
            "SVGElement"
        }
        NodeKind::Element { name } => match name.local.as_ref() {
            "template" => "HTMLTemplateElement",
            "audio" | "video" => "HTMLMediaElement",
            "canvas" => "HTMLCanvasElement",
            "input" => "HTMLInputElement",
            "textarea" => "HTMLTextAreaElement",
            "select" => "HTMLSelectElement",
            "option" => "HTMLOptionElement",
            "button" => "HTMLButtonElement",
            "form" => "HTMLFormElement",
            "a" => "HTMLAnchorElement",
            "iframe" => "HTMLIFrameElement",
            _ => "HTMLElement",
        },
        NodeKind::Text { .. } => "Text",
        NodeKind::CDataSection { .. } => "CDATASection",
        NodeKind::ProcessingInstruction { .. } => "ProcessingInstruction",
        NodeKind::Comment { .. } => "Comment",
        NodeKind::DocumentFragment => "DocumentFragment",
        NodeKind::ShadowRoot { .. } => "ShadowRoot",
        NodeKind::DocumentType { .. } => "DocumentType",
    }
}

/// Return the realm-identity-preserving V8 wrapper for a Rust DOM node.
///
/// Hosts use this only to seed task-local globals before invoking generated
/// interface behavior; page code never receives a Rust pointer.
pub fn wrap_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let kind = state.dom.borrow().arena().get(node)?.kind.clone();
    let brand = most_derived_brand(&kind);
    let wrapper = {
        let mut dom = state.dom.borrow_mut();
        state.wrappers.borrow_mut().get_or_create(
            &mut dom,
            state.realm,
            state.document_generation,
            node,
            brand,
        )?
    };
    if let Some(object) = state.objects.borrow().get(&wrapper)
        && let Some(local) = object.handle.to_local(scope)
    {
        return Ok(local);
    }
    let interface = interface_for_node(&kind);
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .cloned()
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let template = v8::Local::new(scope, &template);
    let object = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("wrapper object"))?;
    let id = v8::BigInt::new_from_u64(scope, wrapper.0);
    object.set_internal_field(0, id.into());
    set_dom_wrapper_identity(scope, object, node, state.realm, state.document_generation);
    // Inline event-handler content attributes are reflected as properties on
    // the Rust-backed wrapper. Event dispatch already invokes the `on*`
    // property, which gives parser-created and fragment-created elements the
    // same receiver, event argument, and cancellation semantics as handlers
    // assigned from JavaScript. The Rust DOM remains the attribute authority;
    // this is only the realm-local compiled callback.
    let inline_handlers = state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .attributes
        .iter()
        .filter(|(name, _)| name.starts_with("on"))
        .map(|(name, source)| (name.clone(), source.clone()))
        .collect::<Vec<_>>();
    for (name, source) in inline_handlers {
        let function_source = format!("(function(event){{{source}}})");
        let function_source = v8::String::new(scope, &function_source)
            .ok_or(V8BindingError::Allocation("inline event handler"))?;
        let function = v8::Script::compile(scope, function_source, None)
            .and_then(|script| script.run(scope))
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| {
                V8BindingError::EventCallback("inline event handler compilation failed".into())
            })?;
        let key = v8::String::new(scope, &name)
            .ok_or(V8BindingError::Allocation("inline event property"))?;
        if !object
            .set(scope, key.into(), function.into())
            .unwrap_or(false)
        {
            return Err(V8BindingError::Allocation("inline event property"));
        }
    }
    let registry: RcWeak<RefCell<WrapperRegistry>> = Rc::downgrade(&state.wrappers);
    let weak = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_isolate| {
            if let Some(registry) = registry.upgrade() {
                registry.borrow_mut().enqueue_weak_release(wrapper);
            }
        }),
    );
    let connected_root = state
        .dom
        .borrow()
        .is_connected(node)?
        .then(|| v8::Global::new(scope, object));
    state.objects.borrow_mut().insert(
        wrapper,
        WrapperObject {
            handle: weak,
            connected_root,
        },
    );
    Ok(object)
}

fn set_dom_wrapper_identity(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    node: NodeId,
    realm: RealmId,
    document_generation: u64,
) {
    object.set_internal_field(1, v8::Integer::new_from_unsigned(scope, node.slot).into());
    object.set_internal_field(
        2,
        v8::Integer::new_from_unsigned(scope, node.generation).into(),
    );
    object.set_internal_field(3, v8::Integer::new_from_unsigned(scope, realm.0).into());
    object.set_internal_field(
        4,
        v8::BigInt::new_from_u64(scope, document_generation).into(),
    );
}

fn embedded_node_identity(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Option<(NodeId, RealmId, u64)> {
    if object.internal_field_count() < 5 {
        return None;
    }
    let wrapper = object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())?;
    if !wrapper.is_big_int() {
        return None;
    }
    let field_value = |index| {
        object
            .get_internal_field(scope, index)
            .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
    };
    let integer = |index| field_value(index)?.integer_value(scope);
    let unsigned = |index| {
        let value = field_value(index)?;
        if let Ok(value) = v8::Local::<v8::BigInt>::try_from(value) {
            let (value, lossless) = value.u64_value();
            return lossless.then_some(value);
        }
        u64::try_from(value.integer_value(scope)?).ok()
    };
    let slot = u32::try_from(integer(1)?).ok()?;
    let generation = u32::try_from(integer(2)?).ok()?;
    let realm = u32::try_from(integer(3)?).ok()?;
    let document_generation = unsigned(4)?;
    Some((
        NodeId { slot, generation },
        RealmId(realm),
        document_generation,
    ))
}

fn receiver_wrapper_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Option<WrapperId> {
    receiver_wrapper_id_at_depth(scope, object, 0)
}

fn receiver_wrapper_id_at_depth(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    depth: u8,
) -> Option<WrapperId> {
    // Framework reactivity layers can place transparent JavaScript Proxies
    // around host objects. Web-platform receivers retain the target's DOM
    // identity in that case; never treat the Proxy object itself as a wrapper
    // and never invoke its traps a second time while performing the brand
    // check. Bound the chain so adversarial nested proxies cannot turn a DOM
    // access into unbounded host work.
    if object.internal_field_count() == 0 {
        if depth >= 8 || !object.is_proxy() {
            return None;
        }
        let proxy = v8::Local::<v8::Proxy>::try_from(v8::Local::<v8::Value>::from(object)).ok()?;
        if proxy.is_revoked() {
            return None;
        }
        let target = v8::Local::<v8::Object>::try_from(proxy.get_target(scope)).ok()?;
        return receiver_wrapper_id_at_depth(scope, target, depth + 1);
    }
    let field = object.get_internal_field(scope, 0)?;
    let value = v8::Local::<v8::Value>::try_from(field).ok()?;
    exact_wrapper_id(value)
}

fn exact_wrapper_id(value: v8::Local<v8::Value>) -> Option<WrapperId> {
    let bigint = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = bigint.u64_value();
    lossless.then_some(WrapperId(value))
}

fn mutation_observer_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<u64, V8BindingError> {
    object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value as u64)
        .ok_or(V8BindingError::Allocation("MutationObserver identity"))
}

fn call_mutation_observer_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let observer_id = mutation_observer_id(scope, object)?;
    match member {
        "observe" => {
            let target = argument_node(scope, state, args.get(0))?;
            let init = args
                .get(1)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("MutationObserverInit"))?;
            let boolean = |scope: &mut v8::PinScope<'s, '_>, name: &str| {
                v8::String::new(scope, name)
                    .and_then(|key| init.get(scope, key.into()))
                    .is_some_and(|value| value.boolean_value(scope))
            };
            let options = MutationObserverOptions {
                child_list: boolean(scope, "childList"),
                attributes: boolean(scope, "attributes"),
                character_data: boolean(scope, "characterData"),
                subtree: boolean(scope, "subtree"),
            };
            if !options.child_list && !options.attributes && !options.character_data {
                return Err(V8BindingError::Unsupported {
                    interface: "MutationObserver".into(),
                    member: "observe requires childList, attributes, or characterData".into(),
                });
            }
            let mut observers = state.mutation_observers.borrow_mut();
            let observer = observers
                .get_mut(&observer_id)
                .ok_or(V8BindingError::Allocation("MutationObserver record"))?;
            observer.target = Some(target);
            observer.options = options;
            Ok(None)
        }
        "disconnect" => {
            if let Some(observer) = state.mutation_observers.borrow_mut().get_mut(&observer_id) {
                observer.target = None;
                observer.records.clear();
            }
            Ok(None)
        }
        "takeRecords" => {
            let records = state
                .mutation_observers
                .borrow_mut()
                .get_mut(&observer_id)
                .map(|observer| std::mem::take(&mut observer.records))
                .unwrap_or_default();
            Ok(Some(mutation_record_array(scope, state, &records)?.into()))
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "MutationObserver".into(),
            member: member.into(),
        }),
    }
}

fn mutation_target(mutation: &Mutation) -> Option<NodeId> {
    match mutation {
        Mutation::Inserted { parent, .. } => *parent,
        Mutation::Removed { former_parent, .. } => *former_parent,
        Mutation::Attribute { node, .. }
        | Mutation::CharacterData { node, .. }
        | Mutation::FormState { node }
        | Mutation::EventListener { node, .. } => Some(*node),
        Mutation::Focus { current, previous } => current.or(*previous),
        Mutation::ShadowAttached { host, .. } => Some(*host),
    }
}

fn observer_accepts(
    dom: &DomRuntime,
    target: NodeId,
    options: MutationObserverOptions,
    mutation: &Mutation,
) -> bool {
    let Some(mutation_target) = mutation_target(mutation) else {
        return false;
    };
    let in_scope = mutation_target == target
        || (options.subtree
            && dom
                .arena()
                .ancestors(mutation_target)
                .is_ok_and(|ancestors| ancestors.contains(&target)));
    in_scope
        && match mutation {
            Mutation::Inserted { .. }
            | Mutation::Removed { .. }
            | Mutation::ShadowAttached { .. } => options.child_list,
            Mutation::Attribute { .. } | Mutation::FormState { .. } => options.attributes,
            Mutation::CharacterData { .. } => options.character_data,
            Mutation::Focus { .. } | Mutation::EventListener { .. } => false,
        }
}

fn set_object_property(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("property name"))?;
    object.create_data_property(scope, key.into(), value);
    Ok(())
}

fn mutation_record_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    records: &[Mutation],
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let array = v8::Array::new(scope, records.len() as i32);
    for (index, mutation) in records.iter().enumerate() {
        let object = v8::Object::new(scope);
        let (kind, target, attribute, old_value, added, removed) = match mutation {
            Mutation::Inserted { node, parent } => {
                ("childList", *parent, None, None, Some(*node), None)
            }
            Mutation::Removed {
                node,
                former_parent,
            } => ("childList", *former_parent, None, None, None, Some(*node)),
            Mutation::Attribute {
                node,
                name,
                old_value,
                ..
            } => (
                "attributes",
                Some(*node),
                Some(name.as_str()),
                old_value.as_deref(),
                None,
                None,
            ),
            Mutation::CharacterData {
                node, old_value, ..
            } => (
                "characterData",
                Some(*node),
                None,
                Some(old_value.as_str()),
                None,
                None,
            ),
            Mutation::FormState { node } => {
                ("attributes", Some(*node), Some("value"), None, None, None)
            }
            Mutation::ShadowAttached { host, root, .. } => {
                ("childList", Some(*host), None, None, Some(*root), None)
            }
            Mutation::Focus { .. } | Mutation::EventListener { .. } => continue,
        };
        let kind_value = v8_string(scope, kind)?;
        set_object_property(scope, object, "type", kind_value)?;
        let target_value = match target {
            Some(node) => wrap_node(scope, state, node)?.into(),
            None => v8::null(scope).into(),
        };
        set_object_property(scope, object, "target", target_value)?;
        let attribute_value = match attribute {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        };
        set_object_property(scope, object, "attributeName", attribute_value)?;
        let old_value = match old_value {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        };
        set_object_property(scope, object, "oldValue", old_value)?;
        let added_nodes = added.map_or_else(Vec::new, |node| vec![node]);
        let removed_nodes = removed.map_or_else(Vec::new, |node| vec![node]);
        let added_value = node_array(scope, state, &added_nodes)?;
        set_object_property(scope, object, "addedNodes", added_value)?;
        let removed_value = node_array(scope, state, &removed_nodes)?;
        set_object_property(scope, object, "removedNodes", removed_value)?;
        array.set_index(scope, index as u32, object.into());
    }
    Ok(array)
}

/// Queue and deliver one stable MutationObserver checkpoint for this realm.
pub fn deliver_mutation_observers(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    mutations: &[Mutation],
) -> Result<usize, V8BindingError> {
    {
        let dom = state.dom.borrow();
        for observer in state.mutation_observers.borrow_mut().values_mut() {
            let Some(target) = observer.target else {
                continue;
            };
            observer.records.extend(
                mutations
                    .iter()
                    .filter(|mutation| observer_accepts(&dom, target, observer.options, mutation))
                    .cloned(),
            );
        }
    }
    let ids = state
        .mutation_observers
        .borrow()
        .iter()
        .filter_map(|(id, observer)| (!observer.records.is_empty()).then_some(*id))
        .collect::<Vec<_>>();
    let mut delivered = 0;
    for id in ids {
        let (callback, object, records) = {
            let mut observers = state.mutation_observers.borrow_mut();
            let observer = observers.get_mut(&id).expect("queued observer exists");
            (
                observer.callback.clone(),
                observer.object.clone(),
                std::mem::take(&mut observer.records),
            )
        };
        let callback = v8::Local::new(scope, &callback);
        let object = v8::Local::new(scope, &object);
        let array = mutation_record_array(scope, state, &records)?;
        callback.call(
            scope,
            v8::undefined(scope).into(),
            &[array.into(), object.into()],
        );
        delivered += 1;
    }
    Ok(delivered)
}

fn css_property_name(member: &str) -> &str {
    match member {
        "cssText" => "css-text",
        "backgroundClip" => "background-clip",
        "backgroundColor" => "background-color",
        "borderCollapse" => "border-collapse",
        "boxSizing" => "box-sizing",
        "cssFloat" => "float",
        "marginRight" => "margin-right",
        "maxHeight" => "max-height",
        "minHeight" => "min-height",
        "MozBoxSizing" => "-moz-box-sizing",
        "WebkitBoxSizing" => "-webkit-box-sizing",
        _ => member,
    }
}

fn inline_style_entries(style: &str) -> BTreeMap<String, String> {
    style
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

fn set_inline_style_property(
    dom: &mut DomRuntime,
    node: NodeId,
    property: &str,
    value: &str,
) -> Result<String, DomError> {
    let current = dom
        .arena()
        .get(node)?
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default();
    let mut entries = inline_style_entries(&current);
    let previous = if value.is_empty() {
        entries.remove(property).unwrap_or_default()
    } else {
        entries
            .insert(property.to_ascii_lowercase(), value.to_string())
            .unwrap_or_default()
    };
    let serialized = entries
        .into_iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("; ");
    dom.set_attribute(node, "style", serialized)?;
    Ok(previous)
}

fn template_content(
    state: &Rc<V8BindingState>,
    template: NodeId,
) -> Result<NodeId, V8BindingError> {
    if let Some(content) = state.template_contents.borrow().get(&template).copied() {
        return Ok(content);
    }
    let content = {
        let mut dom = state.dom.borrow_mut();
        let content = dom.create_fragment();
        let children = dom.arena().get(template)?.children.clone();
        for child in children {
            dom.remove_child(template, child)?;
            dom.append_child(content, child)?;
        }
        content
    };
    state
        .template_contents
        .borrow_mut()
        .insert(template, content);
    Ok(content)
}

fn get_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if interface == "HTMLElement" && member.starts_with("on") {
        return Ok(state
            .event_handlers
            .borrow()
            .get(&(node, member.to_string()))
            .map(|handler| v8::Local::new(scope, handler))
            .unwrap_or_else(|| v8::null(scope).into()));
    }
    if interface == "HTMLTemplateElement" && member == "content" {
        let content = template_content(state, node)?;
        return Ok(wrap_node(scope, state, content)?.into());
    }
    if interface == "Element" && member == "classList" {
        return associated_wrapper(scope, state, node, "DOMTokenList");
    }
    if interface == "HTMLElement" && member == "style" {
        return associated_wrapper(scope, state, node, "CSSStyleDeclaration");
    }
    if interface == "HTMLElement" && member == "dataset" {
        let entries = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .attributes
            .iter()
            .filter_map(|(name, value)| {
                let suffix = name.strip_prefix("data-")?;
                let mut property = String::new();
                let mut uppercase = false;
                for character in suffix.chars() {
                    if character == '-' {
                        uppercase = true;
                    } else if uppercase {
                        property.extend(character.to_uppercase());
                        uppercase = false;
                    } else {
                        property.push(character);
                    }
                }
                Some((property, value.clone()))
            })
            .collect::<Vec<_>>();
        let dataset = v8::Object::new(scope);
        for (name, value) in entries {
            let name = v8::String::new(scope, &name)
                .ok_or(V8BindingError::Allocation("dataset property"))?;
            let value = v8_string(scope, &value)?;
            if !dataset.set(scope, name.into(), value).unwrap_or(false) {
                return Err(V8BindingError::Allocation("dataset property"));
            }
        }
        return Ok(dataset.into());
    }
    if interface == "Element" && member == "attributes" {
        let entries = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .attributes
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>();
        let attributes = v8::Array::new(scope, entries.len() as i32);
        for (index, (name, value)) in entries.into_iter().enumerate() {
            let attribute = v8::Object::new(scope);
            let property_name = v8_string(scope, &name)?;
            set_object_property(scope, attribute, "name", property_name)?;
            let value = v8_string(scope, &value)?;
            set_object_property(scope, attribute, "value", value)?;
            set_object_property(
                scope,
                attribute,
                "expando",
                v8::Boolean::new(scope, false).into(),
            )?;
            attributes.set_index(scope, index as u32, attribute.into());
            attributes.set(scope, property_name, attribute.into());
        }
        return Ok(attributes.into());
    }
    if interface == "Document" && member == "styleSheets" {
        let styles = state.dom.borrow().query_selector_all(node, "style")?;
        return associated_node_array(scope, state, &styles, "CSSStyleSheet");
    }
    if interface == "Document" && member == "defaultView" {
        return Ok(scope.get_current_context().global(scope).into());
    }
    if interface == "Document" && matches!(member, "URL" | "baseURI") {
        let global = scope.get_current_context().global(scope);
        let location_key =
            v8::String::new(scope, "location").ok_or(V8BindingError::Allocation("location key"))?;
        let location = global
            .get(scope, location_key.into())
            .and_then(|value| value.to_object(scope))
            .ok_or(V8BindingError::Allocation("location object"))?;
        let href_key =
            v8::String::new(scope, "href").ok_or(V8BindingError::Allocation("href key"))?;
        return location
            .get(scope, href_key.into())
            .ok_or(V8BindingError::Allocation("location href"));
    }
    if interface == "Document" && member == "currentScript" {
        return optional_node(scope, state, state.current_script.get());
    }
    if interface == "Document" && member == "activeElement" {
        let focused = state.dom.borrow().focused();
        return optional_node(scope, state, focused);
    }
    if interface == "Document" && member == "designMode" {
        return v8_string(scope, if state.design_mode.get() { "on" } else { "off" });
    }
    if interface == "HTMLSelectElement" && member == "options" {
        let options = state.dom.borrow().query_selector_all(node, "option")?;
        return node_array(scope, state, &options);
    }
    if interface == "HTMLIFrameElement" && member == "contentDocument" {
        let document = state.frame_roots.borrow().get(&node).copied();
        return optional_node(scope, state, document);
    }
    if interface == "HTMLIFrameElement" && member == "contentWindow" {
        return Ok(state
            .frame_windows
            .borrow()
            .get(&node)
            .map(|window| v8::Local::new(scope, window).into())
            .unwrap_or_else(|| v8::null(scope).into()));
    }
    if matches!(
        member,
        "parentNode"
            | "parentElement"
            | "firstChild"
            | "lastChild"
            | "previousSibling"
            | "nextSibling"
            | "firstElementChild"
            | "lastElementChild"
            | "previousElementSibling"
            | "nextElementSibling"
    ) {
        let related = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            let node_is_shadow_root = matches!(record.kind, NodeKind::ShadowRoot { .. });
            match member {
                "parentNode" if node_is_shadow_root => None,
                "parentNode" => record.parent,
                "parentElement" if node_is_shadow_root => None,
                "parentElement" => record.parent.filter(|parent| {
                    matches!(
                        dom.arena().get(*parent).ok().map(|record| &record.kind),
                        Some(NodeKind::Element { .. })
                    )
                }),
                "firstChild" => light_children(&dom, node)?.first().copied(),
                "lastChild" => light_children(&dom, node)?.last().copied(),
                "firstElementChild" => record.children.iter().copied().find(|child| {
                    matches!(
                        dom.arena().get(*child).ok().map(|record| &record.kind),
                        Some(NodeKind::Element { .. })
                    )
                }),
                "lastElementChild" => record.children.iter().rev().copied().find(|child| {
                    matches!(
                        dom.arena().get(*child).ok().map(|record| &record.kind),
                        Some(NodeKind::Element { .. })
                    )
                }),
                "previousSibling" | "nextSibling" => record.parent.and_then(|parent| {
                    let siblings = light_children(&dom, parent).ok()?;
                    let index = siblings.iter().position(|candidate| *candidate == node)?;
                    if member == "previousSibling" {
                        index
                            .checked_sub(1)
                            .and_then(|index| siblings.get(index))
                            .copied()
                    } else {
                        siblings.get(index + 1).copied()
                    }
                }),
                "previousElementSibling" | "nextElementSibling" => {
                    record.parent.and_then(|parent| {
                        let siblings = light_children(&dom, parent).ok()?;
                        let index = siblings.iter().position(|candidate| *candidate == node)?;
                        let candidates: Box<dyn Iterator<Item = &NodeId>> =
                            if member == "previousElementSibling" {
                                Box::new(siblings[..index].iter().rev())
                            } else {
                                Box::new(siblings[index + 1..].iter())
                            };
                        candidates.copied().find(|candidate| {
                            matches!(
                                dom.arena().get(*candidate).ok().map(|record| &record.kind),
                                Some(NodeKind::Element { .. })
                            )
                        })
                    })
                }
                _ => unreachable!(),
            }
        };
        return optional_node(scope, state, related);
    }
    if member == "childNodes" {
        let children = light_children(&state.dom.borrow(), node)?;
        return node_array(scope, state, &children);
    }
    if interface == "Element" && member == "children" {
        let children = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .children
            .iter()
            .copied()
            .filter(|child| {
                matches!(
                    state
                        .dom
                        .borrow()
                        .arena()
                        .get(*child)
                        .ok()
                        .map(|record| &record.kind),
                    Some(NodeKind::Element { .. })
                )
            })
            .collect::<Vec<_>>();
        return node_array(scope, state, &children);
    }
    if interface == "Document" && member == "documentElement" {
        let document_element = state.dom.borrow().first_element_child(node)?;
        return optional_node(scope, state, document_element);
    }
    if interface == "Document" && member == "implementation" {
        if let Some(object) = state.implementation_object.borrow().as_ref() {
            return Ok(v8::Local::new(scope, object).into());
        }
        let object = new_stateful_object(scope, state, "DOMImplementation", 1)?;
        state
            .implementation_object
            .borrow_mut()
            .replace(v8::Global::new(scope, object));
        return Ok(object.into());
    }
    if interface == "Document" && matches!(member, "head" | "body") {
        let element = state.dom.borrow().query_selector(node, member)?;
        return optional_node(scope, state, element);
    }
    if interface == "Document" && member == "doctype" {
        let doctype = state
            .dom
            .borrow()
            .children(node)?
            .iter()
            .copied()
            .find(|child| {
                matches!(
                    state
                        .dom
                        .borrow()
                        .arena()
                        .get(*child)
                        .map(|record| &record.kind),
                    Ok(NodeKind::DocumentType { .. })
                )
            });
        return optional_node(scope, state, doctype);
    }
    if interface == "Node" && member == "ownerDocument" {
        let owner = {
            let dom = state.dom.borrow();
            if matches!(dom.arena().get(node)?.kind, NodeKind::Document) {
                None
            } else {
                if let Some(owner) = state.node_documents.borrow().get(&node).copied() {
                    Some(owner)
                } else {
                    dom.arena()
                        .ancestors(node)?
                        .into_iter()
                        .find(|ancestor| {
                            matches!(
                                dom.arena().get(*ancestor).map(|record| &record.kind),
                                Ok(NodeKind::Document)
                            )
                        })
                        .or(Some(dom.document()))
                }
            }
        };
        return optional_node(scope, state, owner);
    }
    if interface == "Element" && member == "shadowRoot" {
        let root = {
            let dom = state.dom.borrow();
            dom.arena()
                .get(node)?
                .children
                .iter()
                .copied()
                .find(|child| {
                    matches!(
                        dom.arena().get(*child).ok().map(|record| &record.kind),
                        Some(NodeKind::ShadowRoot {
                            mode: ShadowMode::Open,
                            ..
                        })
                    )
                })
        };
        return optional_node(scope, state, root);
    }
    if interface == "ShadowRoot" && member == "host" {
        let host = state.dom.borrow().arena().get(node)?.parent;
        return optional_node(scope, state, host);
    }
    let template_inner_html = if interface == "Element"
        && member == "innerHTML"
        && state
            .dom
            .borrow()
            .node_name(node)
            .is_ok_and(|name| name.eq_ignore_ascii_case("template"))
    {
        Some(template_content(state, node)?)
    } else {
        None
    };
    let dom = state.dom.borrow();
    let record = dom.arena().get(node)?;
    let value = match (interface, member) {
        (_, "nodeName") => v8_string(scope, &dom.node_name(node)?)?,
        (_, "nodeType") => v8::Integer::new(scope, node_type(&record.kind)).into(),
        ("Node", "isConnected") => {
            let connected =
                node == dom.document() || dom.arena().ancestors(node)?.contains(&dom.document());
            v8::Boolean::new(scope, connected).into()
        }
        ("Node", "nodeValue") => match &record.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => v8_string(scope, data)?,
            NodeKind::Attribute { value, .. } => v8_string(scope, value)?,
            _ => v8::null(scope).into(),
        },
        ("Attr", "name") => match &record.kind {
            NodeKind::Attribute { name, .. } => v8_string(scope, name)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("Attr", "value") => match &record.kind {
            NodeKind::Attribute { value, .. } => v8_string(scope, value)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("Attr", "ownerElement") => {
            drop(dom);
            let owner = match &state.dom.borrow().arena().get(node)?.kind {
                NodeKind::Attribute { owner, .. } => *owner,
                _ => None,
            };
            return optional_node(scope, state, owner);
        }
        ("DocumentType", "name") => match &record.kind {
            NodeKind::DocumentType { name, .. } => v8_string(scope, name)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("DocumentType", "publicId") => match &record.kind {
            NodeKind::DocumentType { public_id, .. } => v8_string(scope, public_id)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("DocumentType", "systemId") => match &record.kind {
            NodeKind::DocumentType { system_id, .. } => v8_string(scope, system_id)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("ProcessingInstruction", "target") => match &record.kind {
            NodeKind::ProcessingInstruction { target, .. } => v8_string(scope, target)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        (_, "textContent") | ("HTMLElement", "innerText") => {
            v8_string(scope, &dom.arena().text_content(node)?)?
        }
        ("Element", "tagName") => v8_string(scope, &dom.node_name(node)?)?,
        ("Element", "localName") => match &record.kind {
            NodeKind::Element { name } => v8_string(scope, name.local.as_ref())?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("Element", "namespaceURI") => match &record.kind {
            NodeKind::Element { name } => v8_string(scope, name.ns.as_ref())?,
            _ => v8::null(scope).into(),
        },
        ("Element", "childElementCount") => {
            let count = record
                .children
                .iter()
                .filter(|child| {
                    matches!(
                        dom.arena().get(**child).ok().map(|record| &record.kind),
                        Some(NodeKind::Element { .. })
                    )
                })
                .count();
            v8::Integer::new_from_unsigned(scope, count as u32).into()
        }
        ("Element", "id") => v8_string(
            scope,
            record.attributes.get("id").map_or("", String::as_str),
        )?,
        ("Element", "className") => v8_string(
            scope,
            record.attributes.get("class").map_or("", String::as_str),
        )?,
        ("DOMTokenList", "value") => v8_string(
            scope,
            record.attributes.get("class").map_or("", String::as_str),
        )?,
        ("CSSStyleDeclaration", "cssText") => v8_string(
            scope,
            record.attributes.get("style").map_or("", String::as_str),
        )?,
        ("CSSStyleDeclaration", member) => {
            let property = css_property_name(member);
            let styles =
                inline_style_entries(record.attributes.get("style").map_or("", String::as_str));
            v8_string(scope, styles.get(property).map_or("", String::as_str))?
        }
        ("Element" | "ShadowRoot", "innerHTML") => {
            let mut html = String::new();
            let children = template_inner_html
                .map(|content| dom.arena().get(content).map(|record| &record.children))
                .transpose()?
                .unwrap_or(&record.children);
            for child in children {
                if interface == "Element"
                    && matches!(dom.arena().get(*child)?.kind, NodeKind::ShadowRoot { .. })
                {
                    continue;
                }
                html.push_str(&dom.serialize(*child)?);
            }
            v8_string(scope, &html)?
        }
        ("Element", "outerHTML") => v8_string(scope, &dom.serialize(node)?)?,
        ("ShadowRoot", "mode") => match record.kind {
            NodeKind::ShadowRoot {
                mode: ShadowMode::Open,
                ..
            } => v8_string(scope, "open")?,
            NodeKind::ShadowRoot {
                mode: ShadowMode::Closed,
                ..
            } => v8_string(scope, "closed")?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("ShadowRoot", "clonable") => match record.kind {
            NodeKind::ShadowRoot { clonable, .. } => v8::Boolean::new(scope, clonable).into(),
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("HTMLSelectElement", "value") => {
            let options = dom.query_selector_all(node, "option")?;
            let option = options
                .iter()
                .copied()
                .find(|option| {
                    dom.arena()
                        .get(*option)
                        .is_ok_and(|record| record.attributes.contains_key("selected"))
                })
                .or_else(|| options.first().copied());
            let value = option
                .and_then(|option| dom.arena().get(option).ok().map(|record| (option, record)))
                .map(|(option, record)| {
                    record
                        .attributes
                        .get("value")
                        .cloned()
                        .unwrap_or_else(|| dom.arena().text_content(option).unwrap_or_default())
                })
                .unwrap_or_default();
            v8_string(scope, &value)?
        }
        (interface, "defaultValue") if interface.starts_with("HTML") => v8_string(
            scope,
            record.attributes.get("value").map_or("", String::as_str),
        )?,
        (interface, "value") if interface.starts_with("HTML") => v8_string(
            scope,
            dom.value(node)?
                .or_else(|| record.attributes.get("value").map(String::as_str))
                .unwrap_or(""),
        )?,
        (
            interface,
            "checked" | "defaultChecked" | "selected" | "defaultSelected" | "disabled" | "readOnly"
            | "required" | "multiple",
        ) if interface.starts_with("HTML") => {
            let attribute = match member {
                "defaultChecked" => "checked",
                "defaultSelected" => "selected",
                "readOnly" => "readonly",
                _ => member,
            };
            v8::Boolean::new(scope, record.attributes.contains_key(attribute)).into()
        }
        (interface, "type" | "name" | "action" | "method" | "href" | "target" | "src")
            if interface.starts_with("HTML") =>
        {
            let fallback = match member {
                "method" => "get",
                "type" if interface == "HTMLButtonElement" => "submit",
                "type" if interface == "HTMLInputElement" => "text",
                _ => "",
            };
            v8_string(
                scope,
                record
                    .attributes
                    .get(member)
                    .map_or(fallback, String::as_str),
            )?
        }
        ("HTMLSelectElement", "selectedIndex") => {
            let options = dom.query_selector_all(node, "option")?;
            let index = options
                .iter()
                .position(|option| {
                    dom.arena()
                        .get(*option)
                        .is_ok_and(|record| record.attributes.contains_key("selected"))
                })
                .map(|index| index as i32)
                .unwrap_or_else(|| if options.is_empty() { -1 } else { 0 });
            v8::Integer::new(scope, index).into()
        }
        ("CharacterData", "data") => match &record.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => v8_string(scope, data)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("CharacterData", "length") => match &record.kind {
            NodeKind::Text { data }
            | NodeKind::CDataSection { data }
            | NodeKind::ProcessingInstruction { data, .. }
            | NodeKind::Comment { data } => {
                v8::Integer::new_from_unsigned(scope, data.encode_utf16().count() as u32).into()
            }
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("HTMLElement", "contentEditable") => v8_string(
            scope,
            record
                .attributes
                .get("contenteditable")
                .map_or("inherit", String::as_str),
        )?,
        ("HTMLElement", "isContentEditable") => {
            let mut current = Some(node);
            let mut editable = false;
            while let Some(candidate) = current {
                let candidate_record = dom.arena().get(candidate)?;
                if let Some(value) = candidate_record.attributes.get("contenteditable") {
                    editable = !value.eq_ignore_ascii_case("false");
                    break;
                }
                current = candidate_record.parent;
            }
            v8::Boolean::new(scope, editable).into()
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    };
    Ok(value)
}

fn set_attribute_value(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if interface == "Document" && member == "designMode" {
        state
            .design_mode
            .set(value.to_rust_string_lossy(scope).eq_ignore_ascii_case("on"));
        return Ok(());
    }
    if interface == "HTMLElement" && member.starts_with("on") {
        let key = (node, member.to_string());
        let present = if value.is_null_or_undefined() {
            state.event_handlers.borrow_mut().remove(&key);
            false
        } else if v8::Local::<v8::Function>::try_from(value).is_ok() {
            state
                .event_handlers
                .borrow_mut()
                .insert(key, v8::Global::new(scope, value));
            true
        } else {
            state.event_handlers.borrow_mut().remove(&key);
            false
        };
        state.dom.borrow_mut().set_event_listener_presence(
            node,
            member.trim_start_matches("on"),
            present,
        )?;
        return Ok(());
    }
    let value = if matches!(
        (interface, member),
        ("Element", "innerHTML" | "outerHTML") | ("ShadowRoot", "innerHTML")
    ) {
        trusted_string(
            scope,
            state,
            value,
            TrustedKind::Html,
            &format!("{interface}.{member}"),
        )?
    } else if matches!(member, "textContent" | "innerText")
        && state
            .dom
            .borrow()
            .node_name(node)
            .is_ok_and(|name| name.eq_ignore_ascii_case("script"))
    {
        trusted_string(
            scope,
            state,
            value,
            TrustedKind::Script,
            "HTMLScriptElement.text",
        )?
    } else {
        value.to_rust_string_lossy(scope)
    };
    let inner_html_target = if interface == "Element"
        && member == "innerHTML"
        && state
            .dom
            .borrow()
            .node_name(node)
            .is_ok_and(|name| name.eq_ignore_ascii_case("template"))
    {
        Some(template_content(state, node)?)
    } else {
        None
    };
    let mut dom = state.dom.borrow_mut();
    match (interface, member) {
        (_, "textContent") | ("HTMLElement", "innerText") => {
            dom.set_text_content(node, value)?;
            drop(dom);
            prepare_dynamic_script_subtree(scope, state, node)?;
            return Ok(());
        }
        ("Node", "nodeValue")
            if matches!(dom.arena().get(node)?.kind, NodeKind::Attribute { .. }) =>
        {
            dom.set_attribute_node_value(node, value)?;
        }
        ("Node", "nodeValue") => {
            if matches!(
                dom.arena().get(node)?.kind,
                NodeKind::Text { .. }
                    | NodeKind::CDataSection { .. }
                    | NodeKind::ProcessingInstruction { .. }
                    | NodeKind::Comment { .. }
            ) {
                dom.set_text(node, value)?;
            }
        }
        ("Element", "id") => dom.set_attribute(node, "id", value)?,
        ("Element", "className") => dom.set_attribute(node, "class", value)?,
        ("DOMTokenList", "value") => dom.set_attribute(node, "class", value)?,
        ("CSSStyleDeclaration", "cssText") => dom.set_attribute(node, "style", value)?,
        ("CSSStyleDeclaration", member) => {
            set_inline_style_property(&mut dom, node, css_property_name(member), &value)?;
        }
        ("CharacterData", "data") => dom.set_text(node, value)?,
        ("Attr", "value") => dom.set_attribute_node_value(node, value)?,
        ("HTMLElement", "contentEditable") => {
            let normalized = value.to_ascii_lowercase();
            if !matches!(
                normalized.as_str(),
                "true" | "false" | "plaintext-only" | "inherit"
            ) {
                return Err(V8BindingError::Unsupported {
                    interface: "HTMLElement".into(),
                    member: "SyntaxError".into(),
                });
            }
            if normalized == "inherit" {
                dom.remove_attribute(node, "contenteditable")?;
            } else {
                dom.set_attribute(node, "contenteditable", normalized)?;
            }
        }
        ("HTMLSelectElement", "value") => {
            let options = dom.query_selector_all(node, "option")?;
            for option in options {
                let matches = dom
                    .attribute(option, "value")?
                    .is_some_and(|candidate| candidate == value)
                    || dom
                        .arena()
                        .text_content(option)
                        .is_ok_and(|text| text.trim() == value);
                if matches {
                    dom.set_attribute(option, "selected", "")?;
                } else {
                    dom.remove_attribute(option, "selected")?;
                }
            }
        }
        (interface, "value") if interface.starts_with("HTML") => {
            dom.set_value(node, value, false)?
        }
        (interface, "defaultValue") if interface.starts_with("HTML") => {
            dom.set_attribute(node, "value", value)?
        }
        (
            interface,
            "checked" | "defaultChecked" | "selected" | "defaultSelected" | "disabled" | "readOnly"
            | "required" | "multiple",
        ) if interface.starts_with("HTML") => {
            let attribute = match member {
                "defaultChecked" => "checked",
                "defaultSelected" => "selected",
                "readOnly" => "readonly",
                _ => member,
            };
            if value == "true" {
                dom.set_attribute(node, attribute, "")?;
            } else {
                dom.remove_attribute(node, attribute)?;
            }
        }
        (interface, "type" | "name" | "action" | "method" | "href" | "target" | "src")
            if interface.starts_with("HTML") =>
        {
            dom.set_attribute(node, member, value)?;
            if interface == "HTMLIFrameElement" && member == "src" {
                drop(dom);
                prepare_dynamic_frame_subtree(state, node)?;
                return Ok(());
            }
        }
        ("HTMLSelectElement", "selectedIndex") => {
            let index = value.parse::<isize>().unwrap_or(-1);
            let options = dom.query_selector_all(node, "option")?;
            for (option_index, option) in options.into_iter().enumerate() {
                if option_index as isize == index {
                    dom.set_attribute(option, "selected", "")?;
                } else {
                    dom.remove_attribute(option, "selected")?;
                }
            }
        }
        ("Element" | "ShadowRoot", "innerHTML") => {
            let target = inner_html_target.unwrap_or(node);
            dom.set_inner_html(target, &value)?;
            drop(dom);
            // HTML fragment parsing marks script elements already-started;
            // moving the resulting subtree into a document must not execute
            // them later.
            state.mark_script_subtree_inert(target)?;
            upgrade_custom_element_subtree(scope, state, target)?;
            return Ok(());
        }
        ("Element", "outerHTML") => dom.set_outer_html(node, &value)?,
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    }
    Ok(())
}

fn upgrade_custom_element_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let registry_key = v8::String::new(scope, "customElements")
        .ok_or(V8BindingError::Allocation("customElements key"))?;
    let Some(registry) = global
        .get(scope, registry_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Ok(());
    };
    let upgrade_key = v8::String::new(scope, "upgrade")
        .ok_or(V8BindingError::Allocation("customElements.upgrade key"))?;
    let Some(upgrade) = registry
        .get(scope, upgrade_key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Ok(());
    };
    let root = wrap_node(scope, state, root)?;
    upgrade
        .call(scope, registry.into(), &[root.into()])
        .ok_or_else(|| {
            V8BindingError::EventCallback("customElements.upgrade raised an exception".into())
        })?;
    Ok(())
}

fn call_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    match (interface, member) {
        ("CharacterData", "substringData") => {
            let offset = args.get(0).uint32_value(scope).unwrap_or(0);
            let count = args.get(1).uint32_value(scope).unwrap_or(0);
            let dom = state.dom.borrow();
            let data = match &dom.arena().get(node)?.kind {
                NodeKind::Text { data }
                | NodeKind::CDataSection { data }
                | NodeKind::ProcessingInstruction { data, .. }
                | NodeKind::Comment { data } => data,
                _ => return Err(DomError::StaleNode(node).into()),
            };
            if offset as usize > data.encode_utf16().count() {
                return Err(V8BindingError::Unsupported {
                    interface: "CharacterData".into(),
                    member: "IndexSizeError".into(),
                });
            }
            Ok(Some(v8_string(
                scope,
                &utf16_slice(data, offset, offset.saturating_add(count)),
            )?))
        }
        ("CharacterData", "appendData" | "insertData" | "deleteData" | "replaceData") => {
            let current = {
                let dom = state.dom.borrow();
                match &dom.arena().get(node)?.kind {
                    NodeKind::Text { data }
                    | NodeKind::CDataSection { data }
                    | NodeKind::ProcessingInstruction { data, .. }
                    | NodeKind::Comment { data } => data.clone(),
                    _ => return Err(DomError::StaleNode(node).into()),
                }
            };
            let length = current.encode_utf16().count() as u32;
            let (offset, count, replacement) = match member {
                "appendData" => (length, 0, required_string(scope, args, 0)?),
                "insertData" => (
                    args.get(0).uint32_value(scope).unwrap_or(0),
                    0,
                    required_string(scope, args, 1)?,
                ),
                "deleteData" => (
                    args.get(0).uint32_value(scope).unwrap_or(0),
                    args.get(1).uint32_value(scope).unwrap_or(0),
                    String::new(),
                ),
                _ => (
                    args.get(0).uint32_value(scope).unwrap_or(0),
                    args.get(1).uint32_value(scope).unwrap_or(0),
                    required_string(scope, args, 2)?,
                ),
            };
            if offset > length {
                return Err(V8BindingError::Unsupported {
                    interface: "CharacterData".into(),
                    member: "IndexSizeError".into(),
                });
            }
            state
                .dom
                .borrow_mut()
                .set_text(node, utf16_replace(&current, offset, count, &replacement))?;
            Ok(None)
        }
        ("Text", "splitText") => {
            let offset = args.get(0).uint32_value(scope).unwrap_or(0);
            let (data, parent, cdata) = {
                let dom = state.dom.borrow();
                let record = dom.arena().get(node)?;
                match &record.kind {
                    NodeKind::Text { data } => (data.clone(), record.parent, false),
                    NodeKind::CDataSection { data } => (data.clone(), record.parent, true),
                    _ => return Err(DomError::StaleNode(node).into()),
                }
            };
            let length = data.encode_utf16().count() as u32;
            if offset > length {
                return Err(V8BindingError::Unsupported {
                    interface: "Text".into(),
                    member: "IndexSizeError".into(),
                });
            }
            let head = utf16_slice(&data, 0, offset);
            let tail = utf16_slice(&data, offset, length);
            let created = {
                let mut dom = state.dom.borrow_mut();
                dom.set_text(node, head)?;
                let created = if cdata {
                    dom.create_cdata_section(tail)
                } else {
                    dom.create_text(tail)
                };
                if let Some(parent) = parent {
                    let siblings = dom.children(parent)?.to_vec();
                    let index = siblings
                        .iter()
                        .position(|candidate| *candidate == node)
                        .ok_or(DomError::StaleNode(node))?;
                    dom.insert_before(parent, created, siblings.get(index + 1).copied())?;
                }
                created
            };
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("EventTarget", "addEventListener") => {
            let event_type = required_string(scope, args, 0)?;
            if args.get(1).is_null_or_undefined() {
                return Ok(None);
            }
            let callback = v8::Local::<v8::Function>::try_from(args.get(1))
                .map_err(|_| V8BindingError::Allocation("event listener callback"))?;
            state
                .listeners
                .borrow_mut()
                .entry((node, event_type.clone()))
                .or_default()
                .push(EventListener {
                    callback: v8::Global::new(scope, callback),
                });
            state
                .dom
                .borrow_mut()
                .set_event_listener_presence(node, &event_type, true)?;
            Ok(None)
        }
        ("EventTarget", "removeEventListener") => {
            let event_type = required_string(scope, args, 0)?;
            let candidate = v8::Local::<v8::Function>::try_from(args.get(1)).ok();
            let is_click = event_type.eq_ignore_ascii_case("click");
            let mut listener_map = state.listeners.borrow_mut();
            if let Some(listeners) = listener_map.get_mut(&(node, event_type.clone())) {
                listeners.retain(|listener| {
                    candidate.is_none_or(|candidate| {
                        !v8::Local::new(scope, &listener.callback).strict_equals(candidate.into())
                    })
                });
            }
            let no_click_listeners = is_click
                && listener_map
                    .get(&(node, event_type))
                    .is_none_or(Vec::is_empty);
            drop(listener_map);
            if no_click_listeners {
                state
                    .dom
                    .borrow_mut()
                    .set_event_listener_presence(node, "click", false)?;
            }
            Ok(None)
        }
        ("EventTarget", "dispatchEvent") => {
            let event = v8::Local::<v8::Object>::try_from(args.get(0))
                .map_err(|_| V8BindingError::Allocation("event object"))?;
            let accepted = dispatch_event(scope, state, node, event)?;
            Ok(Some(v8::Boolean::new(scope, accepted).into()))
        }
        ("Document", "createElement") => {
            let name = required_string(scope, args, 0)?;
            let created = state
                .dom
                .borrow_mut()
                .create_element(&name.to_ascii_lowercase());
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createElementNS") => {
            let namespace = if args.get(0).is_null_or_undefined() {
                String::new()
            } else {
                required_string(scope, args, 0)?
            };
            let name = required_string(scope, args, 1)?;
            let created = state
                .dom
                .borrow_mut()
                .create_element_ns(&namespace, &name.to_ascii_lowercase());
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createTextNode") => {
            let data = required_string(scope, args, 0)?;
            let created = state.dom.borrow_mut().create_text(data);
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createComment") => {
            let data = required_string(scope, args, 0)?;
            let created = state.dom.borrow_mut().create_comment(data);
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createCDATASection") => {
            let data = required_string(scope, args, 0)?;
            let created = state.dom.borrow_mut().create_cdata_section(data);
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createProcessingInstruction") => {
            let target = required_string(scope, args, 0)?;
            let data = required_string(scope, args, 1)?;
            let created = state
                .dom
                .borrow_mut()
                .create_processing_instruction(target, data);
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createDocumentFragment") => {
            let created = state.dom.borrow_mut().create_fragment();
            state.node_documents.borrow_mut().insert(created, node);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createEvent") => {
            let event = create_event_object(scope, state, "", false, false)?;
            Ok(Some(event.into()))
        }
        (
            "Document",
            "execCommand"
            | "queryCommandEnabled"
            | "queryCommandIndeterm"
            | "queryCommandState"
            | "queryCommandSupported"
            | "queryCommandValue",
        ) => {
            let command = required_string(scope, args, 0)?.to_ascii_lowercase();
            let supported = edit_command_supported(&command);
            if member == "queryCommandSupported" {
                return Ok(Some(v8::Boolean::new(scope, supported).into()));
            }
            if member == "queryCommandEnabled" {
                return Ok(Some(
                    v8::Boolean::new(scope, supported && document_is_editable(state)).into(),
                ));
            }
            if member == "queryCommandIndeterm" {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            }
            if member == "queryCommandState" {
                let active = command == "stylewithcss" && state.style_with_css.get();
                return Ok(Some(v8::Boolean::new(scope, active).into()));
            }
            if member == "queryCommandValue" {
                let value = if command == "stylewithcss" {
                    if state.style_with_css.get() {
                        "true"
                    } else {
                        "false"
                    }
                } else {
                    ""
                };
                return Ok(Some(v8_string(scope, value)?));
            }
            if !supported || !document_is_editable(state) {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            }
            let value = if args.get(2).is_undefined() {
                String::new()
            } else {
                args.get(2).to_rust_string_lossy(scope)
            };
            let executed = match command.as_str() {
                "stylewithcss" => {
                    state.style_with_css.set(value.eq_ignore_ascii_case("true"));
                    true
                }
                "selectall" => {
                    let root = state
                        .dom
                        .borrow()
                        .focused()
                        .or_else(|| {
                            state
                                .dom
                                .borrow()
                                .query_selector_all(node, "body")
                                .ok()
                                .and_then(|nodes| nodes.first().copied())
                        })
                        .unwrap_or(node);
                    let end = boundary_length(&state.dom.borrow(), root)?;
                    set_selection_boundaries(
                        state,
                        BoundaryPoint {
                            container: root,
                            offset: 0,
                        },
                        BoundaryPoint {
                            container: root,
                            offset: end,
                        },
                    )?;
                    true
                }
                "inserttext" => {
                    replace_selection_with_text(state, &value)?;
                    true
                }
                "inserthtml" => {
                    replace_selection_with_html(state, &value)?;
                    true
                }
                "insertlinebreak" | "insertparagraph" => {
                    replace_selection_with_text(state, "\n")?;
                    true
                }
                "delete" => {
                    delete_selection_direction(state, false)?;
                    true
                }
                "forwarddelete" => {
                    delete_selection_direction(state, true)?;
                    true
                }
                _ => false,
            };
            Ok(Some(v8::Boolean::new(scope, executed).into()))
        }
        ("Document", "createRange") => {
            let id = register_live_range(
                state,
                RangeRecord {
                    start_container: node,
                    start_offset: 0,
                    end_container: node,
                    end_offset: 0,
                },
            )?;
            Ok(Some(new_stateful_object(scope, state, "Range", id)?.into()))
        }
        ("Document", "getSelection") => {
            if let Some(object) = state.selection_object.borrow().as_ref() {
                return Ok(Some(v8::Local::new(scope, object).into()));
            }
            let object = new_stateful_object(scope, state, "Selection", 1)?;
            state
                .selection_object
                .borrow_mut()
                .replace(v8::Global::new(scope, object));
            Ok(Some(object.into()))
        }
        ("Document", "createNodeIterator" | "createTreeWalker") => {
            let root = argument_node(scope, state, args.get(0))?;
            let what_to_show = if args.get(1).is_undefined() {
                u32::MAX
            } else {
                args.get(1).uint32_value(scope).unwrap_or(u32::MAX)
            };
            let (nodes, accepted) = filtered_traversal_nodes(
                scope,
                state,
                root,
                what_to_show,
                args.get(2),
                member == "createTreeWalker",
            )?;
            let id = state.next_traversal_id.get();
            state.next_traversal_id.set(id.saturating_add(1));
            state.traversals.borrow_mut().insert(
                id,
                TraversalRecord {
                    root,
                    nodes,
                    accepted,
                    index: 0,
                    pointer_before: member == "createNodeIterator",
                    what_to_show,
                },
            );
            let interface = if member == "createNodeIterator" {
                "NodeIterator"
            } else {
                "TreeWalker"
            };
            Ok(Some(
                new_stateful_object(scope, state, interface, id)?.into(),
            ))
        }
        ("Element", "attachShadow") => {
            let options = args
                .get(0)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("ShadowRootInit"))?;
            let mode_key = v8::String::new(scope, "mode")
                .ok_or(V8BindingError::Allocation("ShadowRootInit.mode"))?;
            let mode = options
                .get(scope, mode_key.into())
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "open".into());
            let mode = if mode.eq_ignore_ascii_case("closed") {
                ShadowMode::Closed
            } else {
                ShadowMode::Open
            };
            let clonable_key = v8::String::new(scope, "clonable")
                .ok_or(V8BindingError::Allocation("ShadowRootInit.clonable"))?;
            let clonable = options
                .get(scope, clonable_key.into())
                .is_some_and(|value| value.boolean_value(scope));
            let created = {
                let mut dom = state.dom.borrow_mut();
                if dom.arena().get(node)?.children.iter().any(|child| {
                    matches!(
                        dom.arena().get(*child).ok().map(|record| &record.kind),
                        Some(NodeKind::ShadowRoot { .. })
                    )
                }) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: "attachShadow called more than once".into(),
                    });
                }
                let root = dom.create(NodeKind::ShadowRoot { mode, clonable });
                dom.append_child(node, root)?;
                root
            };
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Element", "getAttributeNode") => {
            let name = required_string(scope, args, 0)?;
            let attribute = state.dom.borrow_mut().attribute_node(node, &name)?;
            optional_node(scope, state, attribute).map(Some)
        }
        ("Element", "insertAdjacentElement" | "insertAdjacentText" | "insertAdjacentHTML") => {
            let position = required_string(scope, args, 0)?;
            let (parent, reference) =
                adjacent_insertion_point(&state.dom.borrow(), node, &position)?;
            match member {
                "insertAdjacentElement" => {
                    let inserted = argument_node(scope, state, args.get(1))?;
                    state
                        .dom
                        .borrow_mut()
                        .insert_before(parent, inserted, reference)?;
                    prepare_dynamic_script_subtree(scope, state, inserted)?;
                    Ok(Some(wrap_node(scope, state, inserted)?.into()))
                }
                "insertAdjacentText" => {
                    let text = required_string(scope, args, 1)?;
                    let inserted = state.dom.borrow_mut().create_text(text);
                    state
                        .dom
                        .borrow_mut()
                        .insert_before(parent, inserted, reference)?;
                    Ok(None)
                }
                _ => {
                    let markup = trusted_string(
                        scope,
                        state,
                        args.get(1),
                        TrustedKind::Html,
                        "Element.insertAdjacentHTML",
                    )?;
                    let parsed = oryn_dom::html::parse_fragment(&markup, "div");
                    let children = parsed.dom.get(parsed.document)?.children.clone();
                    let mut inserted = Vec::new();
                    for child in children {
                        let child = state.dom.borrow_mut().import_subtree(&parsed.dom, child)?;
                        state
                            .dom
                            .borrow_mut()
                            .insert_before(parent, child, reference)?;
                        inserted.push(child);
                    }
                    for child in inserted {
                        prepare_dynamic_script_subtree(scope, state, child)?;
                    }
                    Ok(None)
                }
            }
        }
        ("Document" | "ShadowRoot", "getElementById") => {
            let id = required_string(scope, args, 0)?;
            let found = if interface == "Document" {
                state.dom.borrow().find_by_id(&id)
            } else {
                state.dom.borrow().query_selector(node, &format!("#{id}"))?
            };
            optional_node(scope, state, found).map(Some)
        }
        ("Document", "open") => {
            if state.parser_input_active.get() {
                state
                    .parser_input_commands
                    .borrow_mut()
                    .push_back(ParserInputCommand::Open);
                return Ok(Some(wrap_node(scope, state, node)?.into()));
            }
            state.document_open.set(true);
            state.current_script.set(None);
            state.document_write_buffer.borrow_mut().clear();
            state.dom.borrow_mut().replace_document_contents("")?;
            Ok(Some(wrap_node(scope, state, node)?.into()))
        }
        ("Document", "write" | "writeln") => {
            let mut markup = trusted_string(
                scope,
                state,
                args.get(0),
                TrustedKind::Html,
                &format!("Document.{member}"),
            )?;
            if member == "writeln" {
                markup.push('\n');
            }
            if markup.len() > 1024 * 1024 {
                return Err(V8BindingError::DocumentWriteLimit {
                    resource: "input size",
                });
            }
            if state.parser_input_active.get() {
                state
                    .parser_input_commands
                    .borrow_mut()
                    .push_back(ParserInputCommand::Write(markup));
            } else if state.document_open.get() {
                if state
                    .document_write_buffer
                    .borrow()
                    .len()
                    .saturating_add(markup.len())
                    > 1024 * 1024
                {
                    return Err(V8BindingError::DocumentWriteLimit {
                        resource: "buffer size",
                    });
                }
                state.document_write_buffer.borrow_mut().push_str(&markup);
            } else if let Some(anchor) = state.current_script.get() {
                let inserted = state.dom.borrow_mut().insert_html_after(anchor, &markup)?;
                if let Some(last) = inserted.last().copied() {
                    state.current_script.set(Some(last));
                }
                execute_parser_inserted_scripts(scope, state, &inserted)?;
            } else {
                let document = state.dom.borrow().document();
                let current = state.dom.borrow().serialize(document)?;
                state
                    .dom
                    .borrow_mut()
                    .replace_document_contents(&(current + &markup))?;
            }
            Ok(None)
        }
        ("Document", "close") => {
            if state.parser_input_active.get() {
                state
                    .parser_input_commands
                    .borrow_mut()
                    .push_back(ParserInputCommand::Close);
                return Ok(None);
            }
            if state.document_open.replace(false) {
                let markup = std::mem::take(&mut *state.document_write_buffer.borrow_mut());
                state.dom.borrow_mut().replace_document_contents(&markup)?;
            }
            Ok(None)
        }
        (_, "appendChild") => {
            let child = argument_node(scope, state, args.get(0))?;
            let fragment_children = {
                let dom = state.dom.borrow();
                matches!(dom.arena().get(child)?.kind, NodeKind::DocumentFragment)
                    .then(|| dom.arena().get(child).map(|record| record.children.clone()))
                    .transpose()?
            };
            if let Some(children) = fragment_children {
                for fragment_child in children {
                    state.dom.borrow_mut().append_child(node, fragment_child)?;
                    prepare_dynamic_script_subtree(scope, state, fragment_child)?;
                }
            } else {
                state.dom.borrow_mut().append_child(node, child)?;
                prepare_dynamic_script_subtree(scope, state, child)?;
            }
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "insertBefore") => {
            let child = argument_node(scope, state, args.get(0))?;
            let reference = optional_argument_node(scope, state, args.get(1))?;
            let fragment_children = {
                let dom = state.dom.borrow();
                matches!(dom.arena().get(child)?.kind, NodeKind::DocumentFragment)
                    .then(|| dom.arena().get(child).map(|record| record.children.clone()))
                    .transpose()?
            };
            if let Some(children) = fragment_children {
                for fragment_child in children {
                    state
                        .dom
                        .borrow_mut()
                        .insert_before(node, fragment_child, reference)?;
                    prepare_dynamic_script_subtree(scope, state, fragment_child)?;
                }
            } else {
                state
                    .dom
                    .borrow_mut()
                    .insert_before(node, child, reference)?;
                prepare_dynamic_script_subtree(scope, state, child)?;
            }
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "removeChild") => {
            let child = argument_node(scope, state, args.get(0))?;
            state.dom.borrow_mut().remove_child(node, child)?;
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "replaceChild") => {
            let replacement = argument_node(scope, state, args.get(0))?;
            let child = argument_node(scope, state, args.get(1))?;
            let fragment_children = {
                let dom = state.dom.borrow();
                matches!(
                    dom.arena().get(replacement)?.kind,
                    NodeKind::DocumentFragment
                )
                .then(|| {
                    dom.arena()
                        .get(replacement)
                        .map(|record| record.children.clone())
                })
                .transpose()?
            };
            if let Some(children) = fragment_children {
                for fragment_child in children {
                    state
                        .dom
                        .borrow_mut()
                        .insert_before(node, fragment_child, Some(child))?;
                    prepare_dynamic_script_subtree(scope, state, fragment_child)?;
                }
                state.dom.borrow_mut().remove_child(node, child)?;
            } else {
                state
                    .dom
                    .borrow_mut()
                    .replace_child(node, replacement, child)?;
                prepare_dynamic_script_subtree(scope, state, replacement)?;
            }
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "cloneNode") => {
            let deep = args.get(0).boolean_value(scope);
            let clone = state.dom.borrow_mut().clone_node(node, deep)?;
            let owner = state
                .node_documents
                .borrow()
                .get(&node)
                .copied()
                .unwrap_or_else(|| state.dom.borrow().document());
            state.node_documents.borrow_mut().insert(clone, owner);
            Ok(Some(wrap_node(scope, state, clone)?.into()))
        }
        ("Node", "append") => {
            let text = required_string(scope, args, 0)?;
            let child = state.dom.borrow_mut().create_text(text);
            let owner = if matches!(
                state.dom.borrow().arena().get(node)?.kind,
                NodeKind::Document
            ) {
                node
            } else {
                state
                    .node_documents
                    .borrow()
                    .get(&node)
                    .copied()
                    .unwrap_or_else(|| state.dom.borrow().document())
            };
            state.node_documents.borrow_mut().insert(child, owner);
            state.dom.borrow_mut().append_child(node, child)?;
            Ok(None)
        }
        ("Node", "before" | "after") => {
            let inserted = argument_node(scope, state, args.get(0))?;
            let (parent, reference) = {
                let dom = state.dom.borrow();
                let Some(parent) = dom.arena().get(node)?.parent else {
                    return Ok(None);
                };
                let siblings = &dom.arena().get(parent)?.children;
                let index = siblings
                    .iter()
                    .position(|candidate| *candidate == node)
                    .ok_or(DomError::StaleNode(node))?;
                let reference = if member == "before" {
                    Some(node)
                } else {
                    siblings.get(index + 1).copied()
                };
                (parent, reference)
            };
            let fragment_children = {
                let dom = state.dom.borrow();
                matches!(dom.arena().get(inserted)?.kind, NodeKind::DocumentFragment)
                    .then(|| {
                        dom.arena()
                            .get(inserted)
                            .map(|record| record.children.clone())
                    })
                    .transpose()?
            };
            let children = fragment_children.unwrap_or_else(|| vec![inserted]);
            for child in children {
                state
                    .dom
                    .borrow_mut()
                    .insert_before(parent, child, reference)?;
                prepare_dynamic_script_subtree(scope, state, child)?;
            }
            Ok(None)
        }
        ("Node", "remove") => {
            state.dom.borrow_mut().remove(node)?;
            Ok(None)
        }
        ("Node", "contains") => {
            let Some(other) = optional_argument_node(scope, state, args.get(0))? else {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            };
            let dom = state.dom.borrow();
            let contains = node == other
                || (tree_root(&dom, node)? == tree_root(&dom, other)?
                    && dom.arena().ancestors(other)?.contains(&node));
            Ok(Some(v8::Boolean::new(scope, contains).into()))
        }
        ("Node", "isEqualNode") => {
            let Some(other) = optional_argument_node(scope, state, args.get(0))? else {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            };
            let dom = state.dom.borrow();
            let equal = dom.serialize(node)? == dom.serialize(other)?;
            Ok(Some(v8::Boolean::new(scope, equal).into()))
        }
        ("Node", "compareDocumentPosition") => {
            let other = argument_node(scope, state, args.get(0))?;
            if node == other {
                return Ok(Some(v8::Integer::new(scope, 0).into()));
            }
            let dom = state.dom.borrow();
            let node_root = tree_root(&dom, node)?;
            let other_root = tree_root(&dom, other)?;
            let position = if node_root != other_root {
                let order = if node < other { 4 } else { 2 };
                1 | 32 | order
            } else if dom.arena().ancestors(node)?.contains(&other) {
                8 | 2
            } else if dom.arena().ancestors(other)?.contains(&node) {
                16 | 4
            } else if traversal_position(&dom, node)? < traversal_position(&dom, other)? {
                4
            } else {
                2
            };
            Ok(Some(v8::Integer::new(scope, position).into()))
        }
        ("Node", "hasChildNodes") => {
            let has_children = !state.dom.borrow().arena().get(node)?.children.is_empty();
            Ok(Some(v8::Boolean::new(scope, has_children).into()))
        }
        ("Node", "getRootNode") => {
            let root = tree_root(&state.dom.borrow(), node)?;
            Ok(Some(wrap_node(scope, state, root)?.into()))
        }
        ("Element", "getAttribute") => {
            let name = required_string(scope, args, 0)?;
            match state.dom.borrow().attribute(node, &name)? {
                Some(value) => Ok(Some(v8_string(scope, value)?)),
                None => Ok(Some(v8::null(scope).into())),
            }
        }
        ("Element", "getAttributeNames") => {
            let names = state
                .dom
                .borrow()
                .arena()
                .get(node)?
                .attributes
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            let values = names
                .iter()
                .map(|name| v8_string(scope, name))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(v8::Array::new_with_elements(scope, &values).into()))
        }
        ("Element", "setAttribute") => {
            let name = required_string(scope, args, 0)?;
            let tag = state.dom.borrow().node_name(node)?.to_ascii_lowercase();
            let required = if tag == "script" && name.eq_ignore_ascii_case("src") {
                Some(TrustedKind::ScriptUrl)
            } else if tag == "iframe" && name.eq_ignore_ascii_case("srcdoc") {
                Some(TrustedKind::Html)
            } else if name.to_ascii_lowercase().starts_with("on") {
                Some(TrustedKind::Script)
            } else {
                None
            };
            let value = match required {
                Some(kind) => trusted_string(
                    scope,
                    state,
                    args.get(1),
                    kind,
                    &format!("Element.setAttribute({name})"),
                )?,
                None => required_string(scope, args, 1)?,
            };
            state.dom.borrow_mut().set_attribute(node, name, value)?;
            prepare_dynamic_script_subtree(scope, state, node)?;
            Ok(None)
        }
        ("Element", "removeAttribute") => {
            let name = required_string(scope, args, 0)?;
            state.dom.borrow_mut().remove_attribute(node, &name)?;
            Ok(None)
        }
        ("Element", "hasAttribute") => {
            let name = required_string(scope, args, 0)?;
            let present = state.dom.borrow().attribute(node, &name)?.is_some();
            Ok(Some(v8::Boolean::new(scope, present).into()))
        }
        ("Element", "hasAttributes") => {
            let has_attributes = !state.dom.borrow().arena().get(node)?.attributes.is_empty();
            Ok(Some(v8::Boolean::new(scope, has_attributes).into()))
        }
        ("Element", "toggleAttribute") => {
            let name = required_string(scope, args, 0)?;
            let present = state.dom.borrow().attribute(node, &name)?.is_some();
            let force = (!args.get(1).is_undefined()).then(|| args.get(1).boolean_value(scope));
            let next = force.unwrap_or(!present);
            if next {
                if !present {
                    state.dom.borrow_mut().set_attribute(node, &name, "")?;
                }
            } else if present {
                state.dom.borrow_mut().remove_attribute(node, &name)?;
            }
            Ok(Some(v8::Boolean::new(scope, next).into()))
        }
        (_, "querySelector") => {
            let selector = required_string(scope, args, 0)?;
            let found = state.dom.borrow().query_selector(node, &selector)?;
            optional_node(scope, state, found).map(Some)
        }
        (_, "querySelectorAll") => {
            let selector = required_string(scope, args, 0)?;
            let found = state.dom.borrow().query_selector_all(node, &selector)?;
            node_array(scope, state, &found).map(Some)
        }
        ("Document" | "Element", "getElementsByTagName") => {
            let name = required_string(scope, args, 0)?;
            let found = state.dom.borrow().query_selector_all(node, &name)?;
            node_array(scope, state, &found).map(Some)
        }
        ("Document" | "Element", "getElementsByClassName") => {
            let classes = required_string(scope, args, 0)?;
            let selector = classes
                .split_ascii_whitespace()
                .filter(|class| !class.is_empty())
                .map(|class| format!(".{class}"))
                .collect::<String>();
            let found = if selector.is_empty() {
                Vec::new()
            } else {
                state.dom.borrow().query_selector_all(node, &selector)?
            };
            node_array(scope, state, &found).map(Some)
        }
        ("Document", "getElementsByName") => {
            let name = required_string(scope, args, 0)?.replace('"', "\\\"");
            let found = state
                .dom
                .borrow()
                .query_selector_all(node, &format!("[name=\"{name}\"]"))?;
            node_array(scope, state, &found).map(Some)
        }
        ("Element", "remove") => {
            state.dom.borrow_mut().remove(node)?;
            Ok(None)
        }
        ("Element", "matches") => {
            let selector = required_string(scope, args, 0)?;
            let matched = state.dom.borrow().matches_selector(node, &selector)?;
            Ok(Some(v8::Boolean::new(scope, matched).into()))
        }
        ("Element", "closest") => {
            let selector = required_string(scope, args, 0)?;
            let mut current = Some(node);
            while let Some(candidate) = current {
                if state.dom.borrow().matches_selector(candidate, &selector)? {
                    return Ok(Some(wrap_node(scope, state, candidate)?.into()));
                }
                current = state.dom.borrow().parent(candidate)?;
            }
            Ok(Some(v8::null(scope).into()))
        }
        ("Element", "getBoundingClientRect") => {
            let layout = state.layout.borrow();
            let rect = match layout.geometry.get(&node_key(node)) {
                Some(BindingGeometry::Available(rect)) => *rect,
                Some(BindingGeometry::Hidden) => BindingRect::default(),
                Some(BindingGeometry::Unsupported(detail)) => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: format!("getBoundingClientRect: {detail}"),
                    });
                }
                None => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member:
                            "getBoundingClientRect: geometry is not available at this checkpoint"
                                .into(),
                    });
                }
            };
            Ok(Some(dom_rect_object(scope, rect)?.into()))
        }
        ("Element", "getClientRects") => {
            let layout = state.layout.borrow();
            let values = match layout.geometry.get(&node_key(node)) {
                Some(BindingGeometry::Available(rect)) => {
                    vec![dom_rect_object(scope, *rect)?.into()]
                }
                Some(BindingGeometry::Hidden) => Vec::new(),
                Some(BindingGeometry::Unsupported(detail)) => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: format!("getClientRects: {detail}"),
                    });
                }
                None => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: "getClientRects: geometry is not available at this checkpoint"
                            .into(),
                    });
                }
            };
            Ok(Some(v8::Array::new_with_elements(scope, &values).into()))
        }
        ("DOMTokenList", "add") => {
            let token = required_string(scope, args, 0)?;
            update_class_token(&mut state.dom.borrow_mut(), node, &token, Some(true))?;
            Ok(None)
        }
        ("DOMTokenList", "remove") => {
            let token = required_string(scope, args, 0)?;
            update_class_token(&mut state.dom.borrow_mut(), node, &token, Some(false))?;
            Ok(None)
        }
        ("DOMTokenList", "contains") => {
            let token = required_string(scope, args, 0)?;
            let present = class_tokens(&state.dom.borrow(), node)?.any(|item| item == token);
            Ok(Some(v8::Boolean::new(scope, present).into()))
        }
        ("DOMTokenList", "toggle") => {
            let token = required_string(scope, args, 0)?;
            let current = class_tokens(&state.dom.borrow(), node)?.any(|item| item == token);
            let next = if args.length() > 1 {
                args.get(1).boolean_value(scope)
            } else {
                !current
            };
            update_class_token(&mut state.dom.borrow_mut(), node, &token, Some(next))?;
            Ok(Some(v8::Boolean::new(scope, next).into()))
        }
        ("CSSStyleSheet", "insertRule") => {
            let rule = required_string(scope, args, 0)?;
            let current = state.dom.borrow().arena().text_content(node)?;
            let index = current.matches('}').count();
            let combined = if current.trim().is_empty() {
                rule
            } else {
                format!("{current}\n{rule}")
            };
            state.dom.borrow_mut().set_text_content(node, combined)?;
            Ok(Some(
                v8::Integer::new_from_unsigned(scope, index as u32).into(),
            ))
        }
        ("CSSStyleDeclaration", "getPropertyValue") => {
            let property = required_string(scope, args, 0)?.to_ascii_lowercase();
            let dom = state.dom.borrow();
            let style = dom
                .arena()
                .get(node)?
                .attributes
                .get("style")
                .cloned()
                .unwrap_or_default();
            let value = inline_style_entries(&style)
                .remove(&property)
                .unwrap_or_default();
            Ok(Some(v8_string(scope, &value)?))
        }
        ("CSSStyleDeclaration", "setProperty") => {
            let property = required_string(scope, args, 0)?.to_ascii_lowercase();
            let value = required_string(scope, args, 1)?;
            set_inline_style_property(&mut state.dom.borrow_mut(), node, &property, &value)?;
            Ok(None)
        }
        ("CSSStyleDeclaration", "removeProperty") => {
            let property = required_string(scope, args, 0)?.to_ascii_lowercase();
            let previous =
                set_inline_style_property(&mut state.dom.borrow_mut(), node, &property, "")?;
            Ok(Some(v8_string(scope, &previous)?))
        }
        ("HTMLElement", "focus") => {
            let previous = state.dom.borrow().focused();
            if previous != Some(node) {
                if let Some(previous) = previous {
                    state.dom.borrow_mut().focus(None)?;
                    let event = create_event_object(scope, state, "blur", false, false)?;
                    dispatch_event(scope, state, previous, event)?;
                }
                state.dom.borrow_mut().focus(Some(node))?;
                let event = create_event_object(scope, state, "focus", false, false)?;
                dispatch_event(scope, state, node, event)?;
            }
            Ok(None)
        }
        ("HTMLElement", "blur") => {
            if state.dom.borrow().focused() == Some(node) {
                state.dom.borrow_mut().focus(None)?;
                let event = create_event_object(scope, state, "blur", false, false)?;
                dispatch_event(scope, state, node, event)?;
            }
            Ok(None)
        }
        ("Element", "scrollIntoView") => Ok(None),
        ("HTMLCanvasElement", "getContext") => {
            let context_id = required_string(scope, args, 0)?;
            if context_id != "2d" {
                return Ok(Some(v8::null(scope).into()));
            }
            let context = v8::Object::new(scope);
            for name in ["beginPath", "arc", "fill", "fillRect", "clearRect"] {
                let function = v8::Function::new(scope, canvas_noop)
                    .ok_or(V8BindingError::Allocation("canvas no-op function"))?;
                set_object_property(scope, context, name, function.into())?;
            }
            Ok(Some(context.into()))
        }
        ("HTMLElement", "click") => {
            let event = create_event_object(scope, state, "click", true, true)?;
            dispatch_event(scope, state, node, event)?;
            Ok(None)
        }
        ("HTMLFormElement", "checkValidity" | "reportValidity") => {
            let valid = form_is_valid(&state.dom.borrow(), node)?;
            Ok(Some(v8::Boolean::new(scope, valid).into()))
        }
        ("HTMLFormElement", "requestSubmit" | "submit") => {
            let event = create_event_object(scope, state, "submit", true, true)?;
            dispatch_event(scope, state, node, event)?;
            Ok(None)
        }
        ("HTMLFormElement", "reset") => {
            let controls = {
                let dom = state.dom.borrow();
                ["input", "textarea"]
                    .into_iter()
                    .flat_map(|selector| dom.query_selector_all(node, selector).unwrap_or_default())
                    .map(|control| {
                        let record = dom.arena().get(control)?;
                        let value = if matches!(&record.kind, NodeKind::Element { name } if name.local.as_ref() == "textarea") {
                            dom.arena().text_content(control).unwrap_or_default()
                        } else {
                            record.attributes.get("value").cloned().unwrap_or_default()
                        };
                        Ok((control, value))
                    })
                    .collect::<Result<Vec<_>, DomError>>()?
            };
            let mut dom = state.dom.borrow_mut();
            for (control, value) in controls {
                dom.set_value(control, value, false)?;
            }
            drop(dom);
            let event = create_event_object(scope, state, "reset", true, false)?;
            dispatch_event(scope, state, node, event)?;
            Ok(None)
        }
        _ => Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: member.into(),
        }),
    }
}

fn canvas_noop(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
}

fn trusted_string(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    required: TrustedKind,
    sink: &str,
) -> Result<String, V8BindingError> {
    if !state.trusted_registry.borrow().requires(required) {
        return Ok(value.to_rust_string_lossy(scope));
    }
    if let Some(id) = trusted_value_id(scope, value, required)
        && let Some(value) = state.trusted_values.borrow().get(&id)
    {
        return Ok(value.value().to_string());
    }
    let default_callback = state
        .trusted_policies
        .borrow()
        .values()
        .find(|policy| policy.name == "default")
        .and_then(|policy| match required {
            TrustedKind::Html => policy.create_html.clone(),
            TrustedKind::Script => policy.create_script.clone(),
            TrustedKind::ScriptUrl => policy.create_script_url.clone(),
        });
    if let Some(callback) = default_callback {
        let callback = v8::Local::new(scope, &callback);
        let undefined = v8::undefined(scope).into();
        if let Some(result) = callback.call(scope, undefined, &[value]) {
            return Ok(result.to_rust_string_lossy(scope));
        }
    }
    Err(V8BindingError::Unsupported {
        interface: required.interface_name().into(),
        member: format!("required by {sink}"),
    })
}

fn execute_parser_inserted_scripts(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    roots: &[NodeId],
) -> Result<(), V8BindingError> {
    if state.document_write_depth.get() >= 32 {
        return Err(V8BindingError::DocumentWriteLimit {
            resource: "nested script depth",
        });
    }
    let scripts = {
        let dom = state.dom.borrow();
        let mut scripts = Vec::new();
        for root in roots {
            if dom.node_name(*root).is_ok_and(|name| name == "SCRIPT") {
                scripts.push(*root);
            }
            scripts.extend(dom.query_selector_all(*root, "script")?);
        }
        scripts
            .into_iter()
            .map(|script| {
                dom.arena()
                    .text_content(script)
                    .map(|source| (script, source))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    for (script, source) in scripts {
        let previous_anchor = state.current_script.replace(Some(script));
        state
            .document_write_depth
            .set(state.document_write_depth.get().saturating_add(1));
        let result: Result<(), V8BindingError> = (|| {
            let source = v8::String::new(scope, &source)
                .ok_or(V8BindingError::Allocation("document.write script source"))?;
            let script = v8::Script::compile(scope, source, None)
                .ok_or(V8BindingError::Allocation("document.write script"))?;
            script
                .run(scope)
                .ok_or(V8BindingError::Allocation("document.write script result"))?;
            Ok(())
        })();
        state
            .document_write_depth
            .set(state.document_write_depth.get().saturating_sub(1));
        state.current_script.set(previous_anchor);
        result?;
    }
    Ok(())
}

/// Run connected inline classic scripts synchronously at the DOM insertion
/// boundary and enqueue external/module scripts for the page resource
/// scheduler. This preserves the observable `appendChild(script)` ordering
/// without granting the bindings layer any network authority.
fn prepare_dynamic_script_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    prepare_dynamic_frame_subtree(state, root)?;
    for script in state.dynamic_script_candidates(root)? {
        if state.prepared_scripts.borrow().contains(&script) {
            continue;
        }
        let (script_type, external, source) = {
            let dom = state.dom.borrow();
            (
                dom.attribute(script, "type")?
                    .unwrap_or_default()
                    .to_owned(),
                dom.attribute(script, "src")?.map(str::to_owned),
                dom.arena().text_content(script)?,
            )
        };
        let kind = script_type.trim().to_ascii_lowercase();
        let classic = matches!(
            kind.as_str(),
            "" | "text/javascript" | "application/javascript" | "text/babel"
        );
        let module = kind == "module";
        if !classic && !module {
            state.prepared_scripts.borrow_mut().insert(script);
            continue;
        }
        state.prepared_scripts.borrow_mut().insert(script);
        if external.is_some() || module {
            state.pending_dynamic_scripts.borrow_mut().push_back(script);
            continue;
        }
        if source.trim().is_empty() {
            continue;
        }
        let source = v8::String::new(scope, &source)
            .ok_or(V8BindingError::Allocation("dynamic script source"))?;
        let compiled = v8::Script::compile(scope, source, None)
            .ok_or(V8BindingError::Allocation("dynamic script"))?;
        compiled
            .run(scope)
            .ok_or(V8BindingError::Allocation("dynamic script result"))?;
        scope.perform_microtask_checkpoint();
    }
    Ok(())
}

fn prepare_dynamic_frame_subtree(
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let frames = {
        let dom = state.dom.borrow();
        if !dom.is_connected(root)? {
            return Ok(());
        }
        let mut frames = Vec::new();
        if dom
            .node_name(root)
            .is_ok_and(|name| name.eq_ignore_ascii_case("iframe"))
        {
            frames.push(root);
        }
        frames.extend(dom.query_selector_all(root, "iframe")?);
        frames
            .into_iter()
            .filter(|frame| {
                dom.attribute(*frame, "src")
                    .ok()
                    .flatten()
                    .is_some_and(|src| !src.trim().is_empty())
            })
            .collect::<Vec<_>>()
    };
    for frame in frames {
        if state.prepared_frames.borrow_mut().insert(frame) {
            state.pending_dynamic_frames.borrow_mut().push_back(frame);
        }
    }
    Ok(())
}

fn associated_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &'static str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let wrapper = state.wrappers.borrow_mut().get_or_create(
        &mut state.dom.borrow_mut(),
        state.realm,
        state.document_generation,
        node,
        InterfaceBrand::Node,
    )?;
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .cloned()
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("associated wrapper object"))?;
    object.set_internal_field(0, v8::BigInt::new_from_u64(scope, wrapper.0).into());
    Ok(object.into())
}

fn associated_node_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    nodes: &[NodeId],
    interface: &'static str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let values = nodes
        .iter()
        .map(|node| associated_wrapper(scope, state, *node, interface))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v8::Array::new_with_elements(scope, &values).into())
}

fn class_tokens(dom: &DomRuntime, node: NodeId) -> Result<impl Iterator<Item = &str>, DomError> {
    Ok(dom
        .attribute(node, "class")?
        .unwrap_or_default()
        .split_ascii_whitespace())
}

fn update_class_token(
    dom: &mut DomRuntime,
    node: NodeId,
    token: &str,
    force: Option<bool>,
) -> Result<(), DomError> {
    let mut tokens = class_tokens(dom, node)?
        .map(str::to_string)
        .collect::<Vec<_>>();
    let present = tokens.iter().any(|item| item == token);
    match force {
        Some(true) if !present => tokens.push(token.into()),
        Some(false) if present => tokens.retain(|item| item != token),
        _ => {}
    }
    dom.set_attribute(node, "class", tokens.join(" "))
}

fn form_is_valid(dom: &DomRuntime, form: NodeId) -> Result<bool, DomError> {
    for control in ["input", "textarea", "select"]
        .into_iter()
        .flat_map(|selector| dom.query_selector_all(form, selector).unwrap_or_default())
    {
        let record = dom.arena().get(control)?;
        if record.attributes.contains_key("required")
            && dom
                .value(control)?
                .or_else(|| record.attributes.get("value").map(String::as_str))
                .unwrap_or("")
                .is_empty()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn event_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<u64, V8BindingError> {
    object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| value.integer_value(scope))
        .map(|value| value as u64)
        .ok_or(V8BindingError::Allocation("event identity"))
}

fn create_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get("Event")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("Event"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("event object"))?;
    let id = state.next_event_id.get();
    state.next_event_id.set(id.saturating_add(1));
    state.events.borrow_mut().insert(
        id,
        EventRecord {
            event_type: event_type.into(),
            target: None,
            current_target: None,
            bubbles,
            cancelable,
            default_prevented: false,
            propagation_stopped: false,
            immediate_stopped: false,
        },
    );
    object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
    Ok(object)
}

fn get_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = event_id(scope, object)?;
    let event = state
        .events
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("event record"))?;
    match member {
        "type" => v8_string(scope, &event.event_type),
        "target" => optional_node(scope, state, event.target),
        "currentTarget" => optional_node(scope, state, event.current_target),
        "bubbles" => Ok(v8::Boolean::new(scope, event.bubbles).into()),
        "cancelable" => Ok(v8::Boolean::new(scope, event.cancelable).into()),
        "defaultPrevented" => Ok(v8::Boolean::new(scope, event.default_prevented).into()),
        _ => Ok(v8::undefined(scope).into()),
    }
}

fn call_event_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = event_id(scope, object)?;
    let mut events = state.events.borrow_mut();
    let event = events
        .get_mut(&id)
        .ok_or(V8BindingError::Allocation("event record"))?;
    match member {
        "initEvent" => {
            event.event_type = required_string(scope, args, 0)?;
            event.bubbles = args.get(1).boolean_value(scope);
            event.cancelable = args.get(2).boolean_value(scope);
            event.default_prevented = false;
        }
        "preventDefault" if event.cancelable => event.default_prevented = true,
        "stopPropagation" => event.propagation_stopped = true,
        "stopImmediatePropagation" => {
            event.propagation_stopped = true;
            event.immediate_stopped = true;
        }
        _ => {}
    }
    Ok(None)
}

fn dispatch_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    object: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    v8::tc_scope!(let scope, scope);
    let id = event_id(scope, object)?;
    {
        let mut events = state.events.borrow_mut();
        let event = events
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("event record"))?;
        event.target = Some(target);
        event.propagation_stopped = false;
        event.immediate_stopped = false;
    }
    let mut path = vec![target];
    if state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.bubbles)
    {
        path.extend(state.dom.borrow().arena().ancestors(target)?);
    }
    for current in path {
        {
            let mut events = state.events.borrow_mut();
            let event = events.get_mut(&id).unwrap();
            if event.propagation_stopped && event.current_target.is_some() {
                break;
            }
            event.current_target = Some(current);
            event.immediate_stopped = false;
        }
        let event_type = state.events.borrow().get(&id).unwrap().event_type.clone();
        let callbacks = state
            .listeners
            .borrow()
            .get(&(current, event_type))
            .map(|listeners| {
                listeners
                    .iter()
                    .map(|listener| listener.callback.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let receiver = wrap_node(scope, state, current)?;
        for callback in callbacks {
            let callback = v8::Local::new(scope, &callback);
            if callback
                .call(scope, receiver.into(), &[object.into()])
                .is_none()
            {
                let detail = scope
                    .stack_trace()
                    .or_else(|| scope.exception())
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "JavaScript listener raised an exception".into());
                return Err(V8BindingError::EventCallback(detail));
            }
            if state
                .events
                .borrow()
                .get(&id)
                .is_some_and(|event| event.immediate_stopped)
            {
                break;
            }
        }
        let event_type = state.events.borrow().get(&id).unwrap().event_type.clone();
        let property = format!("on{event_type}");
        if !state
            .events
            .borrow()
            .get(&id)
            .is_some_and(|event| event.immediate_stopped)
            && let Some(property) = v8::String::new(scope, &property)
            && let Some(handler) = receiver.get(scope, property.into())
            && let Ok(handler) = v8::Local::<v8::Function>::try_from(handler)
        {
            let result = handler
                .call(scope, receiver.into(), &[object.into()])
                .ok_or_else(|| {
                    let detail = scope
                        .stack_trace()
                        .or_else(|| scope.exception())
                        .and_then(|value| value.to_string(scope))
                        .map(|value| value.to_rust_string_lossy(scope))
                        .unwrap_or_else(|| "event property handler raised an exception".into());
                    V8BindingError::EventCallback(detail)
                })?;
            if result.is_false() {
                let mut events = state.events.borrow_mut();
                let event = events.get_mut(&id).unwrap();
                if event.cancelable {
                    event.default_prevented = true;
                }
            }
        }
    }
    let mut events = state.events.borrow_mut();
    let event = events.get_mut(&id).unwrap();
    event.current_target = None;
    Ok(!event.default_prevented)
}

fn optional_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: Option<NodeId>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match node {
        Some(node) => Ok(wrap_node(scope, state, node)?.into()),
        None => Ok(v8::null(scope).into()),
    }
}

fn node_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    nodes: &[NodeId],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let values = nodes
        .iter()
        .map(|node| wrap_node(scope, state, *node).map(Into::into))
        .collect::<Result<Vec<v8::Local<v8::Value>>, _>>()?;
    Ok(v8::Array::new_with_elements(scope, &values).into())
}

fn argument_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
) -> Result<NodeId, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    let wrapper = receiver_wrapper_id(scope, object)
        .ok_or(BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    if let Ok(node) = state.wrappers.borrow().resolve(
        &state.dom.borrow(),
        wrapper,
        state.realm,
        state.document_generation,
        InterfaceBrand::Node,
    ) {
        return Ok(node);
    }
    let (node, realm, document_generation) =
        embedded_node_identity(scope, object).ok_or(BindingError::UnknownWrapper(wrapper))?;
    let owner = binding_state_for_receiver(scope, object)
        .filter(|owner| {
            owner.realm == realm
                && owner.document_generation == document_generation
                && Rc::ptr_eq(&owner.dom, &state.dom)
        })
        .ok_or(BindingError::UnknownWrapper(wrapper))?;
    owner.dom.borrow().arena().get(node)?;
    Ok(node)
}

fn optional_argument_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
) -> Result<Option<NodeId>, V8BindingError> {
    if value.is_null_or_undefined() {
        Ok(None)
    } else {
        argument_node(scope, state, value).map(Some)
    }
}

fn required_string(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
    index: i32,
) -> Result<String, V8BindingError> {
    args.get(index)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .ok_or(V8BindingError::Allocation("string conversion"))
}

fn v8_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    Ok(v8::String::new(scope, value)
        .ok_or(V8BindingError::Allocation("string"))?
        .into())
}

fn node_type(kind: &NodeKind) -> i32 {
    match kind {
        NodeKind::Element { .. } => 1,
        NodeKind::Attribute { .. } => 2,
        NodeKind::Text { .. } => 3,
        NodeKind::CDataSection { .. } => 4,
        NodeKind::ProcessingInstruction { .. } => 7,
        NodeKind::Comment { .. } => 8,
        NodeKind::Document => 9,
        NodeKind::DocumentType { .. } => 10,
        NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => 11,
    }
}

fn throw_type_error(scope: &mut v8::PinScope, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let exception = v8::Exception::type_error(scope, message);
        scope.throw_exception(exception);
    }
}

fn throw_binding_error(scope: &mut v8::PinScope, error: &V8BindingError) {
    let V8BindingError::Unsupported { member, .. } = error else {
        throw_type_error(scope, &error.to_string());
        return;
    };
    if !member.ends_with("Error") {
        throw_type_error(scope, &error.to_string());
        return;
    }
    let Some(message) = v8::String::new(scope, &error.to_string()) else {
        return;
    };
    let exception = v8::Exception::error(scope, message);
    if let Ok(object) = v8::Local::<v8::Object>::try_from(exception)
        && let (Some(name_key), Some(name)) = (
            v8::String::new(scope, "name"),
            v8::String::new(scope, member),
        )
    {
        object.create_data_property(scope, name_key.into(), name.into());
        let code = match member.as_str() {
            "IndexSizeError" => 1,
            "HierarchyRequestError" => 3,
            "WrongDocumentError" => 4,
            "InvalidCharacterError" => 5,
            "NoModificationAllowedError" => 7,
            "NotFoundError" => 8,
            "NotSupportedError" => 9,
            "InUseAttributeError" => 10,
            "InvalidStateError" => 11,
            "SyntaxError" => 12,
            "InvalidModificationError" => 13,
            "NamespaceError" => 14,
            "InvalidAccessError" => 15,
            "SecurityError" => 18,
            "NetworkError" => 19,
            "AbortError" => 20,
            "URLMismatchError" => 21,
            "QuotaExceededError" => 22,
            "TimeoutError" => 23,
            "InvalidNodeTypeError" => 24,
            "DataCloneError" => 25,
            _ => 0,
        };
        if let Some(code_key) = v8::String::new(scope, "code") {
            object.create_data_property(
                scope,
                code_key.into(),
                v8::Integer::new(scope, code).into(),
            );
        }
        let context = scope.get_current_context();
        let global = context.global(scope);
        if let Some(constructor_key) = v8::String::new(scope, "DOMException")
            && let Some(constructor) = global.get(scope, constructor_key.into())
            && let Ok(constructor) = v8::Local::<v8::Object>::try_from(constructor)
            && let Some(prototype_key) = v8::String::new(scope, "prototype")
            && let Some(prototype) = constructor.get(scope, prototype_key.into())
        {
            object.set_prototype(scope, prototype);
        }
    }
    scope.throw_exception(exception);
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, Once};

    use oryn_dom::html;

    use super::*;

    static INITIALIZE_V8: Once = Once::new();

    fn initialize_v8() {
        INITIALIZE_V8.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });
    }

    #[derive(Default)]
    struct TestPlatformHost {
        completions: Mutex<VecDeque<PlatformHostCompletion>>,
    }

    impl PlatformHost for TestPlatformHost {
        fn submit(
            &self,
            operation_id: HostOperationId,
            request: PlatformHostRequest,
        ) -> Result<(), String> {
            let result = match request {
                PlatformHostRequest::HttpStart { request } => PlatformHostResult::HttpStarted {
                    stream_id: 7,
                    status: 201,
                    url: request.url,
                    headers: vec![("content-type".into(), "text/plain".into())],
                },
                PlatformHostRequest::HttpRead { stream_id, .. } => PlatformHostResult::HttpChunk {
                    stream_id,
                    bytes: b"native".to_vec(),
                    done: true,
                },
                PlatformHostRequest::IndexedDb {
                    command: IdbHostCommand::Open { version, .. },
                    ..
                } => PlatformHostResult::IndexedDb {
                    response: crate::platform::IdbHostResponse {
                        version: Some(version),
                        old_version: Some(0),
                        upgrade_needed: true,
                        object_stores: vec![],
                        ..Default::default()
                    },
                },
                PlatformHostRequest::IndexedDb {
                    command:
                        IdbHostCommand::CreateObjectStore { .. } | IdbHostCommand::AbortUpgrade { .. },
                    ..
                } => PlatformHostResult::IndexedDb {
                    response: Default::default(),
                },
                PlatformHostRequest::IndexedDb {
                    command: IdbHostCommand::ListDatabases,
                    ..
                } => PlatformHostResult::IndexedDb {
                    response: crate::platform::IdbHostResponse {
                        databases: vec![crate::platform::IdbDatabaseInfo {
                            name: "app".into(),
                            version: 3,
                        }],
                        ..Default::default()
                    },
                },
                _ => return Err("unexpected test host operation".into()),
            };
            self.completions
                .lock()
                .unwrap()
                .push_back(PlatformHostCompletion {
                    operation_id,
                    result: Ok(result),
                });
            Ok(())
        }
        fn cancel(&self, _operation_id: HostOperationId) {}
        fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
            let mut queue = self.completions.lock().unwrap();
            (0..limit).filter_map(|_| queue.pop_front()).collect()
        }
    }

    #[test]
    fn generated_templates_mutate_the_rust_owned_dom() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const button=document.createElement('button');button.id='go';button.textContent='Run';document.querySelector('#root').appendChild(button);button===document.getElementById('go')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        let button = dom.borrow().find_by_id("go").unwrap();
        assert_eq!(dom.borrow().arena().text_content(button).unwrap(), "Run");
    }

    #[test]
    fn canvas_exposes_non_rendering_two_dimensional_bookkeeping_context() {
        initialize_v8();
        let parsed = html::parse("<canvas id='click-canvas'></canvas>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const canvas=document.getElementById('click-canvas');const ctx=canvas.getContext('2d');ctx.fillStyle='white';ctx.beginPath();ctx.arc(1,2,3,0,6.28);ctx.fill();ctx.fillRect(0,0,160,210);String(canvas instanceof HTMLCanvasElement&&ctx.fillStyle==='white'&&canvas.getContext('webgl')===null)",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true"
        );
    }

    #[test]
    fn transparent_proxy_receivers_preserve_generated_dom_branding() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const element=document.getElementById('root');const proxy=new Proxy(new Proxy(element,{}),{});proxy.setAttribute('data-proxy','yes');proxy.tagName+'|'+proxy.id+'|'+proxy.getAttribute('data-proxy')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "MAIN|root|yes"
        );
        let root = dom.borrow().find_by_id("root").unwrap();
        assert_eq!(
            dom.borrow()
                .arena()
                .get(root)
                .unwrap()
                .attributes
                .get("data-proxy")
                .map(String::as_str),
            Some("yes")
        );
    }

    #[test]
    fn generated_event_target_dispatches_against_rust_parentage() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><button id='go'></button></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "let calls=[];const root=document.getElementById('root');const go=document.getElementById('go');root.addEventListener('click',e=>calls.push('root:'+e.target.id));go.addEventListener('click',e=>{calls.push('go');e.preventDefault()});go.onclick=()=>calls.push('property');const accepted=go.dispatchEvent(new Event('click',{bubbles:true,cancelable:true}));calls.join(',')+'|'+accepted+'|'+('onclick' in go)",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "go,property,root:go|false|true"
        );
    }

    #[test]
    fn generated_history_owns_state_urls_and_traversal() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "globalThis.location={href:'https://example.test/start'};const events=[];globalThis.dispatchEvent=event=>{events.push([event.type,event.state,event.oldURL,event.newURL]);return true};history.pushState({step:1},'', '/next#one');const first=history.state;first.step=9;history.replaceState({step:2},'', '#two');history.back();[history instanceof History,history.length,history.state,location.href,events[0][0],events[1][0],first.step].join('|')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true|2||https://example.test/start|popstate|hashchange|9"
        );
    }

    #[test]
    fn generated_storage_owns_fallback_state_and_wrapper_identity() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "__oryn_restore_storage([['theme','dark']],[]);localStorage.setItem('size','large');const before=[localStorage instanceof Storage,localStorage.length,localStorage.key(0),localStorage.getItem('theme'),localStorage.getItem('size')];localStorage.removeItem('theme');before.concat(localStorage.length,localStorage.getItem('theme')).join('|')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true|2|size|dark|large|1|"
        );
    }

    #[test]
    fn generated_blob_file_and_headers_own_binary_and_header_state() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            r#"
              const file=new File([new Uint8Array([0,255]),'ok'],'a.bin',{type:'APPLICATION/OCTET-STREAM',lastModified:7});
              const slice=file.slice(1,3,'TEXT/PLAIN');
              const headers=new Headers({'X-B':'2','x-a':'1'});headers.append('X-A','3');
              const form=new FormData();form.append('label','first');form.append('asset',file,'renamed.bin');form.set('label','last');
              Promise.all([file.arrayBuffer(),slice.text()]).then(([bytes,text])=>{
                globalThis.platformResult=[file instanceof File,file instanceof Blob,file.size,file.type,file.name,file.lastModified,[...new Uint8Array(bytes)].join(','),slice.type,text,[...headers].map(pair=>pair.join('=')).join('&'),form.get('label'),form.get('asset').name,[...form].length].join('|');
              });
            "#,
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        script.run(scope).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "platformResult").unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true|true|4|application/octet-stream|a.bin|7|0,255,111,107|text/plain|�o|x-a=1, 3&x-b=2|last|renamed.bin|2"
        );
    }

    #[test]
    fn generated_blob_urls_feed_fetch_and_xhr_without_parent_network() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "globalThis.location={origin:'https://example.test'};let result='';const blob=new Blob(['oryn'],{type:'text/plain'});const url=__oryn_blob_url_create(blob);const xhr=new XMLHttpRequest();xhr.open('GET',url,true);xhr.onload=()=>result+='xhr:'+xhr.responseText+':'+xhr.getResponseHeader('content-type')+'|';xhr.send();fetch(url).then(response=>response.text()).then(text=>result+='fetch:'+text);",
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "result").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert!(result.contains("xhr:oryn:text/plain|"));
        assert!(result.contains("fetch:oryn"));
    }

    #[test]
    fn generated_streams_own_queues_backpressure_and_transform_callbacks() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            r#"
              const output=[];
              const source=new ReadableStream({start(controller){controller.enqueue('oryn');controller.close();}});
              const transform=new TransformStream({transform(chunk,controller){controller.enqueue(chunk.toUpperCase());}});
              const sink=new WritableStream({write(chunk){output.push(chunk);}});
              source.pipeThrough(transform).pipeTo(sink).then(()=>globalThis.streamResult=output.join('|')+'|'+source.locked+'|'+sink.locked);
            "#,
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        script.run(scope).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "streamResult").unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "ORYN|false|false"
        );
    }

    #[test]
    fn generated_fetch_abort_and_indexeddb_use_typed_host_completions() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_platform_host(Rc::new(TestPlatformHost::default()));
        let source = v8::String::new(scope, "globalThis.location={href:'https://example.test/app',origin:'https://example.test'};let result='';const controller=new AbortController();controller.signal.addEventListener('abort',()=>result+='abort|');controller.abort('stop');fetch('/data').then(response=>{result+=response.status+'|'+response.ok+'|'+response.headers.get('content-type')});const opened=indexedDB.open('app',2);opened.onupgradeneeded=()=>result+='|upgrade';opened.onsuccess=()=>result+='|db:'+opened.result.version;").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "result").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert_eq!(result, "abort|201|true|text/plain|upgrade|db:2");
    }

    #[test]
    fn generated_indexeddb_cmp_uses_the_browser_key_order() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "let missing='',invalid='';try{indexedDB.cmp()}catch(e){missing=e.name}try{indexedDB.cmp(null,null)}catch(e){invalid=e.name}[indexedDB.cmp([1],new Uint8Array([1])),indexedDB.cmp(new Uint8Array([255]),new Uint8Array([0])),indexedDB.cmp('a',new Date(0)),indexedDB.cmp(new Date(0),0),missing,invalid].join('|')",
        )
        .unwrap();
        let value = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert_eq!(value, "1|1|1|1|TypeError|DataError");
    }

    #[test]
    fn generated_indexeddb_lists_parent_owned_origin_databases() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_platform_host(Rc::new(TestPlatformHost::default()));
        let source = v8::String::new(
            scope,
            "globalThis.location={origin:'https://example.test'};let result='';indexedDB.databases().then(values=>result=values.map(value=>value.name+':'+value.version).join('|'))",
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "result").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert_eq!(result, "app:3");
    }

    #[test]
    fn indexeddb_host_errors_keep_their_dom_exception_names() {
        assert_eq!(
            idb_error_name("IndexedDB key already exists"),
            "ConstraintError"
        );
        assert_eq!(idb_error_name("IndexedDB key is required"), "DataError");
        assert_eq!(
            idb_error_name("IndexedDB structured value cannot be cloned"),
            "DataCloneError"
        );
        assert_eq!(
            idb_error_name("storage quota exceeded for https://example.test"),
            "QuotaExceededError"
        );
        let value = serde_json::json!({"nested": {"id": 7}, "invalid": {}});
        assert!(is_valid_idb_json_key(
            idb_json_key_path(&value, "nested.id").unwrap()
        ));
        assert!(!is_valid_idb_json_key(
            idb_json_key_path(&value, "invalid").unwrap()
        ));
    }

    #[test]
    fn generated_indexeddb_upgrade_exposes_and_aborts_its_versionchange_transaction() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_platform_host(Rc::new(TestPlatformHost::default()));
        let source = v8::String::new(
            scope,
            "let result='';const request=indexedDB.open('abort-me',2);request.onupgradeneeded=e=>{result=e.target.transaction.mode+'|'+(e.target.transaction.db===e.target.result);e.target.transaction.onabort=event=>result+='|abort:'+event.target.db.version;e.target.transaction.abort()};request.onerror=e=>result+='|error:'+e.target.error.name+'|'+String(e.target.transaction)",
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        let source = v8::String::new(scope, "result").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert_eq!(result, "versionchange|true|abort:0|error:AbortError|null");
    }

    #[test]
    fn generated_indexeddb_close_during_upgrade_completes_then_aborts_open() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_platform_host(Rc::new(TestPlatformHost::default()));
        let source = v8::String::new(
            scope,
            "let result='',db;const request=indexedDB.open('close-me',1);request.onupgradeneeded=e=>{db=e.target.result;db.createObjectStore('items');db.close();e.target.transaction.oncomplete=()=>result+='complete|'};request.onsuccess=()=>result+='success';request.onerror=e=>{result+='error:'+e.target.error.name;try{db.transaction('items')}catch(error){result+='|'+error.name}}",
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        let source = v8::String::new(scope, "result").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert_eq!(result, "complete|error:AbortError|InvalidStateError");
    }

    #[test]
    fn generated_edit_commands_are_selection_backed_and_unknown_commands_are_safe() {
        initialize_v8();
        let parsed = html::parse("<main id='editor' contenteditable>abc</main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const editor=document.getElementById('editor');editor.focus();const selection=document.getSelection();selection.selectAllChildren(editor);const inserted=document.execCommand('insertText',false,'oryn');[inserted,editor.textContent,document.execCommand('unknown',false,'x'),document.queryCommandEnabled('unknown'),document.queryCommandIndeterm('unknown'),document.queryCommandState('unknown'),document.queryCommandSupported('unknown'),document.queryCommandValue('unknown')].join('|')",
        )
        .unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert_eq!(result, "true|oryn|false|false|false|false|false|");
    }

    #[test]
    fn generated_xhr_reports_progressive_states_and_typed_response_bytes() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_platform_host(Rc::new(TestPlatformHost::default()));
        let source = v8::String::new(scope, "globalThis.location={href:'https://example.test/app',origin:'https://example.test'};const states=[];let xhrResult='';const xhr=new XMLHttpRequest();xhr.onreadystatechange=()=>states.push(xhr.readyState);xhr.onload=()=>xhrResult=states.join(',')+'|'+xhr.status+'|'+xhr.responseText;xhr.open('GET','/data');xhr.send();").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "xhrResult").unwrap();
        assert_eq!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .to_rust_string_lossy(scope),
            "1,2,3,4|201|native"
        );
    }

    #[test]
    fn generated_fetch_and_xhr_discard_late_completions_after_abort() {
        initialize_v8();
        let parsed = html::parse("<main></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_platform_host(Rc::new(TestPlatformHost::default()));
        let source = v8::String::new(scope, "globalThis.location={href:'https://example.test/app',origin:'https://example.test'};let cancelled='';const controller=new AbortController();fetch('/data',{signal:controller.signal}).catch(reason=>cancelled+='fetch:'+reason);controller.abort('stop');const xhr=new XMLHttpRequest();xhr.onabort=()=>cancelled+='|xhr:abort';xhr.onload=()=>cancelled+='|xhr:load';xhr.open('GET','/data');xhr.send();xhr.abort();").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        state.drain_platform_completions(scope, 8).unwrap();
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "cancelled").unwrap();
        assert_eq!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .to_rust_string_lossy(scope),
            "|xhr:abortfetch:stop"
        );
    }

    #[test]
    fn delegated_listener_state_never_changes_html_serialization() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root');const listener=()=>{};root.addEventListener('click',listener);const attributes=root.attributes;const clean=!root.outerHTML.includes('oryn-click')&&attributes.length===1&&attributes[0].name==='id'&&attributes[0].value==='root';root.removeEventListener('click',listener);clean",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        let root = dom.borrow().find_by_id("root").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(root)
                .unwrap()
                .event_listeners
                .is_empty()
        );
    }

    #[test]
    fn select_options_expose_live_option_wrappers_to_frameworks() {
        initialize_v8();
        let parsed = html::parse(
            "<select id='category'><option value='all'>All</option><option value='tools'>Tools</option></select>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const options=document.getElementById('category').options;options.length+'|'+options[1].value+'|'+(options[0]===document.querySelector('option'))",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "2|tools|true"
        );
    }

    #[test]
    fn document_landmarks_and_node_equality_support_framework_bootstrap() {
        initialize_v8();
        let parsed = html::parse(
            "<!doctype html><html><head><title>App</title></head><body><main id='root' class='shell' name='content'></main></body></html>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const main=document.getElementById('root');main.appendChild(document.createElement('span'));const clone=main.cloneNode(true);const names=main.getAttributeNames().join(',');const toggled=main.toggleAttribute('hidden',true)&&main.hasAttributes();main.toggleAttribute('hidden',false);const collections=document.getElementsByTagName('main').length+document.getElementsByClassName('shell').length+document.getElementsByName('content').length;String(document.head.tagName)+'|'+document.body.tagName+'|'+main.isEqualNode(clone)+'|'+main.hasChildNodes()+'|'+main.childElementCount+'|'+(main.firstElementChild===main.lastElementChild)+'|'+(main.getRootNode()===document)+'|'+names+'|'+toggled+'|'+collections",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "HEAD|BODY|true|true|1|true|true|class,id,name|true|3"
        );
    }

    #[test]
    fn generated_document_and_dom_implementation_support_foreign_tree_setup() {
        initialize_v8();
        let parsed = html::parse("<!doctype html><html><head></head><body></body></html>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const blank=new Document();const doctype=document.implementation.createDocumentType('root','public','system');const xml=document.implementation.createDocument(null,null,doctype);const root=xml.createElement('root');const cdata=xml.createCDATASection('content');const instruction=xml.createProcessingInstruction('target','data');root.appendChild(cdata);xml.appendChild(root);xml.appendChild(instruction);[blank.nodeType,xml.doctype===doctype,root.ownerDocument===xml,root.textContent,xml.compareDocumentPosition(xml)].join('|')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "9|true|true|content|0"
        );
    }

    #[test]
    #[ignore = "requires ORYN_WPT_COMMON_JS pointing at a pinned WPT checkout"]
    fn pinned_wpt_range_common_bootstrap_reaches_assertion_registration() {
        initialize_v8();
        let source_path = std::env::var("ORYN_WPT_COMMON_JS").unwrap();
        let common = std::fs::read_to_string(source_path).unwrap();
        let parsed = html::parse("<!doctype html><html><head></head><body></body></html>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let globals = v8::String::new(scope, "globalThis.window=globalThis").unwrap();
        v8::Script::compile(scope, globals, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let common = v8::String::new(scope, &common).unwrap();
        v8::Script::compile(scope, common, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let probe = v8::String::new(
            scope,
            "try{setupRangeTests();'ok:'+testRanges.length}catch(error){String(error)+'|'+String(error&&error.stack)}",
        )
        .unwrap();
        let result = v8::Script::compile(scope, probe, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        assert!(result.starts_with("ok:"), "{result}");
    }

    #[test]
    fn document_current_script_tracks_the_parser_execution_anchor() {
        initialize_v8();
        let parsed = html::parse("<main><script id='boot'></script></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let script_node = dom.borrow().find_by_id("boot").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_current_script(Some(script_node));
        let source = v8::String::new(
            scope,
            "document.currentScript.id==='boot'&&document.currentScript.parentElement.tagName==='MAIN'",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        state.set_current_script(None);
        let source = v8::String::new(scope, "document.currentScript===null").unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn element_attributes_support_named_legacy_feature_detection() {
        initialize_v8();
        let parsed = html::parse("<div id='probe'></div>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const div=document.getElementById('probe');div.setAttribute('onsubmit','t');div.style.cssText='background-color:rgba(1,1,1,.5);max-height:10px';div.attributes.onsubmit.name==='onsubmit'&&div.attributes.onsubmit.value==='t'&&div.attributes.onsubmit.expando===false&&div.style.backgroundColor.indexOf('rgba')===0&&div.style.maxHeight==='10px'",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn computed_style_uses_the_generated_declaration_surface() {
        initialize_v8();
        let parsed = html::parse("<div id='probe'></div>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let probe = dom.borrow().find_by_id("probe").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            styles: BTreeMap::from([(
                node_key(probe),
                BTreeMap::from([("display".to_string(), "block".to_string())]),
            )]),
            geometry: BTreeMap::new(),
        });
        let source = v8::String::new(
            scope,
            "const computed=getComputedStyle(document.getElementById('probe'));computed instanceof CSSStyleDeclaration&&computed.display==='block'&&computed.getPropertyValue('display')==='block'",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn document_url_focus_and_scroll_surface_support_router_bootstrap() {
        initialize_v8();
        let parsed = html::parse("<body><button id='go'></button></body>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "globalThis.location={href:'https://fixture.test/app'};const button=document.getElementById('go');let events='';button.onfocus=()=>events+='focus';button.onblur=()=>events+=':blur';button.focus();const focused=document.activeElement===button;button.scrollIntoView();button.blur();document.URL+'|'+document.baseURI+'|'+focused+'|'+(document.activeElement===null)+'|'+events",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "https://fixture.test/app|https://fixture.test/app|true|true|focus:blur"
        );
    }

    #[test]
    fn namespace_creation_exposes_svg_element_brand() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const svg=document.createElementNS('http://www.w3.org/2000/svg','svg'),link=document.createElementNS('http://www.w3.org/2000/svg','a');svg.appendChild(link);document.getElementById('root').appendChild(svg);(svg instanceof SVGElement)+'|'+svg.namespaceURI+'|'+svg.localName+'|'+(link instanceof SVGAElement)",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true|http://www.w3.org/2000/svg|svg|true"
        );
    }

    #[test]
    fn generated_form_controls_keep_state_in_the_rust_dom() {
        initialize_v8();
        let parsed = html::parse(
            "<form id='form'><input id='name' required><button id='save'></button></form>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const form=document.getElementById('form'),name=document.getElementById('name');const before=form.checkValidity();name.defaultValue='initial';name.defaultChecked=true;name.readOnly=true;name.value='Ada';const after=form.checkValidity();before+'|'+after+'|'+(name instanceof HTMLInputElement)+'|'+name.defaultValue+'|'+name.defaultChecked+'|'+name.readOnly+'|'+name.required",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "false|true|true|initial|true|true|true"
        );
        let input = dom.borrow().find_by_id("name").unwrap();
        assert_eq!(dom.borrow().value(input).unwrap(), Some("Ada"));
    }

    #[test]
    fn generated_geometry_distinguishes_hidden_from_unsupported() {
        initialize_v8();
        let parsed = html::parse(
            "<main><div id='visible'></div><div id='hidden'></div><table id='unsupported'></table></main>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let visible = dom.borrow().find_by_id("visible").unwrap();
        let hidden = dom.borrow().find_by_id("hidden").unwrap();
        let unsupported = dom.borrow().find_by_id("unsupported").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            styles: BTreeMap::new(),
            geometry: BTreeMap::from([
                (
                    node_key(visible),
                    BindingGeometry::Available(BindingRect {
                        x: 1.0,
                        y: 2.0,
                        width: 3.0,
                        height: 4.0,
                    }),
                ),
                (node_key(hidden), BindingGeometry::Hidden),
                (
                    node_key(unsupported),
                    BindingGeometry::Unsupported("table geometry is unavailable".into()),
                ),
            ]),
        });
        let source = v8::String::new(
            scope,
            "const hidden=document.getElementById('hidden'),visible=document.getElementById('visible');const rect=hidden.getBoundingClientRect();let rejected=false;try{document.getElementById('unsupported').getClientRects()}catch(error){rejected=error instanceof TypeError&&String(error).includes('table geometry is unavailable')}rect.width===0&&rect.height===0&&hidden.getClientRects().length===0&&visible.getClientRects().length===1&&visible.getClientRects()[0].width===3&&rejected",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn detached_wrapper_survives_reinsertion_without_becoming_queryable_early() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><button id='go'></button></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root'),go=document.getElementById('go');go.remove();const absent=document.getElementById('go')===null;root.appendChild(go);absent&&go===document.getElementById('go')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn template_content_and_fragment_insertion_match_framework_expectations() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main><video id='media'></video>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root');const template=document.createElement('template');template.innerHTML='<button>Add</button><span>ready</span>';const content=template.content;const clone=content.cloneNode(true);const first=Object.getOwnPropertyDescriptor(Node.prototype,'firstChild').get.call(clone);root.appendChild(clone);first.tagName==='BUTTON'&&root.children.length===2&&content.childNodes.length===2&&document.getElementById('media') instanceof HTMLMediaElement",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn generated_range_selection_and_traversal_are_rust_backed() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><p id='one'>one</p><p id='two'>two</p></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root'),one=document.getElementById('one'),two=document.getElementById('two');const range=document.createRange();range.selectNodeContents(one);const selection=document.getSelection();selection.addRange(range);const iterator=document.createNodeIterator(root);const walker=document.createTreeWalker(root);const first=walker.firstChild().id,next=walker.nextSibling().id,parent=walker.parentNode().id;range.setStartBefore(one);range.setEndAfter(two);const contains=range.commonAncestorContainer===root&&range.intersectsNode(two)&&range.isPointInRange(root,1);selection.collapseToStart();const collapsed=selection.isCollapsed;range.detach();(range instanceof Range)+'|'+selection.rangeCount+'|'+iterator.nextNode().id+'|'+iterator.nextNode().id+'|'+first+'|'+next+'|'+parent+'|'+contains+'|'+collapsed",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true|1|root|one|one|two|root|true|true"
        );
    }

    #[test]
    fn generated_node_filter_and_what_to_show_control_iteration() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><p id='one'>one</p><p id='two'>two</p></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root');const seen=[];const iterator=document.createNodeIterator(root,NodeFilter.SHOW_ELEMENT,node=>node.id==='one'?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT);for(let node;(node=iterator.nextNode());)seen.push(node.id);seen.join(',')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "root,two"
        );
    }

    #[test]
    fn generated_selection_preserves_anchor_focus_and_direction() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><p id='one'>one</p><p id='two'>two</p></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root'),s=document.getSelection();s.setBaseAndExtent(root,2,root,0);const backward=s.anchorOffset+'|'+s.focusOffset+'|'+s.direction+'|'+s.getRangeAt(0).startOffset+'|'+s.getRangeAt(0).endOffset;s.extend(root,1);backward+'|'+s.focusOffset+'|'+s.direction",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "2|0|backward|0|2|1|backward"
        );
    }

    #[test]
    fn generated_range_content_operations_mutate_the_rust_dom() {
        initialize_v8();
        let parsed =
            html::parse("<main id='root'><i id='a'>A</i><b id='b'>B</b><u id='c'>C</u></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root'),b=document.getElementById('b');const selected=document.createRange();selected.selectNode(b);const copy=selected.cloneContents();const comparison=selected.compareBoundaryPoints(Range.START_TO_START,selected);const wrapper=document.createElement('span');selected.surroundContents(wrapper);const insertion=document.createRange();insertion.selectNodeContents(root);insertion.collapse(true);const prefix=document.createElement('em');prefix.textContent='P';insertion.insertNode(prefix);const deletion=document.getSelection();const tail=document.createRange();tail.selectNode(document.getElementById('c'));deletion.addRange(tail);deletion.deleteFromDocument();comparison+'|'+copy.firstChild.id+'|'+root.firstChild.tagName+'|'+root.children[2].tagName+'|'+root.textContent",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "0|b|EM|SPAN|PAB"
        );
    }

    #[test]
    fn cross_container_range_extracts_partial_ancestors_and_moves_contained_nodes() {
        initialize_v8();
        let parsed = html::parse(
            "<main id='root'><p id='a'>ab<strong id='kept'>cd</strong>ef</p><p id='b'>gh<em>ij</em>kl</p></main>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const a=document.getElementById('a'),b=document.getElementById('b'),kept=document.getElementById('kept');const r=document.createRange();r.setStart(a.firstChild,1);r.setEnd(b.lastChild,1);const f=r.extractContents();[f.textContent,a.textContent,b.textContent,f.querySelector('#kept')===kept,r.collapsed].join('|')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "bcdefghijk|a|l|true|true"
        );
    }

    #[test]
    fn cross_container_range_clone_delete_and_surround_follow_dom_boundaries() {
        initialize_v8();
        let parsed = html::parse(
            "<main><p id='a'>abc<strong>def</strong></p><p id='b'><em>ghi</em>jkl</p></main>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const a=document.getElementById('a'),b=document.getElementById('b');const r=document.createRange();r.setStart(a.firstChild,1);r.setEnd(b.lastChild,2);const clone=r.cloneContents();let rejected=false;try{r.surroundContents(document.createElement('section'))}catch(e){rejected=String(e).includes('InvalidStateError')}r.deleteContents();[clone.textContent,a.textContent,b.textContent,rejected,r.collapsed].join('|')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "bcdefghijk|a|l|true|true"
        );
    }

    #[test]
    fn generated_character_data_and_editing_state_mutate_the_rust_dom() {
        initialize_v8();
        let parsed = html::parse("<main id='editor' contenteditable><p id='line'>hello</p></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const editor=document.getElementById('editor'),text=document.getElementById('line').firstChild;const tail=text.splitText(2);tail.insertData(0,'!');tail.replaceData(1,2,'YY');text.appendData('?');[editor.isContentEditable,editor.contentEditable,text.data,tail.data,tail.length,tail.substringData(1,2),text instanceof CharacterData].join('|')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "true||he?|!YYo|4|YY|true"
        );
    }

    #[test]
    fn trusted_types_are_unforgeable_and_guard_html_sinks() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.configure_trusted_types(TrustedTypesConfig {
            require_script: true,
            allowed_policy_names: ["sanitizer".into()].into(),
            ..TrustedTypesConfig::default()
        });
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root');let raw=false;try{root.innerHTML='<b>raw</b>'}catch(e){raw=e instanceof TypeError}const policy=trustedTypes.createPolicy('sanitizer',{createHTML:s=>s.replace('unsafe','safe')});const value=policy.createHTML('<b>unsafe</b>');root.innerHTML=value;raw&&trustedTypes instanceof TrustedTypePolicyFactory&&policy instanceof TrustedTypePolicy&&value instanceof TrustedHTML&&trustedTypes.emptyHTML instanceof TrustedHTML&&trustedTypes.isHTML(value)&&!trustedTypes.isHTML({toString(){return '<i>fake</i>'}})&&root.textContent==='safe'",
        ).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        assert_eq!(
            dom.borrow()
                .arena()
                .text_content(dom.borrow().find_by_id("root").unwrap())
                .unwrap(),
            "safe"
        );
    }
}
