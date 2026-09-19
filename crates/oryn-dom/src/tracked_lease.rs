//! Native scoped roots whose release authority follows cross-arena adoption.
use super::*;
use std::cell::Cell;
use std::rc::Weak;
struct Record {
    owner: RefCell<(SharedDomRuntime, NodeRef)>,
    released: Cell<bool>,
    queued: Cell<bool>,
}
thread_local! {
    static MIGRATION_REVISION:Cell<u64>=const{Cell::new(0)};
    static RECORDS:RefCell<Vec<Weak<Record>>>=const{RefCell::new(Vec::new())};
    static PARSER_RECORDS:RefCell<Vec<Weak<Record>>>=const{RefCell::new(Vec::new())};
    static PENDING:RefCell<Vec<Rc<Record>>>=const{RefCell::new(Vec::new())};
}
pub struct TrackedNodeLease {
    record: Rc<Record>,
}
impl TrackedNodeLease {
    pub fn new(owner: &SharedDomRuntime, node: NodeId) -> Result<Self, DomError> {
        Self::new_in(owner, &mut owner.borrow_mut(), node)
    }
    pub fn new_in(
        owner: &SharedDomRuntime,
        dom: &mut DomRuntime,
        node: NodeId,
    ) -> Result<Self, DomError> {
        if !std::ptr::eq(owner.as_ptr(), dom as *mut DomRuntime) {
            return Err(DomError::MigrationChanged);
        }
        RECORDS.with(|records| {
            let mut records = records.borrow_mut();
            records.retain(|record| record.strong_count() > 0);
            if records.len() >= 65_536 {
                return Err(DomError::MigrationCapacity);
            }
            records
                .try_reserve(1)
                .map_err(|_| DomError::MigrationCapacity)
        })?;
        if dom
            .roots
            .get(&node)
            .is_some_and(|roots| roots.native == u32::MAX)
        {
            return Err(DomError::MigrationCapacity);
        }
        dom.retain_native(node)?;
        let identity = NodeRef {
            arena: dom.arena.id,
            node,
        };
        let record = Rc::new(Record {
            owner: RefCell::new((owner.clone(), identity)),
            released: Cell::new(false),
            queued: Cell::new(false),
        });
        RECORDS.with(|records| records.borrow_mut().push(Rc::downgrade(&record)));
        Ok(Self { record })
    }
    /// Parser tree handles follow the parser's existing node/source budget.
    /// They do not consume the bounded pool for temporary API operation roots.
    /// Like native DOM creation, storage allocation is infallible; retention is
    /// of an already-live parser node, before any authored construction.
    pub(crate) fn parser(owner: &SharedDomRuntime, node: NodeId) -> Self {
        Self::parser_in(owner, &mut owner.borrow_mut(), node).expect("live parser root")
    }
    /// Retained parser operations share the parser's node/source budget and
    /// can root a just-created node under the existing mutation borrow.
    pub(crate) fn parser_in(
        owner: &SharedDomRuntime,
        dom: &mut DomRuntime,
        node: NodeId,
    ) -> Result<Self, DomError> {
        if !std::ptr::eq(owner.as_ptr(), dom as *mut DomRuntime) {
            return Err(DomError::MigrationChanged);
        }
        dom.retain_native(node)?;
        let identity = NodeRef {
            arena: dom.arena.id,
            node,
        };
        let record = Rc::new(Record {
            owner: RefCell::new((owner.clone(), identity)),
            released: Cell::new(false),
            queued: Cell::new(false),
        });
        PARSER_RECORDS.with(|records| {
            let mut records = records.borrow_mut();
            if records.len() % 1024 == 0 {
                records.retain(|record| record.strong_count() > 0);
            }
            records.push(Rc::downgrade(&record));
        });
        Ok(Self { record })
    }
    /// Transfer an already-counted native root after its original owner has
    /// published a replacement. No counter is incremented here. Only retained
    /// native mutation cursors call this under their validated owner borrow.
    pub(crate) fn take_parser_root(
        owner: &SharedDomRuntime,
        dom: &DomRuntime,
        node: NodeId,
    ) -> Self {
        assert!(std::ptr::eq(
            owner.as_ptr(),
            dom as *const DomRuntime as *mut DomRuntime
        ));
        let record = Rc::new(Record {
            owner: RefCell::new((
                owner.clone(),
                NodeRef {
                    arena: dom.arena.id,
                    node,
                },
            )),
            released: Cell::new(false),
            queued: Cell::new(false),
        });
        PARSER_RECORDS.with(|records| {
            let mut records = records.borrow_mut();
            if records.len() % 1024 == 0 {
                records.retain(|record| record.strong_count() > 0);
            }
            records.push(Rc::downgrade(&record));
        });
        Self { record }
    }
    pub fn release_in(&self, dom: &mut DomRuntime) -> Result<(), DomError> {
        if self.record.released.get() {
            return Ok(());
        }
        let identity = self.identity();
        if dom.arena.id == identity.arena {
            return dom
                .release_native_with_commit(identity.node, || self.record.released.set(true));
        }
        match release(&self.record) {
            Ok(true) => Ok(()),
            Ok(false) => {
                enqueue(&self.record);
                Ok(())
            }
            Err(error) => {
                enqueue(&self.record);
                Err(error)
            }
        }
    }
    pub fn current(&self) -> (SharedDomRuntime, NodeId) {
        let owner = self.record.owner.borrow();
        (owner.0.clone(), owner.1.node)
    }
    pub fn identity(&self) -> NodeRef {
        self.record.owner.borrow().1
    }
}
impl std::fmt::Debug for TrackedNodeLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackedNodeLease")
            .field("identity", &self.identity())
            .field("released", &self.record.released.get())
            .finish()
    }
}
fn enqueue(record: &Rc<Record>) {
    if !record.queued.replace(true) {
        let _ = PENDING.try_with(|pending| pending.borrow_mut().push(record.clone()));
    }
}
fn release(record: &Record) -> Result<bool, DomError> {
    if record.released.get() {
        return Ok(true);
    }
    let (owner, identity) = &*record.owner.borrow();
    let Ok(mut dom) = owner.try_borrow_mut() else {
        return Ok(false);
    };
    // Arena replacement already discarded its entire root accounting.
    if dom.arena.id == identity.arena {
        dom.release_native_with_commit(identity.node, || record.released.set(true))?;
    }
    record.released.set(true);
    Ok(true)
}
impl Drop for TrackedNodeLease {
    fn drop(&mut self) {
        if !matches!(release(&self.record), Ok(true)) {
            enqueue(&self.record);
        }
    }
}
pub fn drain_tracked_node_leases() -> Result<(), DomError> {
    let pending = PENDING.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    let mut retained = Vec::new();
    let mut error = None;
    for record in pending {
        record.queued.set(false);
        match release(&record) {
            Ok(true) => {}
            Ok(false) => retained.push(record),
            Err(failure) => {
                error.get_or_insert(failure);
                retained.push(record);
            }
        }
    }
    for record in retained {
        enqueue(&record);
    }
    error.map_or(Ok(()), Err)
}
pub(crate) fn migration_revision() -> u64 {
    MIGRATION_REVISION.with(Cell::get)
}
pub(super) fn migrate(
    source: &Weak<RefCell<DomRuntime>>,
    source_arena: ArenaId,
    target: &Weak<RefCell<DomRuntime>>,
    arena: ArenaId,
    nodes: &BTreeMap<NodeId, NodeId>,
) {
    MIGRATION_REVISION.with(|revision| revision.set(revision.get().wrapping_add(1)));
    let target = target.upgrade().expect("live migration target");
    let update = |records: &RefCell<Vec<Weak<Record>>>| {
        let mut records = records.borrow_mut();
        records.retain(|record| record.strong_count() > 0);
        for record in records.iter().filter_map(Weak::upgrade) {
            if record.released.get() {
                continue;
            }
            let mut owner = record.owner.borrow_mut();
            if owner.1.arena == source_arena
                && Rc::downgrade(&owner.0).ptr_eq(source)
                && let Some(node) = nodes.get(&owner.1.node)
            {
                *owner = (target.clone(), NodeRef { arena, node: *node });
            }
        }
    };
    RECORDS.with(update);
    PARSER_RECORDS.with(update);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_local_id_cannot_receive_original_lease_release() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let node = source.borrow_mut().create_text("original");
        let lease = TrackedNodeLease::new(&source, node).unwrap();
        let (arena, document) = source.borrow().snapshot();
        let mut snapshot = DomRuntime::from_parsed(arena, document).unwrap();
        assert_ne!(snapshot.arena.id, lease.identity().arena);
        snapshot.retain_native(node).unwrap();
        // Equal local NodeIds in an independent snapshot do not convey the
        // original runtime's cleanup authority, even during a foreign release.
        let held = source.borrow_mut();
        lease.release_in(&mut snapshot).unwrap();
        assert_eq!(snapshot.roots[&node].native, 1);
        assert_eq!(held.roots[&node].native, 1);
        drop(held);
        drop(lease);
        drain_tracked_node_leases().unwrap();
        assert!(source.borrow().arena.get(node).is_err());
        assert_eq!(snapshot.arena.text_content(node).unwrap(), "original");
        assert_eq!(snapshot.roots[&node].native, 1);
        snapshot.release_native(node).unwrap();
        assert!(snapshot.arena.get(node).is_err());
    }
    #[test]
    fn reclamation_failure_does_not_repeat_committed_lease_release() {
        for deferred in [false, true] {
            let owner = Rc::new(RefCell::new(DomRuntime::new()));
            let node = owner.borrow_mut().create_text("cleanup");
            let lease = TrackedNodeLease::new(&owner, node).unwrap();
            // Corrupt only the graph to force a reclamation error after the
            // native root count has already reached zero.
            owner.borrow_mut().arena.get_mut(node).unwrap().parent = Some(node);
            if deferred {
                let held = owner.borrow_mut();
                drop(lease);
                drop(held);
                assert!(matches!(drain_tracked_node_leases(), Err(DomError::Cycle)));
                assert_eq!(owner.borrow().roots[&node].native, 0);
                // The queued retry acknowledges the committed release; it
                // must not underflow the count or keep retrying forever.
                drain_tracked_node_leases().unwrap();
            } else {
                let mut dom = owner.borrow_mut();
                assert!(matches!(lease.release_in(&mut dom), Err(DomError::Cycle)));
                assert!(lease.record.released.get());
                assert_eq!(dom.roots[&node].native, 0);
                lease.release_in(&mut dom).unwrap();
                drop(dom);
                drop(lease);
                drain_tracked_node_leases().unwrap();
            }
            let mut dom = owner.borrow_mut();
            dom.arena.get_mut(node).unwrap().parent = None;
            dom.reclaim_if_unrooted(node).unwrap();
            assert!(dom.arena.get(node).is_err());
        }
    }
    #[test]
    fn active_lease_tracks_repeated_adoption_and_foreign_release_queues_once() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let middle = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let node = source.borrow_mut().create_text("active");
        let lease = TrackedNodeLease::new(&source, node).unwrap();
        let document = middle.borrow().document();
        let mut first =
            DomRuntime::prepare_subtree_migration(&source, &middle, node, document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &middle, &mut first).unwrap();
        assert!(Rc::ptr_eq(&lease.current().0, &middle));
        assert_eq!(
            lease.identity(),
            NodeRef {
                arena: middle.borrow().arena.id,
                node: moved.root
            }
        );
        drop(first);
        drain_node_migration_pins().unwrap();
        let document = target.borrow().document();
        let mut second =
            DomRuntime::prepare_subtree_migration(&middle, &target, moved.root, document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&middle, &target, &mut second).unwrap();
        assert!(Rc::ptr_eq(&lease.current().0, &target));
        assert_eq!(lease.current().1, moved.root);
        let held = target.borrow_mut();
        let mut unrelated = DomRuntime::new();
        lease.release_in(&mut unrelated).unwrap();
        lease.release_in(&mut unrelated).unwrap();
        PENDING.with(|pending| {
            assert_eq!(
                pending
                    .borrow()
                    .iter()
                    .filter(|record| Rc::ptr_eq(record, &lease.record))
                    .count(),
                1
            )
        });
        drop(lease);
        drop(held);
        drain_tracked_node_leases().unwrap();
        assert_eq!(target.borrow().roots[&moved.root].native, 1);
        drop(second);
        drain_node_migration_pins().unwrap();
        assert!(target.borrow().arena.get(moved.root).is_err());
    }
    #[test]
    fn obsolete_lease_does_not_attach_to_reused_id_after_arena_replacement() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let old = source.borrow_mut().create_text("old");
        let obsolete = TrackedNodeLease::new(&source, old).unwrap();
        let mut replacement = DomRuntime::new();
        let fresh = replacement.create_text("fresh");
        assert_eq!(fresh, old);
        let (arena, document) = replacement.snapshot();
        source
            .borrow_mut()
            .replace_parsed_document(arena, document)
            .unwrap();
        source.borrow_mut().retain_native(fresh).unwrap();
        let target_document = target.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &target, fresh, target_document)
                .unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut prepared).unwrap();
        assert_eq!(target.borrow().roots[&moved.root].native, 2);
        drop(obsolete);
        drain_tracked_node_leases().unwrap();
        assert_eq!(target.borrow().roots[&moved.root].native, 2);
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert_eq!(target.borrow().roots[&moved.root].native, 1);
    }
    #[test]
    fn borrowed_acquisition_and_explicit_release_validate_owner_and_are_idempotent() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let node = owner.borrow_mut().create_text("scoped");
        let mut unrelated = DomRuntime::new();
        assert!(matches!(
            TrackedNodeLease::new_in(&owner, &mut unrelated, node),
            Err(DomError::MigrationChanged)
        ));
        let mut dom = owner.borrow_mut();
        dom.retain_native(node).unwrap();
        let lease = TrackedNodeLease::new_in(&owner, &mut dom, node).unwrap();
        assert_eq!(dom.roots[&node].native, 2);
        lease.release_in(&mut dom).unwrap();
        lease.release_in(&mut dom).unwrap();
        assert_eq!(dom.roots[&node].native, 1);
        drop(lease);
        drop(dom);
        drain_tracked_node_leases().unwrap();
        assert_eq!(owner.borrow().roots[&node].native, 1);
    }
    #[test]
    fn queued_scoped_release_follows_adoption_and_waits_for_borrow() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let node = source.borrow_mut().create_text("leased");
        let lease = TrackedNodeLease::new(&source, node).unwrap();
        let document = target.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &target, node, document).unwrap();
        let held = source.borrow_mut();
        drop(lease);
        drop(held);
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut prepared).unwrap();
        let held = target.borrow_mut();
        drain_tracked_node_leases().unwrap();
        assert_eq!(held.roots[&moved.root].native, 2);
        drop(held);
        drain_tracked_node_leases().unwrap();
        assert_eq!(target.borrow().roots[&moved.root].native, 1);
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert!(target.borrow().arena.get(moved.root).is_err());
    }
}
