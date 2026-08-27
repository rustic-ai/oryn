use oryn_common::v3::{ActionId, Revision};
use serde::{Deserialize, Serialize};

use crate::runtime::LifecycleState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub sequence: u64,
    pub revision: Revision,
    pub action_id: Option<ActionId>,
    pub kind: TraceEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceEventKind {
    TaskQueued,
    ScriptStarted,
    ScriptFinished,
    ScriptDiagnostic {
        capability: String,
        detail: String,
    },
    Console {
        level: String,
        message: String,
    },
    Mutation {
        summary: String,
    },
    EventDispatched {
        event_type: String,
    },
    NavigationStarted {
        url: String,
    },
    NavigationCommitted {
        url: String,
    },
    RequestStarted {
        method: String,
        url: String,
    },
    ResponseReceived {
        url: String,
        status: u16,
        bytes: usize,
    },
    LifecycleChanged {
        state: LifecycleState,
    },
    PolicyDenied {
        capability: String,
    },
    WorkerTerminated {
        reason: String,
    },
    PageCreated {
        page_id: u64,
        opener_page_id: Option<u64>,
    },
    PageActivated {
        page_id: u64,
    },
    PageClosed {
        page_id: u64,
    },
    OpenerMessage {
        source_page_id: u64,
        target_page_id: u64,
        origin: String,
    },
    StorageChanged {
        area: String,
        origin: String,
        key: Option<String>,
    },
    XhrStateChanged {
        url: String,
        ready_state: u8,
    },
    StreamChunk {
        direction: String,
        bytes: usize,
    },
    WebSocketFrame {
        socket_id: u64,
        direction: String,
        frame_type: String,
        bytes: usize,
    },
}

#[derive(Debug, Default)]
pub struct TraceLog {
    next_sequence: u64,
    events: Vec<TraceEvent>,
}

impl TraceLog {
    pub fn push(&mut self, revision: Revision, action_id: Option<ActionId>, kind: TraceEventKind) {
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            revision,
            action_id,
            kind,
        });
        self.next_sequence += 1;
    }

    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }
}
