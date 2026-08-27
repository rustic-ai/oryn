use oryn_common::v2::{ActionId, Revision};
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
