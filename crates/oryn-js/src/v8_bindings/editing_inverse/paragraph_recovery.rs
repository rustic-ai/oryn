// Reverse only the edge/selection mutations produced by paragraph undo.
// This is not a generic snapshot restoration path.
use super::*;

impl Segment {
    pub(in crate::v8_bindings) async fn recover_paragraph(&self, context: &editing_continuation::Context) -> Result<(), V8BindingError> {
        let state = &context.state;
        let expected = &self.after;
        let desired = &self.before;
        let endpoints=[desired.selection.anchor,desired.selection.focus].into_iter().flatten()
            .map(|point|oryn_dom::TrackedNodeLease::new(&state.dom,point.container))
            .collect::<Result<Vec<_>,_>>()?;
        if expected.document_identity != (state.document_root(),state.document_generation(),state.document_stream_epoch()) {
            return Err(V8BindingError::Unsupported { interface:"Document".into(),member:"AbortError".into() });
        }
        // The producer is restricted to Context Remove/Append plus Selection.
        // Reject a widened recipe rather than dropping authored field changes.
        let mut detached=BTreeSet::new();
        for target in &desired.nodes {
            if let Some(prior) = expected.nodes.iter().find(|record|record.node==target.node)
                && (prior.attributes!=target.attributes || prior.kind!=target.kind || prior.value!=target.value
                    || prior.sensitive_value!=target.sensitive_value)
            {
                return Err(V8BindingError::Unsupported{interface:"Document".into(),member:"InvalidStateError".into()});
            }
        }
        let old=edges(expected);
        let new=edges(desired);
        let mut eligible=BTreeSet::new();
        {
            let dom=state.dom.borrow();
            for node in old.keys().chain(new.keys()).copied() {
                let position=|snapshot:&EditingSnapshot|snapshot.nodes.iter().find_map(|record|
                    record.children.iter().position(|child|*child==node).map(|index|(record.node,index)));
                if position(expected)==position(desired) {continue}
                if !current(state,expected,node)&&!current(state,desired,node){continue}
                if dom.parent(node)?!=old.get(&node).copied(){continue}
                if let Some((parent,index))=position(expected) {
                    let known=expected.nodes.iter().find(|record|record.node==parent).unwrap();
                    let actual=dom.children(parent)?.iter().filter(|child|known.children.contains(child)).position(|child|*child==node);
                    if actual!=Some(index){continue}
                }
                if let Some(parent)=new.get(&node) && !current(state,desired,*parent){continue}
                if old.contains_key(&node)&&!new.contains_key(&node)
                    && !paragraph_subtree_owned(state,expected,node)? {continue}
                eligible.insert(node);
            }
        }
        for root in old.keys() {
            // Evaluate admission before mutating the eligibility set; the
            // iterator itself must not retain an immutable borrow of that set.
            if new.contains_key(root) || eligible.contains(root) { continue; }
            let dom=state.dom.borrow();
            for descendant in dom.arena().descendants_inclusive(*root)? {eligible.remove(&descendant);}
        }
        // A Context append retains both half/node identities through distinct
        // observable removal and insertion phases; author moves after the first
        // phase are handled by that operation's existing exact receipt.
        for target in &desired.nodes {
            for (index,node) in target.children.iter().copied().enumerate() {
                if !eligible.remove(&node){continue}
                if !current(state,desired,target.node)||!current(state,desired,node)&&!current(state,expected,node){continue}
                let parent=state.dom.borrow().parent(node)?;
                if parent!=old.get(&node).copied(){continue}
                let position_current={
                    let dom=state.dom.borrow();
                    if let Some(parent)=parent {
                        let known=expected.nodes.iter().find(|record|record.node==parent).unwrap();
                        let wanted=known.children.iter().filter(|child|!detached.contains(*child)).position(|child|*child==node);
                        let actual=dom.children(parent)?.iter().filter(|child|known.children.contains(child)).position(|child|*child==node);
                        wanted==actual
                    } else {true}
                };
                if !position_current {continue}
                let reference={
                    let dom=state.dom.borrow();
                    target.children[index+1..].iter().copied().find(|other|dom.parent(*other).ok().flatten()==Some(target.node))
                };
                context.append(target.node,node,reference).await?;
                detached.insert(node);
            }
        }
        for node in eligible {
            if new.contains_key(&node){continue}
            if !current(state,expected,node){continue}
            let removable={
                let dom=state.dom.borrow();
                dom.parent(node)?==old.get(&node).copied()
                    && paragraph_subtree_owned(state,expected,node)?
                    && old.get(&node).is_none_or(|parent|{
                        let known=expected.nodes.iter().find(|record|record.node==*parent).unwrap();
                        let wanted=known.children.iter().filter(|child|!detached.contains(*child)).position(|child|*child==node);
                        let actual=dom.children(*parent).ok().and_then(|children|children.iter().filter(|child|known.children.contains(child)).position(|child|*child==node));
                        wanted==actual
                    })
            };
            if removable {context.remove(node).await?;detached.insert(node);}
        }
        if selection_snapshot(state)==expected.selection {
            for endpoint in &endpoints {
                let (owner,node)=endpoint.current();
                if !Rc::ptr_eq(&owner,&state.dom)||owner.borrow().arena().node_document(node)?!=Some(state.document_root()) {
                    return Err(V8BindingError::Unsupported{interface:"Document".into(),member:"AbortError".into()});
                }
            }
            match(desired.selection.anchor,desired.selection.focus){
                (Some(a),Some(f))=>{context.restore_selection(a,f).await?;}
                _=>context.clear_selection().await?,
            }
        }
        Ok(())
    }
}

fn paragraph_subtree_owned(state:&V8BindingState,expected:&EditingSnapshot,node:NodeId)->Result<bool,V8BindingError>{
    let dom=state.dom.borrow();
    Ok(dom.arena().descendants_inclusive(node)?.iter().all(|id|
        control_witness_matches(state,expected,*id).unwrap_or(false)
            && expected.nodes.iter().find(|record|record.node==*id).is_some_and(|record|
                dom.arena().get(*id).is_ok_and(|actual|
                    actual.children==record.children && actual.attributes==record.attributes
                        && text(&actual.kind)==text(&record.kind) && actual.value==record.value
                        && actual.sensitive_value==record.sensitive_value && record.control_state.matches(actual)))))
}
