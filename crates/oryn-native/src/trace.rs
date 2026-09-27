use oryn_common::v3::{ActionId, PageId, Revision};
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "v8-host", test))]
use std::sync::Weak;
use std::sync::{Arc, Mutex};

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
    CspPolicyInstalled {
        disposition: String,
        source: String,
    },
    CspViolation {
        disposition: String,
        effective_directive: String,
        blocked_uri: String,
    },
    CspReport {
        endpoint: String,
        format: String,
        outcome: String,
    },
    TrustedTypesViolation {
        sink: String,
        required_type: String,
    },
    CookiePolicyDecision {
        included: usize,
        withheld: usize,
        reasons: Vec<String>,
    },
    InsecureRequestUpgraded {
        from: String,
        to: String,
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

// IO diagnostics never acquire page/realm authority. The weak sink cannot
// keep a retired page alive. The bounded mailbox never waits for network IO.
#[cfg(any(feature = "v8-host", test))]
const MAX_PENDING_DIAGNOSTICS: usize = 256;
#[cfg(any(feature = "v8-host", test))]
const MAX_PENDING_DIAGNOSTIC_BYTES: usize = 256 * 1024;

#[derive(Debug, Default)]
struct PendingDiagnostics {
    events: Vec<(Revision, &'static str, String)>,
    #[cfg(any(feature = "v8-host", test))]
    bytes: usize,
    dropped_events: u64,
    dropped_bytes: u64,
    dropped_revision: Option<Revision>,
}

#[cfg(any(feature = "v8-host", test))]
#[derive(Clone, Debug)]
pub(crate) struct DiagnosticSink {
    pending: Weak<Mutex<PendingDiagnostics>>,
    revision: Revision,
}

#[cfg(any(feature = "v8-host", test))]
impl DiagnosticSink {
    pub(crate) fn record(&self, capability: &'static str, detail: String) {
        let Some(pending) = self.pending.upgrade() else {
            return;
        };
        let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
        let bytes = capability.len().saturating_add(detail.len());
        if pending.events.len() >= MAX_PENDING_DIAGNOSTICS
            || bytes > MAX_PENDING_DIAGNOSTIC_BYTES.saturating_sub(pending.bytes)
        {
            // Preserve complete records, never a truncated original error.
            pending.dropped_revision.get_or_insert(self.revision);
            pending.dropped_events = pending.dropped_events.saturating_add(1);
            pending.dropped_bytes = pending.dropped_bytes.saturating_add(bytes as u64);
            return;
        }
        pending.bytes += bytes;
        pending.events.push((self.revision, capability, detail));
    }
}

#[derive(Debug)]
pub struct TraceLog {
    page_id: PageId,
    next_sequence: u64,
    events: Vec<TraceEvent>,
    diagnostics: Arc<Mutex<PendingDiagnostics>>,
}

impl TraceLog {
    pub fn new(page_id: PageId) -> Self {
        Self {
            page_id,
            next_sequence: 0,
            events: Vec::new(),
            diagnostics: Arc::new(Mutex::new(PendingDiagnostics::default())),
        }
    }

    #[cfg(any(feature = "v8-host", test))]
    pub(crate) fn diagnostic_sink(&self, revision: Revision) -> DiagnosticSink {
        DiagnosticSink {
            pending: Arc::downgrade(&self.diagnostics),
            revision,
        }
    }

    fn drain_diagnostics(&mut self) {
        let pending = {
            let mut queue = self
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *queue)
        };
        for (revision, capability, detail) in pending.events {
            self.push_record(
                revision,
                None,
                CausalRelation::Unattributed,
                TraceEventKind::ScriptDiagnostic {
                    capability: capability.into(),
                    detail,
                },
            );
        }
        if pending.dropped_events != 0 {
            self.push_record(
                pending.dropped_revision.unwrap_or(Revision(0)),
                None,
                CausalRelation::Unattributed,
                TraceEventKind::ScriptDiagnostic {
                    capability: "diagnostics.overflow".into(),
                    detail: serde_json::json!({"dropped_events": pending.dropped_events,
                        "dropped_bytes": pending.dropped_bytes})
                    .to_string(),
                },
            );
        }
    }

    /// Record-order sequence does not claim cross-thread execution causality.
    /// Both local and signed-worker trace consumers flush the same mailbox.
    pub(crate) fn snapshot(&mut self) -> Vec<TraceEvent> {
        self.drain_diagnostics();
        self.events.clone()
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
        self.drain_diagnostics();
        self.push_record(revision, action_id, causality, kind);
    }

    fn push_record(
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
    #[test]
    fn asynchronous_diagnostics_keep_original_errors_and_flush_once() {
        let mut trace = TraceLog::new(PageId(9));
        let sink = trace.diagnostic_sink(Revision(4));
        let expected = "broker rejected exact source: α\noriginal detail";
        let thread_sink = sink.clone();
        std::thread::spawn(move || thread_sink.record("navigation.io", expected.into()))
            .join()
            .unwrap();
        trace.push(Revision(5), None, TraceEventKind::ScriptFinished);
        let first = trace.snapshot();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].page_id, PageId(9));
        assert_eq!(first[0].sequence, 0);
        assert_eq!(first[0].revision, Revision(4));
        assert_eq!(first[0].causality, CausalRelation::Unattributed);
        assert!(
            matches!(&first[0].kind, TraceEventKind::ScriptDiagnostic { detail, .. }
            if detail == expected)
        );
        assert_eq!(trace.snapshot(), first);
        drop(trace);
        assert!(sink.pending.upgrade().is_none());
        sink.record("navigation.io", "late result after retirement".into());
    }

    #[test]
    fn asynchronous_diagnostic_overflow_is_explicit_and_does_not_truncate() {
        let mut trace = TraceLog::new(PageId(2));
        let sink = trace.diagnostic_sink(Revision(1));
        for _ in 0..MAX_PENDING_DIAGNOSTICS {
            sink.record("navigation.io", "original".into());
        }
        sink.record("navigation.io", "dropped".into());
        let events = trace.snapshot();
        assert_eq!(events.len(), MAX_PENDING_DIAGNOSTICS + 1);
        assert!(matches!(&events.last().unwrap().kind,
            TraceEventKind::ScriptDiagnostic { capability, detail }
            if capability == "diagnostics.overflow"
                && serde_json::from_str::<serde_json::Value>(detail).unwrap()["dropped_events"] == 1));
        sink.record(
            "navigation.io",
            "z".repeat(MAX_PENDING_DIAGNOSTIC_BYTES + 1),
        );
        assert_eq!(trace.snapshot().len(), events.len() + 1);
        sink.record("navigation.io", "after overflow".into());
        assert_eq!(trace.snapshot().len(), events.len() + 2);
    }
}
