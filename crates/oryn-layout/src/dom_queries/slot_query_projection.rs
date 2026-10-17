// Query-only known payload projection. Native immutable DOM, no V8 callbacks.
use super::{QueryError, DomRuntime, NodeId, NodeKind};
use crate::cssom::native_import_graph::ImportAdmission;
use std::sync::Arc;
pub(super) struct Projection {
    pub slots: Vec<(NodeId, Vec<NodeId>, Option<Arc<dyn Send + Sync>>)>,
    _metadata: Option<Arc<dyn Send + Sync>>,
}
fn error(message: &str) -> QueryError { QueryError::Operational(message.into()) }
fn reserve(admission: Option<&dyn ImportAdmission>, bytes: usize) -> Result<Option<Arc<dyn Send + Sync>>, QueryError> {
    if let Some(admission) = admission {
        admission.checkpoint().map_err(QueryError::Operational)?;
        let lease = admission.reserve(bytes).map_err(QueryError::Operational)?;
        admission.checkpoint().map_err(QueryError::Operational)?;
        Ok(Some(lease))
    } else { Ok(None) }
}
fn slot(dom: &DomRuntime, id: NodeId) -> Result<bool, QueryError> {
    let node = dom.arena().get(id).map_err(|e| error(&e.to_string()))?;
    Ok(matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref() == super::HTML && name.local.as_ref() == "slot")
        && matches!(dom.arena().get(dom.control_tree_root(id).map_err(|e| error(&e.to_string()))?).map_err(|e| error(&e.to_string()))?.kind, NodeKind::ShadowRoot { .. }))
}
pub(super) fn prepare(dom: &DomRuntime, root: NodeId, admission: Option<&dyn ImportAdmission>) -> Result<Projection, QueryError> {
    let document = dom.node_document(root).map_err(|e| error(&e.to_string()))?;
    let mut count = 0usize;
    let mut slot_count = 0usize;
    for (id, _) in dom.arena().iter() {
        count = count.checked_add(1).ok_or_else(|| error("slot scratch overflow"))?;
        if dom.node_document(id).map_err(|e| error(&e.to_string()))? == document && slot(dom,id)? { slot_count = slot_count.checked_add(1).ok_or_else(||error("slot count overflow"))?; }
    }
    let bytes = slot_count.checked_mul(std::mem::size_of::<(NodeId,Vec<NodeId>,Option<Arc<dyn Send + Sync>>)>() ).ok_or_else(|| error("slot metadata overflow"))?;
    let metadata = reserve(admission,bytes)?;
    let mut result = Projection { slots:Vec::new(),_metadata:metadata };
    result.slots.try_reserve_exact(slot_count).map_err(|_| error("slot metadata allocation"))?;
    if result.slots.capacity()>slot_count { return Err(error("slot metadata exceeded admission")); }
    let scratch = reserve(admission,count.checked_mul(2 * std::mem::size_of::<NodeId>()).ok_or_else(||error("slot scratch overflow"))?)?;
    let mut pending=Vec::<NodeId>::new();
    let mut membership=Vec::<NodeId>::new();
    pending.try_reserve_exact(count).map_err(|_|error("slot scratch allocation"))?;
    membership.try_reserve_exact(count).map_err(|_|error("slot membership allocation"))?;
    if pending.capacity()>count || membership.capacity()>count{return Err(error("slot scratch exceeded admission"));}
    for (id,_) in dom.arena().iter() {
        if dom.node_document(id).map_err(|e|error(&e.to_string()))? != document || !slot(dom,id)? {continue;}
        let mut candidates=Vec::new();
        let mut needed=0usize;
        let mut lease=None;
        for pass in 0..2 {
            pending.push(id);
            let mut visits=0usize;
            while let Some(id)=pending.pop(){
                if let Some(a)=admission{a.checkpoint().map_err(QueryError::Operational)?;}
                visits=visits.checked_add(1).ok_or_else(||error("slot visits overflow"))?;if visits>count{return Err(error("cyclic native slot projection"));}
                if slot(dom,id)? {
                    let (assigned, _) = dom.borrow_slot_projection_inputs(id).map_err(|e|error(&e.to_string()))?;
                    let mut has_assigned = false;
                    for &child in assigned {
                        if let Some(a)=admission{a.checkpoint().map_err(QueryError::Operational)?;}
                        if dom.slot_projection_member(child,id,&mut membership,&mut || {
                            admission.map_or(Ok(()),|a|a.checkpoint().map_err(|_|oryn_dom::DomError::ProjectionCancelled))
                        }).map_err(|e|error(&e.to_string()))? {has_assigned=true;break;}
                    }
                    let nodes=if has_assigned { assigned } else { &dom.arena().get(id).map_err(|e|error(&e.to_string()))?.children };
                    if pending.len().checked_add(nodes.len()).is_none_or(|n|n>count){return Err(error("cyclic native slot projection"));}
                    for &child in nodes.iter().rev() {
                        if !has_assigned || dom.slot_projection_member(child,id,&mut membership,&mut || {
                            admission.map_or(Ok(()),|a|a.checkpoint().map_err(|_|oryn_dom::DomError::ProjectionCancelled))
                        }).map_err(|e|error(&e.to_string()))? {pending.push(child);}
                    }
                } else if dom.is_slottable(id).map_err(|e|error(&e.to_string()))? {
                    if pass==0{needed=needed.checked_add(1).ok_or_else(||error("slot candidates overflow"))?;}else{if candidates.len()==needed{return Err(error("slot changed during projection"));}candidates.push(id);}
                }
            }
            if pass==0{
                lease=reserve(admission,needed.checked_mul(std::mem::size_of::<NodeId>()).ok_or_else(||error("slot candidates overflow"))?)?;
                candidates.try_reserve_exact(needed).map_err(|_|error("slot candidates allocation"))?;
                if candidates.capacity()>needed{return Err(error("slot candidates exceeded admission"));}
            }
        }
        if candidates.len()!=needed{return Err(error("slot changed during projection"));}
        result.slots.push((id,candidates,lease));
    }
    result.slots.sort_unstable_by_key(|(id,_,_)|*id);
    drop(pending);drop(membership);drop(scratch);Ok(result)
}
