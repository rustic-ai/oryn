// Native FontFaceSet producer/consumer bridge. The embedding supplies original
// parsed faces and broker work; a missing provider is an explicit capability
// error, never a resolved FontFaceSet.ready promise.
pub trait NativeFontLoading {
    fn same_provider(&self,identity:usize)->bool;
    fn construct(&self, family:&str, source:font_loading_transport::FontFaceSourceInput, descriptors:&[(String,String)])
        ->Result<Rc<dyn NativeFontFace>,String>;
    fn connected(&self)->Result<font_loading_transport::ConnectedFontFaces,String>;
    fn select(&self,font:&str,text:&str,faces:&[Rc<dyn NativeFontFace>])
        ->Result<font_loading_transport::FontSelection,font_loading_transport::FontSelectionError>;
}
fn owner(state:&V8BindingState)->font_loading_transport::FontLoadOwner {
    font_loading_transport::FontLoadOwner {instance:state.instance_id,
        document:state.document_root(),generation:state.document_generation.get(),
        epoch:state.document_stream_epoch.get()}
}
fn begin<'s>(
    scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>,id:u64,text:&str,
)->Result<v8::Local<'s,v8::Promise>,V8BindingError> {
    let (native,cycle_id,loaded_slot) = {
        let fonts=state.font_loading.borrow();
        let face=fonts.face_objects.get(&id).ok_or(V8BindingError::Allocation("font face"))?;
        (face.native.clone(),face.cycle_id,face.loaded.clone())
    };
    let loaded=promise(scope,&loaded_slot)?;
    if state.font_loading.borrow().cycles.status(cycle_id)==Some(font_cycles::Status::Loading){renew_loading_sets(scope,state,id)?;return Ok(loaded);}
    let attempt=state.font_loading.borrow_mut().cycles.start(cycle_id)
        .map_err(|error|V8BindingError::EventCallback(error.into()))?;
    let Some(attempt)=attempt else { return Ok(loaded); };
    let token=font_loading_transport::FontLoadToken {owner:owner(state),
        face:id,revision:native.revision(),serial:attempt.serial};
    // Provider callbacks and native admission occur outside font state borrows.
    let _request_entry=match state.enter_retained_allocation(scope) {
        Ok(entry)=>entry,
        Err(error)=>{
            state.font_loading.borrow_mut().cycles.rollback_start(attempt);
            return Err(error);
        }
    };
    let request=match native.request(token,text) {
        Ok(request)=>request,
        Err(error)=>{
            state.font_loading.borrow_mut().cycles.rollback_start(attempt);
            return Err(V8BindingError::EventCallback(error));
        }
    };
    if owner(state)!=token.owner || state.execution_retired() || native.revision()!=token.revision {
        state.font_loading.borrow_mut().cycles.rollback_start(attempt);
        return Err(V8BindingError::Message("InvalidStateError".into()));
    }
    if let Err(error)=prepare_font_request_capacity(scope,state){
        state.font_loading.borrow_mut().cycles.rollback_start(attempt);
        return Err(error);
    }
    if owner(state)!=token.owner||state.execution_retired()||native.revision()!=token.revision{
        state.font_loading.borrow_mut().cycles.rollback_start(attempt);
        return Err(V8BindingError::Message("InvalidStateError".into()));
    }
    let mut fonts=state.font_loading.borrow_mut();
    if !fonts.requests.iter().any(|request|request.token==token) {
        fonts.requests.push_back(request);
    }
    drop(fonts);
    renew_loading_sets(scope,state,id)?;
    Ok(loaded)
}
fn renew_loading_sets(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,face:u64,
)->Result<(),V8BindingError> {
    let count=state.font_loading.borrow().sets.len();
    let _ids_charge=reserve_font_container(scope,state,count.checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font set snapshot bytes"))?)?;
    let mut ids=Vec::new();ids.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font set snapshot"))?;
    if ids.capacity()>count{return Err(V8BindingError::Allocation("font set snapshot admission"))}
    {let fonts=state.font_loading.borrow();if fonts.sets.len()>count{return Err(V8BindingError::Allocation("font set snapshot changed"))}ids.extend(fonts.sets.iter().filter_map(|(id,set)|set.faces.contains(&face).then_some(*id)));}
    for id in ids {
        let object=new_platform_object(scope,state,"FontFaceSet",id)?;
        let renew=state.font_loading.borrow().sets.get(&id)
            .is_some_and(|set|set.ready.settled);
        if renew {
            let ready=promise_slot(scope,state,object,"oryn.fontset.ready")?;
            let cycle=state.font_loading.borrow().cycles.cycle();
            if let Some(set)=state.font_loading.borrow_mut().sets.get_mut(&id) {
                set.ready=ready;set.ready_cycle=cycle;set.ready_published=None;
            }
        }
        let cycle=state.font_loading.borrow().cycles.cycle();
        if let Some(set)=state.font_loading.borrow_mut().sets.get_mut(&id){set.ready_cycle=cycle;}
        let cycle=state.font_loading.borrow().sets.get(&id).unwrap().ready_cycle;
        if state.font_loading.borrow().sets.get(&id).unwrap().loading_event_cycle!=Some(cycle){
            queue_font_event(scope,state,id,"loading",&[])?;
            state.font_loading.borrow_mut().sets.get_mut(&id).unwrap().loading_event_cycle=Some(cycle);
        }
    }
    Ok(())
}
pub(super) fn complete(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,
    completion:font_loading_transport::FontLoadCompletion,
)->Result<bool,V8BindingError> {
    if owner(state)!=completion.token.owner || state.execution_retired() {return Ok(false);}
    let (cycle_id,native,resolver)={
        let fonts=state.font_loading.borrow();
        let Some(face)=fonts.face_objects.get(&completion.token.face) else {return Ok(false)};
        (face.cycle_id,face.native.clone(),face.loaded.resolver.clone())
    };
    if native.revision()!=completion.token.revision {return Ok(false);}
    let terminal=match completion.terminal {
        font_loading_transport::FontLoadTerminal::Decoded=>font_cycles::Terminal::Decoded,
        font_loading_transport::FontLoadTerminal::Exhausted=>font_cycles::Terminal::Exhausted,
        font_loading_transport::FontLoadTerminal::Cancelled=>font_cycles::Terminal::Cancelled,
    };
    let revision=state.font_loading.borrow().cycles.revision(cycle_id)
        .ok_or(V8BindingError::Allocation("font cycle face"))?;
    let attempt=font_cycles::Attempt{face:cycle_id,serial:completion.token.serial,revision};
    if !state.font_loading.borrow().cycles.current(attempt){return Ok(false);}
    let wrapper=new_platform_object(scope,state,"FontFace",completion.token.face)?;
    let resolver=resolver.to_local(scope).ok_or(V8BindingError::Allocation("font loaded resolver"))?;
        if terminal==font_cycles::Terminal::Decoded {
            if resolver.resolve(scope,wrapper.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        } else {
            let error=dom_exception_value(scope,"NetworkError")?;
            if resolver.reject(scope,error)!=Some(true){return Err(V8BindingError::JavaScriptException);}
        }
    if owner(state)!=completion.token.owner || native.revision()!=completion.token.revision
        || !state.font_loading.borrow_mut().cycles.finish(attempt,terminal){return Ok(false);}
    let completed_cycle=state.font_loading.borrow().cycles.cycle();
    if let Some(face)=state.font_loading.borrow_mut().face_objects.get_mut(&completion.token.face) {
        face.loaded.settled=true;
        face.completed_cycle=Some(completed_cycle);
    }
    settle_loads(scope,state)?;
    settle_sets(scope,state)?;
    Ok(true)
}
fn settle_sets(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError> {
    let count=state.font_loading.borrow().sets.len();
    let _ids_charge=reserve_font_container(scope,state,count.checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font ready snapshot bytes"))?)?;
    let mut ids=Vec::new();ids.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font ready snapshot"))?;
    if ids.capacity()>count{return Err(V8BindingError::Allocation("font ready snapshot admission"))}
    {let fonts=state.font_loading.borrow();if fonts.sets.len()>count{return Err(V8BindingError::Allocation("font ready snapshot changed"))}ids.extend(fonts.sets.iter().filter_map(|(id,set)| {
        let pending=set.faces.iter().any(|id|fonts.face_objects.get(id)
            .is_some_and(|face|fonts.cycles.status(face.cycle_id)==Some(font_cycles::Status::Loading)));
        let layout_ready=set.document.is_none() || fonts.layout_readiness.ready(owner(state));
        (!pending&&layout_ready&&!set.ready.settled).then_some(*id)
    }));}
    for id in ids {
        let expected_owner=owner(state);
        let (committed_stamp,expected_cycle)={let fonts=state.font_loading.borrow();
            (fonts.layout_readiness.stamp(),fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?.ready_cycle)};
        let object=new_platform_object(scope,state,"FontFaceSet",id)?;
        let resolver=state.font_loading.borrow().sets.get(&id).map(|set|set.ready.resolver.clone())
            .ok_or(V8BindingError::Allocation("font set"))?;
        if state.font_loading.borrow().sets.get(&id).is_none_or(|set|set.ready_published!=Some(expected_cycle)){
            let resolver=resolver.to_local(scope).ok_or(V8BindingError::Allocation("font set ready resolver"))?;
            if resolver.resolve(scope,object.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
            let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
            if set.ready_cycle!=expected_cycle{return Err(V8BindingError::Allocation("font ready publication changed"));}
            set.ready_published=Some(expected_cycle);
        }
        if !crate::v8_termination::checkpoint(scope)||state.execution_tasks_stopped(){return Err(V8BindingError::JavaScriptException);}
        if state.execution_retired(){return Ok(());}
        if owner(state)!=expected_owner||state.font_loading.borrow().layout_readiness.stamp()!=committed_stamp
            ||state.font_loading.borrow().sets.get(&id).is_none_or(|set|set.ready_cycle!=expected_cycle){continue;}
        let (count,revision)={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;(set.faces.len(),set.membership_revision)};
        let _terminal_charge=reserve_font_container(scope,state,count.checked_mul(2).and_then(|n|n.checked_mul(std::mem::size_of::<u64>())).ok_or(V8BindingError::Allocation("font terminal snapshot bytes"))?)?;
        let mut loaded=Vec::new();let mut failed=Vec::new();
        loaded.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font loaded snapshot"))?;
        failed.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font failed snapshot"))?;
        if loaded.capacity()>count||failed.capacity()>count{return Err(V8BindingError::Allocation("font terminal snapshot admission"))}
        let (loaded,failed)={
            let fonts=state.font_loading.borrow();
            let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;
            if set.faces.len()!=count||set.membership_revision!=revision{return Err(V8BindingError::Allocation("font terminal snapshot changed"))}
            for face in &set.faces {
                if !fonts.face_objects.get(face).is_some_and(|face|face.completed_cycle==Some(set.ready_cycle)){continue;}
                match fonts.face_objects.get(face).and_then(|face|fonts.cycles.status(face.cycle_id)) {
                    Some(font_cycles::Status::Loaded)=>loaded.push(*face),
                    Some(font_cycles::Status::Error)=>failed.push(*face),
                    _=>{},
                }
            }
            (loaded,failed)
        };
        let cycle=state.font_loading.borrow().sets.get(&id).unwrap().ready_cycle;
        let observed_loading=state.font_loading.borrow().sets.get(&id).unwrap().loading_event_cycle==Some(cycle);
        if observed_loading && state.font_loading.borrow().sets.get(&id).unwrap().done_event_cycle!=Some(cycle){
            queue_font_event(scope,state,id,"loadingdone",&loaded)?;
            state.font_loading.borrow_mut().sets.get_mut(&id).unwrap().done_event_cycle=Some(cycle);
        }
        if observed_loading && !failed.is_empty() && state.font_loading.borrow().sets.get(&id).unwrap().error_event_cycle!=Some(cycle){
            queue_font_event(scope,state,id,"loadingerror",&failed)?;
            state.font_loading.borrow_mut().sets.get_mut(&id).unwrap().error_event_cycle=Some(cycle);
        }
        if let Some(set)=state.font_loading.borrow_mut().sets.get_mut(&id) {set.ready.settled=true;}
    }
    Ok(())
}
pub(super) fn commit_font_layout(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,stamp:font_layout_readiness::FontDemandStamp,
)->Result<bool,V8BindingError>{
    if owner(state)!=stamp.owner || state.execution_retired(){return Ok(false);}
    if !state.font_loading.borrow_mut().layout_readiness.commit(stamp){return Ok(false);}
    settle_sets(scope,state)?;
    Ok(true)
}
pub(super) fn implicit_font_started(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,face_identity:usize,
)->Result<font_loading_transport::ImplicitFontTokens,V8BindingError>{
    let document=state.font_loading.borrow().document_set;
    let Some(set)=document else{return Ok(font_loading_transport::ImplicitFontTokens::empty())};
    synchronize_connected(scope,state,set)?;
    let count=state.font_loading.borrow().face_objects.len();
    let admission=state.retained_allocation.borrow().clone().ok_or(V8BindingError::Allocation("implicit font admission"))?;
    let bytes=count.checked_mul(std::mem::size_of::<(u64,Rc<dyn NativeFontFace>,font_cycles::FaceId)>()
        +std::mem::size_of::<font_loading_transport::FontLoadToken>()+std::mem::size_of::<font_cycles::Attempt>())
        .ok_or(V8BindingError::Allocation("implicit font snapshot overflow"))?;
    let allocation=admission.reserve(bytes).map_err(V8BindingError::EventCallback)?;
    if state.execution_retired()||state.font_loading.borrow().face_objects.len()!=count{
        return Err(V8BindingError::Message("InvalidStateError".into()));
    }
    let mut faces=Vec::new();
    faces.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("implicit font face snapshot"))?;
    if faces.capacity()>count{return Err(V8BindingError::Allocation("implicit font snapshot capacity admission"));}
    {let fonts=state.font_loading.borrow();
        for (id,face) in &fonts.face_objects{faces.push((*id,face.native.clone(),face.cycle_id));}
    }
    let mut tokens=Vec::new();
    tokens.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("implicit font tokens"))?;
    let mut started=Vec::new();
    started.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("implicit font rollback"))?;
    if tokens.capacity()>count||started.capacity()>count{
        return Err(V8BindingError::Allocation("implicit font snapshot capacity admission"));
    }
    for (id,native,cycle) in faces {
        if native.identity()!=face_identity{continue;}
        let was_loading=state.font_loading.borrow().cycles.status(cycle)==Some(font_cycles::Status::Loading);
        let start={state.font_loading.borrow_mut().cycles.start(cycle)};
        let attempt=match start{
            Ok(attempt)=>attempt,
            Err(error)=>{
                for attempt in started{state.font_loading.borrow_mut().cycles.rollback_start(attempt);}
                return Err(V8BindingError::EventCallback(error.into()));
            },
        };
        if !was_loading {if let Some(attempt)=attempt{started.push(attempt);}}
        if let Some(attempt)=attempt {tokens.push(font_loading_transport::FontLoadToken{owner:owner(state),face:id,revision:native.revision(),serial:attempt.serial});}
        if let Err(error)=renew_loading_sets(scope,state,id){
            for attempt in started{state.font_loading.borrow_mut().cycles.rollback_start(attempt);}
            return Err(error);
        }
    }
    Ok(font_loading_transport::ImplicitFontTokens{tokens,allocation:Some(allocation)})
}
pub(super) fn implicit_font_terminal(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,tokens:&[font_loading_transport::FontLoadToken],success:bool,
)->Result<(),V8BindingError>{
    for token in tokens {
        complete(scope,state,font_loading_transport::FontLoadCompletion{
            token:*token,
            terminal:if success{font_loading_transport::FontLoadTerminal::Decoded}else{font_loading_transport::FontLoadTerminal::Exhausted},
        })?;
    }
    Ok(())
}
