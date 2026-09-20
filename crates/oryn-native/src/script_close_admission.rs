//! Parent-owned close admission identities and bounded terminal evidence.
//! A target departure and its physical cleanup share one immutable token.
use super::{PageId, RuntimeError};
use oryn_js::worker_wire::Creator;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const MAX_ACTIVE: usize = 256;
const MAX_TERMINAL: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Querying,
    Preparing(Creator),
    Closing,
    Canceled,
    Closed,
}
#[derive(Clone, Debug)]
pub(super) struct Admission {
    pub token: u64,
    pub source: PageId,
    pub creator: Creator,
    pub target: PageId,
    pub phase: Phase,
    completion: Arc<AtomicU8>,
}
#[derive(Default)]
pub(super) struct Registry {
    next: u64,
    active: BTreeMap<PageId, Admission>,
    terminal: VecDeque<Admission>,
    pruned: u64,
    actions: BTreeMap<PageId, (u64, Vec<Admission>)>,
    action_results: BTreeMap<(PageId, u64), Vec<Admission>>,
}
fn invalid(message: &str) -> RuntimeError {
    RuntimeError::ProtocolViolation(message.into())
}
impl Registry {
    pub fn begin_action(&mut self, page: PageId, action: u64) -> Result<(), RuntimeError> {
        if self.actions.contains_key(&page) || self.action_results.contains_key(&(page, action)) {
            return Err(invalid("native action close scope already owned"));
        }
        if self.actions.len() + self.action_results.len() >= MAX_ACTIVE {
            return Err(invalid("native action close receipt capacity"));
        }
        self.actions.insert(page, (action, Vec::new()));
        Ok(())
    }
    pub fn record_action_close(
        &mut self,
        source: PageId,
        target: PageId,
    ) -> Result<(), RuntimeError> {
        let Some((_, receipts)) = self.actions.get_mut(&source) else {
            return Ok(());
        };
        let admission = self.active.get(&target).or_else(|| {
            self.terminal
                .iter()
                .rev()
                .find(|entry| entry.target == target)
        });
        let Some(admission) = admission else {
            return Ok(());
        };
        if !receipts
            .iter()
            .any(|receipt| receipt.token == admission.token)
        {
            if receipts.len() >= MAX_ACTIVE {
                return Err(invalid("native action close receipt capacity"));
            }
            receipts.push(admission.clone());
        }
        Ok(())
    }
    pub fn end_action(
        &mut self,
        page: PageId,
        action: u64,
        retain: bool,
    ) -> Result<(), RuntimeError> {
        if !self.actions.get(&page).is_some_and(|(id, _)| *id == action) {
            return Err(invalid("native action close scope identity mismatch"));
        }
        let (_, receipts) = self.actions.remove(&page).expect("validated scope");
        if retain && !receipts.is_empty() {
            self.action_results.insert((page, action), receipts);
        }
        Ok(())
    }
    pub fn poll_action(&mut self, page: PageId, action: u64) -> Option<Vec<PageId>> {
        let Some(receipts) = self.action_results.get(&(page, action)) else {
            return Some(Vec::new());
        };
        if receipts
            .iter()
            .any(|receipt| receipt.completion.load(Ordering::Acquire) == 0)
        {
            return None;
        }
        let receipts = self
            .action_results
            .remove(&(page, action))
            .expect("retained receipts");
        Some(
            receipts
                .into_iter()
                .filter(|receipt| receipt.completion.load(Ordering::Acquire) == 2)
                .map(|receipt| receipt.target)
                .collect(),
        )
    }
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
            completion: Arc::new(AtomicU8::new(0)),
        };
        self.next = token;
        self.active.insert(target, admission.clone());
        Ok((admission, true))
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(super) fn diagnostic_phase(&self, target: PageId) -> Option<(u64, &'static str)> {
        let entry = self.active.get(&target).or_else(|| {
            self.terminal
                .iter()
                .rev()
                .find(|entry| entry.target == target)
        })?;
        let phase = match &entry.phase {
            Phase::Querying => "querying",
            Phase::Preparing(_) => "preparing",
            Phase::Closing => "closing",
            Phase::Canceled => "canceled",
            Phase::Closed => "closed",
        };
        Some((entry.token, phase))
    }
    pub fn active_snapshot(&self) -> Vec<Admission> {
        self.active.values().cloned().collect()
    }
    pub fn get(&self, target: PageId, token: u64) -> Option<&Admission> {
        self.active
            .get(&target)
            .filter(|entry| entry.token == token)
    }
    /// A runner owns the immutable admission, not fresh authority from its
    /// source document. Source retirement cannot revoke accepted departure.
    pub fn phase_for(&self, admission: &Admission) -> Option<Phase> {
        self.get(admission.target, admission.token)
            .filter(|stored| {
                stored.source == admission.source && stored.creator == admission.creator
            })
            .map(|stored| stored.phase.clone())
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
        if !matches!(phase, Phase::Canceled | Phase::Closed) {
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
        // Query/departure/cleanup errors retain the active admission for retry.
        // Only cancellation or acknowledged physical completion is terminal.
        entry.completion.store(
            if phase == Phase::Closed { 2 } else { 1 },
            Ordering::Release,
        );
        entry.phase = phase;
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
    #[test]
    fn action_receipts_wait_for_physical_completion_and_survive_terminal_pruning() {
        let mut registry = Registry::default();
        registry.begin_action(PageId(1), 7).unwrap();
        let (close, _) = registry.admit(PageId(1), creator(1), PageId(2)).unwrap();
        registry.record_action_close(PageId(1), PageId(2)).unwrap();
        registry.record_action_close(PageId(1), PageId(2)).unwrap();
        registry.end_action(PageId(1), 7, true).unwrap();
        assert!(registry.poll_action(PageId(1), 7).is_none());
        registry
            .preparing(PageId(2), close.token, creator(2))
            .unwrap();
        registry
            .closing(PageId(2), close.token, &creator(2))
            .unwrap();
        assert!(
            registry.poll_action(PageId(1), 7).is_none(),
            "departure is not physical closure"
        );
        registry
            .finish(PageId(2), close.token, Phase::Closed)
            .unwrap();
        for id in 3..100 {
            let (other, _) = registry.admit(PageId(9), creator(1), PageId(id)).unwrap();
            registry
                .finish(PageId(id), other.token, Phase::Canceled)
                .unwrap();
        }
        assert_eq!(registry.poll_action(PageId(1), 7), Some(vec![PageId(2)]));
        assert_eq!(
            registry.poll_action(PageId(1), 7),
            Some(vec![]),
            "receipt is consumed once"
        );
    }
    #[test]
    fn action_receipts_isolate_sources_and_do_not_report_canceled_close() {
        let mut registry = Registry::default();
        registry.begin_action(PageId(1), 1).unwrap();
        assert!(registry.begin_action(PageId(1), 2).is_err());
        let (other, _) = registry.admit(PageId(3), creator(1), PageId(4)).unwrap();
        registry.record_action_close(PageId(3), PageId(4)).unwrap();
        registry.end_action(PageId(1), 1, true).unwrap();
        assert_eq!(registry.poll_action(PageId(1), 1), Some(vec![]));
        registry.begin_action(PageId(1), 2).unwrap();
        registry.record_action_close(PageId(1), PageId(4)).unwrap();
        assert!(registry.end_action(PageId(1), 3, true).is_err());
        registry.end_action(PageId(1), 2, true).unwrap();
        assert!(registry.poll_action(PageId(1), 2).is_none());
        registry
            .finish(PageId(4), other.token, Phase::Canceled)
            .unwrap();
        assert_eq!(registry.poll_action(PageId(1), 2), Some(vec![]));
    }
    #[test]
    fn failed_actions_release_receipts_without_canceling_admitted_close() {
        let mut registry = Registry::default();
        registry.begin_action(PageId(1), 1).unwrap();
        let (close, _) = registry.admit(PageId(1), creator(1), PageId(2)).unwrap();
        registry.record_action_close(PageId(1), PageId(2)).unwrap();
        registry.end_action(PageId(1), 1, false).unwrap();
        assert_eq!(registry.poll_action(PageId(1), 1), Some(vec![]));
        assert!(registry.get(PageId(2), close.token).is_some());
        for action in 2..300 {
            registry.begin_action(PageId(1), action).unwrap();
            registry.end_action(PageId(1), action, true).unwrap();
        }
    }
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
    fn runner_identity_keeps_original_source_but_observes_current_target_phase() {
        let mut registry = Registry::default();
        let (admission, _) = registry.admit(PageId(1), creator(7), PageId(2)).unwrap();
        assert_eq!(registry.phase_for(&admission), Some(Phase::Querying));
        let mut forged = admission.clone();
        forged.source = PageId(3);
        assert_eq!(registry.phase_for(&forged), None);
        forged = admission.clone();
        forged.creator = creator(8);
        assert_eq!(registry.phase_for(&forged), None);
        forged = admission.clone();
        forged.token += 1;
        assert_eq!(registry.phase_for(&forged), None);
        forged = admission.clone();
        forged.target = PageId(3);
        assert_eq!(registry.phase_for(&forged), None);
        registry
            .preparing(admission.target, admission.token, creator(12))
            .unwrap();
        assert_eq!(
            registry.phase_for(&admission),
            Some(Phase::Preparing(creator(12)))
        );
        assert!(
            registry
                .finish(admission.target, admission.token, Phase::Querying)
                .is_err()
        );
        assert!(
            registry
                .finish(
                    admission.target,
                    admission.token,
                    Phase::Preparing(creator(12))
                )
                .is_err()
        );
        assert!(
            registry
                .finish(admission.target, admission.token, Phase::Closing)
                .is_err()
        );
        assert_eq!(
            registry.phase_for(&admission),
            Some(Phase::Preparing(creator(12))),
            "nonterminal outcomes never release accepted custody"
        );
        registry
            .finish(admission.target, admission.token, Phase::Canceled)
            .unwrap();
        assert_eq!(registry.phase_for(&admission), None);
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
