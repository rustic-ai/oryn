//! Named slot assignment derived from the current native DOM. Keeping this
//! calculation native gives script accessors and event parenting one authority.
use super::{DomError, DomRuntime, NodeId, NodeKind, ShadowMode};

/// One immutable DOM borrow owns this cache; no entry survives a mutation.
pub struct InertChecker<'a> {
    dom: &'a DomRuntime,
    roots: ShadowRootLookup,
}
impl InertChecker<'_> {
    /// Reuse shadow-host discovery while projecting the same immutable DOM.
    pub fn assigned_slot_with_checkpoint(
        &mut self,
        node: NodeId,
        open_only: bool,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        self.dom.assigned_slot_with_shadow_lookup(node, open_only, &mut self.roots, checkpoint)
    }

    pub fn is_inert_with_checkpoint(
        &mut self,
        node: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<bool, DomError> {
        self.dom.is_inert_with_shadow_lookup(node, &mut self.roots, checkpoint)
    }
}

#[derive(Default)]
pub(super) struct ShadowRootLookup {
    cached: bool,
    roots: std::collections::BTreeMap<NodeId, Option<NodeId>>,
}
impl ShadowRootLookup {
    pub(super) fn root(
        &mut self,
        dom: &DomRuntime,
        host: NodeId,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        checkpoint()?;
        if self.cached && let Some(root) = self.roots.get(&host) {
            return Ok(*root);
        }
        let root = dom.attached_shadow_root_with_checkpoint(host, false, checkpoint)?;
        if self.cached { self.roots.insert(host, root); }
        Ok(root)
    }
}

impl DomRuntime {
    pub fn inert_checker(&self) -> InertChecker<'_> {
        InertChecker { dom: self, roots: ShadowRootLookup { cached: true, ..Default::default() } }
    }
    /// Click without pointer capture targets the nearest inclusive common
    /// ancestor of the down/up nodes. Disconnected endpoints cannot activate.
    pub fn pointer_click_target(
        &self,
        down: Option<NodeId>,
        up: Option<NodeId>,
    ) -> Result<Option<NodeId>, DomError> {
        let (Some(down), Some(up)) = (down, up) else {
            return Ok(None);
        };
        if !self.is_connected(down)?
            || !self.is_connected(up)?
            || self.arena.node_document(down)? != self.arena.node_document(up)?
        {
            return Ok(None);
        }
        let mut ancestors = std::collections::BTreeSet::new();
        let mut current = Some(down);
        while let Some(node) = current {
            ancestors.insert(node);
            let record = self.arena.get(node)?;
            current = if matches!(record.kind, NodeKind::Document) {
                None
            } else {
                record.parent
            };
        }
        let mut current = Some(up);
        while let Some(node) = current {
            if ancestors.contains(&node) {
                return Ok(Some(node));
            }
            let record = self.arena.get(node)?;
            current = if matches!(record.kind, NodeKind::Document) {
                None
            } else {
                record.parent
            };
        }
        Ok(None)
    }

    /// Snapshot pointer boundary targets before any out/leave listener can
    /// mutate the tree. Leaves run inside-out; enters run outside-in.
    pub fn pointer_boundary_targets(
        &self,
        previous: Option<NodeId>,
        next: Option<NodeId>,
    ) -> Result<(Vec<NodeId>, Vec<NodeId>), DomError> {
        let ancestry = |start: Option<NodeId>| -> Result<Vec<NodeId>, DomError> {
            let mut chain = Vec::new();
            let mut current = start;
            while let Some(node) = current {
                let record = self.arena.get(node)?;
                if matches!(record.kind, NodeKind::Element { .. }) {
                    chain.push(node);
                }
                current = self.flat_tree_parent(node)?;
            }
            Ok(chain)
        };
        let mut leaves = ancestry(previous)?;
        let mut enters = ancestry(next)?;
        while !leaves.is_empty() && !enters.is_empty() && leaves.last() == enters.last() {
            leaves.pop();
            enters.pop();
        }
        enters.reverse();
        Ok((leaves, enters))
    }

    /// DOM retargeting uses shadow-including ancestry, not slot projection.
    /// Callers capture results before dispatch so listener mutations cannot
    /// expose a related target from another shadow tree.
    pub fn retarget_node(
        &self,
        mut target: NodeId,
        against: Option<NodeId>,
    ) -> Result<NodeId, DomError> {
        loop {
            let root = self.control_tree_root(target)?;
            if !matches!(self.arena.get(root)?.kind, NodeKind::ShadowRoot { .. }) {
                return Ok(target);
            }
            let mut ancestor = against;
            while let Some(node) = ancestor {
                if node == root {
                    return Ok(target);
                }
                let record = self.arena.get(node)?;
                // Hosted document edges are not shadow-including ancestry.
                ancestor = if matches!(record.kind, NodeKind::Document) {
                    None
                } else {
                    record.parent
                };
            }
            let Some(host) = self.arena.get(root)?.parent else {
                return Ok(target);
            };
            target = host;
        }
    }

    /// Hosting edges are native relationships, independent of JavaScript's
    /// public shadowRoot visibility and of the light-DOM child collection.
    pub fn attached_shadow_root(
        &self,
        host: NodeId,
        open_only: bool,
    ) -> Result<Option<NodeId>, DomError> {
        self.attached_shadow_root_with_checkpoint(host, open_only, &mut || Ok(()))
    }
    pub fn attached_shadow_root_with_checkpoint(
        &self,
        host: NodeId,
        open_only: bool,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        checkpoint()?;
        for child in &self.arena.get(host)?.children {
            checkpoint()?;
            if let NodeKind::ShadowRoot { mode, .. } = self.arena.get(*child)?.kind {
                return Ok((!open_only || mode == ShadowMode::Open).then_some(*child));
            }
        }
        Ok(None)
    }

    pub(super) fn index_slot_elements(&mut self) {
        self.slot_elements = self.arena.iter().filter_map(|(id, node)| {
            matches!(&node.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot")
                .then_some(id)
        }).collect();
    }

    pub(super) fn register_slot_element(&mut self, node: NodeId) {
        if self.is_slot(node).unwrap_or(false) {
            self.slot_elements.insert(node);
        }
    }

    fn signal_slot_change(&mut self, slot: NodeId) -> Result<(), DomError> {
        if !self.pending_slot_changes.contains(&slot) {
            self.retain_native(slot)?;
            self.pending_slot_changes.push(slot);
        }
        Ok(())
    }

    pub(super) fn signal_slot_fallback_change(&mut self, parent: NodeId) -> Result<(), DomError> {
        if !self.slot_elements.contains(&parent) {
            return Ok(());
        }
        let root = self.control_tree_root(parent)?;
        if matches!(self.arena.get(root)?.kind, NodeKind::ShadowRoot { .. })
            && self.assigned_nodes(parent, false)?.is_empty()
        {
            self.signal_slot_change(parent)?;
        }
        Ok(())
    }

    /// Capture every effective change when its native mutation is published,
    /// rather than comparing only the final state at the microtask checkpoint.
    pub fn refresh_slot_assignments(&mut self) -> Result<(), DomError> {
        let slots = self.slot_elements.iter().copied().collect::<Vec<_>>();
        for slot in slots {
            let assigned = self.assigned_nodes(slot, false)?;
            let changed = self
                .slot_assignment_snapshots
                .get(&slot)
                .map_or(!assigned.is_empty(), |previous| previous != &assigned);
            self.arena.get_mut(slot)?.native_resolved_slot_nodes = Some(assigned.clone());
            self.slot_assignment_snapshots.insert(slot, assigned);
            if changed {
                // This is native projection state, not an authored attribute or
                // MutationObserver record. Semantic consumers must see assign()
                // even when no light-DOM parent or attribute changed.
                self.mutations
                    .push(super::Mutation::FormState { node: slot });
                self.signal_slot_change(slot)?;
            }
        }
        Ok(())
    }

    /// Borrow the head notification without transferring its native lease.
    /// Allocation failure in a consumer leaves this and later entries intact.
    pub fn pending_slot_change(&self) -> Option<NodeId> {
        self.pending_slot_changes.first().copied()
    }

    pub fn complete_slot_change(&mut self, node: NodeId) -> Result<(), DomError> {
        if let Some(index) = self
            .pending_slot_changes
            .iter()
            .position(|pending| *pending == node)
        {
            self.pending_slot_changes.remove(index);
            self.release_native(node)?;
        }
        Ok(())
    }

    pub fn set_manual_slot_assignment(&mut self, root: NodeId) -> Result<(), DomError> {
        self.arena.get_mut(root)?.manual_slot_assignment = true;
        self.refresh_slot_assignments()
    }

    pub fn has_manual_slot_assignment(&self, root: NodeId) -> bool {
        self.arena
            .get(root)
            .is_ok_and(|node| node.manual_slot_assignment)
    }

    /// Assignment edges are weak: native node reclamation clears both indexes.
    /// A node can be remembered by one slot even while outside its host.
    pub fn assign_slot_nodes(&mut self, slot: NodeId, nodes: &[NodeId]) -> Result<(), DomError> {
        self.arena.get(slot)?;
        for node in nodes {
            self.arena.get(*node)?;
        }
        let mut unique = Vec::new();
        for node in nodes {
            if !unique.contains(node) {
                unique.push(*node);
            }
        }
        if let Some(previous) = self.manual_slot_nodes.remove(&slot) {
            for node in previous {
                self.manual_node_slots.remove(&node);
            }
        }
        for node in &unique {
            if let Some(previous) = self.manual_node_slots.insert(*node, slot)
                && let Some(assigned) = self.manual_slot_nodes.get_mut(&previous)
            {
                assigned.retain(|candidate| candidate != node);
            }
        }
        self.manual_slot_nodes.insert(slot, unique);
        self.refresh_slot_assignments()?;
        Ok(())
    }

    fn is_slot(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(
            matches!(&self.arena.get(node)?.kind, NodeKind::Element { name }
            if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "slot"),
        )
    }

    pub fn is_slottable(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(matches!(
            self.arena.get(node)?.kind,
            NodeKind::Element { .. } | NodeKind::Text { .. } | NodeKind::CDataSection { .. }
        ))
    }

    /// The first matching slot in shadow-tree order wins, including empty slots.
    /// Nested shadow trees are separate assignment scopes.
    pub fn assigned_slot(&self, node: NodeId, open_only: bool) -> Result<Option<NodeId>, DomError> {
        self.assigned_slot_with_checkpoint(node, open_only, &mut || Ok(()))
    }
    pub fn assigned_slot_with_checkpoint(
        &self,
        node: NodeId,
        open_only: bool,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        self.assigned_slot_with_shadow_lookup(node, open_only, &mut ShadowRootLookup::default(), checkpoint)
    }
    pub(super) fn assigned_slot_with_shadow_lookup(
        &self,
        node: NodeId,
        open_only: bool,
        roots: &mut ShadowRootLookup,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Option<NodeId>, DomError> {
        checkpoint()?;
        if !self.is_slottable(node)? {
            return Ok(None);
        }
        let Some(host) = self.arena.get(node)?.parent else {
            return Ok(None);
        };
        let Some(root) = roots.root(self, host, checkpoint)?
        else {
            return Ok(None);
        };
        if open_only && matches!(self.arena.get(root)?.kind, NodeKind::ShadowRoot { mode: ShadowMode::Closed, .. }) {
            return Ok(None);
        }
        if self.has_manual_slot_assignment(root) {
            let candidate = self.manual_node_slots.get(&node).copied();
            return match candidate {
                Some(slot) => Ok((self.control_tree_root_with_checkpoint(slot, checkpoint)?
                    == root)
                    .then_some(slot)),
                None => Ok(None),
            };
        }
        let name = if matches!(self.arena.get(node)?.kind, NodeKind::Element { .. }) {
            self.attribute_in_null_namespace(node, "slot")?
                .unwrap_or("")
        } else {
            ""
        };
        let mut pending = Vec::new();
        for &child in self.arena.get(root)?.children.iter().rev() {
            checkpoint()?;
            pending.push(child);
        }
        while let Some(candidate) = pending.pop() {
            checkpoint()?;
            let record = self.arena.get(candidate)?;
            if matches!(
                record.kind,
                NodeKind::ShadowRoot { .. } | NodeKind::Document
            ) {
                continue;
            }
            if self.is_slot(candidate)?
                && self
                    .attribute_in_null_namespace(candidate, "name")?
                    .unwrap_or("")
                    == name
            {
                return Ok(Some(candidate));
            }
            for &child in record.children.iter().rev() {
                checkpoint()?;
                pending.push(child);
            }
        }
        Ok(None)
    }

    /// Borrow the original assignment inputs without allocating a projection.
    /// The boolean distinguishes assigned inputs from fallback children.
    pub fn borrow_slot_projection_inputs(&self, slot: NodeId) -> Result<(&[NodeId], bool), DomError> {
        let root = self.control_tree_root(slot)?;
        if !self.is_slot(slot)? || !matches!(self.arena.get(root)?.kind, NodeKind::ShadowRoot { .. }) {
            return Ok((&[], false));
        }
        let Some(host) = self.arena.get(root)?.parent else { return Ok((&[], false)); };
        let candidates = if self.has_manual_slot_assignment(root) {
            self.manual_slot_nodes.get(&slot).map(Vec::as_slice).unwrap_or(&[])
        } else { self.arena.get(host)?.children.as_slice() };
        Ok((candidates, true))
    }
    /// Assignment membership with caller-preallocated traversal storage.
    /// No allocation occurs here; insufficient storage is an operational error.
    pub fn slot_projection_member(&self, node: NodeId, slot: NodeId, scratch: &mut Vec<NodeId>, checkpoint: &mut impl FnMut() -> Result<(), DomError>) -> Result<bool, DomError> {
        checkpoint()?;
        scratch.clear();
        if !self.is_slottable(node)? { return Ok(false); }
        let Some(host) = self.arena.get(node)?.parent else { return Ok(false); };
        let Some(root) = self.attached_shadow_root(host, false)? else { return Ok(false); };
        if self.has_manual_slot_assignment(root) {
            return Ok(self.manual_node_slots.get(&node).copied() == Some(slot) && self.control_tree_root(slot)? == root);
        }
        let name = if matches!(self.arena.get(node)?.kind, NodeKind::Element { .. }) {
            self.attribute_in_null_namespace(node, "slot")?.unwrap_or("")
        } else { "" };
        let children = &self.arena.get(root)?.children;
        if children.len() > scratch.capacity() { return Err(DomError::SemanticProjectionCapacity); }
        scratch.extend(children.iter().rev().copied());
        while let Some(candidate) = scratch.pop() {
            checkpoint()?;
            let record = self.arena.get(candidate)?;
            if matches!(record.kind, NodeKind::ShadowRoot { .. } | NodeKind::Document) { continue; }
            if self.is_slot(candidate)? && self.attribute_in_null_namespace(candidate, "name")?.unwrap_or("") == name {
                scratch.clear();
                return Ok(candidate == slot);
            }
            if scratch.len().checked_add(record.children.len()).is_none_or(|n| n > scratch.capacity()) {
                return Err(DomError::SemanticProjectionCapacity);
            }
            scratch.extend(record.children.iter().rev().copied());
        }
        Ok(false)
    }

    pub fn assigned_nodes(&self, slot: NodeId, flatten: bool) -> Result<Vec<NodeId>, DomError> {
        self.assigned_nodes_with_checkpoint(slot, flatten, &mut || Ok(()))
    }
    pub fn assigned_nodes_with_checkpoint(
        &self,
        slot: NodeId,
        flatten: bool,
        checkpoint: &mut impl FnMut() -> Result<(), DomError>,
    ) -> Result<Vec<NodeId>, DomError> {
        checkpoint()?;
        let root = self.control_tree_root_with_checkpoint(slot, checkpoint)?;
        if !self.is_slot(slot)?
            || !matches!(self.arena.get(root)?.kind, NodeKind::ShadowRoot { .. })
        {
            return Ok(Vec::new());
        }
        let Some(host) = self.arena.get(root)?.parent else {
            return Ok(Vec::new());
        };
        let mut assigned = Vec::new();
        let candidates = if self.has_manual_slot_assignment(root) {
            self.manual_slot_nodes
                .get(&slot)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            self.arena.get(host)?.children.as_slice()
        };
        for child in candidates {
            checkpoint()?;
            if self.assigned_slot_with_checkpoint(*child, false, checkpoint)? == Some(slot) {
                assigned.push(*child);
            }
        }
        if !flatten {
            return Ok(assigned);
        }
        if assigned.is_empty() {
            for child in &self.arena.get(slot)?.children {
                checkpoint()?;
                if self.is_slottable(*child)? {
                    assigned.push(*child);
                }
            }
        }
        let mut result = Vec::new();
        for child in assigned {
            checkpoint()?;
            if self.is_slot(child)?
                && matches!(
                    self.arena
                        .get(self.control_tree_root_with_checkpoint(child, checkpoint)?)?
                        .kind,
                    NodeKind::ShadowRoot { .. }
                )
            {
                result.extend(self.assigned_nodes_with_checkpoint(child, true, checkpoint)?);
            } else {
                result.push(child);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    #[test]
    fn inert_checker_coalesces_wide_light_tree_shadow_queries() {
        let dom = html::parse(&format!("<main inert><section>{}</section></main>",
            "<span>x</span>".repeat(1024))).into_runtime().unwrap();
        let spans = dom.arena().iter().filter_map(|(id, node)|
            matches!(&node.kind, NodeKind::Element { name } if name.local.as_ref() == "span").then_some(id)
        ).collect::<Vec<_>>();
        let mut checker = dom.inert_checker();
        let mut checks = 0usize;
        for node in spans {
            assert_eq!(checker.assigned_slot_with_checkpoint(node, false, &mut || { checks += 1; Ok(()) }).unwrap(), None);
            assert!(checker.is_inert_with_checkpoint(node, &mut || { checks += 1; Ok(()) }).unwrap());
        }
        assert!(checks < 32768, "a wide light tree must not rescan its siblings for every inert query: {checks}");
    }

    #[test]
    fn inert_checker_preserves_closed_manual_slots_modal_rules_and_cancellation() {
        let mut dom = html::parse("<div id=host inert><button id=assigned></button></div><dialog id=modal><button id=inside></button></dialog>")
            .into_runtime().unwrap();
        let host = dom.find_by_id("host").unwrap();
        let assigned = dom.find_by_id("assigned").unwrap();
        let modal = dom.find_by_id("modal").unwrap();
        let root = dom.create(NodeKind::ShadowRoot { mode: ShadowMode::Closed, clonable: false });
        dom.set_manual_slot_assignment(root).unwrap();
        dom.append_child(host, root).unwrap();
        let slot = dom.create_element("slot");
        dom.append_child(root, slot).unwrap();
        dom.assign_slot_nodes(slot, &[assigned]).unwrap();
        let compare = |dom: &DomRuntime| {
            let nodes = dom.arena().iter().map(|(id, _)| id).collect::<Vec<_>>();
            let mut checker = dom.inert_checker();
            for node in nodes {
                for open_only in [false, true] {
                    assert_eq!(checker.assigned_slot_with_checkpoint(node, open_only, &mut || Ok(())).unwrap(), dom.assigned_slot(node, open_only).unwrap());
                }
                assert_eq!(checker.is_inert_with_checkpoint(node, &mut || Ok(())).unwrap(), dom.is_inert(node).unwrap());
            }
            assert!(matches!(checker.is_inert_with_checkpoint(assigned, &mut || Err(DomError::ProjectionCancelled)),
                Err(DomError::ProjectionCancelled)));
        };
        compare(&dom);
        dom.assign_slot_nodes(slot, &[]).unwrap();
        dom.remove_attribute(host, "inert").unwrap();
        compare(&dom);
        dom.add_to_top_layer(modal, crate::TopLayerKind::ModalDialog).unwrap();
        compare(&dom);
        dom.set_attribute(modal, "inert", "").unwrap();
        compare(&dom);
    }

    #[test]
    fn pointer_click_common_ancestor_requires_connected_endpoints() {
        let mut dom =
            html::parse("<section id=p><button id=a></button><button id=b></button></section>")
                .into_runtime()
                .unwrap();
        let parent = dom.find_by_id("p").unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        assert_eq!(
            dom.pointer_click_target(Some(a), Some(b)).unwrap(),
            Some(parent)
        );
        assert_eq!(dom.pointer_click_target(Some(a), Some(a)).unwrap(), Some(a));
        assert_eq!(dom.pointer_click_target(None, Some(a)).unwrap(), None);
        dom.retain_native(a).unwrap();
        dom.remove_child(parent, a).unwrap();
        assert_eq!(dom.pointer_click_target(Some(a), Some(b)).unwrap(), None);
        dom.release_native(a).unwrap();
    }

    #[test]
    fn flat_paint_elements_follow_manual_assignment_order_and_rendered_parents() {
        let mut dom = html::parse("<div id=host></div>").into_runtime().unwrap();
        let host = dom.find_by_id("host").unwrap();
        let root = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Closed,
            clonable: false,
        });
        dom.set_manual_slot_assignment(root).unwrap();
        dom.append_child(host, root).unwrap();
        let slot = dom.create_element("slot");
        dom.append_child(root, slot).unwrap();
        let fallback = dom.create_element("i");
        dom.append_child(slot, fallback).unwrap();
        let left = dom.create_element("button");
        dom.append_child(host, left).unwrap();
        let right = dom.create_element("button");
        dom.append_child(host, right).unwrap();
        let excluded = dom.create_element("span");
        dom.append_child(host, excluded).unwrap();
        dom.assign_slot_nodes(slot, &[right, left]).unwrap();
        assert_eq!(
            dom.flat_tree_elements(host).unwrap(),
            vec![
                (host, None),
                (slot, Some(host)),
                (right, Some(slot)),
                (left, Some(slot)),
            ]
        );
        dom.assign_slot_nodes(slot, &[]).unwrap();
        assert_eq!(
            dom.flat_tree_elements(host).unwrap(),
            vec![(host, None), (slot, Some(host)), (fallback, Some(slot)),]
        );
    }

    #[test]
    fn pointer_boundaries_exclude_shared_ancestors_and_use_manual_projection() {
        let mut dom = html::parse("<div id=host></div>").into_runtime().unwrap();
        let host = dom.find_by_id("host").unwrap();
        let root = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Closed,
            clonable: false,
        });
        dom.set_manual_slot_assignment(root).unwrap();
        dom.append_child(host, root).unwrap();
        let left_slot = dom.create_element("slot");
        let right_slot = dom.create_element("slot");
        dom.append_child(root, left_slot).unwrap();
        dom.append_child(root, right_slot).unwrap();
        let left = dom.create_element("button");
        let right = dom.create_element("button");
        dom.append_child(host, left).unwrap();
        dom.append_child(host, right).unwrap();
        dom.assign_slot_nodes(left_slot, &[left]).unwrap();
        dom.assign_slot_nodes(right_slot, &[right]).unwrap();
        let (leaves, enters) = dom
            .pointer_boundary_targets(Some(left), Some(right))
            .unwrap();
        assert_eq!(leaves, vec![left, left_slot]);
        assert_eq!(enters, vec![right_slot, right]);
        assert_eq!(
            dom.pointer_boundary_targets(Some(left), Some(left))
                .unwrap(),
            (vec![], vec![])
        );
        assert_eq!(
            dom.pointer_boundary_targets(Some(left), Some(host))
                .unwrap(),
            (vec![left, left_slot], vec![])
        );
        // Previously captured paths retain their order even after assignment changes.
        dom.assign_slot_nodes(left_slot, &[right, left]).unwrap();
        assert_eq!(enters, vec![right_slot, right]);
        assert_eq!(
            dom.pointer_boundary_targets(Some(left), Some(right))
                .unwrap(),
            (vec![left], vec![right])
        );
    }

    #[test]
    fn retargeting_crosses_only_shadow_boundaries_outside_the_observer_tree() {
        let mut dom = html::parse("<div id=host></div><p id=outside></p>")
            .into_runtime()
            .unwrap();
        let host = dom.find_by_id("host").unwrap();
        let outside = dom.find_by_id("outside").unwrap();
        let outer = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Closed,
            clonable: false,
        });
        dom.append_child(host, outer).unwrap();
        let inner_host = dom.create_element("section");
        dom.append_child(outer, inner_host).unwrap();
        let inner = dom.create(NodeKind::ShadowRoot {
            mode: ShadowMode::Open,
            clonable: false,
        });
        dom.append_child(inner_host, inner).unwrap();
        let target = dom.create_element("button");
        dom.append_child(inner, target).unwrap();
        assert_eq!(dom.retarget_node(target, Some(inner)).unwrap(), target);
        assert_eq!(dom.retarget_node(target, Some(outer)).unwrap(), inner_host);
        assert_eq!(
            dom.retarget_node(target, Some(inner_host)).unwrap(),
            inner_host
        );
        assert_eq!(dom.retarget_node(target, Some(outside)).unwrap(), host);
        assert_eq!(dom.retarget_node(target, None).unwrap(), host);
        let slot = dom.create_element("slot");
        dom.append_child(outer, slot).unwrap();
        let light = dom.create_element("i");
        dom.append_child(host, light).unwrap();
        assert_eq!(dom.assigned_slot(light, false).unwrap(), Some(slot));
        assert_eq!(dom.retarget_node(light, Some(slot)).unwrap(), light);
        assert_eq!(dom.retarget_node(light, Some(outside)).unwrap(), light);
    }

    #[test]
    fn pending_notifications_keep_their_leases_until_acknowledged() {
        let mut dom = html::parse("<div id=a></div><div id=b></div>")
            .into_runtime()
            .unwrap();
        let mut slots = Vec::new();
        for id in ["a", "b"] {
            let host = dom.find_by_id(id).unwrap();
            let root = dom.create(NodeKind::ShadowRoot {
                mode: ShadowMode::Open,
                clonable: false,
            });
            dom.append_child(host, root).unwrap();
            let slot = dom.create_element("slot");
            dom.append_child(root, slot).unwrap();
            let child = dom.create_element("i");
            dom.append_child(host, child).unwrap();
            slots.push(slot);
        }
        // A failed consumer leaves both notifications, their order and native
        // ownership unchanged; querying the head is not a destructive drain.
        assert_eq!(dom.pending_slot_change(), Some(slots[0]));
        assert_eq!(dom.pending_slot_change(), Some(slots[0]));
        assert_eq!(dom.roots.get(&slots[0]).unwrap().native, 1);
        assert_eq!(dom.roots.get(&slots[1]).unwrap().native, 1);
        dom.complete_slot_change(slots[0]).unwrap();
        assert_eq!(dom.pending_slot_change(), Some(slots[1]));
        dom.complete_slot_change(slots[0]).unwrap();
        assert_eq!(dom.pending_slot_change(), Some(slots[1]));
        dom.complete_slot_change(slots[1]).unwrap();
        assert_eq!(dom.pending_slot_change(), None);
        assert!(
            slots
                .iter()
                .all(|slot| dom.roots.get(slot).is_none_or(|roots| roots.native == 0))
        );
    }
}
