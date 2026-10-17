//! Native edit segments invert only fields/edges still owned by that segment.
//! Changes made by intervening author callbacks are never snapshot-restored.
use super::*;
pub(super) mod paragraph_split;
mod paragraph_recovery;
/// Read-only evidence for state that cannot be reconstructed from attributes.
/// File selection uses allocation identity without retaining or copying bytes.
#[derive(Clone, Debug)]
pub(super) struct ControlWitness {
    checkedness: Option<bool>,
    dirty_checkedness: bool,
    selectedness: Option<bool>,
    dirty_selectedness: bool,
    indeterminate: bool,
    user_validity: bool,
    files: std::sync::Weak<Vec<oryn_dom::SelectedFile>>,
}
impl ControlWitness {
    pub(super) fn capture(node: &oryn_dom::Node) -> Self {
        Self {
            checkedness: node.checkedness,
            dirty_checkedness: node.dirty_checkedness,
            selectedness: node.selectedness,
            dirty_selectedness: node.dirty_selectedness,
            indeterminate: node.indeterminate,
            user_validity: node.user_validity,
            files: std::sync::Arc::downgrade(&node.selected_files),
        }
    }
    fn matches(&self, node: &oryn_dom::Node) -> bool {
        self.checkedness == node.checkedness
            && self.dirty_checkedness == node.dirty_checkedness
            && self.selectedness == node.selectedness
            && self.dirty_selectedness == node.dirty_selectedness
            && self.indeterminate == node.indeterminate
            && self.user_validity == node.user_validity
            && self
                .files
                .ptr_eq(&std::sync::Arc::downgrade(&node.selected_files))
    }
}
#[derive(Clone, Debug)]
pub(super) struct Segment {
    pub before: EditingSnapshot,
    pub after: EditingSnapshot,
    progress: Rc<RefCell<Option<Progress>>>,
}
#[derive(Debug)]
struct Progress {
    redo: bool,
    eligible: BTreeSet<NodeId>,
    detach: VecDeque<NodeId>,
    insert: VecDeque<(NodeId, NodeId)>,
    detached: BTreeSet<NodeId>,
    selection: SelectionSnapshot,
    restore_selection: bool,
}
fn current(state: &V8BindingState, snapshot: &EditingSnapshot, node: NodeId) -> bool {
    snapshot
        .leases
        .entries
        .iter()
        .find(|(identity, _)| identity.node == node)
        .is_some_and(|(_, lease)| {
            let (dom, id) = lease.current();
            id == node
                && Rc::ptr_eq(&dom, &state.dom)
                && dom.borrow().arena().node_document(id).ok()
                    == Some(Some(snapshot.document_identity.0))
        })
}
fn text(kind: &NodeKind) -> Option<&str> {
    match kind {
        NodeKind::Text { data }
        | NodeKind::CDataSection { data }
        | NodeKind::Comment { data }
        | NodeKind::ProcessingInstruction { data, .. } => Some(data),
        _ => None,
    }
}
// A control can change without changing its DOM attributes or exposed value.
// Such a change also prevents removing the enclosing native-created subtree.
fn control_witness_matches(
    state: &V8BindingState,
    snapshot: &EditingSnapshot,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    if let Some(record) = snapshot.nodes.iter().find(|record| record.node == node) {
        let actual = custom_elements::capture_internals_revision(state, node);
        let matches = match (&record.internals_revision, actual) {
            (None, None) => true,
            (Some(expected), Some(actual)) => expected.ptr_eq(&actual),
            _ => false,
        };
        if !matches {
            return Ok(false);
        }
    }

    let Some(expected) = snapshot
        .controls
        .iter()
        .find(|control| control.node == node)
    else {
        return Ok(true);
    };
    let dom = state.dom.borrow();
    let actual = dom.arena().get(node)?;
    let value = dom.control_edit_value(node)?;
    let length = oryn_dom::dom_string::len(&value) as u32;
    let selection = state
        .text_control_selections
        .borrow()
        .get(&node)
        .cloned()
        .unwrap_or((length, length, "none".into()));
    Ok(value == expected.value
        && actual.user_edited_value == expected.user_edited
        && actual.dirty_value == expected.dirty_value
        && actual.pending_edit_value == expected.pending_edit
        && selection == expected.selection)
}
fn edges(snapshot: &EditingSnapshot) -> BTreeMap<NodeId, NodeId> {
    snapshot
        .nodes
        .iter()
        .flat_map(|record| {
            record
                .children
                .iter()
                .map(move |child| (*child, record.node))
        })
        .collect()
}
impl Segment {
    pub(super) fn new(before: EditingSnapshot, after: EditingSnapshot) -> Self {
        Self {
            before,
            after,
            progress: Rc::new(RefCell::new(None)),
        }
    }
    pub(super) fn compact(mut self, state: &V8BindingState) -> Result<Self, V8BindingError> {
        let mut changed = self
            .before
            .nodes
            .iter()
            .chain(self.after.nodes.iter())
            .filter(|record| {
                let before = self
                    .before
                    .nodes
                    .iter()
                    .find(|node| node.node == record.node);
                let after = self
                    .after
                    .nodes
                    .iter()
                    .find(|node| node.node == record.node);
                match (before, after) {
                    (Some(a), Some(b)) => {
                        a.children != b.children
                            || a.attributes != b.attributes
                            || a.kind != b.kind
                            || a.value != b.value
                            || a.sensitive_value != b.sensitive_value
                    }
                    _ => true,
                }
            })
            .map(|record| record.node)
            .collect::<BTreeSet<_>>();
        // Detached roots are retained by Context, so presence in the desired
        // lease set is not evidence that removing an inserted subtree is harmless.
        // Keep complete subtree witnesses for every edge that can be removed by
        // undo or redo; unchanged descendants otherwise disappear in compaction.
        let before_edges = edges(&self.before);
        let after_edges = edges(&self.after);
        let mut subtree_controls = BTreeSet::new();
        for (node, parent) in before_edges.iter().chain(&after_edges) {
            if before_edges.get(node) == after_edges.get(node)
                || before_edges.contains_key(node) && after_edges.contains_key(node)
            {
                continue;
            }
            let snapshot = if before_edges.get(node) == Some(parent) {
                &self.before
            } else {
                &self.after
            };
            let mut pending = vec![*node];
            let mut witnessed = BTreeSet::new();
            while let Some(node) = pending.pop() {
                if !witnessed.insert(node) {
                    continue;
                }
                changed.insert(node);
                subtree_controls.insert(node);
                if let Some(record) = snapshot.nodes.iter().find(|record| record.node == node) {
                    pending.extend(record.children.iter().copied());
                }
            }
        }
        let changed_controls = self
            .before
            .controls
            .iter()
            .chain(&self.after.controls)
            .filter(|control| {
                let before = self
                    .before
                    .controls
                    .iter()
                    .find(|candidate| candidate.node == control.node);
                let after = self
                    .after
                    .controls
                    .iter()
                    .find(|candidate| candidate.node == control.node);
                match (before, after) {
                    (Some(a), Some(b)) => {
                        a.value != b.value
                            || a.selection != b.selection
                            || a.user_edited != b.user_edited
                            || a.dirty_value != b.dirty_value
                            || a.pending_edit != b.pending_edit
                    }
                    _ => true,
                }
            })
            .map(|control| control.node)
            .collect::<BTreeSet<_>>();
        for snapshot in [&mut self.before, &mut self.after] {
            snapshot.controls.retain(|control| {
                changed_controls.contains(&control.node) || subtree_controls.contains(&control.node)
            });
            snapshot
                .nodes
                .retain(|record| changed.contains(&record.node));
            let mut retained = BTreeSet::from([snapshot.host]);
            for record in &snapshot.nodes {
                retained.insert(record.node);
                retained.extend(record.children.iter().copied());
            }
            retained.extend(snapshot.controls.iter().map(|control| control.node));
            retained.extend(snapshot.selection.anchor.map(|point| point.container));
            retained.extend(snapshot.selection.focus.map(|point| point.container));
            let mut entries = Vec::new();
            for node in retained {
                let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
                entries.push((lease.identity(), lease));
            }
            snapshot.serialized_bytes = std::mem::size_of::<EditingSnapshot>()
                .saturating_add(entries.len().saturating_mul(std::mem::size_of::<(
                    oryn_dom::NodeRef,
                    oryn_dom::TrackedNodeLease,
                )>()))
                .saturating_add(
                    snapshot
                        .controls
                        .iter()
                        .map(|control| {
                            control
                                .value
                                .len()
                                .saturating_add(
                                    control.pending_edit.as_ref().map_or(0, String::len),
                                )
                                .saturating_add(control.selection.2.len())
                                .saturating_add(std::mem::size_of::<EditingControlSnapshot>())
                        })
                        .sum::<usize>(),
                );
            for record in &snapshot.nodes {
                snapshot.serialized_bytes = snapshot
                    .serialized_bytes
                    .saturating_add(std::mem::size_of::<EditingNodeSnapshot>())
                    .saturating_add(
                        record
                            .children
                            .len()
                            .saturating_mul(std::mem::size_of::<NodeId>()),
                    )
                    .saturating_add(text(&record.kind).map_or(0, str::len))
                    .saturating_add(
                        record
                            .attributes
                            .iter()
                            .map(|(key, value)| key.len().saturating_add(value.len()))
                            .sum::<usize>(),
                    )
                    .saturating_add(record.value.as_ref().map_or(0, String::len));
            }
            snapshot.serialized_bytes =
                snapshot
                    .serialized_bytes
                    .saturating_add(snapshot.overrides.0.iter().fold(
                        0usize,
                        |bytes, (key, value)| {
                            bytes.saturating_add(key.len()).saturating_add(value.len())
                        },
                    ));
            snapshot.leases = Rc::new(EditingSnapshotLeases {
                entries,
                released: Cell::new(false),
            });
        }
        Ok(self)
    }
    pub(super) fn changed(&self) -> bool {
        !self.before.nodes.is_empty()
            || !self.after.nodes.is_empty()
            || self.before.selection != self.after.selection
            || self.before.overrides != self.after.overrides
            || self
                .before
                .controls
                .iter()
                .zip(&self.after.controls)
                .any(|(before, after)| {
                    before.value != after.value
                        || before.selection != after.selection
                        || before.user_edited != after.user_edited
                        || before.dirty_value != after.dirty_value
                        || before.pending_edit != after.pending_edit
                })
    }
    pub(super) fn bytes(&self) -> usize {
        self.before
            .serialized_bytes
            .saturating_add(self.after.serialized_bytes)
    }
    pub(super) fn apply(&self, state: &V8BindingState, redo: bool) -> Result<(), V8BindingError> {
        let (expected, desired) = if redo {
            (&self.before, &self.after)
        } else {
            (&self.after, &self.before)
        };
        if expected.document_identity
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            )
        {
            return Ok(());
        }
        let old = edges(expected);
        let new = edges(desired);
        if self
            .progress
            .borrow()
            .as_ref()
            .is_some_and(|progress| progress.redo != redo)
        {
            return Err(V8BindingError::Unsupported {
                interface: "Document".into(),
                member: "InvalidStateError".into(),
            });
        }
        if self.progress.borrow().is_none() {
            let nodes = old
                .keys()
                .chain(new.keys())
                .copied()
                .collect::<BTreeSet<_>>();
            let mut eligible = BTreeSet::new();
            {
                let dom = state.dom.borrow();
                for node in nodes {
                    let old_position = expected.nodes.iter().find_map(|record| {
                        record
                            .children
                            .iter()
                            .position(|child| *child == node)
                            .map(|index| (record.node, index))
                    });
                    let new_position = desired.nodes.iter().find_map(|record| {
                        record
                            .children
                            .iter()
                            .position(|child| *child == node)
                            .map(|index| (record.node, index))
                    });
                    if old_position == new_position {
                        continue;
                    }
                    if !current(state, expected, node) && !current(state, desired, node) {
                        continue;
                    }
                    if dom.parent(node)? != old.get(&node).copied() {
                        continue;
                    }
                    if let Some((parent, index)) = old_position {
                        let known = expected
                            .nodes
                            .iter()
                            .find(|record| record.node == parent)
                            .unwrap();
                        let actual = dom
                            .children(parent)?
                            .iter()
                            .filter(|child| known.children.contains(child))
                            .position(|child| *child == node);
                        if actual != Some(index) {
                            continue;
                        }
                    }
                    // Removing a native-created container must not remove author-added
                    // descendants or subsequent author changes inside that container.
                    if old.contains_key(&node) && !new.contains_key(&node) {
                        let subtree = dom.arena().descendants_inclusive(node)?;
                        if subtree.iter().any(|id| {
                            !control_witness_matches(state, expected, *id).unwrap_or(false)
                                || expected
                                    .nodes
                                    .iter()
                                    .find(|record| record.node == *id)
                                    .is_none_or(|record| {
                                        dom.arena()
                                            .get(*id)
                                            .map(|actual| {
                                                actual.children != record.children
                                                    || actual.attributes != record.attributes
                                                    || text(&actual.kind) != text(&record.kind)
                                                    || actual.value != record.value
                                                    || actual.sensitive_value
                                                        != record.sensitive_value
                                                    || !record.control_state.matches(actual)
                                            })
                                            .unwrap_or(true)
                                    })
                        }) {
                            continue;
                        }
                    }
                    if let Some(parent) = new.get(&node)
                        && (!current(state, desired, *parent) || dom.arena().get(*parent).is_err())
                    {
                        continue;
                    }
                    eligible.insert(node);
                }
            }
            let blocked = old
                .keys()
                .filter(|node| !new.contains_key(node) && !eligible.contains(node))
                .copied()
                .collect::<Vec<_>>();
            for root in blocked {
                let mut pending = vec![root];
                let mut seen = BTreeSet::new();
                while let Some(node) = pending.pop() {
                    if !seen.insert(node) {
                        continue;
                    }
                    eligible.remove(&node);
                    if let Some(record) = expected.nodes.iter().find(|record| record.node == node) {
                        pending.extend(record.children.iter().copied());
                    }
                }
            }
            let detach = eligible.iter().copied().collect();
            let insert = desired
                .nodes
                .iter()
                .flat_map(|record| {
                    record
                        .children
                        .iter()
                        .filter(|node| eligible.contains(node))
                        .map(move |node| (record.node, *node))
                })
                .collect();
            *self.progress.borrow_mut() = Some(Progress {
                redo,
                eligible,
                detach,
                insert,
                detached: BTreeSet::new(),
                selection: selection_snapshot(state),
                restore_selection: selection_snapshot(state) == expected.selection,
            });
        }
        // Cursor custody is shared by clones of the history unit. A failure after
        // detachment resumes insertion; it cannot lose the detached edge by testing
        // the original parent predicate again. Each retry still rejects author moves.
        loop {
            let Some(node) = self
                .progress
                .borrow()
                .as_ref()
                .unwrap()
                .detach
                .front()
                .copied()
            else {
                break;
            };
            let mut valid = self
                .progress
                .borrow()
                .as_ref()
                .unwrap()
                .eligible
                .contains(&node)
                && (current(state, expected, node) || current(state, desired, node));
            let parent = if valid {
                state.dom.borrow().parent(node)?
            } else {
                None
            };
            if valid && parent == old.get(&node).copied() {
                let dom = state.dom.borrow();
                let progress_guard = self.progress.borrow();
                let detached = &progress_guard.as_ref().unwrap().detached;
                if let Some(parent) = parent
                    && let Some(record) = expected.nodes.iter().find(|record| record.node == parent)
                {
                    let expected_index = record
                        .children
                        .iter()
                        .filter(|child| !detached.contains(child))
                        .position(|child| *child == node);
                    let actual_index = dom
                        .children(parent)?
                        .iter()
                        .filter(|child| record.children.contains(child))
                        .position(|child| *child == node);
                    valid = expected_index == actual_index;
                }
                if valid && old.contains_key(&node) && !new.contains_key(&node) {
                    valid = dom.arena().descendants_inclusive(node)?.iter().all(|id| {
                        control_witness_matches(state, expected, *id).unwrap_or(false)
                            && expected
                                .nodes
                                .iter()
                                .find(|record| record.node == *id)
                                .is_some_and(|record| {
                                    dom.arena().get(*id).is_ok_and(|actual| {
                                        actual.children
                                            == record
                                                .children
                                                .iter()
                                                .copied()
                                                .filter(|child| !detached.contains(child))
                                                .collect::<Vec<_>>()
                                            && actual.attributes == record.attributes
                                            && text(&actual.kind) == text(&record.kind)
                                            && actual.value == record.value
                                            && actual.sensitive_value == record.sensitive_value
                                            && record.control_state.matches(actual)
                                    })
                                })
                    });
                }
            }
            if !valid || parent != old.get(&node).copied() {
                let mut guard = self.progress.borrow_mut();
                let progress = guard.as_mut().unwrap();
                progress.eligible.remove(&node);
                if old.contains_key(&node) && !new.contains_key(&node) {
                    let mut pending = vec![node];
                    let mut seen = BTreeSet::new();
                    while let Some(node) = pending.pop() {
                        if !seen.insert(node) {
                            continue;
                        }
                        progress.eligible.remove(&node);
                        if let Some(record) =
                            expected.nodes.iter().find(|record| record.node == node)
                        {
                            pending.extend(record.children.iter().copied());
                        }
                    }
                }
                progress.detach.pop_front();
                continue;
            }
            let result = if parent.is_some() {
                state
                    .dom
                    .borrow_mut()
                    .remove(node)
                    .map_err(V8BindingError::from)
            } else {
                Ok(())
            };
            // Record actual native commit even if a lower layer reports a late error.
            if state.dom.borrow().parent(node)?.is_none() {
                let mut guard = self.progress.borrow_mut();
                let progress = guard.as_mut().unwrap();
                progress.detached.insert(node);
                progress.detach.pop_front();
            }
            result?;
            #[cfg(test)]
            if FAIL_AFTER_INVERSE_DETACH.with(|flag| flag.replace(false)) {
                return Err(V8BindingError::Allocation(
                    "injected inverse detach completion",
                ));
            }
        }
        loop {
            let Some((parent, node)) = self
                .progress
                .borrow()
                .as_ref()
                .unwrap()
                .insert
                .front()
                .copied()
            else {
                break;
            };
            let eligible = self
                .progress
                .borrow()
                .as_ref()
                .unwrap()
                .eligible
                .contains(&node);
            if !eligible
                || !current(state, desired, parent)
                || !current(state, expected, node) && !current(state, desired, node)
                || state.dom.borrow().parent(node)?.is_some()
            {
                self.progress
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .insert
                    .pop_front();
                continue;
            }
            let record = desired
                .nodes
                .iter()
                .find(|record| record.node == parent)
                .expect("retained insertion parent");
            let index = record
                .children
                .iter()
                .position(|child| *child == node)
                .expect("retained insertion child");
            let reference = record.children[index + 1..]
                .iter()
                .copied()
                .find(|candidate| {
                    state.dom.borrow().parent(*candidate).ok().flatten() == Some(parent)
                });
            let result = state
                .dom
                .borrow_mut()
                .insert_dom_node(parent, node, reference)
                .map_err(V8BindingError::from);
            if state.dom.borrow().parent(node)? == Some(parent) {
                self.progress
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .insert
                    .pop_front();
            }
            result?;
        }
        for target in &desired.nodes {
            let Some(source) = expected
                .nodes
                .iter()
                .find(|source| source.node == target.node)
            else {
                continue;
            };
            if !current(state, desired, target.node) {
                continue;
            }
            let actual = state.dom.borrow().arena().get(target.node)?.clone();
            if source.attributes != target.attributes {
                let keys = source
                    .attributes
                    .records()
                    .chain(target.attributes.records())
                    .map(|entry| (entry.namespace.clone(), entry.local_name.clone()))
                    .collect::<BTreeSet<_>>();
                for (namespace, local) in keys {
                    let find = |attributes: &oryn_dom::AttributeList| {
                        attributes
                            .records()
                            .find(|entry| entry.namespace == namespace && entry.local_name == local)
                            .map(|entry| (entry.name.clone(), entry.value.clone()))
                    };
                    let expected = find(&source.attributes);
                    let desired = find(&target.attributes);
                    if expected == desired || find(&actual.attributes) != expected {
                        continue;
                    }
                    let mut dom = state.dom.borrow_mut();
                    if let Some((name, value)) = desired {
                        dom.set_attribute_ns(target.node, namespace.clone(), name, value)?;
                    } else {
                        dom.remove_attribute_ns(target.node, namespace.as_deref(), &local)?;
                    }
                }
            }
            if text(&source.kind) != text(&target.kind)
                && text(&actual.kind) == text(&source.kind)
                && let Some(data) = text(&target.kind)
            {
                state.dom.borrow_mut().set_text(target.node, data)?;
            }
            if !desired
                .controls
                .iter()
                .any(|control| control.node == target.node)
                && source.value != target.value
                && actual.value == source.value
            {
                if let Some(value) = &target.value {
                    state.dom.borrow_mut().set_value(
                        target.node,
                        value.clone(),
                        target.sensitive_value,
                    )?;
                } else {
                    state
                        .dom
                        .borrow_mut()
                        .arena_mut()
                        .get_mut(target.node)?
                        .value = None;
                }
            }
        }
        for target in &desired.controls {
            let Some(source) = expected
                .controls
                .iter()
                .find(|source| source.node == target.node)
            else {
                continue;
            };
            if !current(state, desired, target.node) {
                continue;
            }
            let mut dom = state.dom.borrow_mut();
            if dom.control_edit_value(target.node)? != source.value {
                continue;
            }
            let record = dom.arena().get(target.node)?;
            let actual_flags = (
                record.user_edited_value,
                record.dirty_value,
                record.pending_edit_value.clone(),
            );
            let sensitive = record.sensitive_value;
            // Value, dirty state and unfinished user input are one native ownership
            // group. A callback can retain the same exposed value while changing its
            // pending edit or dirty state; restoring only the value would erase that
            // author operation and make later value-attribute updates behave incorrectly.
            if actual_flags
                == (
                    source.user_edited,
                    source.dirty_value,
                    source.pending_edit.clone(),
                )
            {
                if source.value != target.value {
                    dom.restore_control_edit_value(target.node, target.value.clone(), sensitive)?;
                }
                let record = dom.arena_mut().get_mut(target.node)?;
                record.user_edited_value = target.user_edited;
                record.dirty_value = target.dirty_value;
                record.pending_edit_value = target.pending_edit.clone();
            }
            if state.text_control_selections.borrow().get(&target.node) == Some(&source.selection) {
                state
                    .text_control_selections
                    .borrow_mut()
                    .insert(target.node, target.selection.clone());
            }
        }
        let selection = selection_snapshot(state);
        {
            let mut guard = self.progress.borrow_mut();
            let progress = guard.as_mut().unwrap();
            if selection != progress.selection {
                progress.restore_selection = false;
            }
        }
        synchronize_live_ranges(state)?;
        self.progress.borrow_mut().as_mut().unwrap().selection = selection_snapshot(state);
        if self.progress.borrow().as_ref().unwrap().restore_selection {
            match (desired.selection.anchor, desired.selection.focus) {
                (Some(start), Some(end)) => {
                    set_selection_boundaries(state, start, end)?;
                }
                _ => clear_selection(state),
            }
            let mut guard = self.progress.borrow_mut();
            let progress = guard.as_mut().unwrap();
            progress.selection = selection_snapshot(state);
            progress.restore_selection = false;
        }
        if *state.editing_overrides.borrow() == expected.overrides {
            *state.editing_overrides.borrow_mut() = desired.overrides.clone();
        }
        self.progress.borrow_mut().take();
        Ok(())
    }
}
#[cfg(test)]
thread_local! {static FAIL_AFTER_INVERSE_DETACH:Cell<bool>=const{Cell::new(false)};}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_inverse_retains_subtree_witnesses_and_independent_attributes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable=true><span id=existing>original</span></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (host, existing) = {
            let dom = state.dom.borrow();
            (
                dom.find_by_id("host").unwrap(),
                dom.find_by_id("existing").unwrap(),
            )
        };
        let before = capture_editing_snapshot(&state, host).unwrap();
        let (wrapper, text) = {
            let mut dom = state.dom.borrow_mut();
            let wrapper = dom.create_element("b");
            let text = dom.create_text("native");
            dom.append_child(wrapper, text).unwrap();
            dom.append_child(host, wrapper).unwrap();
            dom.set_attribute(existing, "title", "native").unwrap();
            (wrapper, text)
        };
        let after = capture_editing_snapshot(&state, host).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        assert!(segment.after.nodes.iter().any(|record| record.node == text));
        state
            .dom
            .borrow_mut()
            .set_attribute(existing, "data-author", "kept")
            .unwrap();
        segment.apply(&state, false).unwrap();
        assert_eq!(state.dom.borrow().parent(wrapper).unwrap(), None);
        assert_eq!(
            state
                .dom
                .borrow()
                .attribute_in_null_namespace(existing, "title")
                .unwrap(),
            None
        );
        assert_eq!(
            state
                .dom
                .borrow()
                .attribute_in_null_namespace(existing, "data-author")
                .unwrap(),
            Some("kept")
        );
        segment.apply(&state, true).unwrap();
        assert_eq!(state.dom.borrow().parent(wrapper).unwrap(), Some(host));
        let authored = state.dom.borrow_mut().create_text("author");
        state
            .dom
            .borrow_mut()
            .append_child(wrapper, authored)
            .unwrap();
        segment.apply(&state, false).unwrap();
        assert_eq!(state.dom.borrow().parent(wrapper).unwrap(), Some(host));
        assert_eq!(state.dom.borrow().parent(authored).unwrap(), Some(wrapper));
    }
    #[test]
    fn untouched_controls_are_not_retained_as_inverse_authority() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable=true><input id=control value=original><span id=text>A</span></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (host, text, control) = {
            let dom = state.dom.borrow();
            (
                dom.find_by_id("host").unwrap(),
                dom.arena()
                    .get(dom.find_by_id("text").unwrap())
                    .unwrap()
                    .children[0],
                dom.find_by_id("control").unwrap(),
            )
        };
        let before = capture_editing_snapshot(&state, host).unwrap();
        state
            .dom
            .borrow_mut()
            .replace_character_data(text, 0, 1, "B")
            .unwrap();
        let after = capture_editing_snapshot(&state, host).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        assert!(segment.before.controls.is_empty());
        assert!(segment.after.controls.is_empty());
        state
            .dom
            .borrow_mut()
            .edit_control_value(control, 0, 8, "author")
            .unwrap();
        segment.apply(&state, false).unwrap();
        assert_eq!(
            state.dom.borrow().control_edit_value(control).unwrap(),
            "author"
        );
    }
    #[test]
    fn inverse_retry_keeps_detached_edges_and_author_control_value_state() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable=true><span id=a>A</span><span id=b>B</span><input id=control value=old></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (host, a, b, control) = {
            let dom = state.dom.borrow();
            (
                dom.find_by_id("host").unwrap(),
                dom.find_by_id("a").unwrap(),
                dom.find_by_id("b").unwrap(),
                dom.find_by_id("control").unwrap(),
            )
        };
        let before = capture_editing_snapshot(&state, host).unwrap();
        state
            .dom
            .borrow_mut()
            .insert_before(host, b, Some(a))
            .unwrap();
        state
            .dom
            .borrow_mut()
            .edit_control_value(control, 0, 3, "native")
            .unwrap();
        let after = capture_editing_snapshot(&state, host).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        state
            .dom
            .borrow_mut()
            .edit_control_value(control, 0, 6, "author")
            .unwrap();
        FAIL_AFTER_INVERSE_DETACH.with(|flag| flag.set(true));
        assert!(segment.apply(&state, false).is_err());
        let retry = segment.clone();
        retry.apply(&state, false).unwrap();
        assert_eq!(state.dom.borrow().children(host).unwrap()[..2], [a, b]);
        assert_eq!(
            state.dom.borrow().control_edit_value(control).unwrap(),
            "author"
        );
        assert!(state.dom.borrow().arena().get(control).unwrap().dirty_value);
        state
            .dom
            .borrow_mut()
            .set_attribute(control, "value", "later default")
            .unwrap();
        assert_eq!(
            state.dom.borrow().control_edit_value(control).unwrap(),
            "author"
        );
        assert!(segment.progress.borrow().is_none());
        segment.apply(&state, true).unwrap();
        assert_eq!(state.dom.borrow().children(host).unwrap()[..2], [b, a]);
    }
    #[test]
    fn inverse_retry_rechecks_remaining_subtree_and_keeps_its_native_children() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=host contenteditable=true></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = state.dom.borrow().find_by_id("host").unwrap();
        let before = capture_editing_snapshot(&state, host).unwrap();
        let (a, b, native) = {
            let mut dom = state.dom.borrow_mut();
            let a = dom.create_element("b");
            let b = dom.create_element("i");
            let native = dom.create_text("native");
            dom.append_child(b, native).unwrap();
            dom.append_child(host, a).unwrap();
            dom.append_child(host, b).unwrap();
            (a, b, native)
        };
        let after = capture_editing_snapshot(&state, host).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        FAIL_AFTER_INVERSE_DETACH.with(|flag| flag.set(true));
        assert!(segment.apply(&state, false).is_err());
        assert_eq!(state.dom.borrow().parent(a).unwrap(), None);
        let author = state.dom.borrow_mut().create_text("author");
        state.dom.borrow_mut().append_child(b, author).unwrap();
        segment.apply(&state, false).unwrap();
        assert_eq!(state.dom.borrow().parent(b).unwrap(), Some(host));
        assert_eq!(
            state.dom.borrow().children(b).unwrap(),
            vec![native, author]
        );
    }

    #[test]
    fn inverse_control_flags_and_pending_input_share_value_ownership() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><input id=control value=old></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let control = state.dom.borrow().find_by_id("control").unwrap();
        for changed_flag in 0..3 {
            {
                let mut dom = state.dom.borrow_mut();
                dom.restore_control_edit_value(control, "old".into(), false)
                    .unwrap();
                let record = dom.arena_mut().get_mut(control).unwrap();
                record.user_edited_value = false;
                record.dirty_value = false;
                record.pending_edit_value = None;
            }
            let before = capture_editing_snapshot(&state, control).unwrap();
            state
                .dom
                .borrow_mut()
                .edit_control_value(control, 0, 3, "native")
                .unwrap();
            let after = capture_editing_snapshot(&state, control).unwrap();
            let segment = Segment::new(before, after).compact(&state).unwrap();
            {
                let mut dom = state.dom.borrow_mut();
                let record = dom.arena_mut().get_mut(control).unwrap();
                match changed_flag {
                    0 => record.user_edited_value = !record.user_edited_value,
                    1 => record.dirty_value = !record.dirty_value,
                    _ => record.pending_edit_value = Some("author pending".into()),
                };
            }
            let expected = {
                let dom = state.dom.borrow();
                let record = dom.arena().get(control).unwrap();
                (
                    record.value.clone(),
                    record.user_edited_value,
                    record.dirty_value,
                    record.pending_edit_value.clone(),
                )
            };
            segment.apply(&state, false).unwrap();
            let dom = state.dom.borrow();
            let record = dom.arena().get(control).unwrap();
            assert_eq!(
                (
                    record.value.clone(),
                    record.user_edited_value,
                    record.dirty_value,
                    record.pending_edit_value.clone()
                ),
                expected,
                "intervening flag {changed_flag}"
            );
        }
    }
    #[test]
    fn compact_inverse_charges_retained_pending_input_bytes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><input id=control></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let control = state.dom.borrow().find_by_id("control").unwrap();
        let before = capture_editing_snapshot(&state, control).unwrap();
        let pending = "unfinished".repeat(4096);
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(control)
            .unwrap()
            .pending_edit_value = Some(pending.clone());
        let after = capture_editing_snapshot(&state, control).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        assert_eq!(
            segment.after.controls[0].pending_edit.as_deref(),
            Some(pending.as_str())
        );
        assert!(
            segment.after.serialized_bytes
                >= pending.len() + std::mem::size_of::<EditingControlSnapshot>()
        );
    }

    #[test]
    fn inverse_keeps_inserted_wrapper_when_unchanged_control_acquires_author_state() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><div id=host contenteditable=true><input id=control value=old></div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (host, control) = {
            let dom = state.dom.borrow();
            (
                dom.find_by_id("host").unwrap(),
                dom.find_by_id("control").unwrap(),
            )
        };
        let before = capture_editing_snapshot(&state, host).unwrap();
        let wrapper = {
            let mut dom = state.dom.borrow_mut();
            let wrapper = dom.create_element("b");
            dom.append_child(host, wrapper).unwrap();
            dom.append_child(wrapper, control).unwrap();
            wrapper
        };
        let after = capture_editing_snapshot(&state, host).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        assert!(
            segment
                .after
                .controls
                .iter()
                .any(|entry| entry.node == control)
        );
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(control)
            .unwrap()
            .dirty_value = true;
        segment.apply(&state, false).unwrap();
        assert_eq!(state.dom.borrow().parent(wrapper).unwrap(), Some(host));
        assert_eq!(state.dom.borrow().parent(control).unwrap(), Some(wrapper));
        assert!(state.dom.borrow().arena().get(control).unwrap().dirty_value);
    }

    #[test]
    fn inverse_subtree_witness_preserves_checkbox_option_and_file_state() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=host contenteditable=true><input id=check type=checkbox><option id=option>choice</option><input id=file type=file></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let (host, controls) = {
            let dom = state.dom.borrow();
            (
                dom.find_by_id("host").unwrap(),
                [
                    dom.find_by_id("check").unwrap(),
                    dom.find_by_id("option").unwrap(),
                    dom.find_by_id("file").unwrap(),
                ],
            )
        };
        let before = capture_editing_snapshot(&state, host).unwrap();
        let mut wrappers = Vec::new();
        for control in controls {
            let mut dom = state.dom.borrow_mut();
            let wrapper = dom.create_element("b");
            dom.append_child(host, wrapper).unwrap();
            dom.append_child(wrapper, control).unwrap();
            wrappers.push(wrapper);
        }
        let after = capture_editing_snapshot(&state, host).unwrap();
        let segment = Segment::new(before, after).compact(&state).unwrap();
        state
            .dom
            .borrow_mut()
            .set_checkedness(controls[0], true)
            .unwrap();
        state
            .dom
            .borrow_mut()
            .set_option_selected(controls[1], true)
            .unwrap();
        let files = std::sync::Arc::new(vec![oryn_dom::SelectedFile {
            name: "owned.bin".into(),
            media_type: "application/octet-stream".into(),
            bytes: std::sync::Arc::from([0u8, 255]),
            last_modified: 1,
        }]);
        state
            .dom
            .borrow_mut()
            .set_selected_files(controls[2], files.clone())
            .unwrap();
        segment.apply(&state, false).unwrap();
        for (control, wrapper) in controls.into_iter().zip(wrappers) {
            assert_eq!(state.dom.borrow().parent(wrapper).unwrap(), Some(host));
            assert_eq!(state.dom.borrow().parent(control).unwrap(), Some(wrapper));
        }
        assert!(state.dom.borrow().checked(controls[0]).unwrap());
        assert_eq!(
            state
                .dom
                .borrow()
                .arena()
                .get(controls[1])
                .unwrap()
                .selectedness,
            Some(true)
        );
        assert!(std::sync::Arc::ptr_eq(
            &state
                .dom
                .borrow()
                .arena()
                .get(controls[2])
                .unwrap()
                .selected_files,
            &files
        ));
    }

    #[test]
    fn inverse_preserves_author_internals_without_rooting_form_values() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn eval(scope: &mut v8::PinScope, source: &str) {
            let source = v8::String::new(scope, source).unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
        }
        eval(
            scope,
            "customElements.define('x-edit-witness',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.states.add('existing');this.i.ariaLabel='before'}})",
        );
        for (action, preserve) in [
            ("x.i.setFormValue('author')", true),
            ("x.i.setValidity({valueMissing:true},'Required')", true),
            ("x.i.states.add('author')", true),
            ("x.i.states.delete('existing')", true),
            ("x.i.states.clear()", true),
            ("x.i.ariaLabel='author'", true),
            ("x.i.ariaControlsElements=[host]", true),
            ("x.i.states.add('existing')", false),
            ("x.i.states.delete('absent')", false),
            ("x.i.ariaLabel='before'", false),
            ("try{x.i.setValidity({valueMissing:true},'')}catch{}", false),
        ] {
            eval(
                scope,
                "globalThis.host=document.createElement('div');host.contentEditable='true';document.body.append(host);globalThis.x=document.createElement('x-edit-witness');host.append(x);host.id='current-host';x.id='current-control'",
            );
            let (host, control) = {
                let dom = state.dom.borrow();
                (
                    dom.find_by_id("current-host").unwrap(),
                    dom.find_by_id("current-control").unwrap(),
                )
            };
            let before = capture_editing_snapshot(&state, host).unwrap();
            let wrapper = {
                let mut dom = state.dom.borrow_mut();
                let wrapper = dom.create_element("b");
                dom.append_child(host, wrapper).unwrap();
                dom.append_child(wrapper, control).unwrap();
                wrapper
            };
            let after = capture_editing_snapshot(&state, host).unwrap();
            let segment = Segment::new(before, after).compact(&state).unwrap();
            eval(scope, action);
            segment.apply(&state, false).unwrap();
            assert_eq!(
                state.dom.borrow().parent(wrapper).unwrap(),
                preserve.then_some(host),
                "{action}"
            );
            assert_eq!(
                state.dom.borrow().parent(control).unwrap(),
                Some(if preserve { wrapper } else { host }),
                "{action}"
            );
            eval(scope, "host.removeAttribute('id');x.removeAttribute('id')");
        }
    }
}
