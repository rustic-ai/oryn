// Native private Stylo mirror projection. Original selector ancestry is intact.
use blitz_dom::{BaseDocument, node::NodeData};
use blitz_traits::node_id::NodeId;
use crate::{LayoutError, cssom::native_import_graph::ImportAdmission};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct Admission(pub Arc<dyn ImportAdmission + Send + Sync>);
impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OwningSlotProjectionAdmission")
    }
}

pub(super) struct Projection {
    // Payload storage is released before its owning-scope charge.
    slots: Vec<(NodeId, Vec<NodeId>, Option<Arc<dyn Send + Sync>>)>,
    _metadata_lease: Option<Arc<dyn Send + Sync>>,
}
impl Projection {
    pub(super) fn publish(&mut self, document: &mut BaseDocument) -> Result<(), LayoutError> {
        if self.slots.iter().any(|(slot, _, _)| document.get_node(*slot).is_none()) {
            return Err(fail("native slot source changed before publication"));
        }
        for (slot, candidates, lease) in &mut self.slots {
            let node = document.get_node_mut(*slot).expect("validated native slot");
            node.native_flattened_slot_candidates = Some(std::mem::take(candidates));
            node.native_flattened_slot_charge = lease.take();
        }
        Ok(())
    }
}
fn fail(message: &str) -> LayoutError { LayoutError::StyleTraversal(message.into()) }
fn is_slot(node: &blitz_dom::node::Node) -> bool {
    node.containing_shadow.is_some()
        && node.element_data().is_some_and(|element| element.name.local.as_ref() == "slot"
            && element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
}

/// Iterative flattening follows assignment/fallback edges, not selector edges.
/// All allocation failures/cancellation abort this private mirror explicitly.
pub(super) fn prepare(
    document: &BaseDocument,
    admission: Option<&dyn ImportAdmission>,
    check: &mut impl FnMut() -> Result<(), LayoutError>,
) -> Result<Projection, LayoutError> {
    check()?;
    let tree = document.tree();
    let count = tree.len();
    let slots = tree.iter().filter(|(_, node)| is_slot(node)).count();
    if slots == 0 { return Ok(Projection { slots: Vec::new(), _metadata_lease: None }); }
    let scratch_bytes = count.checked_mul(std::mem::size_of::<NodeId>())
        .ok_or_else(|| fail("slot scratch capacity overflow"))?;
    let metadata = slots.checked_mul(std::mem::size_of::<(NodeId, Vec<NodeId>, Option<Arc<dyn Send + Sync>>)>() )
        .ok_or_else(|| fail("slot metadata capacity overflow"))?;
    let metadata_lease = admission.map(|a| a.reserve(metadata)).transpose().map_err(|e| fail(&e))?;
    check()?;
    let scratch_lease = admission.map(|a| a.reserve(scratch_bytes)).transpose().map_err(|e| fail(&e))?;
    check()?;
    let mut output = Projection { slots: Vec::new(), _metadata_lease: metadata_lease };
    output.slots.try_reserve_exact(slots).map_err(|_| fail("slot metadata allocation failed"))?;
    if output.slots.capacity() > slots { return Err(fail("slot metadata allocator capacity exceeded admission")); }
    let mut stack = Vec::<NodeId>::new();
    stack.try_reserve_exact(count).map_err(|_| fail("slot scratch allocation failed"))?;
    if stack.capacity() > count { return Err(fail("slot scratch allocator capacity exceeded admission")); }
    // Valid assignment/fallback edges form an acyclic forest. The canonical
    // tree cardinality detects malformed cycles, not a valid-depth limit.
    for (id, _) in tree.iter().filter(|(_, node)| is_slot(node)) {
        check()?;
        stack.clear();
        stack.push(id);
        let mut candidate_count = 0usize;
        let mut visits = 0usize;
        while let Some(current) = stack.pop() {
            check()?;
            visits = visits.checked_add(1).ok_or_else(|| fail("slot traversal overflow"))?;
            if visits > count { return Err(fail("cyclic native slot projection")); }
            let current = tree.get(current).ok_or_else(|| fail("missing native slot candidate"))?;
            if is_slot(current) {
                let children = current.style_children.as_deref().unwrap_or(&current.children);
                if stack.len().checked_add(children.len()).is_none_or(|length| length > count) {
                    return Err(fail("cyclic native slot projection"));
                }
                for child in children.iter().rev() { stack.push(*child); }
            } else if matches!(&current.data, NodeData::Element(_) | NodeData::Text(_)) {
                candidate_count = candidate_count.checked_add(1).ok_or_else(|| fail("slot candidate capacity overflow"))?;
            }
        }
        let bytes = candidate_count.checked_mul(std::mem::size_of::<NodeId>()).ok_or_else(|| fail("slot candidate capacity overflow"))?;
        let lease = admission.map(|a| a.reserve(bytes)).transpose().map_err(|e| fail(&e))?;
        check()?;
        let mut candidates = Vec::new();
        candidates.try_reserve_exact(candidate_count).map_err(|_| fail("slot candidate allocation failed"))?;
        if candidates.capacity() > candidate_count { return Err(fail("slot candidate allocator capacity exceeded admission")); }
        stack.push(id);
        let mut visits = 0usize;
        while let Some(current) = stack.pop() {
            check()?;
            visits = visits.checked_add(1).ok_or_else(|| fail("slot traversal overflow"))?;
            if visits > count { return Err(fail("cyclic native slot projection")); }
            let current = tree.get(current).ok_or_else(|| fail("missing native slot candidate"))?;
            if is_slot(current) {
                let children = current.style_children.as_deref().unwrap_or(&current.children);
                if stack.len().checked_add(children.len()).is_none_or(|length| length > count) {
                    return Err(fail("cyclic native slot projection"));
                }
                for child in children.iter().rev() { stack.push(*child); }
            } else if matches!(&current.data, NodeData::Element(_) | NodeData::Text(_)) {
                if candidates.len() == candidate_count { return Err(fail("native slot source changed during projection")); }
                candidates.push(current.id);
            }
        }
        if candidates.len() != candidate_count { return Err(fail("native slot source changed during projection")); }
        output.slots.push((id, candidates, lease));
    }
    drop(stack);
    drop(scratch_lease);
    check()?;
    Ok(output)
}
