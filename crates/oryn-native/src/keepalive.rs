//! Fetch-group body budget, shared by ordinary and activated deferred Fetch.
//! A group is an exact environment, not an origin or browsing context. Its
//! in-flight entries outlive document retirement and leave only at transport
//! completion/cancellation acknowledgement.
use crate::agent_registry::AgentOwner;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

const LIMIT: usize = 64 * 1024;
#[derive(Clone, Default)]
pub(crate) struct Budget(Arc<Mutex<BTreeMap<AgentOwner, usize>>>);
pub(crate) struct Lease {
    budget: Budget,
    owner: AgentOwner,
    bytes: AtomicUsize,
}
impl Budget {
    pub(crate) fn reserve(&self, owner: AgentOwner, bytes: usize) -> Result<Lease, String> {
        let mut groups = self.0.lock().map_err(|_| "keepalive budget poisoned")?;
        let used = groups.get(&owner).copied().unwrap_or(0);
        let total = used
            .checked_add(bytes)
            .filter(|total| *total <= LIMIT)
            .ok_or("keepalive body budget exceeded")?;
        if bytes != 0 {
            groups.insert(owner.clone(), total);
        }
        Ok(Lease {
            budget: self.clone(),
            owner,
            bytes: AtomicUsize::new(bytes),
        })
    }
}
impl Lease {
    /// Redirects may rewrite POST to GET and remove the body from the fetch
    /// record. Every cleanup owner shares this one charge; it is never copied.
    pub(crate) fn shrink_to(&self, bytes: usize) {
        let mut groups = self
            .budget
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = self.bytes.load(Ordering::Acquire);
        if bytes >= previous {
            return;
        }
        if let Some(used) = groups.get_mut(&self.owner) {
            *used = used.saturating_sub(previous - bytes);
            if *used == 0 {
                groups.remove(&self.owner);
            }
        }
        self.bytes.store(bytes, Ordering::Release);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let bytes = self.bytes.load(Ordering::Acquire);
        if bytes == 0 {
            return;
        }
        let mut groups = self
            .budget
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(used) = groups.get_mut(&self.owner) {
            *used = used.saturating_sub(bytes);
            if *used == 0 {
                groups.remove(&self.owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::v3::PageId;
    use oryn_js::message_wire::WindowTarget;
    fn owner(generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(1),
            target: WindowTarget::Root,
            generation,
            epoch: 0,
        }
    }
    #[test]
    fn ordinary_and_deferred_bodies_share_exact_environment_budget() {
        let budget = Budget::default();
        let ordinary = budget.reserve(owner(1), 48 * 1024).unwrap();
        let deferred = budget.reserve(owner(1), 16 * 1024).unwrap();
        assert!(budget.reserve(owner(1), 1).is_err());
        let replacement = budget.reserve(owner(2), LIMIT).unwrap();
        drop(ordinary);
        let next = budget.reserve(owner(1), 48 * 1024).unwrap();
        drop((deferred, next, replacement));
        assert!(budget.0.lock().unwrap().is_empty());
    }
    #[test]
    fn response_custody_keeps_charge_until_last_completion_owner_drops() {
        let budget = Budget::default();
        let pending = Arc::new(budget.reserve(owner(1), LIMIT).unwrap());
        let cancellation_ack = pending.clone();
        drop(pending);
        assert!(budget.reserve(owner(1), 1).is_err());
        drop(cancellation_ack);
        assert!(budget.reserve(owner(1), LIMIT).is_ok());
    }
    #[test]
    fn retirement_does_not_erase_existing_transport_charge_and_overflow_fails() {
        let budget = Budget::default();
        let transport = budget.reserve(owner(1), LIMIT).unwrap();
        let surviving_budget = budget.clone();
        drop(budget);
        assert!(surviving_budget.reserve(owner(1), 1).is_err());
        assert!(surviving_budget.reserve(owner(2), usize::MAX).is_err());
        drop(transport);
        assert!(surviving_budget.reserve(owner(1), LIMIT).is_ok());
    }
    #[test]
    fn redirect_body_removal_releases_only_its_shared_charge() {
        let budget = Budget::default();
        let first = Arc::new(budget.reserve(owner(1), 48 * 1024).unwrap());
        let cleanup = first.clone();
        let sibling = budget.reserve(owner(1), 16 * 1024).unwrap();
        cleanup.shrink_to(0);
        let replacement = budget.reserve(owner(1), 48 * 1024).unwrap();
        drop((first, cleanup));
        assert!(budget.reserve(owner(1), 1).is_err());
        drop((sibling, replacement));
        assert!(budget.0.lock().unwrap().is_empty());
    }
}
