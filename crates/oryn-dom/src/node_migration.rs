//! Native adoption planning. Record transfer and wrapper remapping consume one
//! complete plan; planning never detaches, clones or publishes a foreign node.
use super::*;

#[derive(Debug, PartialEq, Eq)]
struct MigrationNodeStructure {
    node: NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    template_content: Option<NodeId>,
    template_host: Option<NodeId>,
    attribute_owner: Option<NodeId>,
}

#[derive(Debug)]
pub struct NodeMigrationPlan {
    pub source_arena: ArenaId,
    pub destination_arena: ArenaId,
    pub root: NodeId,
    pub destination_document: NodeId,
    /// Includes materialized Attr records and non-light-tree template/shadow nodes.
    pub nodes: Vec<NodeId>,
    structure: Vec<MigrationNodeStructure>,
    /// Browsing context documents are not adopted DOM children.
    pub hosted_documents: Vec<(NodeId, NodeId)>,
}
// A dropped preparation releases its pins at the next bridge checkpoint.
// The queue keeps custody if a runtime is currently borrowed; no destructor
// reaches through an active RefCell borrow or silently loses a release.
thread_local! {static MIGRATION_PINS:RefCell<Vec<MigrationPins>>=const{RefCell::new(Vec::new())};}
struct MigrationPins {
    active: Rc<std::cell::Cell<bool>>,
    source: std::rc::Weak<RefCell<DomRuntime>>,
    source_node: Option<NodeId>,
    source_arena: ArenaId,
    destination: std::rc::Weak<RefCell<DomRuntime>>,
    destination_node: Option<NodeId>,
    destination_arena: ArenaId,
}
pub struct PreparedNodeMigration {
    pub plan: NodeMigrationPlan,
    active: Rc<std::cell::Cell<bool>>,
    committed: bool,
}
impl Drop for PreparedNodeMigration {
    fn drop(&mut self) {
        self.active.set(false);
    }
}
/// Called after dropping a prepared bridge transaction and at bridge checkpoints.
pub fn drain_node_migration_pins() -> Result<(), DomError> {
    MIGRATION_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        let mut failure = None;
        for pin in pins.iter_mut().filter(|pin| !pin.active.get()) {
            for (runtime, node, arena) in [
                (&pin.source, &mut pin.source_node, pin.source_arena),
                (
                    &pin.destination,
                    &mut pin.destination_node,
                    pin.destination_arena,
                ),
            ] {
                let Some(id) = *node else { continue };
                let Some(runtime) = runtime.upgrade() else {
                    *node = None;
                    continue;
                };
                if let Ok(mut runtime) = runtime.try_borrow_mut() {
                    if runtime.arena.id != arena {
                        *node = None;
                        continue;
                    }
                    if let Err(error) = runtime.release_native_with_commit(id, || *node = None) {
                        failure.get_or_insert(error);
                    }
                }
            }
        }
        pins.retain(|pin| pin.source_node.is_some() || pin.destination_node.is_some());
        failure.map_or(Ok(()), Err)
    })
}
#[derive(Debug)]
pub struct NodeMigration {
    pub root: NodeId,
    pub nodes: BTreeMap<NodeId, NodeId>,
    pub ranges: BTreeMap<LiveRangeId, LiveRangeId>,
    pub iterators: BTreeMap<NodeIteratorId, NodeIteratorId>,
    pub hosted_documents: Vec<(NodeId, NodeId)>,
    /// Host/content links that now cross arenas, in post-migration identities.
    pub external_template_links: Vec<(NodeRef, NodeRef)>,
    /// Cross-document ARIA references remain associated with their original
    /// identities for the wrapper bridge; local native projections exclude them.
    pub external_accessibility_references: Vec<(NodeRef, bool, String, Vec<NodeRef>)>,
}
fn move_node_map<T>(
    source: &mut BTreeMap<NodeId, T>,
    destination: &mut BTreeMap<NodeId, T>,
    nodes: &BTreeMap<NodeId, NodeId>,
) {
    for (old, new) in nodes {
        if let Some(value) = source.remove(old) {
            destination.insert(*new, value);
        }
    }
}
impl DomRuntime {
    pub fn commit_subtree_migration(
        source: &SharedDomRuntime,
        destination: &SharedDomRuntime,
        prepared: &mut PreparedNodeMigration,
    ) -> Result<NodeMigration, DomError> {
        if prepared.committed || Rc::ptr_eq(source, destination) {
            return Err(DomError::MigrationChanged);
        }
        let source_runtime = Rc::downgrade(source);
        let destination_runtime = Rc::downgrade(destination);
        let mut source = source.borrow_mut();
        let mut destination = destination.borrow_mut();
        let plan = &prepared.plan;
        if source.arena.id != plan.source_arena || destination.arena.id != plan.destination_arena {
            return Err(DomError::MigrationChanged);
        }
        let current =
            source.plan_subtree_migration(&destination, plan.root, plan.destination_document)?;
        if current.nodes != plan.nodes
            || current.structure != plan.structure
            || current.hosted_documents != plan.hosted_documents
            || source.parent(plan.root)?.is_some()
        {
            return Err(DomError::MigrationChanged);
        }
        if matches!(
            source.arena.get(plan.root)?.kind,
            NodeKind::Attribute { owner: Some(_), .. }
        ) {
            return Err(DomError::MigrationChanged);
        }
        let mut moved_ranges = Vec::new();
        let included = plan.nodes.iter().copied().collect::<BTreeSet<_>>();
        for (id, range) in &source.ranges {
            let start = included.contains(&range.start.container);
            let end = included.contains(&range.end.container);
            if start != end {
                return Err(DomError::MigrationChanged);
            }
            if start {
                moved_ranges.push(*id);
            }
        }
        let mut moved_iterators = Vec::new();
        for (id, record) in &source.node_iterators {
            if included.contains(&record.state.get().root) {
                moved_iterators.push(*id);
            }
        }
        destination
            .next_iterator
            .max(1)
            .checked_add(moved_iterators.len() as u64)
            .ok_or(DomError::MigrationCapacity)?;
        if !moved_ranges.is_empty() {
            destination
                .next_range
                .max(1)
                .checked_add(moved_ranges.len() as u64)
                .filter(|next| *next <= 1_u64 << 53)
                .ok_or(DomError::MigrationCapacity)?;
        }
        let additional = plan
            .nodes
            .len()
            .saturating_sub(destination.arena.free.len());
        if destination
            .arena
            .slots
            .len()
            .checked_add(additional + 1)
            .is_none_or(|length| length > u32::MAX as usize)
        {
            return Err(DomError::MigrationCapacity);
        }
        destination
            .arena
            .slots
            .try_reserve(additional + 1)
            .map_err(|_| DomError::MigrationCapacity)?;
        source
            .arena
            .free
            .try_reserve(plan.nodes.len())
            .map_err(|_| DomError::MigrationCapacity)?;
        // Allocate the destination inert owner before extracting original records.
        if plan.nodes.iter().any(|id| {
            source
                .arena
                .get(*id)
                .is_ok_and(|record| record.template_content.is_some())
        }) {
            destination
                .arena
                .template_contents_owner_document(plan.destination_document)?;
        }
        // Resolve every ownership edge before extracting any record. Publication
        // below must not encounter a fallible template-document allocation.
        let mut ownership = BTreeMap::new();
        let mut ownership_pending = vec![(plan.root, plan.destination_document)];
        while let Some((id, document)) = ownership_pending.pop() {
            ownership.insert(id, document);
            let record = source.arena.get(id)?;
            ownership_pending.extend(
                record
                    .children
                    .iter()
                    .filter(|child| included.contains(*child))
                    .map(|child| (*child, document)),
            );
            ownership_pending.extend(
                source
                    .arena
                    .attribute_node_ids(id)
                    .into_iter()
                    .map(|attr| (attr, document)),
            );
            if let Some(content) = record.template_content {
                let owner = destination
                    .arena
                    .template_contents_owner_document(document)?;
                ownership_pending.push((content, owner));
            }
        }
        // Validate journal lease accounting before any slot publication. Both
        // undrained entries and batches already held by a consumer are counted.
        for old in &plan.nodes {
            let count = source
                .mutations
                .outstanding_removed_roots
                .get(old)
                .copied()
                .unwrap_or(0);
            if source.roots.get(old).map_or(0, |roots| roots.native) < count {
                return Err(DomError::NativeRootUnderflow(*old));
            }
        }
        // Bulk publication writes slots directly instead of going through
        // get_mut. Both arenas must rebuild their derived membership index.
        source.arena.invalidate_registry_index();
        destination.arena.invalidate_registry_index();
        let mut nodes = BTreeMap::new();
        for old in &plan.nodes {
            let new = if let Some(slot) = destination.arena.free.pop() {
                NodeId {
                    slot,
                    generation: destination.arena.slots[slot as usize].generation,
                }
            } else {
                let slot = u32::try_from(destination.arena.slots.len())
                    .map_err(|_| DomError::MigrationCapacity)?;
                destination.arena.slots.push(Slot {
                    generation: 0,
                    node: None,
                });
                NodeId {
                    slot,
                    generation: 0,
                }
            };
            nodes.insert(*old, new);
        }
        for (host, document) in &plan.hosted_documents {
            if source.arena.get(*document)?.parent == Some(*host) {
                source.arena.get_mut(*document)?.parent = None;
            }
        }
        // Observer records have been materialized by the bridge before commit.
        // Settle source journal leases without reclaiming the still-moving graph;
        // preparation pins retain it until destination publication completes.
        for old in &plan.nodes {
            if let Some(count) = source.mutations.outstanding_removed_roots.remove(old) {
                source
                    .roots
                    .get_mut(old)
                    .expect("validated journal roots")
                    .native -= count;
                if let Some(leases) = source.mutations.removed_leases.remove(old) {
                    for lease in leases {
                        lease.consume();
                    }
                }
            }
        }
        let mut external_template_links = Vec::new();
        for old in &plan.nodes {
            if let Some(host) = source
                .arena
                .get(*old)
                .expect("validated migration record")
                .template_host
                && !nodes.contains_key(&host)
            {
                external_template_links.push((
                    NodeRef {
                        arena: plan.source_arena,
                        node: host,
                    },
                    NodeRef {
                        arena: plan.destination_arena,
                        node: nodes[old],
                    },
                ));
            }
        }
        // Detached nodes and orphan Attrs have no source removal record.
        // Ownership migration still invalidates XPath iterators associated with
        // their original documents. Publish only after all commit preflight.
        let original_documents = plan
            .nodes
            .iter()
            .filter_map(|node| {
                source
                    .arena
                    .get(*node)
                    .expect("validated migration record")
                    .owner_document
            })
            .collect::<BTreeSet<_>>();
        for document in original_documents {
            if let Some(revision) = source.xpath_revisions.get_mut(&document) {
                *revision = Rc::new(());
            }
        }
        let mut external_accessibility_references = Vec::new();
        for old in &plan.nodes {
            let new = nodes[old];
            let slot = source
                .arena
                .slot_mut(*old)
                .expect("validated migration slot");
            let mut record = slot.node.take().expect("validated migration record");
            if let Some(generation) = slot.generation.checked_add(1) {
                slot.generation = generation;
                source.arena.free.push(old.slot);
            }
            record.parent = record.parent.and_then(|id| nodes.get(&id).copied());
            record.children.retain(|id| nodes.contains_key(id));
            for id in &mut record.children {
                *id = nodes[id];
            }
            record.owner_document = Some(ownership[old]);
            if let Some(parser) = record.script_parser_document.as_mut()
                && parser.arena == source.arena.id()
                && let Some(document) = nodes.get(&parser.node)
            {
                *parser = NodeRef {
                    arena: destination.arena.id(),
                    node: *document,
                };
            }

            record.associated_inert_document = None;
            record.template_content = record
                .template_content
                .and_then(|id| nodes.get(&id).copied());
            record.template_host = record.template_host.and_then(|id| nodes.get(&id).copied());
            if let Some(assigned) = record.native_resolved_slot_nodes.as_mut() {
                *assigned = assigned
                    .iter()
                    .filter_map(|id| nodes.get(id).copied())
                    .collect();
            }
            // Constructed sheets cannot remain adopted by another Document.
            record.adopted_stylesheets.clear();
            record.permission_granted = false;
            // UI requests were admitted by the previous document. Preserve
            // control values/files, but never transfer an outstanding chooser
            // or permission capability into the destination document.
            record.file_picker = None;
            record.temporal_picker = None;
            record.permission_prompt = None;
            if let NodeKind::Attribute { owner, .. } = &mut record.kind {
                *owner = owner.and_then(|id| nodes.get(&id).copied());
            }
            for (explicit, references) in [
                (false, &mut record.default_accessibility_references),
                (true, &mut record.explicit_accessibility_references),
            ] {
                for (name, ids) in references {
                    if ids.iter().any(|id| !nodes.contains_key(id)) {
                        let ordered = ids
                            .iter()
                            .map(|id| match nodes.get(id) {
                                Some(new) => NodeRef {
                                    arena: plan.destination_arena,
                                    node: *new,
                                },
                                None => NodeRef {
                                    arena: plan.source_arena,
                                    node: *id,
                                },
                            })
                            .collect();
                        external_accessibility_references.push((
                            NodeRef {
                                arena: plan.destination_arena,
                                node: new,
                            },
                            explicit,
                            name.clone(),
                            ordered,
                        ));
                    }
                    ids.retain(|id| nodes.contains_key(id));
                    for id in ids {
                        *id = nodes[id];
                    }
                }
            }
            destination.arena.slots[new.slot as usize].node = Some(record);
        }
        // References owned by nodes that stay in the source also need their
        // complete ordering preserved. Never leave stale source IDs in the
        // native reachability graph after moving their targets.
        for (index, slot) in source.arena.slots.iter_mut().enumerate() {
            let Some(record) = slot.node.as_mut() else {
                continue;
            };
            let owner = NodeRef {
                arena: plan.source_arena,
                node: NodeId {
                    slot: index as u32,
                    generation: slot.generation,
                },
            };
            if let Some(content) = record
                .template_content
                .and_then(|old| nodes.get(&old).copied())
            {
                let edge = (
                    owner,
                    NodeRef {
                        arena: plan.destination_arena,
                        node: content,
                    },
                );
                if !external_template_links.contains(&edge) {
                    external_template_links.push(edge);
                }
                record.template_content = None;
            }
            for (explicit, references) in [
                (false, &mut record.default_accessibility_references),
                (true, &mut record.explicit_accessibility_references),
            ] {
                for (name, ids) in references {
                    if ids.iter().any(|id| nodes.contains_key(id)) {
                        let ordered = ids
                            .iter()
                            .map(|id| match nodes.get(id) {
                                Some(new) => NodeRef {
                                    arena: plan.destination_arena,
                                    node: *new,
                                },
                                None => NodeRef {
                                    arena: plan.source_arena,
                                    node: *id,
                                },
                            })
                            .collect();
                        external_accessibility_references.push((
                            owner,
                            explicit,
                            name.clone(),
                            ordered,
                        ));
                        ids.retain(|id| !nodes.contains_key(id));
                    }
                }
            }
        }
        // Native root counters move intact: wrapper remapping must not acquire
        // another lease or release the retired source slot.
        move_node_map(&mut source.roots, &mut destination.roots, &nodes);
        source.pending_stylesheet_reclaims.retain(|node| {
            if let Some(target) = nodes.get(node) {
                destination.pending_stylesheet_reclaims.insert(*target);
                false
            } else {
                true
            }
        });
        source.pending_cursor_releases.retain(|node| {
            if let Some(target) = nodes.get(node) {
                destination.pending_cursor_releases.push(*target);
                false
            } else {
                true
            }
        });
        move_node_map(
            &mut source.arena.stylesheet_revisions,
            &mut destination.arena.stylesheet_revisions,
            &nodes,
        );
        move_node_map(
            &mut source.arena.name_attribute_revisions,
            &mut destination.arena.name_attribute_revisions,
            &nodes,
        );
        move_node_map(
            &mut source.arena.accessibility_generated_content,
            &mut destination.arena.accessibility_generated_content,
            &nodes,
        );
        move_node_map(
            &mut source.arena.accessibility_computed_visibility,
            &mut destination.arena.accessibility_computed_visibility,
            &nodes,
        );
        move_node_map(
            &mut source.arena.accessibility_computed_text_style,
            &mut destination.arena.accessibility_computed_text_style,
            &nodes,
        );
        move_node_map(
            &mut source.custom_element_validity,
            &mut destination.custom_element_validity,
            &nodes,
        );
        move_node_map(
            &mut source.xpath_revisions,
            &mut destination.xpath_revisions,
            &nodes,
        );
        for (old, new) in &nodes {
            source.forget_pointer_hover_target(*old);
            source.connected.remove(old);
            source.internal_targets.remove(old);
            source.slot_elements.remove(old);
            source.frame_context_generations.remove(old);
            source.frame_navigation_identities.remove(old);
            source.radio_form_owners.remove(old);
            if source.arena.form_associated_custom_elements.remove(old) {
                destination
                    .arena
                    .form_associated_custom_elements
                    .insert(*new);
            }
            source.arena.hosted_documents.remove(old);
        }
        // Other source controls may remember a moved invoker/focus target.
        // These references belong to source UI state and cannot be rehomed.
        source
            .top_layer
            .retain(|entry| !nodes.contains_key(&entry.node));
        let mut changed_ui = Vec::new();
        for entry in &mut source.top_layer {
            let previous = *entry;
            entry.previous_focus = entry
                .previous_focus
                .filter(|node| !nodes.contains_key(node));
            entry.source = entry.source.filter(|node| !nodes.contains_key(node));
            entry.active_option = entry.active_option.filter(|node| !nodes.contains_key(node));
            if *entry != previous {
                changed_ui.push(entry.node);
            }
        }
        for node in changed_ui {
            source.record_mutation(Mutation::FormState { node });
        }
        source
            .popover_transitions
            .retain(|node| !nodes.contains_key(node));
        if source
            .popover_pointer_down
            .flatten()
            .is_some_and(|node| nodes.contains_key(&node))
        {
            source.popover_pointer_down = None;
        }
        if source.focused.is_some_and(|node| nodes.contains_key(&node)) {
            source.focused = None;
        }
        for old in &plan.nodes {
            if let Some(values) = source.manual_slot_nodes.remove(old) {
                destination.manual_slot_nodes.insert(
                    nodes[old],
                    values
                        .into_iter()
                        .filter_map(|id| nodes.get(&id).copied())
                        .collect(),
                );
            }
            if let Some(slot) = source
                .manual_node_slots
                .remove(old)
                .and_then(|slot| nodes.get(&slot).copied())
            {
                destination.manual_node_slots.insert(nodes[old], slot);
            }
            if let Some(values) = source.slot_assignment_snapshots.remove(old) {
                destination.slot_assignment_snapshots.insert(
                    nodes[old],
                    values
                        .into_iter()
                        .filter_map(|id| nodes.get(&id).copied())
                        .collect(),
                );
            }
        }
        for values in source.manual_slot_nodes.values_mut() {
            values.retain(|id| !nodes.contains_key(id));
        }
        for values in source.slot_assignment_snapshots.values_mut() {
            values.retain(|id| !nodes.contains_key(id));
        }
        let source_slots = source.slot_elements.iter().copied().collect::<Vec<_>>();
        for slot in source_slots {
            if let Ok(record) = source.arena.get_mut(slot)
                && let Some(assigned) = record.native_resolved_slot_nodes.as_mut()
            {
                assigned.retain(|id| !nodes.contains_key(id));
            }
        }
        source
            .manual_node_slots
            .retain(|node, slot| !nodes.contains_key(node) && !nodes.contains_key(slot));
        source.pending_slot_changes.retain(|old| {
            if let Some(new) = nodes.get(old) {
                destination.pending_slot_changes.push(*new);
                false
            } else {
                true
            }
        });
        source
            .arena
            .attribute_nodes
            .retain(|(owner, _), id| !nodes.contains_key(owner) && !nodes.contains_key(id));
        for new in nodes.values() {
            if let NodeKind::Attribute {
                owner: Some(owner),
                data,
            } = &destination
                .arena
                .get(*new)
                .expect("published migration record")
                .kind
            {
                let key = (*owner, data.key());
                destination.arena.attribute_nodes.insert(key, *new);
            }
        }
        let mut ranges = BTreeMap::new();
        source.note_range_endpoint_change();
        destination.note_range_endpoint_change();
        for id in moved_ranges {
            let mut range = source
                .ranges
                .remove(&id)
                .expect("validated migration range");
            range.start.container = nodes[&range.start.container];
            range.end.container = nodes[&range.end.container];
            let new = LiveRangeId(destination.next_range.max(1));
            destination.next_range = new.0 + 1;
            if source.composed_selection_ranges.remove(&id) {
                destination.composed_selection_ranges.insert(new);
            }
            if let Some(revision) = source.range_shadow_removals.remove(&id) {
                destination.range_shadow_removals.insert(new, revision);
            }
            destination.ranges.insert(new, range);
            ranges.insert(id, new);
        }
        let mut iterators = BTreeMap::new();
        for id in moved_iterators {
            let record = source
                .node_iterators
                .remove(&id)
                .expect("validated migration iterator");
            let new = NodeIteratorId(destination.next_iterator.max(1));
            destination.next_iterator = new.0 + 1;
            destination.node_iterators.insert(new, record);
            iterators.insert(id, new);
        }
        tracked_lease::migrate(
            &source_runtime,
            plan.source_arena,
            &destination_runtime,
            plan.destination_arena,
            &nodes,
        );
        node_iterators::migrate_references(
            plan.source_arena,
            source_runtime.clone(),
            plan.destination_arena,
            destination_runtime.clone(),
            &nodes,
        );
        destination.index_internal_targets();
        for new in nodes.values() {
            destination.record_mutation(Mutation::DocumentOwnership {
                node: *new,
                previous_document: None,
            });
        }
        // Every outstanding preparation lease follows its record, including
        // overlapping preparations and dropped pins awaiting a safe checkpoint.
        // Their old plans may no longer commit, but cleanup still owns a lease.
        MIGRATION_PINS.with(|pins| {
            for pin in pins.borrow_mut().iter_mut() {
                for (runtime, node, arena) in [
                    (&mut pin.source, &mut pin.source_node, &mut pin.source_arena),
                    (
                        &mut pin.destination,
                        &mut pin.destination_node,
                        &mut pin.destination_arena,
                    ),
                ] {
                    if *arena == plan.source_arena
                        && runtime.ptr_eq(&source_runtime)
                        && let Some(new) = node.and_then(|old| nodes.get(&old).copied())
                    {
                        *runtime = destination_runtime.clone();
                        *node = Some(new);
                        *arena = plan.destination_arena;
                    }
                }
            }
        });
        prepared.committed = true;
        Ok(NodeMigration {
            root: nodes[&plan.root],
            nodes,
            ranges,
            iterators,
            hosted_documents: plan.hosted_documents.clone(),
            external_template_links,
            external_accessibility_references,
        })
    }
}

impl DomRuntime {
    /// Split adoption at the observer-materialization boundary. Original records
    /// remain in the source arena until commit; dropping this preparation aborts
    /// relocation, but does not undo the observable standard removal algorithm.
    pub fn prepare_subtree_migration(
        source: &SharedDomRuntime,
        destination: &SharedDomRuntime,
        node: NodeId,
        document: NodeId,
    ) -> Result<PreparedNodeMigration, DomError> {
        drain_node_migration_pins()?;
        if Rc::ptr_eq(source, destination) {
            return Err(DomError::MigrationCapacity);
        }
        source.borrow_mut().drain_committed_tree_tails()?;
        destination.borrow_mut().drain_committed_tree_tails()?;
        let plan = source
            .borrow()
            .plan_subtree_migration(&destination.borrow(), node, document)?;
        MIGRATION_PINS.with(|pins| {
            let mut pins = pins.borrow_mut();
            if pins.len() >= 128 {
                return Err(DomError::MigrationCapacity);
            }
            pins.try_reserve(1).map_err(|_| DomError::MigrationCapacity)
        })?;
        {
            let mut target = destination.borrow_mut();
            let additional = plan.nodes.len().saturating_sub(target.arena.free.len());
            target
                .arena
                .slots
                .try_reserve(additional)
                .map_err(|_| DomError::MigrationCapacity)?;
            if target
                .roots
                .get(&document)
                .is_some_and(|roots| roots.native == u32::MAX)
            {
                return Err(DomError::MigrationCapacity);
            }
        }
        if source
            .borrow()
            .roots
            .get(&node)
            .is_some_and(|roots| roots.native >= u32::MAX - 1)
        {
            return Err(DomError::MigrationCapacity);
        }
        source.borrow_mut().retain_native(node)?;
        destination.borrow_mut().retain_native(document)?;
        let active = Rc::new(std::cell::Cell::new(true));
        MIGRATION_PINS.with(|pins| {
            pins.borrow_mut().push(MigrationPins {
                active: active.clone(),
                source: Rc::downgrade(source),
                source_node: Some(node),
                source_arena: plan.source_arena,
                destination: Rc::downgrade(destination),
                destination_node: Some(document),
                destination_arena: plan.destination_arena,
            })
        });
        let prepared = PreparedNodeMigration {
            plan,
            active,
            committed: false,
        };
        let result = (|| {
            let mut source = source.borrow_mut();
            let owner = match source.arena.get(node)?.kind {
                NodeKind::Attribute { owner, .. } => owner,
                _ => None,
            };
            if let Some(owner) = owner {
                source.remove_attribute_node(owner, node)?;
            } else if source.parent(node)?.is_some() {
                source.remove(node)?;
            }
            source.disconnect_opaque_subtree(node)?;
            Ok(())
        })();
        if let Err(error) = result {
            drop(prepared);
            drain_node_migration_pins()?;
            return Err(error);
        }
        Ok(prepared)
    }
}

impl DomRuntime {
    pub fn plan_subtree_migration(
        &self,
        destination: &DomRuntime,
        node: NodeId,
        document: NodeId,
    ) -> Result<NodeMigrationPlan, DomError> {
        destination.arena.document_id(document)?;
        let root = self.arena.get(node)?;
        if matches!(root.kind, NodeKind::Document | NodeKind::ShadowRoot { .. }) {
            return Err(DomError::DocumentTreeEdge);
        }
        let mut pending = vec![node];
        let mut seen = BTreeSet::new();
        let mut nodes = Vec::new();
        let mut hosted_documents = Vec::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current) {
                return Err(DomError::Cycle);
            }
            let record = self.arena.get(current)?;
            nodes.push(current);
            for attribute in self.arena.attribute_node_ids(current) {
                self.arena.get(attribute)?;
                pending.push(attribute);
            }
            if let Some(content) = record.template_content {
                self.arena.get(content)?;
                pending.push(content);
            }
            if let Some(document) = self.arena.hosted_documents.get(&current) {
                self.arena.document_id(*document)?;
                hosted_documents.push((current, *document));
            }
            for child in record.children.iter().rev() {
                if matches!(self.arena.get(*child)?.kind, NodeKind::Document) {
                    if !hosted_documents.contains(&(current, *child)) {
                        hosted_documents.push((current, *child));
                    }
                } else {
                    pending.push(*child);
                }
            }
        }
        // Slot identity must be representable before any source removal. Actual
        // destination vector reservation belongs to the committing transaction.
        let additional = nodes.len().saturating_sub(destination.arena.free.len());
        if destination
            .arena
            .slots
            .len()
            .checked_add(additional)
            .is_none_or(|length| length > u32::MAX as usize)
        {
            return Err(DomError::MigrationCapacity);
        }
        let structure = nodes
            .iter()
            .map(|id| {
                let record = self.arena.get(*id)?;
                let owner = match record.kind {
                    NodeKind::Attribute { owner, .. } if *id != node => owner,
                    _ => None,
                };
                Ok(MigrationNodeStructure {
                    node: *id,
                    parent: if *id == node { None } else { record.parent },
                    children: record.children.clone(),
                    template_content: record.template_content,
                    template_host: record.template_host,
                    attribute_owner: owner,
                })
            })
            .collect::<Result<Vec<_>, DomError>>()?;
        Ok(NodeMigrationPlan {
            source_arena: self.arena.id,
            destination_arena: destination.arena.id,
            root: node,
            destination_document: document,
            nodes,
            structure,
            hosted_documents,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_migration_invalidates_only_original_xpath_documents_after_preflight() {
        for attribute in [false, true] {
            let source = Rc::new(RefCell::new(DomRuntime::new()));
            let target = Rc::new(RefCell::new(DomRuntime::new()));
            let element = source.borrow_mut().create_element("div");
            let root = if attribute {
                source
                    .borrow_mut()
                    .set_attribute(element, "title", "native")
                    .unwrap();
                let attr = source
                    .borrow_mut()
                    .attribute_node_ns(element, None, "title")
                    .unwrap()
                    .unwrap();
                source.borrow_mut().retain_native(attr).unwrap();
                source
                    .borrow_mut()
                    .remove_attribute_node(element, attr)
                    .unwrap();
                attr
            } else {
                element
            };
            let document = source.borrow().document();
            let unrelated = source.borrow_mut().create(NodeKind::Document);
            let revision = source
                .borrow_mut()
                .xpath_document_revision(document)
                .unwrap();
            let other = source
                .borrow_mut()
                .xpath_document_revision(unrelated)
                .unwrap();
            let destination = target.borrow().document();
            let mut prepared =
                DomRuntime::prepare_subtree_migration(&source, &target, root, destination).unwrap();
            assert!(
                source
                    .borrow()
                    .xpath_document_revision_matches(document, &revision)
            );
            prepared.plan.nodes.push(root);
            assert!(DomRuntime::commit_subtree_migration(&source, &target, &mut prepared).is_err());
            assert!(
                source
                    .borrow()
                    .xpath_document_revision_matches(document, &revision)
            );
            prepared.plan.nodes.pop();
            let moved =
                DomRuntime::commit_subtree_migration(&source, &target, &mut prepared).unwrap();
            assert!(
                !source
                    .borrow()
                    .xpath_document_revision_matches(document, &revision)
            );
            assert!(
                source
                    .borrow()
                    .xpath_document_revision_matches(unrelated, &other)
            );
            drop(prepared);
            drain_node_migration_pins().unwrap();
            if attribute {
                target.borrow_mut().release_native(moved.root).unwrap();
            }
        }
    }
    #[test]
    fn migrated_manual_slots_publish_destination_node_identities() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let (host, slot, child) = {
            let mut dom = source.borrow_mut();
            let host = dom.create_element("button");
            let document = dom.document();
            dom.append_child(document, host).unwrap();
            let shadow = dom.create(NodeKind::ShadowRoot {
                mode: super::super::ShadowMode::Open,
                clonable: false,
            });
            dom.set_manual_slot_assignment(shadow).unwrap();
            dom.append_child(host, shadow).unwrap();
            let slot = dom.create_element("slot");
            dom.append_child(shadow, slot).unwrap();
            let child = dom.create_element("span");
            dom.append_child(host, child).unwrap();
            dom.assign_slot_nodes(slot, &[child]).unwrap();
            (host, slot, child)
        };
        let document = target.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &target, host, document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&source, &target, &mut prepared).unwrap();
        let target = target.borrow();
        assert_eq!(
            target
                .arena
                .get(moved.nodes[&slot])
                .unwrap()
                .native_resolved_slot_nodes,
            Some(vec![moved.nodes[&child]])
        );
    }

    #[test]
    fn abandoned_pin_reclamation_error_does_not_repeat_release() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let node = source.borrow_mut().create_text("prepared");
        let document = target.borrow().document();
        let prepared =
            DomRuntime::prepare_subtree_migration(&source, &target, node, document).unwrap();
        source.borrow_mut().arena.get_mut(node).unwrap().parent = Some(node);
        drop(prepared);
        assert!(matches!(drain_node_migration_pins(), Err(DomError::Cycle)));
        assert_eq!(source.borrow().roots[&node].native, 0);
        drain_node_migration_pins().unwrap();
        let mut source = source.borrow_mut();
        source.arena.get_mut(node).unwrap().parent = None;
        source.reclaim_if_unrooted(node).unwrap();
        assert!(source.arena.get(node).is_err());
    }
    #[test]
    fn abandoned_preparation_does_not_release_replacement_arena_nodes() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let target = Rc::new(RefCell::new(DomRuntime::new()));
        let old = source.borrow_mut().create_text("old");
        let document = target.borrow().document();
        let prepared =
            DomRuntime::prepare_subtree_migration(&source, &target, old, document).unwrap();
        let mut replacement = DomRuntime::new();
        let new = replacement.create_text("replacement");
        assert_eq!(old, new);
        let (arena, root) = replacement.snapshot();
        source
            .borrow_mut()
            .replace_parsed_document(arena, root)
            .unwrap();
        source.borrow_mut().retain_native(new).unwrap();
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert_eq!(source.borrow().roots[&new].native, 1);
        assert_eq!(
            source.borrow().arena.text_content(new).unwrap(),
            "replacement"
        );
    }
    #[test]
    fn pending_foreign_iterator_release_follows_another_adoption() {
        let owner = Rc::new(RefCell::new(DomRuntime::new()));
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = owner.borrow_mut().create_element("div");
        let iterator = owner.borrow_mut().create_node_iterator(root).unwrap();
        let reference = source.borrow_mut().create_text("moving reference");
        source.borrow_mut().retain_native(reference).unwrap();
        DomRuntime::set_node_iterator_reference_from(&owner, iterator, &source, reference, false)
            .unwrap();
        let held = source.borrow_mut();
        owner.borrow_mut().release_node_iterator(iterator).unwrap();
        drain_node_iterator_releases().unwrap();
        assert_eq!(held.roots[&reference].native, 2);
        drop(held);
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, reference, document)
                .unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        assert_eq!(destination.borrow().roots[&moved.root].native, 3);
        drain_node_iterator_releases().unwrap();
        assert_eq!(destination.borrow().roots[&moved.root].native, 2);
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert_eq!(destination.borrow().roots[&moved.root].native, 1);
        destination.borrow_mut().release_native(moved.root).unwrap();
        assert!(destination.borrow().arena.get(moved.root).is_err());
    }

    #[test]
    fn iterator_root_only_adoption_keeps_reference_in_original_arena() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("div");
        let reference = source.borrow_mut().create_text("outside");
        source.borrow_mut().retain_native(reference).unwrap();
        let iterator = source.borrow_mut().create_node_iterator(root).unwrap();
        source
            .borrow_mut()
            .set_node_iterator_reference(iterator, reference, false)
            .unwrap();
        let source_arena = source.borrow().arena.id;
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        let id = moved.iterators[&iterator];
        assert_eq!(
            destination.borrow().node_iterator(id).unwrap(),
            NodeIteratorState {
                root: moved.root,
                reference,
                before: false
            }
        );
        assert_eq!(
            destination
                .borrow()
                .node_iterator_reference_arena(id)
                .unwrap(),
            source_arena
        );
        drop(prepared);
        drain_node_migration_pins().unwrap();
        destination.borrow_mut().release_node_iterator(id).unwrap();
        drain_node_iterator_releases().unwrap();
        assert_eq!(source.borrow().roots[&reference].native, 1);
        assert!(destination.borrow().arena.get(moved.root).is_err());
    }

    #[test]
    fn foreign_template_fragment_uses_its_original_parser_context() {
        let mut dom = DomRuntime::new();
        let fragment = dom.create_fragment();
        dom.set_inner_html_with_context(fragment, "<tr><td>cell</td></tr>", "template")
            .unwrap();
        let row = dom.children(fragment).unwrap()[0];
        assert!(dom.is_element(row, "http://www.w3.org/1999/xhtml", "tr"));
        let cell = dom.children(row).unwrap()[0];
        assert!(dom.is_element(cell, "http://www.w3.org/1999/xhtml", "td"));
        assert_eq!(dom.arena().text_content(fragment).unwrap(), "cell");
        let text = dom.create_text("unchanged");
        assert!(
            dom.set_inner_html_with_context(text, "<b>replacement</b>", "template")
                .is_err()
        );
        assert_eq!(dom.arena().text_content(text).unwrap(), "unchanged");
    }
    #[test]
    fn migration_moves_native_iterator_identity_and_exact_leases() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("div");
        let text = source.borrow_mut().create_text("value");
        source.borrow_mut().append_child(root, text).unwrap();
        let iterator = source.borrow_mut().create_node_iterator(root).unwrap();
        source
            .borrow_mut()
            .set_node_iterator_reference(iterator, text, false)
            .unwrap();
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        let transferred = moved.iterators[&iterator];
        assert_eq!(
            destination.borrow().node_iterator(transferred).unwrap(),
            NodeIteratorState {
                root: moved.root,
                reference: moved.nodes[&text],
                before: false
            }
        );
        assert!(source.borrow().node_iterator(iterator).is_err());
        let later = destination
            .borrow_mut()
            .create_node_iterator(document)
            .unwrap();
        assert_ne!(transferred, later);
        drop(prepared);
        drain_node_migration_pins().unwrap();
        destination
            .borrow_mut()
            .release_node_iterator(transferred)
            .unwrap();
        assert!(destination.borrow().arena.get(moved.root).is_err());
        destination
            .borrow_mut()
            .release_node_iterator(later)
            .unwrap();
    }

    #[test]
    fn imports_respect_shadow_clonability_and_shallow_graph_rules() {
        for clonable in [false, true] {
            for deep in [false, true] {
                let mut source = DomRuntime::new();
                let mut destination = DomRuntime::new();
                let host = source.create_element("div");
                let light = source.create_text("light");
                source.append_child(host, light).unwrap();
                let shadow = source.create(NodeKind::ShadowRoot {
                    mode: ShadowMode::Closed,
                    clonable,
                });
                let text = source.create_text("shadow");
                source.append_child(shadow, text).unwrap();
                source.append_child(host, shadow).unwrap();
                let imported = destination
                    .import_node_from(source.arena(), host, deep)
                    .unwrap();
                let children = destination.children(imported).unwrap();
                let copied_shadow = children.iter().copied().find(|node| {
                    matches!(
                        destination.arena().get(*node).unwrap().kind,
                        NodeKind::ShadowRoot { .. }
                    )
                });
                assert_eq!(copied_shadow.is_some(), clonable);
                assert_eq!(
                    destination.arena().text_content(imported).unwrap(),
                    if deep { "light" } else { "" }
                );
                if let Some(shadow) = copied_shadow {
                    assert!(matches!(
                        destination.arena().get(shadow).unwrap().kind,
                        NodeKind::ShadowRoot {
                            mode: ShadowMode::Closed,
                            clonable: true
                        }
                    ));
                    assert_eq!(destination.arena().text_content(shadow).unwrap(), "shadow");
                }
                assert_eq!(source.arena().text_content(shadow).unwrap(), "shadow");
                let cloned = source.clone_node(host, deep).unwrap();
                let cloned_shadow = source
                    .children(cloned)
                    .unwrap()
                    .iter()
                    .copied()
                    .find(|node| {
                        matches!(
                            source.arena().get(*node).unwrap().kind,
                            NodeKind::ShadowRoot { .. }
                        )
                    });
                assert_eq!(cloned_shadow.is_some(), clonable);
                if let Some(root) = cloned_shadow {
                    assert_eq!(source.arena().text_content(root).unwrap(), "shadow");
                }
            }
        }
    }

    #[test]
    fn migration_preserves_cross_arena_template_edge_metadata() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let host = source.borrow_mut().create_element("template");
        source.borrow_mut().retain_native(host).unwrap();
        let content = source.borrow_mut().create(NodeKind::DocumentFragment);
        source
            .borrow_mut()
            .arena
            .get_mut(host)
            .unwrap()
            .template_content = Some(content);
        source
            .borrow_mut()
            .arena
            .get_mut(content)
            .unwrap()
            .template_host = Some(host);
        let source_arena = source.borrow().arena.id;
        let destination_arena = destination.borrow().arena.id;
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, content, document)
                .unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        assert_eq!(
            moved.external_template_links,
            vec![(
                NodeRef {
                    arena: source_arena,
                    node: host
                },
                NodeRef {
                    arena: destination_arena,
                    node: moved.root
                }
            )]
        );
        assert!(
            source
                .borrow()
                .arena
                .get(host)
                .unwrap()
                .template_content
                .is_none()
        );
        assert!(
            destination
                .borrow()
                .arena
                .get(moved.root)
                .unwrap()
                .template_host
                .is_none()
        );
        // The bridge installs cross-arena traced wrapper links before releasing
        // the preparation; this native regression retains its own target lease.
        destination.borrow_mut().retain_native(moved.root).unwrap();
        drop(prepared);
        drain_node_migration_pins().unwrap();
    }

    #[test]
    fn migration_retires_source_ui_requests_and_foreign_auxiliary_targets() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let input = source.borrow_mut().create_element("input");
        source
            .borrow_mut()
            .set_attribute(input, "type", "file")
            .unwrap();
        let dialog = source.borrow_mut().create_element("dialog");
        let document = source.borrow().document();
        source.borrow_mut().append_child(document, input).unwrap();
        source.borrow_mut().append_child(document, dialog).unwrap();
        source.borrow_mut().retain_native(input).unwrap();
        source.borrow_mut().open_file_picker(input).unwrap();
        assert!(
            source
                .borrow()
                .arena
                .get(input)
                .unwrap()
                .file_picker
                .is_some()
        );
        source
            .borrow_mut()
            .add_to_top_layer(dialog, TopLayerKind::ModalDialog)
            .unwrap();
        {
            let mut source = source.borrow_mut();
            let entry = source.top_layer.last_mut().unwrap();
            entry.previous_focus = Some(input);
            entry.source = Some(input);
            entry.active_option = Some(input);
            source.popover_pointer_down(Some(input));
            source.begin_popover_transition(input);
        }
        let target_document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, input, target_document)
                .unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        destination
            .borrow_mut()
            .append_child(target_document, moved.root)
            .unwrap();
        assert!(
            destination
                .borrow()
                .file_picker_snapshot(moved.root)
                .unwrap()
                .is_none()
        );
        let source_state = source.borrow();
        let entry = source_state.top_layer.last().unwrap();
        assert_eq!(
            (entry.previous_focus, entry.source, entry.active_option),
            (None, None, None)
        );
        assert_eq!(entry.node, dialog);
        assert!(source_state.popover_pointer_down.is_none());
        assert!(!source_state.popover_transitions.contains(&input));
        drop(source_state);
        drop(prepared);
        drain_node_migration_pins().unwrap();
    }

    #[test]
    fn overlapping_preparation_cleanup_follows_repeated_migration() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let final_owner = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("div");
        source.borrow_mut().retain_native(root).unwrap();
        let document = destination.borrow().document();
        let mut first =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let mut overlap =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut first).unwrap();
        assert!(DomRuntime::commit_subtree_migration(&source, &destination, &mut overlap).is_err());
        assert_eq!(destination.borrow().roots[&moved.root].native, 3);
        let final_document = final_owner.borrow().document();
        let mut again = DomRuntime::prepare_subtree_migration(
            &destination,
            &final_owner,
            moved.root,
            final_document,
        )
        .unwrap();
        let moved_again =
            DomRuntime::commit_subtree_migration(&destination, &final_owner, &mut again).unwrap();
        assert_eq!(final_owner.borrow().roots[&moved_again.root].native, 4);
        drop(overlap);
        drop(first);
        drop(again);
        drain_node_migration_pins().unwrap();
        assert_eq!(final_owner.borrow().roots[&moved_again.root].native, 1);
        assert_eq!(destination.borrow().roots[&document].native, 0);
        final_owner
            .borrow_mut()
            .release_native(moved_again.root)
            .unwrap();
        assert!(final_owner.borrow().arena.get(moved_again.root).is_err());
    }

    #[test]
    fn migration_preserves_ordered_cross_arena_relations_in_both_directions() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("section");
        let child = source.borrow_mut().create_element("span");
        let outside = source.borrow_mut().create_element("label");
        source.borrow_mut().append_child(root, child).unwrap();
        source.borrow_mut().retain_native(root).unwrap();
        source.borrow_mut().retain_native(outside).unwrap();
        source
            .borrow_mut()
            .arena
            .get_mut(root)
            .unwrap()
            .explicit_accessibility_references
            .insert("aria-labelledby".into(), vec![child, outside, root]);
        source
            .borrow_mut()
            .arena
            .get_mut(outside)
            .unwrap()
            .default_accessibility_references
            .insert("aria-controls".into(), vec![outside, child, root]);
        let source_arena = source.borrow().arena.id;
        let destination_arena = destination.borrow().arena.id;
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        let new_root = NodeRef {
            arena: destination_arena,
            node: moved.root,
        };
        let new_child = NodeRef {
            arena: destination_arena,
            node: moved.nodes[&child],
        };
        let old_outside = NodeRef {
            arena: source_arena,
            node: outside,
        };
        assert!(moved.external_accessibility_references.contains(&(
            new_root,
            true,
            "aria-labelledby".into(),
            vec![new_child, old_outside, new_root]
        )));
        assert!(moved.external_accessibility_references.contains(&(
            old_outside,
            false,
            "aria-controls".into(),
            vec![old_outside, new_child, new_root]
        )));
        assert_eq!(
            source
                .borrow()
                .arena
                .get(outside)
                .unwrap()
                .default_accessibility_references["aria-controls"],
            vec![outside]
        );
        assert_eq!(
            destination
                .borrow()
                .arena
                .get(moved.root)
                .unwrap()
                .explicit_accessibility_references["aria-labelledby"],
            vec![moved.nodes[&child], moved.root]
        );
        drop(prepared);
        drain_node_migration_pins().unwrap();
    }

    #[test]
    fn migration_rejects_changed_topology_with_unchanged_preorder() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create_element("section");
        let first = source.borrow_mut().create_element("div");
        let second = source.borrow_mut().create_element("span");
        source.borrow_mut().append_child(root, first).unwrap();
        source.borrow_mut().append_child(root, second).unwrap();
        source.borrow_mut().retain_native(root).unwrap();
        let document = destination.borrow().document();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        source.borrow_mut().append_child(first, second).unwrap();
        let current = source
            .borrow()
            .plan_subtree_migration(&destination.borrow(), root, document)
            .unwrap();
        assert_eq!(prepared.plan.nodes, current.nodes);
        assert!(matches!(
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared),
            Err(DomError::MigrationChanged)
        ));
        assert_eq!(source.borrow().parent(second).unwrap(), Some(first));
        assert!(destination.borrow().children(document).unwrap().is_empty());
        drop(prepared);
        drain_node_migration_pins().unwrap();
    }

    #[test]
    fn migration_settles_retained_drained_and_evicted_removal_leases() {
        for mode in 0..3 {
            let source = Rc::new(RefCell::new(DomRuntime::new()));
            let destination = Rc::new(RefCell::new(DomRuntime::new()));
            let root = source.borrow_mut().create_element("section");
            source.borrow_mut().retain_native(root).unwrap();
            let source_document = source.borrow().document();
            source
                .borrow_mut()
                .append_child(source_document, root)
                .unwrap();
            // Include an older outstanding removal of the same node, in
            // addition to the source removal performed by preparation.
            source.borrow_mut().remove(root).unwrap();
            source
                .borrow_mut()
                .append_child(source_document, root)
                .unwrap();
            let document = destination.borrow().document();
            let mut prepared =
                DomRuntime::prepare_subtree_migration(&source, &destination, root, document)
                    .unwrap();
            assert_eq!(
                source.borrow().mutations.outstanding_removed_roots[&root],
                2
            );
            let held = if mode == 1 {
                source.borrow_mut().drain_mutations()
            } else {
                Vec::new()
            };
            if mode == 2 {
                for _ in 0..MutationJournal::MAX_ENTRIES {
                    source.borrow_mut().focus(None).unwrap();
                }
                assert_eq!(
                    source
                        .borrow()
                        .mutations
                        .discarded_removed_roots
                        .iter()
                        .map(|(node, _)| *node)
                        .collect::<Vec<_>>(),
                    vec![root, root]
                );
            }
            let moved =
                DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
            // Only the caller's retain and preparation pin move to destination.
            assert_eq!(destination.borrow().roots[&moved.root].native, 2);
            assert!(
                !source
                    .borrow()
                    .mutations
                    .outstanding_removed_roots
                    .contains_key(&root)
            );
            source.borrow_mut().release_mutation_roots(&held).unwrap();
            let remaining = source.borrow_mut().drain_mutations();
            source
                .borrow_mut()
                .release_mutation_roots(&remaining)
                .unwrap();
            assert_eq!(destination.borrow().roots[&moved.root].native, 2);
            drop(prepared);
            drain_node_migration_pins().unwrap();
            assert_eq!(destination.borrow().roots[&moved.root].native, 1);
            destination.borrow_mut().release_native(moved.root).unwrap();
            assert!(destination.borrow().arena.get(moved.root).is_err());
        }
    }

    #[test]
    fn abandoned_preparation_retains_release_until_source_borrow_ends() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create(NodeKind::Text {
            data: "original".into(),
        });
        source.borrow_mut().retain_native(root).unwrap();
        let document = destination.borrow().document.unwrap();
        let prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        assert_eq!(source.borrow().roots[&root].native, 2);
        let held = source.borrow();
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert_eq!(held.roots[&root].native, 2);
        drop(held);
        drain_node_migration_pins().unwrap();
        assert_eq!(source.borrow().roots[&root].native, 1);
        assert!(
            matches!(&source.borrow().arena.get(root).unwrap().kind,NodeKind::Text{data} if data=="original")
        );
        assert!(MIGRATION_PINS.with(|pins| pins.borrow().is_empty()));
    }
    #[test]
    fn commit_moves_original_storage_and_root_counters_without_cloning() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create(NodeKind::Text {
            data: "original allocated storage".into(),
        });
        source.borrow_mut().retain_native(root).unwrap();
        let address = match &source.borrow().arena.get(root).unwrap().kind {
            NodeKind::Text { data } => data.as_ptr(),
            _ => unreachable!(),
        };
        let document = destination.borrow().document.unwrap();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        assert_eq!(moved.nodes[&root], moved.root);
        assert!(source.borrow().arena.get(root).is_err());
        assert!(
            matches!(&destination.borrow().arena.get(moved.root).unwrap().kind,NodeKind::Text{data} if data.as_ptr()==address)
        );
        assert_eq!(
            destination
                .borrow()
                .arena
                .node_document(moved.root)
                .unwrap(),
            Some(document)
        );
        assert_eq!(destination.borrow().roots[&moved.root].native, 2);
        assert!(
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).is_err()
        );
        drop(prepared);
        drain_node_migration_pins().unwrap();
        assert_eq!(destination.borrow().roots[&moved.root].native, 1);
    }
    #[test]
    fn migrated_range_identity_does_not_collide_with_next_destination_range() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create(NodeKind::Text {
            data: "range".into(),
        });
        let old = source
            .borrow_mut()
            .create_live_range(
                BoundaryPoint {
                    container: root,
                    offset: 1,
                },
                BoundaryPoint {
                    container: root,
                    offset: 4,
                },
            )
            .unwrap();
        let document = destination.borrow().document.unwrap();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        let transferred = moved.ranges[&old];
        let next = destination
            .borrow_mut()
            .create_live_range(
                BoundaryPoint {
                    container: moved.root,
                    offset: 0,
                },
                BoundaryPoint {
                    container: moved.root,
                    offset: 0,
                },
            )
            .unwrap();
        assert_ne!(transferred, next);
        assert_eq!(
            destination.borrow().live_range(transferred).unwrap(),
            LiveRange {
                start: BoundaryPoint {
                    container: moved.root,
                    offset: 1
                },
                end: BoundaryPoint {
                    container: moved.root,
                    offset: 4
                }
            }
        );
        assert!(source.borrow().live_range(old).is_err());
        drop(prepared);
        drain_node_migration_pins().unwrap();
    }
    #[test]
    fn migrated_template_contents_use_destination_inert_document() {
        let source = Rc::new(RefCell::new(DomRuntime::new()));
        let destination = Rc::new(RefCell::new(DomRuntime::new()));
        let root = source.borrow_mut().create(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("template"),
            ),
        });
        source.borrow_mut().retain_native(root).unwrap();
        let content = source.borrow_mut().create(NodeKind::DocumentFragment);
        let text = source.borrow_mut().create(NodeKind::Text {
            data: "template".into(),
        });
        source.borrow_mut().append_child(content, text).unwrap();
        source
            .borrow_mut()
            .arena
            .get_mut(root)
            .unwrap()
            .template_content = Some(content);
        source
            .borrow_mut()
            .arena
            .get_mut(content)
            .unwrap()
            .template_host = Some(root);
        let document = destination.borrow().document.unwrap();
        let mut prepared =
            DomRuntime::prepare_subtree_migration(&source, &destination, root, document).unwrap();
        let moved =
            DomRuntime::commit_subtree_migration(&source, &destination, &mut prepared).unwrap();
        let destination = destination.borrow();
        let owner = destination
            .arena
            .node_document(moved.nodes[&content])
            .unwrap()
            .unwrap();
        assert_ne!(owner, document);
        assert_eq!(
            destination.arena.node_document(moved.nodes[&text]).unwrap(),
            Some(owner)
        );
        assert_eq!(
            destination
                .arena
                .get(moved.nodes[&content])
                .unwrap()
                .template_host,
            Some(moved.root)
        );
        assert_eq!(
            destination.arena.get(moved.root).unwrap().template_content,
            Some(moved.nodes[&content])
        );
        drop(destination);
        drop(prepared);
        drain_node_migration_pins().unwrap();
    }
    #[test]
    fn migration_plan_includes_template_graph_without_mutating_source() {
        let mut source = DomRuntime::new();
        let destination = DomRuntime::new();
        let root = source.create(NodeKind::Element {
            name: QualName::new(
                None,
                Namespace::from("http://www.w3.org/1999/xhtml"),
                LocalName::from("template"),
            ),
        });
        let content = source.create(NodeKind::DocumentFragment);
        let text = source.create(NodeKind::Text {
            data: "preserved".into(),
        });
        source.append_child(content, text).unwrap();
        source.arena.get_mut(root).unwrap().template_content = Some(content);
        source.arena.get_mut(content).unwrap().template_host = Some(root);
        let plan = source
            .plan_subtree_migration(&destination, root, destination.document.unwrap())
            .unwrap();
        assert_eq!(
            plan.nodes.iter().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from([root, content, text])
        );
        assert_eq!(source.arena.get(content).unwrap().children, vec![text]);
        assert!(
            matches!(&source.arena.get(text).unwrap().kind,NodeKind::Text{data} if data=="preserved")
        );
        assert!(
            source
                .plan_subtree_migration(
                    &destination,
                    source.document.unwrap(),
                    destination.document.unwrap()
                )
                .is_err()
        );
        // Malformed cyclic ownership rejects before either arena is touched.
        source.arena.get_mut(content).unwrap().children.push(root);
        assert!(matches!(
            source.plan_subtree_migration(&destination, root, destination.document.unwrap()),
            Err(DomError::Cycle)
        ));
        assert!(
            destination
                .arena
                .get(destination.document.unwrap())
                .unwrap()
                .children
                .is_empty()
        );
    }
}
