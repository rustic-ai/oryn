//! Parent-owned cancellation and admission for script resource operations.
use crate::runtime::RuntimeError;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Default)]
pub(crate) struct ScriptLoadRegistry(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    next: u64,
    retired: bool,
    entries: BTreeMap<u64, Entry>,
}
struct Entry {
    cancel: Arc<AtomicBool>,
    claimed: bool,
    owner: Option<crate::agent_registry::AgentOwner>,
}
pub(crate) struct ScriptLoadLease {
    registry: Arc<Mutex<State>>,
    token: u64,
    pub cancel: Arc<AtomicBool>,
}
fn failure(message: &str) -> RuntimeError {
    RuntimeError::ContextService(message.into())
}
impl ScriptLoadRegistry {
    pub(crate) fn is_drained(&self) -> Result<bool, RuntimeError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| failure("script registry poisoned"))?
            .entries
            .is_empty())
    }
    pub fn reserve(&self) -> Result<u64, RuntimeError> {
        self.reserve_owner(None)
    }
    pub fn reserve_for(
        &self,
        owner: crate::agent_registry::AgentOwner,
    ) -> Result<u64, RuntimeError> {
        self.reserve_owner(Some(owner))
    }
    fn reserve_owner(
        &self,
        owner: Option<crate::agent_registry::AgentOwner>,
    ) -> Result<u64, RuntimeError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| failure("script lease registry poisoned"))?;
        if state.retired {
            return Err(RuntimeError::Closed);
        }
        if state.entries.len() >= 128 {
            return Err(failure("script fetch admission capacity exceeded"));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| failure("script lease identities exhausted"))?;
        let token = state.next;
        state.entries.insert(
            token,
            Entry {
                cancel: Arc::new(AtomicBool::new(false)),
                claimed: false,
                owner,
            },
        );
        Ok(token)
    }
    pub fn claim(&self, token: u64) -> Result<ScriptLoadLease, RuntimeError> {
        self.claim_owner(token, None)
    }
    pub fn claim_for(
        &self,
        token: u64,
        owner: &crate::agent_registry::AgentOwner,
    ) -> Result<ScriptLoadLease, RuntimeError> {
        self.claim_owner(token, Some(owner))
    }
    fn claim_owner(
        &self,
        token: u64,
        owner: Option<&crate::agent_registry::AgentOwner>,
    ) -> Result<ScriptLoadLease, RuntimeError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| failure("script lease registry poisoned"))?;
        if state.retired {
            return Err(RuntimeError::Closed);
        }
        let entry = state
            .entries
            .get_mut(&token)
            .ok_or_else(|| failure("script lease is absent or cancelled"))?;
        if entry.owner.as_ref() != owner {
            return Err(failure("script lease belongs to another document lifetime"));
        }
        if entry.claimed || entry.cancel.load(Ordering::Acquire) {
            return Err(failure("script lease is already consumed"));
        }
        entry.claimed = true;
        Ok(ScriptLoadLease {
            registry: self.0.clone(),
            token,
            cancel: entry.cancel.clone(),
        })
    }
    pub fn cancel(&self, token: u64) -> Result<(), RuntimeError> {
        // Revocation is safe after poisoning and must remain available during
        // admission rollback; it only cancels/removes an already-owned entry.
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = state.entries.get(&token) {
            entry.cancel.store(true, Ordering::Release);
            // Claimed operations retain capacity until their actual completion.
            if !entry.claimed {
                state.entries.remove(&token);
            }
        }
        Ok(())
    }
    /// Cancellation admission and terminal release are distinct: a claimed
    /// request keeps its entry until the running lease is actually dropped.
    pub(crate) fn cancel_and_observe(&self, token: u64) -> Result<bool, RuntimeError> {
        self.cancel(token)?;
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        Ok(!state.entries.contains_key(&token))
    }
    pub fn retire(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.retired = true;
            state.entries.retain(|_, entry| {
                entry.cancel.store(true, Ordering::Release);
                entry.claimed
            });
        }
    }
}
impl Drop for ScriptLoadLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.lock() {
            state.entries.remove(&self.token);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_script_reservations_cannot_cross_document_generation_or_frame() {
        use crate::agent_registry::AgentOwner;
        use oryn_js::message_wire::WindowTarget;
        let root = AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let replacement = AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: WindowTarget::Root,
            generation: 2,
            epoch: 0,
        };
        let frame = AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: WindowTarget::Frame("frame".into()),
            generation: 1,
            epoch: 0,
        };
        let registry = ScriptLoadRegistry::default();
        let token = registry.reserve_for(root.clone()).unwrap();
        assert!(registry.claim_for(token, &replacement).is_err());
        assert!(registry.claim_for(token, &frame).is_err());
        assert!(registry.claim(token).is_err());
        let lease = registry.claim_for(token, &root).unwrap();
        assert!(!lease.cancel.load(Ordering::Acquire));
    }
    #[test]
    fn cancellation_acknowledgement_waits_for_claimed_request_release() {
        let registry = ScriptLoadRegistry::default();
        let untouched = registry.reserve().unwrap();
        assert!(registry.cancel_and_observe(untouched).unwrap());
        let token = registry.reserve().unwrap();
        let running = registry.claim(token).unwrap();
        assert!(!registry.cancel_and_observe(token).unwrap());
        assert!(running.cancel.load(Ordering::Acquire));
        assert!(!registry.cancel_and_observe(token).unwrap());
        drop(running);
        assert!(registry.cancel_and_observe(token).unwrap());
        assert!(registry.cancel_and_observe(token).unwrap());
    }

    #[test]
    fn cancellation_before_and_after_claim_is_terminal_and_tokens_are_single_use() {
        let registry = ScriptLoadRegistry::default();
        let first = registry.reserve().unwrap();
        registry.cancel(first).unwrap();
        assert!(registry.claim(first).is_err());
        let next = registry.reserve().unwrap();
        assert_ne!(first, next);
        let lease = registry.claim(next).unwrap();
        assert!(registry.claim(next).is_err());
        registry.cancel(next).unwrap();
        assert!(lease.cancel.load(Ordering::Acquire));
        assert!(registry.0.lock().unwrap().entries.contains_key(&next));
        drop(lease);
        assert!(registry.0.lock().unwrap().entries.is_empty());
        assert!(registry.claim(next).is_err());
    }
    #[test]
    fn cancellation_does_not_release_active_request_capacity_before_retirement() {
        let registry = ScriptLoadRegistry::default();
        let mut leases = Vec::new();
        for _ in 0..128 {
            let token = registry.reserve().unwrap();
            leases.push(registry.claim(token).unwrap());
            registry.cancel(token).unwrap();
        }
        assert!(registry.reserve().is_err());
        drop(leases.pop());
        assert!(registry.reserve().is_ok());
        registry.retire();
        assert!(registry.reserve().is_err());
        assert!(
            leases
                .iter()
                .all(|lease| lease.cancel.load(Ordering::Acquire))
        );
        drop(leases);
        assert!(registry.0.lock().unwrap().entries.is_empty());
    }
}
