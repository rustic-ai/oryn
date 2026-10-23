// Producer cancellation has no V8 scope. Keep pending publication records in
// their original binding State; disposal is naturally bounded by State lifetime.
pub(super) fn cancel_retired(state:&V8BindingState){
    let (requests,retry,capacity,provider,events,event_retry,event_capacity)={let mut fonts=state.font_loading.borrow_mut();
        fonts.cycles.retire();fonts.layout_readiness.retire();
        (std::mem::take(&mut fonts.requests),fonts.request_retry.take(),fonts.request_capacity.take(),fonts.provider.take(),
            std::mem::take(&mut fonts.events),fonts.event_retry.take(),fonts.event_capacity.take())};
    drop(requests);drop(retry);drop(capacity);drop(provider);
    drop(events);drop(event_retry);drop(event_capacity);
}
pub(super) fn replacement_document(state:&V8BindingState){
    let mut fonts=state.font_loading.borrow_mut();
    // Retained wrappers continue to address their original scalar records.
    // Only the Window's active document-set slot and producer authority change.
    fonts.document_set=None;
    fonts.cycles.replacement_document();
    fonts.layout_readiness=font_layout_readiness::LayoutReadiness::default();
}
pub(super) fn document_set_for_receiver<'s>(scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>,receiver:v8::Local<'s,v8::Object>,interface:&str)
    ->Result<v8::Local<'s,v8::Object>,V8BindingError>{
    if interface=="Document"{
        let (_,document)=node_identity::canonical(scope,receiver.into())?
            .ok_or(V8BindingError::Message("TypeError".into()))?;
        if document!=state.document_root(){
            let key=v8::String::new(scope,"oryn.document.fonts").ok_or(V8BindingError::Allocation("font document edge"))?;
            let key=v8::Private::for_api(scope,Some(key));
            if let Some(value)=receiver.get_private(scope,key).filter(|value|value.is_object()){
                return v8::Local::<v8::Object>::try_from(value).map_err(|_|V8BindingError::Allocation("font document set"));
            }
            let id=state.font_loading.borrow().sets.iter()
                .find_map(|(id,set)|(set.document==Some(document)).then_some(*id));
            let set=if let Some(id)=id{new_platform_object(scope,state,"FontFaceSet",id)?}else{
                create_document_set(scope,state,document,
                    font_loading_transport::ConnectedFontFaces{faces:Vec::new(),allocation:None},false)?};
            if receiver.set_private(scope,key,set.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
            return Ok(set);
        }
    }
    let set=document_set(scope,state)?;
    if interface=="Document"{
        let key=v8::String::new(scope,"oryn.document.fonts").ok_or(V8BindingError::Allocation("font document edge"))?;
        let key=v8::Private::for_api(scope,Some(key));
        if receiver.set_private(scope,key,set.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
    }
    Ok(set)
}
// A live suspended realm must not publish. Permanent retirement is different:
// settling its retained promises does not reactivate input, navigation, or
// execution authority, and cannot wait for a cursor which retirement revoked.
fn publication_unavailable(state: &V8BindingState) -> bool {
    state.execution_suspended() && !state.execution_retired()
}
pub(super) fn retire(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    let original_owner=owner(state);
    cancel_retired(state);
    let error=dom_exception_value(scope,"AbortError")?;
    // Scalar ID cursor: no Vec snapshot, clone or registry allocation. Mark the
    // exact resolver's terminal state before the post-publication checkpoint.
    let mut cursor=None;
    loop{
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        if owner(state)!=original_owner||publication_unavailable(state){return Ok(())}
        let next={let fonts=state.font_loading.borrow();fonts.face_objects.iter().find_map(|(id,face)|{
            if cursor.is_some_and(|last|*id<=last)||face.loaded.settled{return None}
            Some((*id,face.loaded.resolver.to_local(scope)))
        })};
        let Some((id,resolver))=next else{break};
        if let Some(resolver)=resolver{if resolver.reject(scope,error)!=Some(true){return Err(V8BindingError::JavaScriptException)}}
        {let mut fonts=state.font_loading.borrow_mut();let face=fonts.face_objects.get_mut(&id).ok_or(V8BindingError::Allocation("retiring font face missing"))?;
            if face.loaded.resolver.to_local(scope)!=resolver{return Err(V8BindingError::Allocation("retiring font resolver changed"))}face.loaded.settled=true;}
        cursor=Some(id);
    }
    cursor=None;
    loop{
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        if owner(state)!=original_owner||publication_unavailable(state){return Ok(())}
        let next={let fonts=state.font_loading.borrow();fonts.sets.iter().find_map(|(id,set)|{
            if cursor.is_some_and(|last|*id<=last)||set.ready.settled{return None}
            Some((*id,set.ready_published.is_some(),set.ready.resolver.to_local(scope)))
        })};
        let Some((id,published,resolver))=next else{break};
        if !published{if let Some(resolver)=resolver{if resolver.reject(scope,error)!=Some(true){return Err(V8BindingError::JavaScriptException)}}}
        {let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("retiring font set missing"))?;
            if set.ready.resolver.to_local(scope)!=resolver{return Err(V8BindingError::Allocation("retiring font resolver changed"))}set.ready.settled=true;}
        cursor=Some(id);
    }
    loop{
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        if owner(state)!=original_owner||publication_unavailable(state){return Ok(())}
        let next={let fonts=state.font_loading.borrow();fonts.loads.first().map(|load|(load.published,v8::Local::new(scope,&load.resolver)))};
        let Some((published,resolver))=next else{return Ok(())};
        if !published{
            if resolver.reject(scope,error)!=Some(true){return Err(V8BindingError::JavaScriptException)}
            let mut fonts=state.font_loading.borrow_mut();let load=fonts.loads.first_mut().ok_or(V8BindingError::Allocation("retiring font load missing"))?;
            if v8::Local::new(scope,&load.resolver)!=resolver{return Err(V8BindingError::Allocation("retiring font load changed"))}load.published=true;
        }
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        if owner(state)!=original_owner||publication_unavailable(state){return Ok(())}
        let removed={let mut fonts=state.font_loading.borrow_mut();let load=fonts.loads.first().ok_or(V8BindingError::Allocation("retiring font load missing"))?;
            if v8::Local::new(scope,&load.resolver)!=resolver{return Err(V8BindingError::Allocation("retiring font load changed"))}fonts.loads.remove(0)};
        drop(removed);
    }
}
