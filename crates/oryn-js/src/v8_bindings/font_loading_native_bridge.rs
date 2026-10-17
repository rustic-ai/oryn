// Executable-owned bridge. Tokens are verified against current realm state;
// draining requests transfers ownership, not an unaccounted deep clone.
pub(super) fn layout_inputs(state:&Rc<V8BindingState>)->Result<font_loading_transport::FontLayoutInputs,V8BindingError>{
    let expected_owner=owner(state);
    let (count,bytes)={let states=state.custom_states.borrow();
        states.iter().try_fold(0usize,|bytes,(_,names)|{
            names.iter().try_fold(bytes.checked_add(std::mem::size_of::<(oryn_dom::NodeId,Vec<String>)>())?,|bytes,name|
                bytes.checked_add(name.len())?.checked_add(std::mem::size_of::<String>()))
        }).map(|bytes|(states.len(),bytes)).ok_or(V8BindingError::Allocation("font state snapshot overflow"))?
    };
    let admission=state.retained_allocation.borrow().clone().ok_or(V8BindingError::Allocation("font state snapshot admission"))?;
    let allocation=admission.reserve(bytes).map_err(V8BindingError::EventCallback)?;
    if state.execution_retired()||owner(state)!=expected_owner{return Err(V8BindingError::Message("InvalidStateError".into()));}
    // The allocation checkpoint is non-author and the following copy contains
    // no callbacks. Its native owner must remain live through the final commit.
    let mut output=Vec::new();output.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font state snapshot"))?;
    if output.capacity()>count{return Err(V8BindingError::Allocation("font state snapshot capacity"));}
    let states=state.custom_states.borrow();
    if states.len()!=count{return Err(V8BindingError::Allocation("font state snapshot changed"));}
    let fresh_bytes=states.iter().try_fold(0usize,|bytes,(_,names)|{
        names.iter().try_fold(bytes.checked_add(std::mem::size_of::<(NodeId,Vec<String>)>())?,|bytes,name|
            bytes.checked_add(name.len())?.checked_add(std::mem::size_of::<String>()))
    }).ok_or(V8BindingError::Allocation("font state snapshot overflow"))?;
    if fresh_bytes!=bytes{return Err(V8BindingError::Allocation("font state snapshot admission changed"));}
    for (node,names) in states.iter(){
        let mut owned=Vec::new();owned.try_reserve_exact(names.len()).map_err(|_|V8BindingError::Allocation("font state names"))?;
        if owned.capacity()>names.len(){return Err(V8BindingError::Allocation("font state name capacity"));}
        for name in names{let mut value=String::new();value.try_reserve_exact(name.len()).map_err(|_|V8BindingError::Allocation("font state name"))?;
            if value.capacity()>name.len(){return Err(V8BindingError::Allocation("font state string capacity"));}value.push_str(name);owned.push(value);}
        output.push((*node,owned));
    }
    Ok(font_loading_transport::FontLayoutInputs{states:output,allocation})
}
pub(super) fn take_requests(state:&Rc<V8BindingState>)
    ->font_loading_transport::FontRequestQueue{
    let mut fonts=state.font_loading.borrow_mut();
    if let Some(queue)=fonts.request_retry.take(){return queue;}
    font_loading_transport::FontRequestQueue{requests:std::mem::take(&mut fonts.requests),allocation:fonts.request_capacity.take()}
}
pub(super) fn restore_requests(state:&Rc<V8BindingState>,queue:&mut font_loading_transport::FontRequestQueue)->Result<(),V8BindingError>{
    // Scheduling preparation never invokes author callbacks. A newly populated
    // queue here is therefore an invariant violation, not a reorder opportunity.
    let mut fonts=state.font_loading.borrow_mut();
    if !fonts.requests.is_empty(){
        if fonts.request_retry.is_some(){return Err(V8BindingError::Allocation("font scheduling duplicate retry custody"));}
        fonts.request_retry=Some(font_loading_transport::FontRequestQueue{requests:std::mem::take(&mut queue.requests),allocation:queue.allocation.take()});return Ok(());
    }
    std::mem::swap(&mut fonts.requests,&mut queue.requests);
    std::mem::swap(&mut fonts.request_capacity,&mut queue.allocation);Ok(())
}
pub(super) fn request_current(state:&Rc<V8BindingState>,token:font_loading_transport::FontLoadToken)->bool{
    if owner(state)!=token.owner || state.execution_retired(){return false;}
    let captured={
        let fonts=state.font_loading.borrow();
        fonts.face_objects.get(&token.face).map(|face|(face.cycle_id,face.native.clone()))
    };
    let Some((cycle,native))=captured else{return false};
    if native.revision()!=token.revision{return false;}
    let fonts=state.font_loading.borrow();
    let Some(revision)=fonts.cycles.revision(cycle) else{return false};
    fonts.cycles.current(font_cycles::Attempt{face:cycle,revision,serial:token.serial})
}
pub(super) fn demand_font_layout(state:&Rc<V8BindingState>,stamp:font_layout_readiness::FontDemandStamp)->bool{
    if owner(state)!=stamp.owner || state.execution_retired(){return false;}
    state.font_loading.borrow_mut().layout_readiness.demand(stamp)
}
pub(super) fn install_provider(state:&Rc<V8BindingState>,provider:Rc<dyn NativeFontLoading>){
    let previous=state.font_loading.borrow_mut().provider.replace(provider);
    drop(previous);
}
pub(super) fn synchronize_set(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    let set=state.font_loading.borrow().document_set;
    if let Some(set)=set{synchronize_connected(scope,state,set)?;}
    settle_loads(scope,state)?;
    settle_sets(scope,state)
}
pub(super) fn request_face(state:&Rc<V8BindingState>,token:font_loading_transport::FontLoadToken)
    ->Option<std::rc::Weak<dyn NativeFontFace>>{
    if !request_current(state,token){return None;}
    state.font_loading.borrow().face_objects.get(&token.face)
        .map(|face|Rc::downgrade(&face.native))
}
