fn publish_inline_owner_before_read(
    state: &V8BindingState,
    owner: NodeId,
) -> Result<(), V8BindingError> {
    state.reconcile_inline_style_policy()?;
    let provider = state.layout_provider.borrow().clone();
    let Some(provider) = provider else { return Ok(()); };
    let document = BindingStylesheetDocumentOwner {
        realm:state.realm, document:state.document_root(),
        document_generation:state.document_generation(),stream_epoch:state.document_stream_epoch(),
    };
    if !provider.needs_inline_publication(&state.dom,owner,document)
        .map_err(V8BindingError::EventCallback)? {return Ok(());}
    let base = state.native_document_base_url(true)?;
    let allocation=state.retained_allocation.borrow().clone();
    let Some(allocation)=allocation else {return Ok(());};
    let lifecycle = state.stylesheet_lifecycle_admitted(&allocation)
        .map_err(V8BindingError::EventCallback)?;
    let policy=state.document_referrer_policy();
    let encoding=hyperlinks::document_encoding(state,state.document_root())?;
    provider.publish_inline_owner(&state.dom,owner,document,lifecycle,&base,&policy,encoding.name())
        .map_err(V8BindingError::EventCallback)
}

fn publish_inline_owners_before_layout(state:&V8BindingState)->Result<(),V8BindingError> {
    let entry=(state.document_generation(),state.document_stream_epoch(),state.dom.borrow().mutation_cursor().0);
    if state.inline_publication_cursor.get()==Some(entry) {return Ok(());}
    let allocation=state.retained_allocation.borrow().clone();
    let Some(allocation)=allocation else {return Ok(());};
    let (cursor,count)={
        let dom=state.dom.borrow();
        (dom.mutation_cursor(),dom.arena().iter().filter(|(id,node)|
            matches!(&node.kind,NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" && name.local.as_ref()=="style")
                && dom.node_document(*id).ok().flatten()==Some(state.document_root())
                && dom.is_connected(*id).unwrap_or(false)).count())
    };
    let bytes=count.checked_mul(std::mem::size_of::<NodeId>()).ok_or(V8BindingError::Allocation("inline owner snapshot overflow"))?;
    let _charge=allocation.reserve(bytes).map_err(V8BindingError::EventCallback)?;
    let mut owners=Vec::new();owners.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("inline owner snapshot"))?;
    {
        let dom=state.dom.borrow();
        if dom.mutation_cursor()!=cursor {return Err(V8BindingError::EventCallback("inline owner snapshot changed".into()));}
        for (id,node) in dom.arena().iter() {
            if matches!(&node.kind,NodeKind::Element { name } if name.ns.as_ref()=="http://www.w3.org/1999/xhtml" && name.local.as_ref()=="style")
                && dom.node_document(id).ok().flatten()==Some(state.document_root())
                && dom.is_connected(id).unwrap_or(false) {owners.push(id);}
        }
    }
    for owner in owners {publish_inline_owner_before_read(state,owner)?;}
    let final_entry=(state.document_generation(),state.document_stream_epoch(),state.dom.borrow().mutation_cursor().0);
    // Admission can introduce an owner absent from the captured snapshot.
    // Cache only the exact unchanged entry, never a newer callback cursor.
    state.inline_publication_cursor.set(if final_entry==entry {Some(entry)} else {None});
    Ok(())
}
