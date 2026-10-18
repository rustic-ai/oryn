pub(super) fn operation<'s>(
    scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>,object:v8::Local<'s,v8::Object>,
    interface:&str,member:&str,args:&v8::FunctionCallbackArguments<'s>,
)->Result<v8::Local<'s,v8::Value>,V8BindingError> {
    let id=stateful_id(scope,object)?;
    if interface=="FontFace" && member=="load" {return Ok(begin(scope,state,id," ")?.into());}
    if interface!="FontFaceSet" {return Err(V8BindingError::Allocation("font receiver"));}
    match member {
        "add"|"has"|"delete"=>{
            let face=v8::Local::<v8::Object>::try_from(args.get(0)).map_err(|_|V8BindingError::Message("TypeError".into()))?;
            let owner=binding_state_for_receiver(scope,face).ok_or(V8BindingError::Allocation("font owner"))?;
            if !Rc::ptr_eq(&owner,state) {return Err(V8BindingError::Message("TypeError: foreign FontFace".into()));}
            let face_id=stateful_id(scope,face)?;
            let (connected,loading)={
                let fonts=state.font_loading.borrow();let face=fonts.face_objects.get(&face_id)
                    .ok_or(V8BindingError::Message("TypeError: FontFace required".into()))?;
                (face.connected,fonts.cycles.status(face.cycle_id)==Some(font_cycles::Status::Loading))
            };
            if member=="has"{
                let present=state.font_loading.borrow().sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?.faces.contains(&face_id);
                return Ok(v8::Boolean::new(scope,present).into());
            }
            let expected_owner=super::font_loading::owner(state);
            let _edge_name_charge=reserve_font_container(scope,state,64)?;
            let edge_name=format!("oryn.fontset.face.{face_id}");
            let key=v8::String::new(scope,&edge_name).ok_or(V8BindingError::Allocation("font membership edge"))?;
            let edge_key=v8::Private::for_api(scope,Some(key));
            if member=="add"{let missing={let fonts=state.font_loading.borrow();!fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?.faces.contains(&face_id)};if missing{font_prepare_membership_capacity(scope,state,id,1)?;}}
            let mut fonts=state.font_loading.borrow_mut();
            let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
            let present=set.faces.contains(&face_id);
            if member=="delete" {
                if connected {return Ok(v8::Boolean::new(scope,false).into());}
                let revision=if present{set.membership_revision.checked_add(1).ok_or(V8BindingError::Allocation("font membership revision exhausted"))?}else{set.membership_revision};
                let expected_revision=set.membership_revision;drop(fonts);
                if object.delete_private(scope,edge_key)!=Some(true){return Err(V8BindingError::JavaScriptException);}
                if state.execution_tasks_stopped()||super::font_loading::owner(state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
                let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
                if set.membership_revision!=expected_revision{return Err(V8BindingError::Allocation("font membership changed during edge publication"));}
                set.faces.retain(|old|*old!=face_id);
                set.iteration_order.retain(|(_,id)|*id!=face_id);set.membership_revision=revision;
                drop(fonts);
                settle_sets(scope,state)?;
                return Ok(v8::Boolean::new(scope,present).into());
            }
            if !present {
                let serial=set.next_serial.checked_add(1).ok_or(V8BindingError::Allocation("font insertion serial exhausted"))?;
                let revision=set.membership_revision.checked_add(1).ok_or(V8BindingError::Allocation("font membership revision exhausted"))?;
                if set.faces.len()==set.faces.capacity()||set.iteration_order.len()==set.iteration_order.capacity(){return Err(V8BindingError::Allocation("font membership changed during preparation"))}
                let expected_revision=set.membership_revision;drop(fonts);
                if object.set_private(scope,edge_key,face.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
                if state.execution_tasks_stopped()||super::font_loading::owner(state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
                let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
                if set.membership_revision!=expected_revision{return Err(V8BindingError::Allocation("font membership changed during edge publication"));}
                set.faces.push(face_id);set.iteration_order.push((serial,face_id));set.next_serial=serial;set.membership_revision=revision;
            }else{
                drop(fonts);
            }
            if loading {renew_loading_sets(scope,state,face_id)?;}
            Ok(object.into())
        },
        "clear"=>{
            let (face_count,count,revision)={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;(fonts.face_objects.len(),set.faces.len(),set.membership_revision)};
            let _clear_charge=reserve_font_container(scope,state,face_count.checked_add(count).and_then(|n|n.checked_mul(std::mem::size_of::<u64>())).ok_or(V8BindingError::Allocation("font clear snapshot bytes"))?)?;
            let mut connected=Vec::new();let mut removed=Vec::new();
            connected.try_reserve_exact(face_count).map_err(|_|V8BindingError::Allocation("font clear connected"))?;
            removed.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font clear removed"))?;
            if connected.capacity()>face_count||removed.capacity()>count{return Err(V8BindingError::Allocation("font clear snapshot admission"))}
            let mut fonts=state.font_loading.borrow_mut();
            if fonts.face_objects.len()>face_count{return Err(V8BindingError::Allocation("font clear registry changed"))}
            connected.extend(fonts.face_objects.iter().filter_map(|(id,face)|face.connected.then_some(*id)));
            let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
            if set.faces.len()!=count||set.membership_revision!=revision{return Err(V8BindingError::Allocation("font clear membership changed"))}
            removed.extend(set.faces.iter().copied().filter(|id|!connected.contains(id)));
            let mut revision=set.membership_revision;
            drop(fonts);
            let expected_owner=super::font_loading::owner(state);
            for face in removed {
                let next_revision=revision.checked_add(1).ok_or(V8BindingError::Allocation("font membership revision exhausted"))?;
                let _edge_charge=reserve_font_container(scope,state,64)?;
                let key=v8::String::new(scope,&format!("oryn.fontset.face.{face}"))
                    .ok_or(V8BindingError::Allocation("font membership edge"))?;
                let key=v8::Private::for_api(scope,Some(key));
                if object.delete_private(scope,key)!=Some(true){return Err(V8BindingError::JavaScriptException);}
                if state.execution_tasks_stopped()||super::font_loading::owner(state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
                let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
                if set.membership_revision!=revision{return Err(V8BindingError::Allocation("font clear changed during edge publication"));}
                set.faces.retain(|id|*id!=face);set.iteration_order.retain(|(_,id)|*id!=face);
                set.membership_revision=next_revision;revision=next_revision;
            }
            settle_sets(scope,state)?;
            Ok(v8::undefined(scope).into())
        },
        "load"|"check"=>{
            if args.length() == 0 {
                return Err(V8BindingError::EventCallback("TypeError: font argument is required".into()));
            }
            let expected_owner=super::font_loading::owner(state);
            let font=font_owned_input(scope,state,args.get(0))?;
            let default_text=v8::String::new(scope," ").ok_or(V8BindingError::Allocation("font default text"))?;
            let text=font_owned_input(scope,state,if args.length()>1{args.get(1)}else{default_text.into()})?;
            if state.execution_tasks_stopped()||super::font_loading::owner(state)!=expected_owner{return Err(V8BindingError::Message("InvalidStateError".into()));}
            synchronize_connected(scope,state,id)?;
            let (count,revision)={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;(set.faces.len(),set.membership_revision)};
            let _snapshot_charge=reserve_font_container(scope,state,count.checked_mul(std::mem::size_of::<u64>()+std::mem::size_of::<Rc<dyn NativeFontFace>>()).ok_or(V8BindingError::Allocation("font selection snapshot bytes"))?)?;
            let mut ids=Vec::new();ids.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font selection ids"))?;
            let mut faces=Vec::new();faces.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font selection faces"))?;
            if ids.capacity()>count||faces.capacity()>count{return Err(V8BindingError::Allocation("font selection capacity admission"))}
            let (provider,ids,faces)={
                let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;
                if set.faces.len()!=count||set.membership_revision!=revision{return Err(V8BindingError::Allocation("font selection membership changed"))}
                ids.extend(set.faces.iter().copied());faces.extend(set.faces.iter().filter_map(|id|fonts.face_objects.get(id).map(|face|face.native.clone())));
                (fonts.provider.clone().ok_or(V8BindingError::Allocation("font provider"))?,ids,faces)
            };
            let selected=provider.select(&font.value,&text.value,&faces).map_err(|error|match error{
                font_loading_transport::FontSelectionError::Syntax=>V8BindingError::Message("SyntaxError: invalid font shorthand".into()),
                font_loading_transport::FontSelectionError::Resource(error)=>V8BindingError::EventCallback(error),
            })?;
            let selected_charge=reserve_font_container(scope,state,selected.indices.len().checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font selected bytes"))?)?;
            let mut selected_faces=Vec::new();selected_faces.try_reserve_exact(selected.indices.len()).map_err(|_|V8BindingError::Allocation("font selected faces"))?;
            if selected_faces.capacity()>selected.indices.len(){return Err(V8BindingError::Allocation("font selected capacity admission"))}
            for index in &selected.indices{selected_faces.push(ids.get(*index).copied().ok_or(V8BindingError::Allocation("font selection"))?);}
            drop(selected);
            let selected=selected_faces;
            if member=="check" {
                let fonts=state.font_loading.borrow();
                let loaded=selected.iter().all(|id|fonts.face_objects.get(id).is_some_and(|face|
                    fonts.cycles.status(face.cycle_id)==Some(font_cycles::Status::Loaded)));
                drop(fonts);return Ok(v8::Boolean::new(scope,loaded).into());
            }
            let resolver=v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("font set load"))?;
            let promise=resolver.get_promise(scope);
            let mut wrappers=Vec::new();
            let _wrapper_charge=reserve_font_container(scope,state,selected.len().checked_mul(std::mem::size_of::<v8::Local<v8::Value>>()).ok_or(V8BindingError::Allocation("font wrapper bytes"))?)?;
            wrappers.try_reserve_exact(selected.len()).map_err(|_|V8BindingError::Allocation("font wrapper capacity"))?;
            if wrappers.capacity()>selected.len(){return Err(V8BindingError::Allocation("font wrapper capacity admission"))}
            for face in &selected {begin(scope,state,*face,&text.value)?;wrappers.push(new_platform_object(scope,state,"FontFace",*face)?.into());}
            let array=v8::Array::new_with_elements(scope,&wrappers);
            if selected.is_empty() {
                if resolver.resolve(scope,array.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
            } else {
                font_prepare_load_capacity(scope,state)?;
                state.font_loading.borrow_mut().loads.push(LoadRecord{
                    published:false,
                    _faces: selected_charge,
                    faces:selected,resolver:v8::Global::new(scope,resolver),array:v8::Global::new(scope,array)});
            }
            Ok(promise.into())
        },
        _=>Err(V8BindingError::Unsupported{interface:interface.into(),member:member.into()})
    }
}
struct LoadRecord {published:bool,_faces:Option<std::sync::Arc<dyn Send+Sync>>,faces:Vec<u64>,resolver:v8::Global<v8::PromiseResolver>,array:v8::Global<v8::Array>}
fn settle_loads(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError> {
    let original_owner=owner(state);
    loop {
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        if owner(state)!=original_owner||state.execution_retired(){return Ok(())}
        if state.execution_suspended()||state.execution_tasks_stopped(){return Ok(())}
        let next={
            let fonts=state.font_loading.borrow();
            fonts.loads.iter().enumerate().find_map(|(index,load)|{
                let waiting=load.faces.iter().any(|id|fonts.face_objects.get(id).is_some_and(|face|fonts.cycles.status(face.cycle_id)==Some(font_cycles::Status::Loading)));
                if waiting&&!load.published{return None}
                let error=load.faces.iter().any(|id|fonts.face_objects.get(id).is_none_or(|face|fonts.cycles.status(face.cycle_id)!=Some(font_cycles::Status::Loaded)));
                Some((index,error,load.published,v8::Local::new(scope,&load.resolver),v8::Local::new(scope,&load.array)))
            })
        };
        let Some((index,error,published,resolver,array))=next else{return Ok(())};
        #[cfg(test)]
        if !published{settlement_controls::before_publication()?;}
        // Keep the original admitted record in State until publication succeeds;
        // a failure leaves this record and every later ready record retryable.
        if !published {if error {let error=dom_exception_value(scope,"NetworkError")?;
            if resolver.reject(scope,error)!=Some(true){return Err(V8BindingError::JavaScriptException)}
        }else {if resolver.resolve(scope,array.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}}
            let mut fonts=state.font_loading.borrow_mut();
            let load=fonts.loads.get_mut(index).ok_or(V8BindingError::Allocation("font load publication changed"))?;
            if v8::Local::new(scope,&load.resolver)!=resolver{return Err(V8BindingError::Allocation("font load publication changed"))}
            load.published=true;
        }
        #[cfg(test)]
        settlement_controls::after_publication(state);
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        if owner(state)!=original_owner||state.execution_retired(){return Ok(())}
        if state.execution_suspended()||state.execution_tasks_stopped(){return Ok(())}
        // Promise resolve/reject queues reactions; it does not run author code.
        // Verify exact original resolver identity rather than trust an index.
        let removed={let mut fonts=state.font_loading.borrow_mut();
            let Some(load)=fonts.loads.get(index) else{return Err(V8BindingError::Allocation("font load publication changed"))};
            if v8::Local::new(scope,&load.resolver)!=resolver{return Err(V8BindingError::Allocation("font load publication changed"))}
            fonts.loads.remove(index)
        };drop(removed);
    }
}
