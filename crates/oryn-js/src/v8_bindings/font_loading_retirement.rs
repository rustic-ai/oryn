// Producer cancellation has no V8 scope. Keep pending publication records in
// their original binding State; disposal is naturally bounded by State lifetime.
pub(super) fn cancel_retired(state:&V8BindingState){
    let (requests,retry,capacity,provider)={let mut fonts=state.font_loading.borrow_mut();
        fonts.cycles.retire();fonts.layout_readiness.retire();
        (std::mem::take(&mut fonts.requests),fonts.request_retry.take(),fonts.request_capacity.take(),fonts.provider.take())};
    drop(requests);drop(retry);drop(capacity);drop(provider);
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
