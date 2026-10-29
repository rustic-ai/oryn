//! Ordered, bounded ownership for native rejection notification tasks.
//! Payload and environment policy are supplied by the V8 producer; this ledger
//! never turns a canceled diagnostic into an implicit handler or reuses an ID.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectionPhase {
    Fresh,
    NotifyQueued,
    Dispatching,
    Outstanding,
    HandledQueued,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectionAdmissionError {
    Capacity,
    Allocation,
    IdentityExhausted,
}

pub(crate) struct RejectionEntry<O, P> {
    pub(crate) id: u64,
    pub(crate) owner: O,
    pub(crate) payload: P,
    pub(crate) phase: RejectionPhase,
}

pub(crate) struct RejectionLedger<O, P> {
    entries: Vec<RejectionEntry<O, P>>,
    maximum: usize,
    next: u64,
}

impl<O: PartialEq, P> RejectionLedger<O, P> {
    pub(crate) fn new(maximum: usize) -> Result<Self, RejectionAdmissionError> {
        let mut entries = Vec::new();
        // Reserve physical storage once; a logical entry bound must not hide
        // allocator failure or silently spill into an unbounded fallback map.
        entries.try_reserve_exact(maximum).map_err(|_| RejectionAdmissionError::Allocation)?;
        Ok(Self { entries, maximum, next: 0 })
    }

    pub(crate) fn admit(&mut self, owner: O, payload: P) -> Result<u64, RejectionAdmissionError> {
        if self.entries.len() >= self.maximum {
            return Err(RejectionAdmissionError::Capacity);
        }
        let id = self.next.checked_add(1).ok_or(RejectionAdmissionError::IdentityExhausted)?;
        self.next = id;
        self.entries.push(RejectionEntry { id, owner, payload, phase: RejectionPhase::Fresh });
        Ok(id)
    }

    pub(crate) fn entry(&self, id: u64) -> Option<&RejectionEntry<O, P>> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub(crate) fn checkpoint(&mut self, owner: &O) -> Result<Vec<u64>, RejectionAdmissionError> {
        let count = self.entries.iter().filter(|entry|
            &entry.owner == owner && entry.phase == RejectionPhase::Fresh).count();
        let mut tasks = Vec::new();
        tasks.try_reserve_exact(count).map_err(|_| RejectionAdmissionError::Allocation)?;
        for entry in &mut self.entries {
            if &entry.owner == owner && entry.phase == RejectionPhase::Fresh {
                entry.phase = RejectionPhase::NotifyQueued;
                tasks.push(entry.id);
            }
        }
        Ok(tasks)
    }

    pub(crate) fn begin_dispatch(&mut self, id: u64, owner: &O) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else { return false };
        if &entry.owner != owner || entry.phase != RejectionPhase::NotifyQueued { return false }
        entry.phase = RejectionPhase::Dispatching;
        true
    }

    /// Returns whether a late handled task must be queued. Early and dispatch-
    /// reentrant handling removes the record before outstanding publication.
    pub(crate) fn handler_added(&mut self, id: u64) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else { return false };
        match self.entries[index].phase {
            RejectionPhase::Outstanding => {
                self.entries[index].phase = RejectionPhase::HandledQueued;
                true
            }
            RejectionPhase::HandledQueued => false,
            _ => { self.entries.remove(index); false }
        }
    }

    /// Diagnostic cancellation does not imply handling. The caller rechecks
    /// the Promise and live owner after author callbacks before this publication.
    pub(crate) fn finish_dispatch(&mut self, id: u64, owner: &O, still_unhandled: bool) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else { return false };
        if &self.entries[index].owner != owner || self.entries[index].phase != RejectionPhase::Dispatching {
            return false;
        }
        if !still_unhandled { self.entries.remove(index); return false }
        self.entries[index].phase = RejectionPhase::Outstanding;
        true
    }

    pub(crate) fn take_handled(&mut self, id: u64, owner: &O) -> Option<P> {
        let index = self.entries.iter().position(|entry|
            entry.id == id && &entry.owner == owner && entry.phase == RejectionPhase::HandledQueued)?;
        Some(self.entries.remove(index).payload)
    }

    pub(crate) fn retire(&mut self, owner: &O) {
        self.entries.retain(|entry| &entry.owner != owner);
    }

    pub(crate) fn retain_payloads(&mut self, mut live: impl FnMut(&P) -> bool) {
        self.entries.retain(|entry| live(&entry.payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_lifecycle_early_and_dispatch_reentrant_catches_suppress_late_events() {
        for catch_in_dispatch in [false, true] {
            let mut ledger = RejectionLedger::new(2).unwrap();
            let id = ledger.admit((1, 4), 42).unwrap();
            if catch_in_dispatch {
                assert_eq!(ledger.checkpoint(&(1, 4)).unwrap(), [id]);
                assert!(ledger.begin_dispatch(id, &(1, 4)));
            }
            assert!(!ledger.handler_added(id));
            assert!(!ledger.finish_dispatch(id, &(1, 4), true));
            assert!(ledger.entry(id).is_none());
            assert!(ledger.take_handled(id, &(1, 4)).is_none());
        }
    }

    #[test]
    fn rejection_lifecycle_checkpoint_preserves_order_and_owner_epoch() {
        let mut ledger = RejectionLedger::new(4).unwrap();
        let a = ledger.admit((1, 1), 0).unwrap();
        let other = ledger.admit((1, 2), 1).unwrap();
        let b = ledger.admit((1, 1), 2).unwrap();
        assert_eq!(ledger.checkpoint(&(1, 1)).unwrap(), [a, b]);
        assert!(ledger.checkpoint(&(1, 1)).unwrap().is_empty());
        assert!(!ledger.begin_dispatch(a, &(1, 2)));
        assert_eq!(ledger.checkpoint(&(1, 2)).unwrap(), [other]);
        ledger.retire(&(1, 1));
        assert!(ledger.entry(a).is_none() && ledger.entry(b).is_none());
        assert!(ledger.entry(other).is_some());
    }

    #[test]
    fn rejection_lifecycle_late_handling_is_one_separate_task_with_original_payload() {
        let mut ledger = RejectionLedger::new(1).unwrap();
        let id = ledger.admit(7, "reason").unwrap();
        ledger.checkpoint(&7).unwrap();
        assert!(ledger.begin_dispatch(id, &7));
        assert!(ledger.finish_dispatch(id, &7, true));
        // Even when author preventDefault suppresses diagnostics, the caller
        // uses this same transition, rather than deleting an unhandled record.
        assert!(ledger.handler_added(id));
        assert!(!ledger.handler_added(id));
        assert!(ledger.take_handled(id, &8).is_none());
        assert_eq!(ledger.take_handled(id, &7), Some("reason"));
        assert!(ledger.take_handled(id, &7).is_none());
    }

    #[test]
    fn rejection_lifecycle_capacity_and_weak_collection_do_not_reuse_identity() {
        let mut ledger = RejectionLedger::new(1).unwrap();
        let id = ledger.admit(1, false).unwrap();
        assert_eq!(ledger.admit(1, true), Err(RejectionAdmissionError::Capacity));
        ledger.retain_payloads(|live| *live);
        let next = ledger.admit(1, true).unwrap();
        assert!(next > id);
        ledger.next = u64::MAX;
        ledger.retire(&1);
        assert_eq!(ledger.admit(1, true), Err(RejectionAdmissionError::IdentityExhausted));
        assert_eq!(RejectionLedger::<u64, u64>::new(usize::MAX).err(), Some(RejectionAdmissionError::Allocation));
    }
}
