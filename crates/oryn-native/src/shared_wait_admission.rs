//! Atomic wait admission plus compact per-native-lane cancellation tombstones.
use super::*;
use oryn_js::shared_memory::{SharedWaitResult, WaitAdmissionId};
const WINDOW: u64 = 1024;
#[derive(Clone)]
struct Admission {
    descriptor: SharedMemoryDescriptor,
    acquire: bool,
    offset: u64,
    width64: bool,
    expected: u64,
    timeout_ns: Option<u64>,
    result: SharedWaitResult,
}
#[derive(Default)]
pub(super) struct Lane {
    completed: u64,
    cancelled: BTreeSet<u64>,
    active: BTreeMap<u64, Admission>,
}
impl Lane {
    fn validate(&self, id: WaitAdmissionId) -> Result<(), String> {
        if id.lane == [0; 32]
            || id.sequence == 0
            || id.sequence > self.completed.saturating_add(WINDOW)
        {
            return Err("invalid or excessive wait admission sequence".into());
        }
        Ok(())
    }
    fn cancelled(&self, sequence: u64) -> bool {
        sequence <= self.completed || self.cancelled.contains(&sequence)
    }
    fn complete(&mut self, sequence: u64) {
        if sequence <= self.completed {
            return;
        }
        self.cancelled.insert(sequence);
        while self
            .completed
            .checked_add(1)
            .is_some_and(|next| self.cancelled.remove(&next))
        {
            self.completed += 1;
        }
    }
}
impl SharedMemoryRegistry {
    pub(crate) fn admit_wait(
        &mut self,
        owner: &AgentOwner,
        id: WaitAdmissionId,
        descriptor: SharedMemoryDescriptor,
        acquire: bool,
        offset: u64,
        width64: bool,
        expected: u64,
        timeout_ns: Option<u64>,
    ) -> Result<SharedWaitResult, String> {
        // Revalidate under the same registry lock as retirement; the outer
        // native document authority check alone leaves a commit/retire race.
        if !self.isolated(owner) {
            return Err("wait creator retired".into());
        }
        let key = (owner.clone(), id.lane);
        if !self.admissions.contains_key(&key) && self.admissions.len() >= 4096 {
            return Err("wait admission lane quota".into());
        }
        let lane = self.admissions.entry(key.clone()).or_default();
        lane.validate(id)?;
        if lane.cancelled(id.sequence) {
            return Ok(SharedWaitResult::Cancelled);
        }
        if let Some(saved) = lane.active.get(&id.sequence) {
            if saved.descriptor != descriptor
                || saved.acquire != acquire
                || saved.offset != offset
                || saved.width64 != width64
                || saved.expected != expected
                || saved.timeout_ns != timeout_ns
            {
                return Err("wait admission identity reused with different arguments".into());
            }
            return Ok(saved.result);
        }
        if !acquire && self.backing(owner, descriptor.token)?.descriptor != descriptor {
            return Err("wait admission descriptor mismatch".into());
        }
        if acquire {
            drop(self.open(owner, descriptor)?);
        }
        let result = match self.wait_enqueue(
            owner,
            descriptor.token,
            offset,
            width64,
            expected,
            timeout_ns,
        ) {
            Ok(result) => result,
            Err(error) => {
                if acquire {
                    self.release(owner, descriptor.token)?;
                }
                self.admissions.get_mut(&key).unwrap().complete(id.sequence);
                return Err(error);
            }
        };
        self.admissions.get_mut(&key).unwrap().active.insert(
            id.sequence,
            Admission {
                descriptor,
                acquire,
                offset,
                width64,
                expected,
                timeout_ns,
                result,
            },
        );
        Ok(result)
    }
    pub(crate) fn cancel_wait_admission(
        &mut self,
        owner: &AgentOwner,
        id: WaitAdmissionId,
    ) -> Result<(), String> {
        let key = (owner.clone(), id.lane);
        // Retired owners cannot admit again. Their native retirement already
        // disposed all admission holders and waiters, so acknowledgement is safe.
        if !self.isolated(owner) && !self.admissions.contains_key(&key) {
            return Ok(());
        }
        if !self.admissions.contains_key(&key) && self.admissions.len() >= 4096 {
            return Err("wait admission lane quota".into());
        }
        let lane = self.admissions.entry(key.clone()).or_default();
        lane.validate(id)?;
        if lane.cancelled(id.sequence) {
            return Ok(());
        }
        if let Some(saved) = lane.active.get(&id.sequence).cloned() {
            if let SharedWaitResult::Pending(waiter) = saved.result {
                self.waits.cancel_if_present(owner, waiter)?;
            }
            if saved.acquire {
                self.release(owner, saved.descriptor.token)?;
            }
        }
        let lane = self.admissions.get_mut(&key).unwrap();
        lane.active.remove(&id.sequence);
        lane.complete(id.sequence);
        Ok(())
    }
    pub(super) fn retire_wait_admissions(&mut self, owner: &AgentOwner) {
        let keys: Vec<_> = self
            .admissions
            .keys()
            .filter(|(owned, _)| owned == owner)
            .cloned()
            .collect();
        for key in keys {
            if let Some(lane) = self.admissions.remove(&key) {
                for (_, saved) in lane.active {
                    if let SharedWaitResult::Pending(waiter) = saved.result {
                        let _ = self.waits.cancel_if_present(owner, waiter);
                    }
                    if saved.acquire {
                        let _ = self.release(owner, saved.descriptor.token);
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::worker_wire::AgentId;
    fn id(sequence: u64) -> WaitAdmissionId {
        WaitAdmissionId {
            lane: [9; 32],
            sequence,
        }
    }
    #[test]
    fn cancellation_before_admission_and_lost_reply_replay_never_duplicate_ownership() {
        let owner = AgentOwner::Agent(AgentId(1));
        let receiver = AgentOwner::Agent(AgentId(2));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        registry
            .admit_child(&owner, receiver.clone(), "origin")
            .unwrap();
        let (descriptor, _) = registry
            .allocate(&owner, 4, None, 4096, 0, false, false)
            .unwrap();
        registry.cancel_wait_admission(&receiver, id(1)).unwrap();
        assert_eq!(
            registry
                .admit_wait(&receiver, id(1), descriptor, true, 0, false, 0, None)
                .unwrap(),
            SharedWaitResult::Cancelled
        );
        assert!(
            !registry.backings[&descriptor.token]
                .holders
                .contains_key(&receiver)
        );
        let first = registry
            .admit_wait(&receiver, id(2), descriptor, true, 0, false, 0, None)
            .unwrap();
        assert_eq!(
            registry
                .admit_wait(&receiver, id(2), descriptor, true, 0, false, 0, None)
                .unwrap(),
            first
        );
        assert_eq!(registry.backings[&descriptor.token].holders[&receiver], 1);
        registry.cancel_wait_admission(&receiver, id(2)).unwrap();
        registry.cancel_wait_admission(&receiver, id(2)).unwrap();
        assert!(
            !registry.backings[&descriptor.token]
                .holders
                .contains_key(&receiver)
        );
        assert_eq!(
            registry.admissions[&(receiver.clone(), id(1).lane)].completed,
            2
        );
        assert!(
            registry.admissions[&(receiver.clone(), id(1).lane)]
                .cancelled
                .is_empty()
        );
        assert_eq!(
            registry
                .admit_wait(&receiver, id(2), descriptor, true, 0, false, 0, None)
                .unwrap(),
            SharedWaitResult::Cancelled
        );
    }
    #[test]
    fn retirement_disposes_admission_alias_but_preserves_original_physical_mapping() {
        let owner = AgentOwner::Agent(AgentId(3));
        let receiver = AgentOwner::Agent(AgentId(4));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        registry
            .admit_child(&owner, receiver.clone(), "origin")
            .unwrap();
        let (descriptor, _) = registry
            .allocate(&owner, 4, None, 4096, 0, false, false)
            .unwrap();
        registry
            .admit_wait(&receiver, id(1), descriptor, true, 0, false, 0, None)
            .unwrap();
        registry.retire(&receiver);
        assert_eq!(registry.backings[&descriptor.token].holders[&owner], 1);
        assert!(
            !registry.backings[&descriptor.token]
                .holders
                .contains_key(&receiver)
        );
        registry.cancel_wait_admission(&receiver, id(1)).unwrap();
        assert!(
            registry
                .admit_wait(&receiver, id(1), descriptor, true, 0, false, 0, None)
                .is_err()
        );
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use oryn_js::worker_wire::AgentId;
    #[test]
    fn out_of_order_cleanup_compacts_only_after_all_earlier_admissions_are_cancelled() {
        let owner = AgentOwner::Agent(AgentId(21));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        let (descriptor, _) = registry
            .allocate(&owner, 4, None, 4096, 0, false, false)
            .unwrap();
        let id = |sequence| WaitAdmissionId {
            lane: [21; 32],
            sequence,
        };
        let earlier = registry
            .admit_wait(&owner, id(1), descriptor, false, 0, false, 0, None)
            .unwrap();
        registry.cancel_wait_admission(&owner, id(2)).unwrap();
        assert_eq!(
            registry.admissions[&(owner.clone(), id(1).lane)].completed,
            0
        );
        assert_eq!(
            registry
                .admit_wait(&owner, id(1), descriptor, false, 0, false, 0, None)
                .unwrap(),
            earlier
        );
        assert_eq!(
            registry
                .admit_wait(&owner, id(2), descriptor, false, 0, false, 0, None)
                .unwrap(),
            SharedWaitResult::Cancelled
        );
        registry.cancel_wait_admission(&owner, id(1)).unwrap();
        assert_eq!(
            registry.admissions[&(owner.clone(), id(1).lane)].completed,
            2
        );
        for sequence in 3..10000 {
            registry
                .cancel_wait_admission(&owner, id(sequence))
                .unwrap();
        }
        let lane = &registry.admissions[&(owner.clone(), id(1).lane)];
        assert_eq!(lane.completed, 9999);
        assert!(lane.cancelled.is_empty());
        assert!(lane.active.is_empty());
    }
    #[test]
    fn exact_owner_descriptor_and_sequence_window_reject_forged_admission() {
        let owner = AgentOwner::Agent(AgentId(31));
        let foreign = AgentOwner::Agent(AgentId(32));
        let mut registry = SharedMemoryRegistry::default();
        registry.admit_root(owner.clone(), "origin".into()).unwrap();
        registry
            .admit_root(foreign.clone(), "foreign".into())
            .unwrap();
        let (descriptor, _) = registry
            .allocate(&owner, 4, None, 4096, 0, false, false)
            .unwrap();
        let id = WaitAdmissionId {
            lane: [31; 32],
            sequence: 1,
        };
        assert!(
            registry
                .admit_wait(&foreign, id, descriptor, true, 0, false, 0, None)
                .is_err()
        );
        let mut forged = descriptor;
        forged.memory64 = true;
        assert!(
            registry
                .admit_wait(&owner, id, forged, false, 0, false, 0, None)
                .is_err()
        );
        assert!(
            registry
                .cancel_wait_admission(
                    &owner,
                    WaitAdmissionId {
                        sequence: WINDOW + 1,
                        ..id
                    }
                )
                .is_err()
        );
        assert!(
            registry
                .admit_wait(&owner, id, descriptor, false, 0, false, 0, None)
                .is_ok()
        );
        assert!(
            registry
                .admit_wait(&owner, id, descriptor, false, 0, false, 1, None)
                .is_err()
        );
    }
}
