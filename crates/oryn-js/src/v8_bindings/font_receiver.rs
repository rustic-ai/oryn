pub(super) fn font_receiver<'s>(
    scope:&mut v8::PinScope<'s,'_>,receiver:v8::Local<'s,v8::Object>,interface:&str,
)->Result<(Rc<V8BindingState>,u64),V8BindingError>{
    let invalid=||V8BindingError::Unsupported{interface:interface.into(),member:"TypeError".into()};
    let state=binding_state_for_receiver(scope,receiver).ok_or_else(invalid)?;
    let id=stateful_id(scope,receiver).map_err(|_|invalid())?;
    if platform_interface_for_id(&state,id)!=Some(interface){return Err(invalid());}
    Ok((state,id))
}
pub(super) fn font_document_receiver<'s>(
    scope:&mut v8::PinScope<'s,'_>,receiver:v8::Local<'s,v8::Object>,interface:&str,
)->Result<Rc<V8BindingState>,V8BindingError>{
    let invalid=||V8BindingError::Unsupported{interface:interface.into(),member:"TypeError".into()};
    if interface=="Document"{
        let (state,node)=node_identity::canonical(scope,receiver.into())?.ok_or_else(invalid)?;
        let document=matches!(state.dom.borrow().arena().get(node).map_err(V8BindingError::Dom)?.kind,NodeKind::Document);
        if !document{return Err(invalid());}
        Ok(state)
    }else{
        let state=binding_state_for_receiver(scope,receiver).ok_or_else(invalid)?;
        if state.binding_global()!=BindingGlobal::Worker||!global_events::is_global(scope,&state,receiver){return Err(invalid());}
        Ok(state)
    }
}
