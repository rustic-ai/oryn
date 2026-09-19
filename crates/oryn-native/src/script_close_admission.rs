//! Parent-owned close admission identities and bounded terminal evidence.
//! A target departure and its physical cleanup share one immutable token.
use super::{PageId, RuntimeError};
use oryn_js::worker_wire::Creator;
use std::collections::{BTreeMap, VecDeque};

const MAX_ACTIVE: usize = 256;
const MAX_TERMINAL: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Querying,
    Preparing(Creator),
    Closing,
    Canceled,
    Closed,
    Failed(String),
}
#[derive(Clone, Debug)]
pub(super) struct Admission {
    pub token: u64,
    pub source: PageId,
    pub creator: Creator,
    pub target: PageId,
    pub phase: Phase,
}
#[derive(Default)]
pub(super) struct Registry {
    next: u64,
    active: BTreeMap<PageId, Admission>,
    terminal: VecDeque<Admission>,
    pruned: u64,
}
fn invalid(message: &str) -> RuntimeError {
    RuntimeError::ProtocolViolation(message.into())
}
impl Registry {
    /// A concurrent close of the same traversable joins the existing native
    /// job; it never replaces the source or restarts its beforeunload tree.
    pub fn admit(
        &mut self,
        source: PageId,
        creator: Creator,
        target: PageId,
    ) -> Result<(Admission, bool), RuntimeError> {
        if let Some(existing) = self.active.get(&target) {
            return Ok((existing.clone(), false));
        }
        if self.active.len() >= MAX_ACTIVE {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending script-close admissions".into(),
            });
        }
        let token = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("script-close identity exhausted"))?;
        let admission = Admission {
            token,
            source,
            creator,
            target,
            phase: Phase::Querying,
        };
        self.next = token;
        self.active.insert(target, admission.clone());
        Ok((admission, true))
    }
    pub fn active_snapshot(&self) -> Vec<Admission> {
        self.active.values().cloned().collect()
    }
    pub fn get(&self, target: PageId, token: u64) -> Option<&Admission> {
        self.active
            .get(&target)
            .filter(|entry| entry.token == token)
    }
    pub fn preparing(
        &mut self,
        target: PageId,
        token: u64,
        creator: Creator,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .active
            .get_mut(&target)
            .filter(|entry| entry.token == token)
            .ok_or_else(|| invalid("obsolete script-close admission"))?;
        match &entry.phase {
            Phase::Querying => entry.phase = Phase::Preparing(creator),
            Phase::Preparing(previous) if previous == &creator => {}
            _ => return Err(invalid("script-close target identity changed")),
        }
        Ok(())
    }
    pub fn closing(
        &mut self,
        target: PageId,
        token: u64,
        creator: &Creator,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .active
            .get_mut(&target)
            .filter(|entry| entry.token == token)
            .ok_or_else(|| invalid("obsolete script-close admission"))?;
        match &entry.phase {
            Phase::Preparing(expected) if expected == creator => entry.phase = Phase::Closing,
            Phase::Closing => {}
            _ => {
                return Err(invalid(
                    "script-close departure has no matching target proof",
                ));
            }
        }
        Ok(())
    }
    pub fn finish(&mut self, target: PageId, token: u64, phase: Phase) -> Result<(), RuntimeError> {
        if !matches!(phase, Phase::Canceled | Phase::Closed | Phase::Failed(_)) {
            return Err(invalid("script-close outcome is not terminal"));
        }
        let previous = self
            .get(target, token)
            .ok_or_else(|| invalid("obsolete script-close terminal receipt"))?;
        if matches!(phase, Phase::Closed) && previous.phase != Phase::Closing {
            return Err(invalid("script-close completion preceded departure"));
        }
        let mut entry = self
            .active
            .remove(&target)
            .expect("validated admission under same borrow");
        entry.phase = match phase {
            Phase::Failed(message) => Phase::Failed(message.chars().take(2048).collect()),
            other => other,
        };
        self.terminal.push_back(entry);
        if self.terminal.len() > MAX_TERMINAL {
            self.terminal.pop_front();
            self.pruned = self.pruned.saturating_add(1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn creator(generation: u64) -> Creator {
        Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        }
    }
    #[test]
    fn concurrent_close_keeps_original_job_and_replacement_cannot_ack_it() {
        let mut registry = Registry::default();
        let (first, fresh) = registry.admit(PageId(1), creator(1), PageId(2)).unwrap();
        assert!(fresh);
        let (same, fresh) = registry.admit(PageId(3), creator(3), PageId(2)).unwrap();
        assert!(!fresh);
        assert_eq!(same.token, first.token);
        assert_eq!(same.source, PageId(1));
        assert_eq!(same.creator, creator(1));
        registry
            .preparing(PageId(2), first.token, creator(5))
            .unwrap();
        assert!(
            registry
                .preparing(PageId(2), first.token, creator(6))
                .is_err()
        );
        assert!(
            registry
                .closing(PageId(2), first.token, &creator(6))
                .is_err()
        );
        assert!(
            registry
                .finish(PageId(2), first.token, Phase::Closed)
                .is_err()
        );
        registry
            .closing(PageId(2), first.token, &creator(5))
            .unwrap();
        registry
            .finish(PageId(2), first.token, Phase::Closed)
            .unwrap();
        let (next, _) = registry.admit(PageId(3), creator(3), PageId(2)).unwrap();
        assert!(next.token > first.token);
        assert!(
            registry
                .finish(PageId(2), first.token, Phase::Canceled)
                .is_err()
        );
        assert!(registry.get(PageId(2), next.token).is_some());
    }
    #[test]
    fn cancellation_releases_admission_and_terminal_history_is_explicitly_bounded() {
        let mut registry = Registry::default();
        for i in 0..MAX_TERMINAL + 20 {
            let (entry, _) = registry.admit(PageId(1), creator(1), PageId(2)).unwrap();
            registry
                .finish(entry.target, entry.token, Phase::Canceled)
                .unwrap();
            assert!(registry.active.is_empty());
            assert_eq!(registry.next, i as u64 + 1);
        }
        assert_eq!(registry.terminal.len(), MAX_TERMINAL);
        assert_eq!(registry.pruned, 20);
        registry.next = u64::MAX;
        assert!(registry.admit(PageId(1), creator(1), PageId(2)).is_err());
        assert!(registry.active.is_empty());
    }
}
