// Projected child module of editing_inverse. No phase is retained in history.
use super::*;

#[derive(Clone, Debug)]
pub(in crate::v8_bindings) struct ParagraphSplit {
    left: oryn_dom::TrackedNodeLease,
    right: oryn_dom::TrackedNodeLease,
    placeholders: [Option<oryn_dom::TrackedNodeLease>; 2],
    owner: (NodeId, u64, u64, u64),
}
impl ParagraphSplit {
    pub(in crate::v8_bindings) fn retain(
        state: &V8BindingState,
        left: NodeId,
        right: NodeId,
        placeholders: [Option<NodeId>; 2],
    ) -> Result<Self, V8BindingError> {
        Ok(Self {
            left: oryn_dom::TrackedNodeLease::new(&state.dom, left)?,
            right: oryn_dom::TrackedNodeLease::new(&state.dom, right)?,
            placeholders: [
                placeholders[0].map(|node| oryn_dom::TrackedNodeLease::new(&state.dom, node)).transpose()?,
                placeholders[1].map(|node| oryn_dom::TrackedNodeLease::new(&state.dom, node)).transpose()?,
            ],
            owner: (state.document_root(), state.document_generation(), state.document_stream_epoch(), state.document_authority_epoch()),
        })
    }
    fn node(&self, state: &V8BindingState, lease: &oryn_dom::TrackedNodeLease) -> Result<NodeId, V8BindingError> {
        let (dom, node) = lease.current();
        if state.execution_retired()
            || self.owner != (state.document_root(), state.document_generation(), state.document_stream_epoch(), state.document_authority_epoch())
            || !Rc::ptr_eq(&dom, &state.dom)
            || dom.borrow().arena().node_document(node)? != Some(state.document_root())
        {
            return Err(V8BindingError::Unsupported { interface: "Document".into(), member: "AbortError".into() });
        }
        Ok(node)
    }
    fn halves(&self, state: &V8BindingState) -> Result<(NodeId, NodeId), V8BindingError> {
        Ok((self.node(state, &self.left)?, self.node(state, &self.right)?))
    }
    /// A new future is created per history operation. Pending requests retain
    /// their Context operation receipt; an await never replays its mutation.
    pub(in crate::v8_bindings) async fn undo(
        &self,
        context: &editing_continuation::Context,
        selection: SelectionSnapshot,
    ) -> Result<(), V8BindingError> {
        let state = &context.state;
        let endpoints=[selection.anchor,selection.focus].into_iter().flatten()
            .map(|point|oryn_dom::TrackedNodeLease::new(&state.dom,point.container))
            .collect::<Result<Vec<_>,_>>()?;
        for (index, placeholder) in self.placeholders.iter().enumerate() {
            let (left, right) = self.halves(state)?;
            if let Some(lease) = placeholder {
                let node = self.node(state, lease)?;
                let expected = if index == 0 { left } else { right };
                let remove = {
                    let dom = state.dom.borrow();
                    dom.parent(node)? == Some(expected)
                        && element_local_name(&dom, node) == Some("br")
                        && light_children(&dom, node)?.is_empty()
                };
                // Author-reparented placeholders are no longer owned by this
                // edge. They must not be deleted merely because still empty.
                if remove { context.remove(node).await?; }
            }
        }
        let (left, right) = self.halves(state)?;
        let nested_left = state.dom.borrow().parent(left)? == Some(right);
        if nested_left {
            context.remove(left).await?;
        }
        let (left, right) = self.halves(state)?;
        let nested_right = state.dom.borrow().parent(right)? == Some(left);
        if nested_right {
            context.remove(right).await?;
        }
        loop {
            let (left, right) = self.halves(state)?;
            let child = {
                let dom = state.dom.borrow();
                dom.children(right)?.iter().copied().find(|node| {
                    *node != left && !matches!(dom.arena().get(*node).map(|record| &record.kind),
                        Ok(NodeKind::Document | NodeKind::ShadowRoot { .. }))
                })
            };
            let Some(child) = child else { break };
            context.append(left, child, None).await?;
        }
        let (_, right) = self.halves(state)?;
        let attached_right = state.dom.borrow().parent(right)?.is_some();
        if attached_right {
            context.remove(right).await?;
        }
        self.halves(state)?;
        for endpoint in &endpoints {self.node(state,endpoint)?;}
        match (selection.anchor, selection.focus) {
            (Some(anchor), Some(focus)) => {
                // Context must retain these endpoint leases before polling the
                // future. Ownership is validated again after every callback.
                {
                    let dom = state.dom.borrow();
                    validate_boundary_offset(&dom, anchor.container, anchor.offset)?;
                    validate_boundary_offset(&dom, focus.container, focus.offset)?;
                    if dom.arena().node_document(anchor.container)? != Some(state.document_root())
                        || dom.arena().node_document(focus.container)? != Some(state.document_root())
                    {
                        return Err(V8BindingError::Unsupported { interface: "Document".into(), member: "AbortError".into() });
                    }
                }
                context.restore_selection(anchor, focus).await?;
            }
            _ => context.clear_selection().await?,
        }
        Ok(())
    }
}
