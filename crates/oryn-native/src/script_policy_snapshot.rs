//! Native-only policy snapshots for scripts prepared before a later meta policy.
//! Tokens select immutable policy; callers must still validate the live Creator.
use crate::{agent_registry::AgentOwner, runtime::RuntimeError};
use oryn_js::worker_wire::AgentPolicy;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
pub(crate) struct ScriptPolicySnapshots(Mutex<State>, u64);
static NEXT_REGISTRY: AtomicU64 = AtomicU64::new(1);
impl Default for ScriptPolicySnapshots {
    fn default() -> Self {
        let id = NEXT_REGISTRY
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("script policy registry identities exhausted");
        Self(Mutex::new(State::default()), id)
    }
}
// Count serialized custody without allocating another arbitrarily large copy.
struct PolicySize(usize);
impl std::io::Write for PolicySize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= 4 * 1024 * 1024)
            .ok_or_else(|| std::io::Error::other("script policy snapshot capacity exceeded"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[derive(Default)]
struct State {
    next: u64,
    retired: bool,
    bytes: usize,
    entries: BTreeMap<u64, Entry>,
}
struct Entry {
    owner: AgentOwner,
    policy: Arc<AgentPolicy>,
    bytes: usize,
}
fn failure(message: &str) -> RuntimeError {
    RuntimeError::ContextService(message.into())
}
impl ScriptPolicySnapshots {
    pub(crate) fn capture(
        &self,
        owner: AgentOwner,
        policy: AgentPolicy,
    ) -> Result<String, RuntimeError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| failure("script policy registry poisoned"))?;
        if state.retired {
            return Err(RuntimeError::Closed);
        }
        // Repeated preparations under an unchanged document policy share custody.
        if let Some((id, _)) = state
            .entries
            .iter()
            .find(|(_, entry)| entry.owner == owner && *entry.policy == policy)
        {
            return Ok(format!("{}:{id}", self.1));
        }
        let mut size = PolicySize(0);
        serde_json::to_writer(&mut size, &policy).map_err(|error| failure(&error.to_string()))?;
        let bytes = size.0;
        if state.entries.len() >= 1024 || bytes > 4 * 1024 * 1024 - state.bytes {
            return Err(failure("script policy snapshot capacity exceeded"));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| failure("script policy identities exhausted"))?;
        let id = state.next;
        state.bytes += bytes;
        state.entries.insert(
            id,
            Entry {
                owner,
                policy: Arc::new(policy),
                bytes,
            },
        );
        Ok(format!("{}:{id}", self.1))
    }
    pub(crate) fn get(
        &self,
        token: &str,
        owner: &AgentOwner,
    ) -> Result<Arc<AgentPolicy>, RuntimeError> {
        let (registry, id) = token
            .split_once(':')
            .ok_or_else(|| failure("invalid script policy token"))?;
        if registry.parse::<u64>().ok() != Some(self.1) {
            return Err(failure("script policy belongs to another native registry"));
        }
        let id = id
            .parse::<u64>()
            .map_err(|_| failure("invalid script policy token"))?;
        let state = self
            .0
            .lock()
            .map_err(|_| failure("script policy registry poisoned"))?;
        if state.retired {
            return Err(RuntimeError::Closed);
        }
        let entry = state
            .entries
            .get(&id)
            .ok_or_else(|| failure("unknown script policy token"))?;
        if &entry.owner != owner {
            return Err(failure(
                "script policy belongs to another document lifetime",
            ));
        }
        Ok(entry.policy.clone())
    }
    pub(crate) fn retire(&self) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.retired = true;
        state.entries.clear();
        state.bytes = 0;
    }
    pub(crate) fn retain_live(
        &self,
        mut live: impl FnMut(&AgentOwner) -> bool,
    ) -> Result<(), RuntimeError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| failure("script policy registry poisoned"))?;
        state.entries.retain(|_, entry| live(&entry.owner));
        state.bytes = state.entries.values().map(|entry| entry.bytes).sum();
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn owner(generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: oryn_common::v3::PageId(1),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation,
            epoch: 0,
        }
    }
    #[test]
    fn captured_policy_is_immutable_reusable_and_document_bound() {
        let registry = ScriptPolicySnapshots::default();
        let before = AgentPolicy {
            url: "https://a.test/".into(),
            ..Default::default()
        };
        let first = registry.capture(owner(1), before.clone()).unwrap();
        assert_eq!(registry.capture(owner(1), before.clone()).unwrap(), first);
        let mut after = before.clone();
        after
            .headers
            .push(("Content-Security-Policy".into(), "script-src 'none'".into()));
        let second = registry.capture(owner(1), after.clone()).unwrap();
        assert_ne!(first, second);
        assert_eq!(*registry.get(&first, &owner(1)).unwrap(), before);
        assert_eq!(*registry.get(&second, &owner(1)).unwrap(), after);
        assert!(registry.get(&first, &owner(2)).is_err());
        registry
            .retain_live(|candidate| candidate == &owner(2))
            .unwrap();
        assert!(registry.get(&first, &owner(1)).is_err());
        registry.retire();
        assert!(registry.capture(owner(2), AgentPolicy::default()).is_err());
    }
    #[test]
    fn tokens_do_not_alias_across_registries_and_capacity_recovers_after_pruning() {
        let first = ScriptPolicySnapshots::default();
        let second = ScriptPolicySnapshots::default();
        let policy = AgentPolicy::default();
        let a = first.capture(owner(1), policy.clone()).unwrap();
        let b = second.capture(owner(1), policy.clone()).unwrap();
        assert_ne!(a, b);
        assert!(second.get(&a, &owner(1)).is_err());
        for generation in 2..=1024 {
            first.capture(owner(generation), policy.clone()).unwrap();
        }
        assert!(first.capture(owner(1025), policy.clone()).is_err());
        // Deduplication works at capacity and never consumes another slot.
        assert_eq!(first.capture(owner(1), policy.clone()).unwrap(), a);
        first
            .retain_live(|candidate| candidate == &owner(1))
            .unwrap();
        first.capture(owner(1025), policy).unwrap();
        let large = AgentPolicy {
            url: "x".repeat(4 * 1024 * 1024 + 1),
            ..Default::default()
        };
        assert!(first.capture(owner(2), large).is_err());
        assert!(first.get(&a, &owner(1)).is_ok());
    }
}
