//! Live NodeIterator positions, including references adopted into another arena.
use super::*;
use std::cell::Cell;
use std::rc::Weak;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeIteratorId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeIteratorState {
    pub root: NodeId,
    pub reference: NodeId,
    pub before: bool,
}
#[derive(Debug)]
pub(super) struct NodeIteratorRecord {
    pub(super) state: Cell<NodeIteratorState>,
    alive: Cell<bool>,
    root_released: Cell<bool>,
    reference_released: Cell<bool>,
    root_arena: Cell<ArenaId>,
    reference_arena: Cell<ArenaId>,
    reference_runtime: RefCell<Weak<RefCell<DomRuntime>>>,
}
impl Drop for NodeIteratorRecord {
    fn drop(&mut self) {
        if !self.reference_released.get() && self.reference_arena.get() != self.root_arena.get() {
            release_foreign(
                self.reference_runtime.get_mut().clone(),
                self.reference_arena.get(),
                self.state.get().reference,
            );
        }
    }
}
thread_local! {
    static ITERATORS:RefCell<Vec<Weak<NodeIteratorRecord>>>=const{RefCell::new(Vec::new())};
    static RELEASES:RefCell<Vec<(Weak<RefCell<DomRuntime>>,NodeRef)>>=const{RefCell::new(Vec::new())};
}
fn all_records() -> Vec<Rc<NodeIteratorRecord>> {
    ITERATORS.with(|items| {
        let mut items = items.borrow_mut();
        items.retain(|item| item.strong_count() > 0);
        items.iter().filter_map(Weak::upgrade).collect()
    })
}
fn records() -> Vec<Rc<NodeIteratorRecord>> {
    all_records()
        .into_iter()
        .filter(|record| record.alive.get())
        .collect()
}
fn release_foreign(runtime: Weak<RefCell<DomRuntime>>, arena: ArenaId, node: NodeId) {
    RELEASES.with(|pending| {
        pending
            .borrow_mut()
            .push((runtime, NodeRef { arena, node }))
    });
}
pub fn drain_node_iterator_releases() -> Result<(), DomError> {
    let pending = RELEASES.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    let mut retained = Vec::new();
    let mut failure = None;
    for (runtime, node) in pending {
        let Some(owner) = runtime.upgrade() else {
            continue;
        };
        let released = match owner.try_borrow_mut() {
            Ok(owner) if owner.arena.id != node.arena => true,
            Ok(mut owner) => {
                let mut committed = false;
                match owner.release_native_with_commit(node.node, || committed = true) {
                    Ok(()) => true,
                    Err(error) => {
                        failure.get_or_insert(error);
                        committed
                    }
                }
            }
            Err(_) => false,
        };
        if !released {
            retained.push((runtime, node));
        }
    }
    RELEASES.with(|pending| pending.borrow_mut().extend(retained));
    failure.map_or(Ok(()), Err)
}
pub(super) fn migrate_references(
    source: ArenaId,
    source_owner: Weak<RefCell<DomRuntime>>,
    destination: ArenaId,
    target: Weak<RefCell<DomRuntime>>,
    nodes: &BTreeMap<NodeId, NodeId>,
) {
    for record in all_records() {
        let mut state = record.state.get();
        if record.reference_arena.get() == source && !nodes.contains_key(&state.reference) {
            record.reference_runtime.replace(source_owner.clone());
        }
        if record.root_arena.get() == source
            && let Some(root) = nodes.get(&state.root)
        {
            state.root = *root;
            record.root_arena.set(destination);
        }
        if record.reference_arena.get() == source
            && let Some(reference) = nodes.get(&state.reference)
        {
            state.reference = *reference;
            record.reference_arena.set(destination);
            record.reference_runtime.replace(target.clone());
        }
        record.state.set(state);
    }
    // A deferred finalizer's lease moves with the same native root counters.
    RELEASES.with(|pending| {
        for (runtime, node) in pending.borrow_mut().iter_mut() {
            if node.arena == source
                && runtime.ptr_eq(&source_owner)
                && let Some(new) = nodes.get(&node.node)
            {
                *runtime = target.clone();
                *node = NodeRef {
                    arena: destination,
                    node: *new,
                };
            }
        }
    });
}
pub(crate) struct IteratorRemoval {
    arena: ArenaId,
    roots: crate::insertion::CursorRoots,
    pending:
        std::collections::VecDeque<(Rc<NodeIteratorRecord>, NodeIteratorState, NodeIteratorState)>,
    release: Option<crate::insertion::RootRelease>,
}
impl IteratorRemoval {
    pub(crate) fn new(
        owner: Option<&SharedDomRuntime>,
        dom: &mut DomRuntime,
        parent: NodeId,
        removed: NodeId,
        descendants: &BTreeSet<NodeId>,
    ) -> Result<Self, DomError> {
        let mut pending = std::collections::VecDeque::new();
        for record in records() {
            let state = record.state.get();
            if record.reference_arena.get() != dom.arena.id
                || !descendants.contains(&state.reference)
                || (record.root_arena.get() == dom.arena.id && descendants.contains(&state.root))
            {
                continue;
            }
            let root = NodeRef {
                arena: record.root_arena.get(),
                node: state.root,
            };
            let following = if state.before {
                dom.iterator_following_from(root, removed, true)?
            } else {
                None
            };
            let (reference, before) = if let Some(next) = following {
                (next, true)
            } else {
                (
                    dom.iterator_preceding_from(root, removed)?
                        .unwrap_or(parent),
                    false,
                )
            };
            pending.push_back((
                record,
                state,
                NodeIteratorState {
                    reference,
                    before,
                    ..state
                },
            ));
        }
        let mut roots = crate::insertion::CursorRoots::new(owner, dom);
        for (_, old, new) in &pending {
            for node in [old.reference, new.reference] {
                if let Err(error) = roots.retain(dom, node) {
                    let _ = roots.release(dom);
                    return Err(error);
                }
            }
        }
        Ok(Self {
            arena: dom.arena.id,
            roots,
            pending,
            release: None,
        })
    }
    pub(crate) fn resume(&mut self, dom: &mut DomRuntime) -> Result<(), DomError> {
        self.roots.validate(dom)?;
        loop {
            if let Some(release) = &mut self.release {
                release.resume(dom)?;
                self.release = None;
            }
            let Some((record, old, new)) = self.pending.front() else {
                return Ok(());
            };
            // A traversal after an earlier failure owns its new position. Do
            // not overwrite it with the pre-removal snapshot on continuation.
            if !record.alive.get()
                || record.reference_arena.get() != self.arena
                || record.state.get() != *old
            {
                self.pending.pop_front();
                continue;
            }
            let (old, new) = (*old, *new);
            if old.reference != new.reference {
                dom.retain_native(new.reference)?;
            }
            record.state.set(new);
            if old.reference != new.reference {
                self.release = Some(crate::insertion::RootRelease::take(
                    self.roots.owner(),
                    dom,
                    old.reference,
                ));
            }
            self.pending.pop_front();
            #[cfg(test)]
            crate::insertion::checkpoint()?;
        }
    }
    pub(crate) fn abort(&mut self, dom: &mut DomRuntime) {
        if let Some(release) = &mut self.release {
            release.abort(dom);
        }
        let _ = self.roots.release(dom);
    }
}
impl DomRuntime {
    // The old arena and its local counters are about to be discarded. Retire
    // positions even if a traversal checkpoint still holds a shared record.
    pub(super) fn retire_node_iterators(&mut self) {
        for record in self.node_iterators.values() {
            record.alive.set(false);
            if !record.reference_released.replace(true)
                && record.reference_arena.get() != record.root_arena.get()
            {
                release_foreign(
                    record.reference_runtime.borrow().clone(),
                    record.reference_arena.get(),
                    record.state.get().reference,
                );
            }
        }
        self.node_iterators.clear();
    }
    pub fn create_node_iterator(&mut self, root: NodeId) -> Result<NodeIteratorId, DomError> {
        self.arena.get(root)?;
        let id = NodeIteratorId(self.next_iterator.max(1));
        let next = id.0.checked_add(1).ok_or(DomError::MigrationCapacity)?;
        if self
            .roots
            .get(&root)
            .is_some_and(|roots| roots.native > u32::MAX - 2)
        {
            return Err(DomError::MigrationCapacity);
        }
        self.retain_native(root)?;
        self.retain_native(root)?;
        let record = Rc::new(NodeIteratorRecord {
            alive: Cell::new(true),
            root_released: Cell::new(false),
            reference_released: Cell::new(false),
            state: Cell::new(NodeIteratorState {
                root,
                reference: root,
                before: true,
            }),
            root_arena: Cell::new(self.arena.id),
            reference_arena: Cell::new(self.arena.id),
            reference_runtime: RefCell::new(Weak::new()),
        });
        ITERATORS.with(|items| {
            let mut items = items.borrow_mut();
            items.retain(|item| item.strong_count() > 0);
            items.push(Rc::downgrade(&record));
        });
        self.node_iterators.insert(id, record);
        self.next_iterator = next;
        Ok(id)
    }
    pub fn node_iterator(&self, id: NodeIteratorId) -> Result<NodeIteratorState, DomError> {
        self.node_iterators
            .get(&id)
            .map(|record| record.state.get())
            .ok_or(DomError::UnknownIterator(id))
    }
    /// The exact still-owned native leases, including equal root/reference
    /// nodes and foreign references. A committed partial release is excluded;
    /// a queued foreign release belongs to its release queue, not this record.
    pub fn node_iterator_native_roots(&self, id: NodeIteratorId) -> Result<Vec<NodeRef>, DomError> {
        let record = self
            .node_iterators
            .get(&id)
            .ok_or(DomError::UnknownIterator(id))?;
        let state = record.state.get();
        let mut roots = Vec::with_capacity(2);
        if !record.root_released.get() {
            roots.push(NodeRef {
                arena: record.root_arena.get(),
                node: state.root,
            });
        }
        if !record.reference_released.get() {
            roots.push(NodeRef {
                arena: record.reference_arena.get(),
                node: state.reference,
            });
        }
        Ok(roots)
    }
    pub fn node_iterator_reference_arena(&self, id: NodeIteratorId) -> Result<ArenaId, DomError> {
        self.node_iterators
            .get(&id)
            .map(|record| record.reference_arena.get())
            .ok_or(DomError::UnknownIterator(id))
    }
    pub fn set_node_iterator_reference(
        &mut self,
        id: NodeIteratorId,
        reference: NodeId,
        before: bool,
    ) -> Result<(), DomError> {
        let record = self
            .node_iterators
            .get(&id)
            .cloned()
            .ok_or(DomError::UnknownIterator(id))?;
        self.arena.get(reference)?;
        let old = record.state.get();
        let old_arena = record.reference_arena.get();
        if old_arena != self.arena.id || old.reference != reference {
            if self
                .roots
                .get(&reference)
                .is_some_and(|roots| roots.native == u32::MAX)
            {
                return Err(DomError::MigrationCapacity);
            }
            self.retain_native(reference)?;
            record.state.set(NodeIteratorState {
                reference,
                before,
                ..old
            });
            record.reference_arena.set(self.arena.id);
            let previous = record.reference_runtime.replace(Weak::new());
            if old_arena == self.arena.id {
                self.release_native(old.reference)?;
            } else {
                release_foreign(previous, old_arena, old.reference);
            }
        } else {
            record.state.set(NodeIteratorState { before, ..old });
        }
        Ok(())
    }
    pub fn set_node_iterator_reference_from(
        owner: &SharedDomRuntime,
        id: NodeIteratorId,
        target: &SharedDomRuntime,
        reference: NodeId,
        before: bool,
    ) -> Result<(), DomError> {
        if Rc::ptr_eq(owner, target) {
            return owner
                .borrow_mut()
                .set_node_iterator_reference(id, reference, before);
        }
        let record = owner
            .borrow()
            .node_iterators
            .get(&id)
            .cloned()
            .ok_or(DomError::UnknownIterator(id))?;
        let old = record.state.get();
        let old_arena = record.reference_arena.get();
        let target_arena = target.borrow().arena.id;
        if old_arena == target_arena && old.reference == reference {
            record.state.set(NodeIteratorState { before, ..old });
            return Ok(());
        }
        {
            let mut target = target.borrow_mut();
            if target
                .roots
                .get(&reference)
                .is_some_and(|roots| roots.native == u32::MAX)
            {
                return Err(DomError::MigrationCapacity);
            }
            target.retain_native(reference)?;
        }
        record.state.set(NodeIteratorState {
            reference,
            before,
            ..old
        });
        record.reference_arena.set(target_arena);
        let previous = record.reference_runtime.replace(Rc::downgrade(target));
        let owner_arena = owner.borrow().arena.id;
        if old_arena == owner_arena {
            owner.borrow_mut().release_native(old.reference)?;
        } else {
            release_foreign(previous, old_arena, old.reference);
        }
        drain_node_iterator_releases()
    }
    pub fn release_node_iterator(&mut self, id: NodeIteratorId) -> Result<(), DomError> {
        let record = self
            .node_iterators
            .get(&id)
            .cloned()
            .ok_or(DomError::UnknownIterator(id))?;
        let state = record.state.get();
        record.alive.set(false);
        let mut failure = None;
        if !record.reference_released.get() {
            if record.reference_arena.get() == self.arena.id {
                if let Err(error) = self.release_native_with_commit(state.reference, || {
                    record.reference_released.set(true)
                }) {
                    failure = Some(error);
                }
            } else {
                release_foreign(
                    record.reference_runtime.borrow().clone(),
                    record.reference_arena.get(),
                    state.reference,
                );
                record.reference_released.set(true);
            }
        }
        // Always settle the independent root obligation, even when reference
        // reclamation failed after committing its native decrement.
        if !record.root_released.get()
            && let Err(error) =
                self.release_native_with_commit(state.root, || record.root_released.set(true))
        {
            failure.get_or_insert(error);
        }
        if record.root_released.get() && record.reference_released.get() {
            self.node_iterators.remove(&id);
        }
        failure.map_or(Ok(()), Err)
    }
    fn iterator_children(&self, node: NodeId) -> Result<Vec<NodeId>, DomError> {
        self.arena
            .get(node)?
            .children
            .iter()
            .copied()
            .filter_map(|child| match self.arena.get(child) {
                Ok(record)
                    if matches!(
                        record.kind,
                        NodeKind::Document | NodeKind::ShadowRoot { .. }
                    ) =>
                {
                    None
                }
                Ok(_) => Some(Ok(child)),
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
    pub fn iterator_following(
        &self,
        root: NodeId,
        node: NodeId,
        skip_children: bool,
    ) -> Result<Option<NodeId>, DomError> {
        self.iterator_following_from(
            NodeRef {
                arena: self.arena.id,
                node: root,
            },
            node,
            skip_children,
        )
    }
    pub fn iterator_following_from(
        &self,
        root: NodeRef,
        node: NodeId,
        skip_children: bool,
    ) -> Result<Option<NodeId>, DomError> {
        if !skip_children && let Some(child) = self.iterator_children(node)?.first() {
            return Ok(Some(*child));
        }
        let mut current = node;
        let mut seen = BTreeSet::new();
        while root.arena != self.arena.id || current != root.node {
            if !seen.insert(current) {
                return Err(DomError::Cycle);
            }
            let Some(parent) = self.arena.get(current)?.parent else {
                return Ok(None);
            };
            let siblings = self.iterator_children(parent)?;
            if let Some(index) = siblings.iter().position(|node| *node == current)
                && let Some(next) = siblings.get(index + 1)
            {
                return Ok(Some(*next));
            }
            current = parent;
        }
        Ok(None)
    }
    pub fn iterator_preceding(
        &self,
        root: NodeId,
        node: NodeId,
    ) -> Result<Option<NodeId>, DomError> {
        self.iterator_preceding_from(
            NodeRef {
                arena: self.arena.id,
                node: root,
            },
            node,
        )
    }
    pub fn iterator_preceding_from(
        &self,
        root: NodeRef,
        node: NodeId,
    ) -> Result<Option<NodeId>, DomError> {
        if root.arena == self.arena.id && node == root.node {
            return Ok(None);
        }
        let Some(parent) = self.arena.get(node)?.parent else {
            return Ok(None);
        };
        let siblings = self.iterator_children(parent)?;
        let Some(index) = siblings.iter().position(|candidate| *candidate == node) else {
            return Ok(None);
        };
        if index == 0 {
            return Ok(Some(parent));
        }
        let mut current = siblings[index - 1];
        let mut seen = BTreeSet::new();
        while let Some(child) = self.iterator_children(current)?.last() {
            if !seen.insert(current) {
                return Err(DomError::Cycle);
            }
            current = *child;
        }
        Ok(Some(current))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iterator_release_settles_both_leases_after_reclamation_error() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let reference = dom.create_element("span");
        let id = dom.create_node_iterator(root).unwrap();
        dom.set_node_iterator_reference(id, reference, false)
            .unwrap();
        dom.arena.get_mut(reference).unwrap().parent = Some(reference);
        assert!(matches!(
            dom.release_node_iterator(id),
            Err(DomError::Cycle)
        ));
        assert!(matches!(
            dom.node_iterator(id),
            Err(DomError::UnknownIterator(_))
        ));
        assert!(dom.arena.get(root).is_err());
        assert_eq!(dom.roots[&reference].native, 0);
        dom.arena.get_mut(reference).unwrap().parent = None;
        dom.reclaim_if_unrooted(reference).unwrap();
    }
    #[test]
    fn iterator_release_keeps_uncommitted_obligation_without_repeating_root_release() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let reference = dom.create_element("span");
        let id = dom.create_node_iterator(root).unwrap();
        dom.set_node_iterator_reference(id, reference, false)
            .unwrap();
        dom.roots.get_mut(&reference).unwrap().native = 0;
        assert!(matches!(
            dom.release_node_iterator(id),
            Err(DomError::NativeRootUnderflow(_))
        ));
        assert!(dom.node_iterators[&id].root_released.get());
        assert!(!dom.node_iterators[&id].reference_released.get());
        assert!(!dom.node_iterators[&id].alive.get());
        assert_eq!(
            dom.node_iterator_native_roots(id).unwrap(),
            vec![NodeRef {
                arena: dom.arena.id,
                node: reference
            }],
            "only the uncommitted lease remains owned"
        );
        dom.roots.get_mut(&reference).unwrap().native = 1;
        dom.release_node_iterator(id).unwrap();
        assert!(dom.node_iterator(id).is_err());
        assert!(dom.arena.get(reference).is_err());
    }
    #[test]
    fn native_root_query_preserves_duplicate_and_foreign_lease_identity() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow_mut().create_element("root");
        let id = owner.borrow_mut().create_node_iterator(root).unwrap();
        let root_ref = NodeRef {
            arena: owner.borrow().arena.id,
            node: root,
        };
        assert_eq!(
            owner.borrow().node_iterator_native_roots(id).unwrap(),
            vec![root_ref, root_ref]
        );
        assert_eq!(owner.borrow().native_root_count(root).unwrap(), 2);
        let foreign = target.borrow_mut().create_element("foreign");
        target.borrow_mut().retain_wrapper(foreign).unwrap();
        DomRuntime::set_node_iterator_reference_from(&owner, id, &target, foreign, false).unwrap();
        let foreign_ref = NodeRef {
            arena: target.borrow().arena.id,
            node: foreign,
        };
        assert_eq!(
            owner.borrow().node_iterator_native_roots(id).unwrap(),
            vec![root_ref, foreign_ref]
        );
        {
            let held = target.borrow();
            owner.borrow_mut().release_node_iterator(id).unwrap();
            assert!(owner.borrow().node_iterator_native_roots(id).is_err());
            assert_eq!(
                held.native_root_count(foreign).unwrap(),
                1,
                "queued release remains independent native custody"
            );
        }
        drain_node_iterator_releases().unwrap();
        assert_eq!(target.borrow().native_root_count(foreign).unwrap(), 0);
        target.borrow_mut().release_wrapper(foreign).unwrap();
    }
    #[test]
    fn queued_reference_reclamation_error_consumes_release_once() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow_mut().create_element("root");
        let id = owner.borrow_mut().create_node_iterator(root).unwrap();
        let reference = target.borrow_mut().create_text("foreign");
        DomRuntime::set_node_iterator_reference_from(&owner, id, &target, reference, false)
            .unwrap();
        target.borrow_mut().arena.get_mut(reference).unwrap().parent = Some(reference);
        owner.borrow_mut().release_node_iterator(id).unwrap();
        assert!(matches!(
            drain_node_iterator_releases(),
            Err(DomError::Cycle)
        ));
        assert_eq!(target.borrow().roots[&reference].native, 0);
        drain_node_iterator_releases().unwrap();
        let mut target = target.borrow_mut();
        target.arena.get_mut(reference).unwrap().parent = None;
        target.reclaim_if_unrooted(reference).unwrap();
        assert!(target.arena.get(reference).is_err());
    }
    #[test]
    fn queued_foreign_release_ignores_replaced_arena_with_reused_node_id() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow_mut().create_element("root");
        let id = owner.borrow_mut().create_node_iterator(root).unwrap();
        let old = target.borrow_mut().create_text("old");
        DomRuntime::set_node_iterator_reference_from(&owner, id, &target, old, false).unwrap();
        owner.borrow_mut().release_node_iterator(id).unwrap();
        let mut replacement = DomRuntime::new();
        let new = replacement.create_text("new");
        assert_eq!(old, new);
        let (arena, document) = replacement.snapshot();
        target
            .borrow_mut()
            .replace_parsed_document(arena, document)
            .unwrap();
        target.borrow_mut().retain_native(new).unwrap();
        drain_node_iterator_releases().unwrap();
        assert_eq!(target.borrow().roots[&new].native, 1);
    }
    #[test]
    fn runtime_replacement_retires_shared_positions_before_arena_change() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow_mut().create_element("div");
        let id = owner.borrow_mut().create_node_iterator(root).unwrap();
        let parent = target.borrow_mut().create_element("div");
        let reference = target.borrow_mut().create_text("reference");
        target.borrow_mut().append_child(parent, reference).unwrap();
        target.borrow_mut().retain_native(parent).unwrap();
        DomRuntime::set_node_iterator_reference_from(&owner, id, &target, reference, true).unwrap();
        let held = owner.borrow().node_iterators[&id].clone();
        let before = held.state.get();
        let replacement = DomRuntime::new();
        let (arena, document) = replacement.snapshot();
        owner
            .borrow_mut()
            .replace_parsed_document(arena, document)
            .unwrap();
        assert!(!held.alive.get());
        assert!(owner.borrow().node_iterator(id).is_err());
        drain_node_iterator_releases().unwrap();
        assert_eq!(target.borrow().roots[&reference].native, 0);
        target.borrow_mut().remove(reference).unwrap();
        assert_eq!(held.state.get(), before);
        drop(held);
        drain_node_iterator_releases().unwrap();
        let mutations = target.borrow_mut().drain_mutations();
        target
            .borrow_mut()
            .release_mutation_roots(&mutations)
            .unwrap();
    }
    #[test]
    fn parser_reparenting_adjusts_live_iterator_before_detach() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let a = dom.create_element("a");
        let b = dom.create_element("b");
        dom.append_child(root, a).unwrap();
        dom.append_child(root, b).unwrap();
        let iterator = dom.create_node_iterator(root).unwrap();
        dom.set_node_iterator_reference(iterator, a, true).unwrap();
        dom.parser_detach(a).unwrap();
        assert_eq!(
            dom.node_iterator(iterator).unwrap(),
            NodeIteratorState {
                root,
                reference: b,
                before: true
            }
        );
        assert_eq!(dom.parent(a).unwrap(), None);
        dom.append_child(b, a).unwrap();
        assert_eq!(dom.iterator_following(root, b, false).unwrap(), Some(a));
        assert_eq!(dom.node_iterator(iterator).unwrap().reference, b);
    }
    #[test]
    fn document_snapshot_does_not_alias_live_iterator_positions() {
        let mut source = DomRuntime::new();
        let root = source.create_element("div");
        let child = source.create_text("child");
        source.append_child(source.document(), root).unwrap();
        source.append_child(root, child).unwrap();
        let iterator = source.create_node_iterator(root).unwrap();
        source
            .set_node_iterator_reference(iterator, child, true)
            .unwrap();
        let (arena, document) = source.snapshot();
        let mut snapshot = DomRuntime::from_parsed(arena, document).unwrap();
        assert!(snapshot.node_iterator(iterator).is_err());
        let independent = snapshot.create_node_iterator(root).unwrap();
        snapshot
            .set_node_iterator_reference(independent, child, false)
            .unwrap();
        snapshot.remove(child).unwrap();
        assert_eq!(
            source.node_iterator(iterator).unwrap(),
            NodeIteratorState {
                root,
                reference: child,
                before: true
            }
        );
        assert_eq!(source.parent(child).unwrap(), Some(root));
    }
    #[test]
    fn foreign_reference_removal_and_deferred_release_keep_exact_custody() {
        let root_owner = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let root = root_owner.borrow_mut().create_element("root");
        let iterator = root_owner.borrow_mut().create_node_iterator(root).unwrap();
        let parent = target.borrow_mut().create_element("div");
        let first = target.borrow_mut().create_element("a");
        let second = target.borrow_mut().create_element("b");
        target.borrow_mut().append_child(parent, first).unwrap();
        target.borrow_mut().append_child(parent, second).unwrap();
        target.borrow_mut().retain_native(parent).unwrap();
        DomRuntime::set_node_iterator_reference_from(&root_owner, iterator, &target, first, true)
            .unwrap();
        assert_eq!(
            root_owner
                .borrow()
                .node_iterator_reference_arena(iterator)
                .unwrap(),
            target.borrow().arena.id
        );
        target.borrow_mut().remove(first).unwrap();
        assert_eq!(
            root_owner.borrow().node_iterator(iterator).unwrap(),
            NodeIteratorState {
                root,
                reference: second,
                before: true
            }
        );
        let held = target.borrow_mut();
        root_owner
            .borrow_mut()
            .release_node_iterator(iterator)
            .unwrap();
        drain_node_iterator_releases().unwrap();
        assert_eq!(held.roots[&second].native, 1);
        drop(held);
        drain_node_iterator_releases().unwrap();
        assert_eq!(target.borrow().roots[&second].native, 0);
    }
    #[test]
    fn foreign_reference_lease_is_released_when_root_runtime_drops() {
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let reference = target.borrow_mut().create_text("foreign");
        target.borrow_mut().retain_native(reference).unwrap();
        {
            let owner = Rc::new(RefCell::new(DomRuntime::new()));
            let root = owner.borrow_mut().create_element("root");
            let iterator = owner.borrow_mut().create_node_iterator(root).unwrap();
            DomRuntime::set_node_iterator_reference_from(
                &owner, iterator, &target, reference, false,
            )
            .unwrap();
            assert_eq!(target.borrow().roots[&reference].native, 2);
        }
        drain_node_iterator_releases().unwrap();
        assert_eq!(target.borrow().roots[&reference].native, 1);
    }
    #[test]
    fn removal_repositions_live_iterator_without_filter_calls() {
        let mut dom = DomRuntime::new();
        let root = dom.create_element("div");
        let a = dom.create_element("a");
        let b = dom.create_element("b");
        let c = dom.create_element("span");
        let nested = dom.create_text("nested");
        dom.append_child(root, a).unwrap();
        dom.append_child(root, b).unwrap();
        dom.append_child(root, c).unwrap();
        dom.append_child(b, nested).unwrap();
        let before = dom.create_node_iterator(root).unwrap();
        let after = dom.create_node_iterator(root).unwrap();
        let inner = dom.create_node_iterator(b).unwrap();
        dom.set_node_iterator_reference(before, nested, true)
            .unwrap();
        dom.set_node_iterator_reference(after, nested, false)
            .unwrap();
        dom.remove(b).unwrap();
        assert_eq!(
            dom.node_iterator(before).unwrap(),
            NodeIteratorState {
                root,
                reference: c,
                before: true
            }
        );
        assert_eq!(
            dom.node_iterator(after).unwrap(),
            NodeIteratorState {
                root,
                reference: a,
                before: false
            }
        );
        assert_eq!(dom.node_iterator(inner).unwrap().reference, b);
        assert_eq!(dom.iterator_following(root, a, false).unwrap(), Some(c));
        dom.remove(c).unwrap();
        assert_eq!(dom.node_iterator(before).unwrap().reference, a);
        assert!(!dom.node_iterator(before).unwrap().before);
    }
}
