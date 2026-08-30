use std::collections::BTreeMap;

use oryn_dom::NodeId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RealmId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealmDescriptor {
    pub id: RealmId,
    pub origin: String,
    pub document_generation: u64,
    pub frame_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WrapperKey {
    pub realm: RealmId,
    pub node: NodeId,
}

#[derive(Debug, Default)]
pub struct RealmRegistry {
    next_id: u32,
    realms: BTreeMap<RealmId, RealmDescriptor>,
}

impl RealmRegistry {
    pub fn create(
        &mut self,
        origin: impl Into<String>,
        document_generation: u64,
        frame_id: Option<u64>,
    ) -> RealmId {
        let id = RealmId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.realms.insert(
            id,
            RealmDescriptor {
                id,
                origin: origin.into(),
                document_generation,
                frame_id,
            },
        );
        id
    }

    pub fn get(&self, id: RealmId) -> Option<&RealmDescriptor> {
        self.realms.get(&id)
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
    use super::*;

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
