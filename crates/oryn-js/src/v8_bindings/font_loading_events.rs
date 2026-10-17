fn queue_font_event(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,set:u64,name:&str,faces:&[u64],
)->Result<(),V8BindingError>{
    let expected_owner=owner(state);
    font_prepare_event_capacity(scope,state,1)?;
    let scratch=reserve_font_container(scope,state,faces.len().checked_mul(std::mem::size_of::<v8::Local<v8::Value>>())
        .ok_or(V8BindingError::Allocation("font event scratch overflow"))?)?;
    let event=create_event_object_with_interface(scope,state,"FontFaceSetLoadEvent",name,false,false)?;
    let id=event_id(scope,event)?;
    state.events.borrow_mut().get_mut(&id).ok_or(V8BindingError::Allocation("font event record"))?.is_trusted=true;
    let mut values=Vec::new();
    values.try_reserve_exact(faces.len()).map_err(|_|V8BindingError::Allocation("font event faces"))?;
    if values.capacity()>faces.len(){return Err(V8BindingError::Allocation("font event scratch capacity"));}
    for face in faces {values.push(new_platform_object(scope,state,"FontFace",*face)?.into());}
    let array=v8::Array::new_with_elements(scope,&values);
    if array.set_integrity_level(scope,v8::IntegrityLevel::Frozen)!=Some(true){return Err(V8BindingError::JavaScriptException);}
    let key=v8::String::new(scope,"oryn.fontset.event.faces").ok_or(V8BindingError::Allocation("font event faces"))?;
    let key=v8::Private::for_api(scope,Some(key));
    if event.set_private(scope,key,array.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
    let notification=native_platform::publication::pending_platform_dispatch(scope,state,set,name,event);
    if state.execution_tasks_stopped()||owner(state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
    let mut fonts=state.font_loading.borrow_mut();
    if fonts.events.len()==fonts.events.capacity(){return Err(V8BindingError::Allocation("font event capacity changed during publication"));}
    fonts.events.push_back(notification);drop(fonts);
    drop(values);drop(scratch);
    Ok(())
}
pub(super) fn font_event_faces<'s>(scope:&mut v8::PinScope<'s,'_>,event:v8::Local<'s,v8::Object>)
    ->Result<v8::Local<'s,v8::Value>,V8BindingError>{
    event_id(scope,event).map_err(|_|V8BindingError::Unsupported{interface:"FontFaceSetLoadEvent".into(),member:"TypeError".into()})?;
    let key=v8::String::new(scope,"oryn.fontset.event.faces").ok_or(V8BindingError::Allocation("font event faces"))?;
    let key=v8::Private::for_api(scope,Some(key));
    let value=event.get_private(scope,key).ok_or(V8BindingError::JavaScriptException)?;
    if value.is_undefined(){return Err(V8BindingError::Unsupported{interface:"FontFaceSetLoadEvent".into(),member:"TypeError".into()});}
    Ok(value)
}
pub(super) fn initialize_font_event(scope:&mut v8::PinScope,event:v8::Local<v8::Object>,init:Option<v8::Local<v8::Object>>)->Result<(),V8BindingError>{
    let value=event_init_member(scope,init,"fontfaces").ok_or(V8BindingError::JavaScriptException)?;
    let array=v8::Array::new(scope,0);
    if !value.is_undefined(){
        let source=v8::Local::<v8::Object>::try_from(value).map_err(|_|V8BindingError::Message("TypeError".into()))?;
        let symbol=v8::Symbol::get_iterator(scope);
        let method=source.get(scope,symbol.into()).ok_or(V8BindingError::JavaScriptException)?;
        let method=v8::Local::<v8::Function>::try_from(method).map_err(|_|V8BindingError::Message("TypeError".into()))?;
        let mut index=0u32;
        iterable_conversion::consume(scope,source,method,|scope,value|{
            let object=v8::Local::<v8::Object>::try_from(value).map_err(|_|V8BindingError::Message("TypeError".into()))?;
            let state=binding_state_for_receiver(scope,object).ok_or(V8BindingError::Message("TypeError".into()))?;
            let id=stateful_id(scope,object)?;
            if !state.font_loading.borrow().face_objects.contains_key(&id){return Err(V8BindingError::Message("TypeError".into()));}
            if array.set_index(scope,index,value)!=Some(true){return Err(V8BindingError::JavaScriptException);}
            index=index.checked_add(1).ok_or(V8BindingError::Allocation("font event sequence overflow"))?;
            Ok(())
        })?;
    }
    if array.set_integrity_level(scope,v8::IntegrityLevel::Frozen)!=Some(true){return Err(V8BindingError::JavaScriptException);}
    let key=v8::String::new(scope,"oryn.fontset.event.faces").ok_or(V8BindingError::Allocation("font event faces"))?;
    let key=v8::Private::for_api(scope,Some(key));
    if event.set_private(scope,key,array.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
    Ok(())
}
pub(super) fn drain_events(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    struct DrainGuard<'a>(&'a V8BindingState);
    impl Drop for DrainGuard<'_>{fn drop(&mut self){self.0.font_loading.borrow_mut().event_draining=false;}}
    let admitted={let mut fonts=state.font_loading.borrow_mut();
        if fonts.event_draining{return Ok(());}
        fonts.event_draining=true;
        fonts.events.len()+usize::from(fonts.event_retry.is_some())
    };
    let _drain=DrainGuard(state);
    for _ in 0..admitted {
        let next={let mut fonts=state.font_loading.borrow_mut();fonts.event_retry.take().or_else(||fonts.events.pop_front())};
        let Some(mut notification)=next else{break};
        match notification.resume(scope,state){
            Ok(true)=>notification.finish(scope,state),
            Ok(false)=>{state.font_loading.borrow_mut().event_retry=Some(notification);break;},
            Err(error)=>{state.font_loading.borrow_mut().event_retry=Some(notification);return Err(error);},
        }
        if state.execution_tasks_stopped(){break;}
    }
    Ok(())
}
