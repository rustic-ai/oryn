//! Protocol-neutral host operations used by generated web-platform bindings.
//!
//! This module contains no V8 handles and grants no direct I/O authority. A
//! page host forwards these bounded requests to parent-owned services while
//! generated bindings retain JavaScript object state and correlations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HostOperationId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub credentials: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbObjectStoreDefinition {
    pub name: String,
    pub key_path: Option<String>,
    pub auto_increment: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbIndexDefinition {
    pub name: String,
    pub key_path: String,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbWrite {
    pub store: String,
    pub key: Option<serde_json::Value>,
    pub value: serde_json::Value,
    pub add_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IdbHostCommand {
    Open {
        name: String,
        version: u64,
    },
    DeleteDatabase {
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdbHostRecord {
    pub key: serde_json::Value,
    pub primary_key: serde_json::Value,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IdbHostResponse {
    pub version: Option<u64>,
    pub old_version: Option<u64>,
    pub upgrade_needed: bool,
    pub object_stores: Vec<String>,
    pub key: Option<serde_json::Value>,
    pub value: Option<serde_json::Value>,
    pub values: Vec<serde_json::Value>,
    pub records: Vec<IdbHostRecord>,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PlatformHostRequest {
    HttpStart {
        request: PlatformHttpRequest,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PlatformHostResult {
    HttpStarted {
        stream_id: u64,
        status: u16,
        url: String,
        headers: Vec<(String, String)>,
    },
    HttpChunk {
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
    WebSocketClosed,
    IndexedDb {
        response: IdbHostResponse,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformHostCompletion {
    pub operation_id: HostOperationId,
    pub result: Result<PlatformHostResult, String>,
}

/// Non-blocking authority boundary used by generated bindings.
pub trait PlatformHost {
    fn submit(
        &self,
        operation_id: HostOperationId,
        request: PlatformHostRequest,
    ) -> Result<(), String>;
    fn cancel(&self, operation_id: HostOperationId);
    fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion>;
}
