//! Derived registry membership; never a native lease or a reachability cache.
use super::*;

#[derive(Debug)]
pub(super) struct RegistryIndex {
    rebuild: bool,
    dirty: BTreeSet<NodeId>,
    membership: BTreeMap<NodeId, u64>,
    groups: BTreeMap<u64, BTreeSet<NodeId>>,
}

impl Default for RegistryIndex {
    fn default() -> Self {
        Self {
            rebuild: true,
            dirty: BTreeSet::new(),
            membership: BTreeMap::new(),
            groups: BTreeMap::new(),
        }
    }
}

impl RegistryIndex {
    pub(super) fn mark_dirty(&mut self, node: NodeId) {
        if !self.rebuild {
            self.dirty.insert(node);
        }
    }

    fn update(&mut self, node: NodeId, registry: Option<u64>) {
        if self.membership.get(&node).copied() == registry {
            return;
        }
        if let Some(previous) = self.membership.remove(&node) {
            let group = self.groups.get_mut(&previous).expect("indexed registry");
            group.remove(&node);
            if group.is_empty() {
                self.groups.remove(&previous);
            }
        }
        if let Some(registry) = registry {
            self.membership.insert(node, registry);
            self.groups.entry(registry).or_default().insert(node);
        }
    }
}

impl DomArena {
    pub(super) fn invalidate_registry_index(&mut self) {
        *self
            .registry_index
            .get_mut()
            .unwrap_or_else(|p| p.into_inner()) = Default::default();
    }

    /// Nodes with this explicit registry association, in stable NodeId order.
    /// Inherited associations and explicit null are not members. The returned
    /// identities do not retain nodes; callers must determine reachability.
    pub fn custom_registry_nodes(&self, registry: u64) -> Vec<NodeId> {
        let mut index = self
            .registry_index
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if index.rebuild {
            for (id, node) in self.iter() {
                index.update(id, node.custom_element_registry.flatten());
            }
            index.rebuild = false;
        } else {
            for id in std::mem::take(&mut index.dirty) {
                let registry = self
                    .get(id)
                    .ok()
                    .and_then(|n| n.custom_element_registry.flatten());
                index.update(id, registry);
            }
        }
        index
            .groups
            .get(&registry)
            .map(|nodes| nodes.iter().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_matches_arena(arena: &DomArena) {
        for registry in [1, 2, 3] {
            let expected: Vec<_> = arena
                .iter()
                .filter_map(|(id, n)| {
                    (n.custom_element_registry.flatten() == Some(registry)).then_some(id)
                })
                .collect();
            assert_eq!(arena.custom_registry_nodes(registry), expected);
        }
    }

    #[test]
    fn registry_membership_tracks_edits_reuse_and_independent_snapshots() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<DomArena>();
        let mut dom = DomRuntime::new();
        let document = dom.document();
        let node = dom.create_element("div");
        let other_document = dom.create(NodeKind::Document);
        dom.set_custom_element_registry(node, Some(1)).unwrap();
        dom.set_custom_element_registry(other_document, Some(2))
            .unwrap();
        assert_matches_arena(dom.arena());
        // Direct mutable access, including null/unassigned transitions, must
        // work even without the runtime registry-association setter.
        for association in [Some(Some(3)), Some(None), None, Some(Some(1))] {
            dom.arena_mut()
                .get_mut(node)
                .unwrap()
                .custom_element_registry = association;
            assert_matches_arena(dom.arena());
        }
        let mut clone = dom.arena().clone();
        let projection = dom.arena().snapshot_document_scope(document).unwrap();
        assert_matches_arena(&clone);
        assert_matches_arena(&projection);
        assert_eq!(projection.custom_registry_nodes(1), vec![node]);
        assert!(projection.custom_registry_nodes(2).is_empty());
        clone.get_mut(node).unwrap().custom_element_registry = Some(Some(2));
        assert_matches_arena(&clone);
        assert_eq!(dom.arena().custom_registry_nodes(1), vec![node]);
        dom.arena_mut().remove(node).unwrap();
        let reused = dom.create_element("span");
        assert_eq!(reused.slot, node.slot);
        assert_ne!(reused.generation, node.generation);
        dom.arena_mut()
            .get_mut(reused)
            .unwrap()
            .custom_element_registry = Some(Some(2));
        assert_matches_arena(dom.arena());
        assert!(dom.arena().get(node).is_err());
        let mut rebuilt = DomArena::prepare_rebuild(&clone, &[node]);
        assert_matches_arena(&rebuilt);
        rebuilt
            .restore(
                node,
                NodeKind::Text {
                    data: "restored".into(),
                },
            )
            .unwrap();
        rebuilt.get_mut(node).unwrap().custom_element_registry = Some(Some(3));
        assert_matches_arena(&rebuilt);
    }

    #[test]
    fn registry_membership_moves_with_bulk_cross_arena_publication() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("div");
        let text = source.borrow_mut().create_text("child");
        source.borrow_mut().append_child(root, text).unwrap();
        source
            .borrow_mut()
            .set_custom_element_registry(root, Some(1))
            .unwrap();
        source
            .borrow_mut()
            .arena_mut()
            .get_mut(text)
            .unwrap()
            .custom_element_registry = Some(Some(2));
        // Populate both caches before the direct slot publication occurs.
        assert_matches_arena(source.borrow().arena());
        assert_matches_arena(destination.borrow().arena());
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        assert_matches_arena(source.borrow().arena());
        assert_matches_arena(destination.borrow().arena());
        assert!(source.borrow().arena().custom_registry_nodes(1).is_empty());
        assert_eq!(
            destination.borrow().arena().custom_registry_nodes(1),
            vec![moved.root]
        );
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert_matches_arena(source.borrow().arena());
        assert_matches_arena(destination.borrow().arena());
    }
}
