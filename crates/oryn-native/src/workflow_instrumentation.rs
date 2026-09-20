//! Explicit qualification instrumentation. No ambient environment activation and
//! no author-readable state. A shared clock measures actual native OIL futures.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone)]
pub struct WorkflowAudit(Arc<Mutex<State>>);
struct State {
    epoch: Instant,
    next: u64,
    overflow: bool,
    spans: Vec<OperationSpan>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationSpan {
    pub operation_id: u64,
    pub context_id: u64,
    pub input_sha256: String,
    pub start_ns: u128,
    pub end_ns: Option<u128>,
    pub terminal: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    pub schema_version: u32,
    pub overflow: bool,
    pub operations: Vec<OperationSpan>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    DelayedResponse,
    WorkerFailure,
    DuplicateDelivery,
    Cancellation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultPlan {
    pub kind: FaultKind,
    pub before_step: usize,
    pub delay_ms: u64,
    #[serde(default)]
    pub commit_gate: Option<CommitGate>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitGate {
    pub token: String,
    pub phase: String,
    pub marker: std::path::PathBuf,
    pub release: std::path::PathBuf,
}
impl CommitGate {
    pub fn validate(&self) -> Result<(), String> {
        if self.token.is_empty()
            || !matches!(self.phase.as_str(), "before_commit" | "after_commit")
            || !self.marker.is_absolute()
            || !self.release.is_absolute()
            || self.marker == self.release
            || self.marker.exists()
            || self.release.exists()
        {
            return Err(
                "commit gate requires a token, phase and fresh distinct absolute paths".into(),
            );
        }
        Ok(())
    }
}
impl FaultPlan {
    pub fn validate(&self, steps: usize) -> Result<(), String> {
        if self.before_step >= steps || !(1..=30_000).contains(&self.delay_ms) {
            return Err("invalid fault step or bounded delay".into());
        }
        if let Some(gate) = &self.commit_gate {
            if !matches!(
                self.kind,
                FaultKind::Cancellation | FaultKind::WorkerFailure
            ) {
                return Err("commit gates require native interruption".into());
            }
            gate.validate()?;
        }
        Ok(())
    }
}
impl Default for WorkflowAudit {
    fn default() -> Self {
        Self::new()
    }
}
impl WorkflowAudit {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            epoch: Instant::now(),
            next: 1,
            overflow: false,
            spans: Vec::new(),
        })))
    }
    pub fn begin(&self, context_id: u64, input: &str) -> OperationGuard {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let id = state.next;
        state.next = state.next.saturating_add(1);
        let start_ns = state.epoch.elapsed().as_nanos();
        if state.spans.len() < 100_000 {
            state.spans.push(OperationSpan {
                operation_id: id,
                context_id,
                input_sha256: format!("{:x}", Sha256::digest(input.as_bytes())),
                start_ns,
                end_ns: None,
                terminal: None,
            });
        } else {
            state.overflow = true;
        }
        OperationGuard {
            audit: self.clone(),
            id,
            finished: false,
        }
    }
    pub fn snapshot(&self) -> WorkflowSnapshot {
        let state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        WorkflowSnapshot {
            schema_version: 1,
            overflow: state.overflow,
            operations: state.spans.clone(),
        }
    }
    fn end(&self, id: u64, terminal: &str) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let end_ns = state.epoch.elapsed().as_nanos();
        if let Some(span) = state.spans.iter_mut().find(|span| span.operation_id == id)
            && span.end_ns.is_none()
        {
            span.end_ns = Some(end_ns);
            span.terminal = Some(terminal.into());
        }
    }
}
pub struct OperationGuard {
    audit: WorkflowAudit,
    id: u64,
    finished: bool,
}
impl OperationGuard {
    pub fn finish(mut self, success: bool) {
        self.audit
            .end(self.id, if success { "completed" } else { "failed" });
        self.finished = true;
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.audit.end(self.id, "dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropped_operation_is_not_a_completed_or_cancelled_worker() {
        let audit = WorkflowAudit::new();
        let guard = audit.begin(7, "type password secret");
        drop(guard);
        let snapshot = audit.snapshot();
        assert_eq!(snapshot.operations[0].terminal.as_deref(), Some("dropped"));
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("secret"));
    }
    #[test]
    fn shared_clock_and_ids_preserve_overlapping_native_operation_spans() {
        let audit = WorkflowAudit::new();
        let first = audit.begin(1, "observe");
        let second = audit.clone().begin(2, "observe");
        first.finish(true);
        second.finish(false);
        let snapshot = audit.snapshot();
        let a = &snapshot.operations[0];
        let b = &snapshot.operations[1];
        assert_ne!(a.operation_id, b.operation_id);
        assert!(b.start_ns <= a.end_ns.unwrap());
        assert_eq!(b.terminal.as_deref(), Some("failed"));
    }
    #[test]
    fn fault_plans_reject_missing_steps_and_unbounded_delays() {
        assert!(
            FaultPlan {
                kind: FaultKind::Cancellation,
                before_step: 1,
                delay_ms: 1,
                commit_gate: None
            }
            .validate(1)
            .is_err()
        );
        assert!(
            FaultPlan {
                kind: FaultKind::WorkerFailure,
                before_step: 0,
                delay_ms: 30_001,
                commit_gate: None
            }
            .validate(1)
            .is_err()
        );
    }
}
