fn synchronize_connected(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,id:u64)->Result<(),V8BindingError>{
    sweep_retired_font_edges(scope,state,id)?;
    let document=state.font_loading.borrow().sets.get(&id).and_then(|set|set.document);
    if document.is_none(){return Ok(());}
    if document!=Some(state.document_root())||state.execution_retired(){return Err(V8BindingError::Message("InvalidStateError".into()));}
    let provider=state.font_loading.borrow().provider.clone().ok_or(V8BindingError::Allocation("font provider"))?;
    let original_owner=owner(state);
    // Conversion guards have closed before the connected-face provider walk.
    let _connected_entry=state.enter_retained_allocation(scope)?;
    let current=provider.connected().map_err(V8BindingError::EventCallback)?;
    if owner(state)!=original_owner||state.execution_retired(){return Err(V8BindingError::Message("InvalidStateError".into()))}
    let count=state.font_loading.borrow().face_objects.len();
    let _existing_charge=reserve_font_container(scope,state,count.checked_mul(std::mem::size_of::<(u64,Rc<dyn NativeFontFace>)>()).ok_or(V8BindingError::Allocation("font existing snapshot bytes"))?)?;
    let mut existing=Vec::new();existing.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font existing snapshot"))?;
    if existing.capacity()>count{return Err(V8BindingError::Allocation("font existing snapshot admission"))}
    {let fonts=state.font_loading.borrow();if fonts.face_objects.len()!=count{return Err(V8BindingError::Allocation("font registry changed"))}existing.extend(fonts.face_objects.iter().filter_map(|(id,face)|face.connected.then_some((*id,face.native.clone()))));}
    let (explicit_count,revision)={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;(set.faces.len(),set.membership_revision)};
    let target=current.faces.len().checked_add(explicit_count).ok_or(V8BindingError::Allocation("font members overflow"))?;
    let members_capacity=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font members bytes"))?)?;
    let mut members=Vec::new();
    members.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font members"))?;
    if members.capacity()>target{return Err(V8BindingError::Allocation("font members admission"))}
    let _updates_charge=reserve_font_container(scope,state,current.faces.len().checked_mul(std::mem::size_of::<(u64,Rc<dyn NativeFontFace>,u64)>()).ok_or(V8BindingError::Allocation("font connected updates bytes"))?)?;
    let mut updates=Vec::new();updates.try_reserve_exact(current.faces.len()).map_err(|_|V8BindingError::Allocation("font connected updates"))?;
    if updates.capacity()>current.faces.len(){return Err(V8BindingError::Allocation("font connected updates admission"))}
    for native in &current.faces {
        let id=existing.iter().find(|(_,old)|old.same_face(native.as_ref())).map(|(id,_)|*id);
        members.push(match id {
            Some(id)=>{
                updates.push((id,native.clone(),native.revision()));
                id
            },
            None=>register_face(scope,state,native.clone(),true)?,
        });
    }
    {let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;
        if set.faces.len()!=explicit_count||set.membership_revision!=revision{return Err(V8BindingError::Allocation("font membership changed"))}
        members.extend(set.faces.iter().copied().filter(|id|fonts.face_objects.get(id).is_some_and(|face|!face.connected)));
    }
    // Prepare wrapper roots before publishing new native membership. These
    // private edges are non-author observable; retained old edges keep removed
    // wrappers safe until the native transaction has committed.
    let pending_charge=reserve_font_container(scope,state,members.len().checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font pending roots overflow"))?)?;
    let mut pending=Vec::new();pending.try_reserve_exact(members.len()).map_err(|_|V8BindingError::Allocation("font pending roots"))?;
    if pending.capacity()>members.len(){return Err(V8BindingError::Allocation("font pending roots admission"));}
    pending.extend(members.iter().copied());
    {let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
        if set.pending_edges.is_some(){return Err(V8BindingError::Allocation("font pending roots not swept"));}
        set.pending_edges=Some((pending,pending_charge));}
    let set_object=new_platform_object(scope,state,"FontFaceSet",id)?;
    for face in &members{
        let value=new_platform_object(scope,state,"FontFace",*face)?;
        let _key_charge=reserve_font_container(scope,state,64)?;
        let name=v8::String::new(scope,&format!("oryn.fontset.face.{face}")).ok_or(V8BindingError::Allocation("font membership edge"))?;
        let key=v8::Private::for_api(scope,Some(name));
        if set_object.set_private(scope,key,value.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
    }
    let added={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;members.iter().filter(|id|!set.faces.contains(id)).count()};
    font_prepare_membership_capacity(scope,state,id,added)?;
    for (_,native,observed) in &updates{if native.revision()!=*observed{return Err(V8BindingError::Allocation("font connected revision changed during preparation"))}}
    let _displaced_charge=reserve_font_container(scope,state,updates.len().checked_mul(std::mem::size_of::<Rc<dyn NativeFontFace>>()).ok_or(V8BindingError::Allocation("font displaced handles bytes"))?)?;
    let mut displaced=Vec::new();displaced.try_reserve_exact(updates.len()).map_err(|_|V8BindingError::Allocation("font displaced handles"))?;
    if displaced.capacity()>updates.len(){return Err(V8BindingError::Allocation("font displaced handles admission"))}
    for (_,native,observed) in &updates{if native.revision()!=*observed{return Err(V8BindingError::Allocation("font connected revision changed during preparation"))}}
    // Validate all fallible cycle transitions before any original face is
    // replaced. No callbacks or native lease drops occur in this borrow.
    if owner(state)!=original_owner||state.execution_retired(){return Err(V8BindingError::Message("InvalidStateError".into()))}
    {let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;
        if set.membership_revision!=revision{return Err(V8BindingError::Allocation("font membership changed during preparation"))}
        for (id,_,observed) in &updates{let face=fonts.face_objects.get(id).ok_or(V8BindingError::Allocation("font connected record missing"))?;
            if face.observed_revision!=*observed{fonts.cycles.revision(face.cycle_id).and_then(|n|n.checked_add(1)).ok_or(V8BindingError::Allocation("font cycle revision exhausted"))?;}
        }
    }
    let old={let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
        if set.membership_revision!=revision{return Err(V8BindingError::Allocation("font membership changed during preparation"))}
        let changed=set.faces!=members;
        let next_revision=if changed{revision.checked_add(1).ok_or(V8BindingError::Allocation("font membership revision exhausted"))?}else{revision};
        let end=set.next_serial.checked_add(u64::try_from(added).map_err(|_|V8BindingError::Allocation("font insertion serial exhausted"))?).ok_or(V8BindingError::Allocation("font insertion serial exhausted"))?;
        set.iteration_order.retain(|(_,id)|members.contains(id));
        for member in &members{if !set.faces.contains(member){set.next_serial+=1;set.iteration_order.push((set.next_serial,*member));}}
        debug_assert_eq!(set.next_serial,end);
        let old_faces=std::mem::replace(&mut set.faces,members);
        let old_capacity=std::mem::replace(&mut set.members_capacity,members_capacity);
        set.retired_edges=Some((old_faces,old_capacity));set.membership_revision=next_revision;
        ()
    };
    // Membership commit has no callback. Transfer prepared original handles one
    // at a time and release displaced handles only outside font state borrow.
    {let mut fonts=state.font_loading.borrow_mut();for (id,native,observed) in updates{
            let face=fonts.face_objects.get(&id).ok_or(V8BindingError::Allocation("font connected record missing"))?;
            let cycle=face.cycle_id;let changed=face.observed_revision!=observed;
            if changed{fonts.cycles.replace(cycle).map_err(|error|V8BindingError::EventCallback(error.into()))?;}
            let face=fonts.face_objects.get_mut(&id).ok_or(V8BindingError::Allocation("font connected record missing"))?;
            face.observed_revision=observed;displaced.push(std::mem::replace(&mut face.native,native));
    }}drop(displaced);drop(old);
    let pending=state.font_loading.borrow_mut().sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?.pending_edges.take();
    drop(pending);
    sweep_retired_font_edges(scope,state,id)
}
fn sweep_retired_font_edges(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,id:u64)->Result<(),V8BindingError>{
    let expected_owner=owner(state);
    {let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
        if set.retired_edges.is_none(){set.retired_edges=set.pending_edges.take();}}
    let object=new_platform_object(scope,state,"FontFaceSet",id)?;
    loop{
        let candidate={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;
            set.retired_edges.as_ref().and_then(|(edges,_)|edges.last().copied()).map(|face|(face,set.faces.contains(&face)))};
        let Some((face,present))=candidate else{break;};
        if !present{
            let _charge=reserve_font_container(scope,state,64)?;
            let name=v8::String::new(scope,&format!("oryn.fontset.face.{face}")).ok_or(V8BindingError::Allocation("font retired edge"))?;
            let key=v8::Private::for_api(scope,Some(name));
            if object.delete_private(scope,key)!=Some(true){return Err(V8BindingError::JavaScriptException);}
        }
        if !crate::v8_termination::checkpoint(scope)||state.execution_tasks_stopped()||owner(state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
        let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
        let Some((edges,_))=set.retired_edges.as_mut()else{continue;};
        if edges.last()==Some(&face){edges.pop();}
    }
    let retired=state.font_loading.borrow_mut().sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?.retired_edges.take();
    drop(retired);
    if state.font_loading.borrow().sets.get(&id).is_some_and(|set|set.pending_edges.is_some()){
        return sweep_retired_font_edges(scope,state,id);
    }
    Ok(())
}
pub(super) fn construct<'s>(scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>,name:&str,args:&v8::FunctionCallbackArguments<'s>)
    ->Result<(),V8BindingError>{
    // The pinned authoritative IDL exposes FontFaceSet but does not declare
    // a constructor. Creation is the Document/Worker FontFaceSource operation.
    if name!="FontFace" { return Err(V8BindingError::Message("TypeError".into())); }
    let expected_owner=owner(state);
    let family=font_owned_input(scope,state,args.get(0))?;
    let source=font_source_input(scope,state,args.get(1))?;
    let mut descriptors=Vec::new();
    let mut descriptor_inputs=Vec::new();
    let _descriptor_capacity=reserve_font_container(scope,state,10usize.checked_mul(std::mem::size_of::<(String,String)>()+std::mem::size_of::<(FontOwnedInput,Option<std::sync::Arc<dyn Send+Sync>>)>()).ok_or(V8BindingError::Allocation("font dictionary capacity"))?)?;
    descriptors.try_reserve_exact(10).map_err(|_|V8BindingError::Allocation("font descriptors"))?;
    descriptor_inputs.try_reserve_exact(10).map_err(|_|V8BindingError::Allocation("font descriptor inputs"))?;
    if descriptors.capacity()>10||descriptor_inputs.capacity()>10{return Err(V8BindingError::Allocation("font dictionary admission"));}
    if args.length()>2 && !args.get(2).is_null_or_undefined() {
        if !args.get(2).is_object(){return Err(V8BindingError::Message("TypeError".into()));}
        let object=args.get(2).to_object(scope).ok_or(V8BindingError::JavaScriptException)?;
        for name in ["ascentOverride","descentOverride","display","featureSettings","lineGapOverride","stretch","style","unicodeRange","variationSettings","weight"] {
            let key=v8::String::new(scope,name).ok_or(V8BindingError::Allocation("font descriptor"))?;
            let value=object.get(scope,key.into()).ok_or(V8BindingError::JavaScriptException)?;
            if !value.is_undefined(){let mut value=font_owned_input(scope,state,value)?;
                let _name_charge=reserve_font_container(scope,state,name.len())?;
                let mut owned_name=String::new();owned_name.try_reserve_exact(name.len()).map_err(|_|V8BindingError::Allocation("font descriptor name"))?;
                if owned_name.capacity()>name.len(){return Err(V8BindingError::Allocation("font descriptor name admission"));}owned_name.push_str(name);
                descriptors.push((owned_name,std::mem::take(&mut value.value)));descriptor_inputs.push((value,_name_charge));}
        }
    }
    let provider=state.font_loading.borrow().provider.clone().ok_or(V8BindingError::Allocation("native font loading provider"))?;
    if state.execution_retired()||owner(state)!=expected_owner{return Err(V8BindingError::Message("InvalidStateError".into()));}
    let _construction_entry=state.enter_retained_allocation(scope)?;
    let native=provider.construct(&family.value,source,&descriptors).map_err(V8BindingError::EventCallback)?;
    if state.execution_retired()||owner(state)!=expected_owner{return Err(V8BindingError::Message("InvalidStateError".into()));}
    drop(descriptors);drop(descriptor_inputs);
    let id=register_platform_object(state,PlatformObjectRecord::FontFace);
    bind_platform_object(scope,state,id,args.this());
    let loaded=promise_slot(scope,state,args.this(),"oryn.fontface.loaded")?;
    let cycle_id=state.font_loading.borrow_mut().cycles.create().map_err(|error|V8BindingError::EventCallback(error.into()))?;
    let binary=native.binary_source();
    let invalid=native.initial_error();
    let observed_revision=native.revision();
    font_prepare_face_registry(scope,state)?;
    state.font_loading.borrow_mut().face_objects.insert(id,FaceRecord{cycle_id,native,loaded,connected:false,observed_revision,completed_cycle:None})?;
    if invalid {
        let attempt=state.font_loading.borrow_mut().cycles.start(cycle_id)
            .map_err(|error|V8BindingError::EventCallback(error.into()))?.ok_or(V8BindingError::Allocation("font initial error"))?;
        let resolver=state.font_loading.borrow().face_objects.get(&id).unwrap().loaded.resolver.clone();
        let resolver=resolver.to_local(scope).ok_or(V8BindingError::Allocation("font initial resolver"))?;
        let error=dom_exception_value(scope,"SyntaxError")?;
        if resolver.reject(scope,error)!=Some(true){return Err(V8BindingError::JavaScriptException);}
        state.font_loading.borrow_mut().cycles.finish(attempt,font_cycles::Terminal::Exhausted);
        state.font_loading.borrow_mut().face_objects.get_mut(&id).unwrap().loaded.settled=true;
    }else if binary {begin(scope,state,id," ")?;}
    Ok(())
}
