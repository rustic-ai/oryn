use oryn_common::v3::{ActionId, PageId, Revision};
use serde::{Deserialize, Serialize};

use crate::runtime::LifecycleState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub sequence: u64,
    #[serde(default)]
    pub logical_time: u64,
    #[serde(default)]
    pub page_id: PageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
    pub revision: Revision,
    pub action_id: Option<ActionId>,
    /// Strength of the action-to-effect claim. Events remain unattributed
    /// unless the runtime can establish the relationship at the scheduling
    /// boundary; temporal proximity alone is never deterministic causality.
    #[serde(default, skip_serializing_if = "CausalRelation::is_unattributed")]
    pub causality: CausalRelation,
    pub kind: TraceEventKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalRelation {
    #[default]
    Unattributed,
    Direct,
    Derived,
    Correlated,
}

impl CausalRelation {
    const fn is_unattributed(&self) -> bool {
        matches!(self, Self::Unattributed)
    }
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
    ChallengeAssessed {
        disposition: crate::challenge::ChallengeDisposition,
        provider: Option<String>,
        signals: Vec<String>,
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
    StyleRecalculated {
        revision: u64,
        nodes: usize,
    },
    LayoutCompleted {
        revision: u64,
        nodes: usize,
    },
    ViewportChanged {
        width: f64,
        height: f64,
        device_pixel_ratio: f64,
    },
    Scrolled {
        x: f64,
        y: f64,
    },
    InteractabilityRejected {
        target: u64,
        reason: String,
    },
    FontLoaded {
        family: String,
        sha256: String,
    },
}

#[derive(Debug)]
pub struct TraceLog {
    page_id: PageId,
    next_sequence: u64,
    events: Vec<TraceEvent>,
}

impl TraceLog {
    pub fn new(page_id: PageId) -> Self {
        Self {
            page_id,
            next_sequence: 0,
            events: Vec::new(),
        }
    }

    pub fn push(&mut self, revision: Revision, action_id: Option<ActionId>, kind: TraceEventKind) {
        self.push_causal(revision, action_id, CausalRelation::Unattributed, kind);
    }

    pub fn push_causal(
        &mut self,
        revision: Revision,
        action_id: Option<ActionId>,
        causality: CausalRelation,
        kind: TraceEventKind,
    ) {
        self.events.push(TraceEvent {
            sequence: self.next_sequence,
            logical_time: self.next_sequence,
            page_id: self.page_id,
            frame_id: None,
            revision,
            action_id,
            causality,
            kind,
        });
        self.next_sequence += 1;
    }

    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }
}

impl Default for TraceLog {
    fn default() -> Self {
        Self::new(PageId::LEGACY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn causal_strength_is_explicit_and_unattributed_is_not_promoted() {
        let mut trace = TraceLog::new(PageId(7));
        trace.push(
            Revision(1),
            Some(ActionId(3)),
            TraceEventKind::Console {
                level: "log".into(),
                message: "later work".into(),
            },
        );
        trace.push_causal(
            Revision(1),
            Some(ActionId(3)),
            CausalRelation::Direct,
            TraceEventKind::EventDispatched {
                event_type: "click".into(),
            },
        );

        assert_eq!(trace.events()[0].causality, CausalRelation::Unattributed);
        assert_eq!(trace.events()[1].causality, CausalRelation::Direct);
        let encoded = serde_json::to_value(trace.events()).unwrap();
        assert!(encoded[0].get("causality").is_none());
        assert_eq!(encoded[1]["causality"], "direct");
    }
}
