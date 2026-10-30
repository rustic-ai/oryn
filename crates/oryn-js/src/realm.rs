use std::collections::{BTreeMap, BTreeSet};

use crate::origin::Origin;
use oryn_dom::NodeId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RealmId(pub u32);

/// Native browsing-context policy for contexts without a parent IPC backend.
/// Set at creation; document replacement retains the owning context's policy.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowPopupPolicy(pub bool);

/// An execution environment identity, scoped to its owning agent. Unlike a
/// document generation, this is never reused after retirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EnvironmentId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvironmentDescriptor {
    pub id: EnvironmentId,
    pub realm: RealmId,
    pub document_generation: u64,
    pub owner: Option<EnvironmentId>,
}

#[derive(Debug, Default)]
pub struct AgentEnvironments {
    next_id: u64,
    live: BTreeMap<EnvironmentId, EnvironmentDescriptor>,
    quarantined: BTreeSet<EnvironmentId>,
}

impl AgentEnvironments {
    pub fn create(
        &mut self,
        realm: RealmId,
        document_generation: u64,
        owner: Option<EnvironmentId>,
    ) -> Result<EnvironmentDescriptor, &'static str> {
        if owner.is_some_and(|owner| {
            self.live
                .get(&owner)
                .is_none_or(|descriptor| !self.is_live(descriptor))
        }) {
            return Err("execution environment owner is retired");
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or("environment identity space exhausted")?;
        let descriptor = EnvironmentDescriptor {
            id: EnvironmentId(next),
            realm,
            document_generation,
            owner,
        };
        self.next_id = next;
        self.live.insert(descriptor.id, descriptor);
        Ok(descriptor)
    }

    pub fn is_live(&self, descriptor: &EnvironmentDescriptor) -> bool {
        self.live.get(&descriptor.id) == Some(descriptor)
            && !self.quarantined.contains(&descriptor.id)
    }

    /// Stop native execution/admission while retaining the exact records for
    /// later physical cleanup. This neither executes callbacks nor frees a VM.
    pub fn quarantine(&mut self, id: EnvironmentId) {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if !self.live.contains_key(&id) || !self.quarantined.insert(id) {
                continue;
            }
            pending.extend(
                self.live
                    .values()
                    .filter(|entry| entry.owner == Some(id))
                    .map(|entry| entry.id),
            );
        }
    }

    /// Revoke an environment and its owned worker environments. Related
    /// windows are not owned workers and must be retired by their coordinator.
    pub fn retire(&mut self, id: EnvironmentId) -> Vec<EnvironmentDescriptor> {
        let mut pending = vec![id];
        let mut retired = Vec::new();
        while let Some(id) = pending.pop() {
            if let Some(descriptor) = self.live.remove(&id) {
                self.quarantined.remove(&id);
                pending.extend(
                    self.live
                        .values()
                        .filter(|entry| entry.owner == Some(id))
                        .map(|entry| entry.id),
                );
                retired.push(descriptor);
            }
        }
        retired
    }
}

/// Committed native secure-context ancestry, separate from origin authority.
#[derive(Debug, Clone, Copy)]
pub struct SecureContextState(pub bool);
pub fn potentially_trustworthy_origin(origin: &Origin) -> bool {
    url::Url::parse(&origin.serialize()).is_ok_and(|url| {
        url.scheme() == "https"
            || url.host_str().is_some_and(|host| {
                host == "localhost"
                    || host.ends_with(".localhost")
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmDescriptor {
    pub id: RealmId,
    pub origin: Origin,
    pub document_generation: u64,
    pub frame_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WrapperKey {
    pub arena: oryn_dom::ArenaId,
    pub realm: RealmId,
    pub node: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("realm identity space exhausted")]
pub struct RealmLimit;

#[derive(Debug, Default)]
pub struct RealmRegistry {
    next_id: u32,
    realms: BTreeMap<RealmId, RealmDescriptor>,
}

impl RealmRegistry {
    pub fn ensure_capacity(&self) -> Result<(), RealmLimit> {
        self.next_id.checked_add(1).map(|_| ()).ok_or(RealmLimit)
    }

    pub fn create(
        &mut self,
        origin: impl Into<Origin>,
        document_generation: u64,
        frame_id: Option<u64>,
    ) -> RealmId {
        self.try_create(origin, document_generation, frame_id)
            .expect("realm identity space exhausted")
    }

    pub fn try_create(
        &mut self,
        origin: impl Into<Origin>,
        document_generation: u64,
        frame_id: Option<u64>,
    ) -> Result<RealmId, RealmLimit> {
        let next = self.next_id.checked_add(1).ok_or(RealmLimit)?;
        let id = RealmId(self.next_id);
        self.next_id = next;
        self.realms.insert(
            id,
            RealmDescriptor {
                id,
                origin: origin.into(),
                document_generation,
                frame_id,
            },
        );
        Ok(id)
    }

    /// Embedder-only finalization of a committed document's broker identity.
    /// Generation comparison prevents a late policy reply changing its successor.
    pub fn bind_native_origin(
        &mut self,
        id: RealmId,
        generation: u64,
        origin: Origin,
    ) -> Option<RealmDescriptor> {
        let realm = self.realms.get_mut(&id)?;
        if realm.document_generation != generation {
            return None;
        }
        realm.origin = origin;
        Some(realm.clone())
    }

    pub fn get(&self, id: RealmId) -> Option<&RealmDescriptor> {
        self.realms.get(&id)
    }

    /// Advance the active document of a reused initial Window. Realm and
    /// origin identity remain unchanged; environment grants rotate separately.
    pub fn advance_document(&mut self, id: RealmId, generation: u64) -> Option<RealmDescriptor> {
        let descriptor = self.realms.get_mut(&id)?;
        if generation <= descriptor.document_generation {
            return None;
        }
        descriptor.document_generation = generation;
        Some(descriptor.clone())
    }

    pub fn remove(&mut self, id: RealmId) -> Option<RealmDescriptor> {
        self.realms.remove(&id)
    }

    pub fn same_origin(&self, left: RealmId, right: RealmId) -> bool {
        self.get(left)
            .zip(self.get(right))
            .is_some_and(|(left, right)| left.origin == right.origin)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reused_window_advances_document_without_changing_origin_or_realm() {
        let mut registry = super::RealmRegistry::default();
        let realm = registry.create("https://example.test", 1, Some(7));
        let original = registry.get(realm).unwrap().clone();
        let current = registry.advance_document(realm, 2).unwrap();
        assert_eq!(current.id, original.id);
        assert_eq!(current.origin, original.origin);
        assert_eq!(current.frame_id, original.frame_id);
        assert_eq!(current.document_generation, 2);
        assert!(registry.advance_document(realm, 2).is_none());
        assert!(registry.advance_document(realm, 1).is_none());
        assert_eq!(registry.get(realm), Some(&current));
    }

    use super::*;

    #[test]
    fn environment_retirement_revokes_descendants_without_reusing_identity() {
        let mut registry = AgentEnvironments::default();
        let window = registry.create(RealmId(1), 7, None).unwrap();
        let sibling = registry.create(RealmId(2), 7, None).unwrap();
        let worker = registry.create(RealmId(3), 1, Some(window.id)).unwrap();
        let nested = registry.create(RealmId(4), 1, Some(worker.id)).unwrap();
        let forged = EnvironmentDescriptor {
            document_generation: 8,
            ..window
        };
        assert!(!registry.is_live(&forged));
        assert_eq!(registry.retire(window.id).len(), 3);
        for descriptor in [window, worker, nested] {
            assert!(!registry.is_live(&descriptor));
        }
        assert!(registry.is_live(&sibling));
        assert!(registry.create(RealmId(5), 1, Some(window.id)).is_err());
        let replacement = registry.create(window.realm, 8, None).unwrap();
        assert_ne!(replacement.id, window.id);
        registry.next_id = u64::MAX;
        assert!(registry.create(RealmId(6), 1, None).is_err());
        assert!(registry.is_live(&replacement));
    }

    #[test]
    fn quarantine_denies_admission_but_preserves_exact_retirement_and_siblings() {
        let mut registry = AgentEnvironments::default();
        let root = registry.create(RealmId(1), 7, None).unwrap();
        let child = registry.create(RealmId(2), 1, Some(root.id)).unwrap();
        let sibling = registry.create(RealmId(3), 1, None).unwrap();
        registry.quarantine(root.id);
        registry.quarantine(root.id);
        assert!(!registry.is_live(&root) && !registry.is_live(&child));
        assert!(registry.is_live(&sibling));
        assert!(registry.create(RealmId(4), 1, Some(root.id)).is_err());
        assert!(registry.create(RealmId(4), 1, Some(child.id)).is_err());
        assert_eq!(registry.live.get(&root.id), Some(&root));
        assert_eq!(registry.live.get(&child.id), Some(&child));
        let retired = registry.retire(root.id);
        assert_eq!(retired.len(), 2);
        assert!(retired.contains(&root) && retired.contains(&child));
        assert!(registry.quarantined.is_empty());
        assert!(registry.retire(root.id).is_empty());
        assert!(registry.is_live(&sibling));
        let replacement = registry.create(root.realm, 8, None).unwrap();
        assert_ne!(replacement.id, root.id);
        assert!(registry.is_live(&replacement));
        assert!(!registry.is_live(&root));
    }

    #[test]
    fn exhaustion_does_not_replace_an_existing_realm() {
        let mut registry = RealmRegistry::default();
        let original = registry.create("https://example.test", 1, None);
        registry.next_id = u32::MAX;
        assert_eq!(
            registry.try_create("https://other.test", 2, None),
            Err(RealmLimit)
        );
        assert_eq!(registry.get(original).unwrap().document_generation, 1);
        assert_eq!(registry.realms.len(), 1);
    }

    #[test]
    fn explicit_opaque_inheritance_preserves_identity() {
        let mut registry = RealmRegistry::default();
        let origin = Origin::opaque();
        let first = registry.create(origin.clone(), 1, None);
        let second = registry.create(origin, 1, Some(1));
        assert!(registry.same_origin(first, second));
    }

    #[test]
    fn serialized_opaque_origins_do_not_share_identity() {
        let mut registry = RealmRegistry::default();
        let first = registry.create("null", 1, None);
        let second = registry.create("null", 1, None);
        assert!(!registry.same_origin(first, second));
    }

    #[test]
    fn tuple_origins_use_url_normalization() {
        let mut registry = RealmRegistry::default();
        let first = registry.create("https://EXAMPLE.test:443/path", 1, None);
        let second = registry.create("https://example.test", 1, None);
        assert!(registry.same_origin(first, second));
    }

    #[test]
    fn realm_origin_and_document_generation_are_not_implicit() {
        let mut registry = RealmRegistry::default();
        let top = registry.create("https://example.test", 7, None);
        let same_origin_frame = registry.create("https://example.test", 3, Some(1));
        let cross_origin_frame = registry.create("https://login.test", 1, Some(2));
        assert!(registry.same_origin(top, same_origin_frame));
        assert!(!registry.same_origin(top, cross_origin_frame));
        assert_eq!(registry.get(top).unwrap().document_generation, 7);
    }
}
