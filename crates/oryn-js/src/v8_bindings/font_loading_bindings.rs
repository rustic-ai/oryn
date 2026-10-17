// Additive generated binding dispatch will call these native operations.
pub(super) fn get<'s>(
    scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>,id:u64,interface:&str,member:&str,
)->Result<v8::Local<'s,v8::Value>,V8BindingError> {
    if interface=="FontFace" {
        let fonts=state.font_loading.borrow();
        let face=fonts.face_objects.get(&id).ok_or(V8BindingError::Allocation("FontFace receiver"))?;
        return match member {
            "loaded"=>{
                let loaded=face.loaded.clone();drop(fonts);
                Ok(promise(scope,&loaded)?.into())
            },
            "status"=> {
                let status=match fonts.cycles.status(face.cycle_id) {
                    Some(font_cycles::Status::Unloaded)=>"unloaded",
                    Some(font_cycles::Status::Loading)=>"loading",
                    Some(font_cycles::Status::Loaded)=>"loaded",
                    _=>"error",
                };
                drop(fonts);
                Ok(v8_string(scope,status)?)
            },
            descriptor=>{
                let native=face.native.clone();drop(fonts);
                let value=native.descriptor(descriptor).map_err(V8BindingError::EventCallback)?;
                let string=v8_string(scope,&value.value)?;
                drop(value);
                Ok(string)
            }
        };
    }
    let fonts=state.font_loading.borrow();
    let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("FontFaceSet receiver"))?;
    match member {
        "ready"=>{
            let ready=set.ready.clone();drop(fonts);
            Ok(promise(scope,&ready)?.into())
        },
        "size"=>{let size=u32::try_from(set.faces.len()).map_err(|_|V8BindingError::Allocation("font set size overflow"))?;drop(fonts);Ok(v8::Integer::new_from_unsigned(scope,size).into())},
        "status"=> {
            let pending=set.faces.iter().any(|id|fonts.face_objects.get(id)
                .is_some_and(|face|fonts.cycles.status(face.cycle_id)==Some(font_cycles::Status::Loading)));
            drop(fonts);
            Ok(v8_string(scope,if pending{"loading"}else{"loaded"})?)
        },
        _=>Err(V8BindingError::Unsupported{interface:interface.into(),member:member.into()})
    }
}
pub(super) fn set(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,id:u64,member:&str,value:v8::Local<v8::Value>)
    ->Result<(),V8BindingError> {
    let expected_owner=owner(state);
    let (native,cycle)={
        let fonts=state.font_loading.borrow();
        let face=fonts.face_objects.get(&id).ok_or(V8BindingError::Allocation("FontFace receiver"))?;
        (face.native.clone(),face.cycle_id)
    };
    let expected_revision=native.revision();
    let value=font_owned_input(scope,state,value)?;
    if state.execution_tasks_stopped()||owner(state)!=expected_owner||native.revision()!=expected_revision{
        return Err(V8BindingError::Message("InvalidStateError".into()));
    }
    if native.set_descriptor(member,&value.value).map_err(V8BindingError::EventCallback)? {
        let revision=native.revision();
        state.font_loading.borrow_mut().face_objects.get_mut(&id)
            .ok_or(V8BindingError::Allocation("FontFace receiver"))?.observed_revision=revision;
        state.font_loading.borrow_mut().cycles.replace(cycle).map_err(|error|V8BindingError::EventCallback(error.into()))?;
        settle_sets(scope,state)?;
    }
    Ok(())
}
pub(super) fn document_set<'s>(scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>)
    ->Result<v8::Local<'s,v8::Object>,V8BindingError> {
    if let Some(id)=state.font_loading.borrow().document_set {
        return new_platform_object(scope,state,"FontFaceSet",id);
    }
    let provider=state.font_loading.borrow().provider.clone()
        .ok_or_else(||V8BindingError::Unsupported{interface:"Document".into(),member:"native font loading provider unavailable".into()})?;
    let faces=provider.connected().map_err(V8BindingError::EventCallback)?;
    let id=register_platform_object(state,PlatformObjectRecord::FontFaceSet);
    let object=new_platform_object(scope,state,"FontFaceSet",id)?;
    let ready=promise_slot(scope,state,object,"oryn.fontset.ready")?;
    font_prepare_set_registry(scope,state)?;
    let mut ids=Vec::new();
    let members_capacity=reserve_font_container(scope,state,faces.faces.len().checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font set member bytes"))?)?;
    ids.try_reserve_exact(faces.faces.len()).map_err(|_|V8BindingError::Allocation("font set members"))?;
    if ids.capacity()>faces.faces.len(){return Err(V8BindingError::Allocation("font set member admission"))}
    for native in &faces.faces {
        let face=register_face(scope,state,native.clone(),true)?;
        let value=new_platform_object(scope,state,"FontFace",face)?;
        let _key_charge=reserve_font_container(scope,state,64)?;
        let name=v8::String::new(scope,&format!("oryn.fontset.face.{face}")).ok_or(V8BindingError::Allocation("font membership edge"))?;
        let key=v8::Private::for_api(scope,Some(name));
        if object.set_private(scope,key,value.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        ids.push(face);
    }
    let iteration_capacity=reserve_font_container(scope,state,ids.len().checked_mul(std::mem::size_of::<(u64,u64)>()).ok_or(V8BindingError::Allocation("font iteration bytes"))?)?;
    let mut iteration_order=Vec::new();iteration_order.try_reserve_exact(ids.len()).map_err(|_|V8BindingError::Allocation("font iteration buffer"))?;
    if iteration_order.capacity()>ids.len(){return Err(V8BindingError::Allocation("font iteration admission"))}
    for(index,face)in ids.iter().enumerate(){iteration_order.push((u64::try_from(index).ok().and_then(|n|n.checked_add(1)).ok_or(V8BindingError::Allocation("font insertion serial exhausted"))?,*face));}
    let next_serial=u64::try_from(ids.len()).map_err(|_|V8BindingError::Allocation("font insertion serial exhausted"))?;
    state.font_loading.borrow_mut().sets.insert(id,SetRecord{
        retired_edges:None,
        pending_edges:None,
        members_capacity,iteration_capacity,next_serial,membership_revision:0,
        faces:ids,iteration_order,ready,ready_cycle:0,ready_published:None,loading_event_cycle:None,done_event_cycle:None,error_event_cycle:None,document:Some(state.document_root())})?;
    state.font_loading.borrow_mut().document_set=Some(id);
    settle_sets(scope,state)?;
    Ok(object)
}
fn register_face(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,native:Rc<dyn NativeFontFace>,connected:bool)
    ->Result<u64,V8BindingError> {
    font_prepare_face_registry(scope,state)?;
    let id=register_platform_object(state,PlatformObjectRecord::FontFace);
    let object=new_platform_object(scope,state,"FontFace",id)?;
    let loaded=promise_slot(scope,state,object,"oryn.fontface.loaded")?;
    let cycle_id=state.font_loading.borrow_mut().cycles.create().map_err(|error|V8BindingError::EventCallback(error.into()))?;
    let observed_revision=native.revision();
    state.font_loading.borrow_mut().face_objects.insert(id,FaceRecord{cycle_id,native,loaded,connected,observed_revision,completed_cycle:None})?;
    let decoded=state.font_loading.borrow().face_objects.get(&id).map(|face|face.native.clone()).unwrap().decoded_backing();
    if connected&&decoded{
        let attempt=state.font_loading.borrow_mut().cycles.start(cycle_id).map_err(|error|V8BindingError::EventCallback(error.into()))?
            .ok_or(V8BindingError::Allocation("decoded font initial attempt"))?;
        complete(scope,state,font_loading_transport::FontLoadCompletion{token:font_loading_transport::FontLoadToken{
            owner:owner(state),face:id,revision:observed_revision,serial:attempt.serial},terminal:font_loading_transport::FontLoadTerminal::Decoded})?;
    }
    Ok(id)
}
