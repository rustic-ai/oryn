//! Parser-local handle keys remain stable when native nodes migrate arenas.
//! Keys are opaque to tree construction; every DOM access resolves the lease.
use super::*;
use std::collections::BTreeMap;

pub(super) struct ParserHandles {
    origin: SharedDomRuntime,
    document: crate::NodeRef,
    entries: RefCell<BTreeMap<NodeId, crate::TrackedNodeLease>>,
    identities: RefCell<BTreeMap<crate::NodeRef, NodeId>>,
    migration_revision: Cell<u64>,
}
impl ParserHandles {
    pub(super) fn new(origin: SharedDomRuntime, document: NodeId) -> Self {
        let identity = crate::NodeRef {
            arena: origin.borrow().arena().id(),
            node: document,
        };
        Self {
            origin,
            document: identity,
            entries: RefCell::new(BTreeMap::new()),
            identities: RefCell::new(BTreeMap::new()),
            migration_revision: Cell::new(crate::tracked_lease::migration_revision()),
        }
    }
    pub(super) fn register(&self, owner: &SharedDomRuntime, node: NodeId) -> NodeId {
        let identity = crate::NodeRef {
            arena: owner.borrow().arena().id(),
            node,
        };
        self.refresh_identities();
        if let Some(key) = self.identities.borrow().get(&identity).copied() {
            return key;
        }
        // Foreign nodes need a parser-local key. The detached fragment is only
        // an identity reservation; it is never inserted or exposed as content.
        let key = if std::rc::Rc::ptr_eq(owner, &self.origin)
            && !self.entries.borrow().contains_key(&node)
        {
            node
        } else {
            let reservation = self.origin.borrow_mut().create_fragment();
            // Only its generational identity is needed. Release the detached
            // reservation instead of leaking one arena node per foreign key.
            drop(crate::TrackedNodeLease::parser(&self.origin, reservation));
            reservation
        };
        let lease = crate::TrackedNodeLease::parser(owner, node);
        self.entries.borrow_mut().insert(key, lease);
        self.identities.borrow_mut().insert(identity, key);
        key
    }
    fn refresh_identities(&self) {
        let revision = crate::tracked_lease::migration_revision();
        if self.migration_revision.replace(revision) != revision {
            *self.identities.borrow_mut() = self
                .entries
                .borrow()
                .iter()
                .map(|(key, lease)| (lease.identity(), *key))
                .collect();
        }
    }
    pub(super) fn resolve(&self, key: NodeId) -> (SharedDomRuntime, NodeId) {
        if let Some(lease) = self.entries.borrow().get(&key) {
            return lease.current();
        }
        self.register(&self.origin, key);
        self.entries
            .borrow()
            .get(&key)
            .expect("parser handle registered")
            .current()
    }
    pub(super) fn replace(&self, key: NodeId, owner: &SharedDomRuntime, node: NodeId) {
        if self.entries.borrow().get(&key).is_some_and(|lease| {
            lease.identity()
                == crate::NodeRef {
                    arena: owner.borrow().arena().id(),
                    node,
                }
        }) {
            return;
        }
        let lease = crate::TrackedNodeLease::parser(owner, node);
        self.refresh_identities();
        if let Some(previous) = self.entries.borrow_mut().insert(key, lease) {
            self.identities.borrow_mut().remove(&previous.identity());
        }
        self.identities.borrow_mut().insert(
            crate::NodeRef {
                arena: owner.borrow().arena().id(),
                node,
            },
            key,
        );
    }
    pub(super) fn keys(&self) -> Vec<NodeId> {
        self.entries.borrow().keys().copied().collect()
    }
    pub(super) fn release(&self, key: NodeId) {
        self.refresh_identities();
        if let Some(lease) = self.entries.borrow_mut().remove(&key) {
            self.identities.borrow_mut().remove(&lease.identity());
        }
    }
}
impl Drop for ParserHandles {
    fn drop(&mut self) {
        for lease in self.entries.get_mut().values() {
            let (owner, node) = lease.current();
            if let Ok(mut dom) = owner.try_borrow_mut()
                && let Ok(record) = dom.arena_mut().get_mut(node)
                && record.parser_open_document == Some(self.document)
            {
                record.parser_open_document = None;
            }
        }
        // Tracked leases release from their current arena, including after an
        // interrupted constructor. No stale original-arena NodeId is released.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_handle_and_release_follow_two_arena_migrations() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = source.borrow().document();
        let handles = ParserHandles::new(source.clone(), document);
        let node = source.borrow_mut().create_text("tracked");
        let key = handles.register(&source, node);
        let destination = target.borrow().document();
        let mut migration =
            DomRuntime::prepare_subtree_migration(&source, &target, node, destination).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut migration).unwrap();
        assert_eq!(handles.resolve(key).1, moved.root);
        assert!(std::rc::Rc::ptr_eq(&handles.resolve(key).0, &target));
        assert_eq!(handles.register(&target, moved.root), key);
        let mut back =
            DomRuntime::prepare_subtree_migration(&target, &source, moved.root, document).unwrap();
        let returned = DomRuntime::commit_subtree_migration(&target, &source, &mut back).unwrap();
        assert_eq!(handles.resolve(key).1, returned.root);
        drop(handles);
        drop(migration);
        drop(back);
        crate::drain_node_migration_pins().unwrap();
        crate::drain_tracked_node_leases().unwrap();
        assert!(source.borrow().arena().get(returned.root).is_err());
    }
    #[test]
    fn foreign_local_id_collision_never_aliases_parser_document() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = source.borrow().document();
        let foreign = target.borrow().document();
        assert_eq!(document, foreign);
        let handles = ParserHandles::new(source.clone(), document);
        let original = handles.register(&source, document);
        let other = handles.register(&target, foreign);
        assert_ne!(original, other);
        assert!(std::rc::Rc::ptr_eq(&handles.resolve(other).0, &target));
    }
    #[test]
    fn foreign_keys_release_reservations_without_reusing_live_handles() {
        let source = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let target = std::rc::Rc::new(RefCell::new(DomRuntime::new()));
        let document = source.borrow().document();
        let handles = ParserHandles::new(source.clone(), document);
        handles.register(&source, document);
        let first = target.borrow_mut().create_text("one");
        let second = target.borrow_mut().create_text("two");
        let a = handles.register(&target, first);
        let b = handles.register(&target, second);
        assert_ne!(a, b);
        assert!(source.borrow().arena().get(a).is_err());
        assert!(source.borrow().arena().get(b).is_err());
        assert_eq!(handles.resolve(a).1, first);
        assert_eq!(handles.resolve(b).1, second);
        let local = source.borrow_mut().create_text("local");
        let local_key = handles.register(&source, local);
        assert_ne!(local_key, a);
        assert_ne!(local_key, b);
        assert!(std::rc::Rc::ptr_eq(&handles.resolve(a).0, &target));
    }
}
