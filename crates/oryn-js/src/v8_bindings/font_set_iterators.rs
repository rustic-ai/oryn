// Dedicated Set iterator semantics: keys===values, entries emit [value,value],
// Symbol.iterator aliases values. Mutations are observed live by the cursor.
pub(super) fn install_setlike(scope:&mut v8::PinScope,prototype:v8::Local<v8::ObjectTemplate>)
    ->Result<(),V8BindingError>{
    let values=v8::FunctionTemplate::builder(set_iterator_callback)
        .data(v8::Integer::new(scope,0).into()).build(scope);
    let entries=v8::FunctionTemplate::builder(set_iterator_callback)
        .data(v8::Integer::new(scope,1).into()).build(scope);
    for name in ["keys","values"] {
        let name=v8::String::new(scope,name).ok_or(V8BindingError::Allocation("font set iterator"))?;
        prototype.set(name.into(),values.into());
    }
    prototype.set(v8::Symbol::get_iterator(scope).into(),values.into());
    let name=v8::String::new(scope,"entries").ok_or(V8BindingError::Allocation("font set entries"))?;
    prototype.set(name.into(),entries.into());
    let for_each=v8::FunctionTemplate::builder(set_for_each).length(1).build(scope);
    let name=v8::String::new(scope,"forEach").ok_or(V8BindingError::Allocation("font set forEach"))?;
    prototype.set(name.into(),for_each.into());
    Ok(())
}
fn iterator_private<'s>(scope:&mut v8::PinScope<'s,'_>,name:&str)->Result<v8::Local<'s,v8::Private>,V8BindingError>{
    let name=v8::String::new(scope,name).ok_or(V8BindingError::Allocation("font iterator private key"))?;
    Ok(v8::Private::for_api(scope,Some(name)))
}
fn iterator_prototype<'s>(scope:&mut v8::PinScope<'s,'_>)->Result<v8::Local<'s,v8::Object>,V8BindingError>{
    let context=scope.get_current_context();
    let global=context.global(scope);
    let key=iterator_private(scope,"oryn.fontSetIteratorPrototype")?;
    if let Some(value)=global.get_private(scope,key).filter(|value|value.is_object()){
        return v8::Local::<v8::Object>::try_from(value).map_err(|_|V8BindingError::Allocation("font iterator prototype"));
    }
    let holder=v8::ObjectTemplate::new(scope);
    let name=v8::String::new(scope,"parent").ok_or(V8BindingError::Allocation("font iterator parent"))?;
    holder.set_intrinsic_data_property(name.into(),v8::Intrinsic::IteratorPrototype,v8::PropertyAttribute::NONE);
    let holder=holder.new_instance(scope).ok_or(V8BindingError::Allocation("font iterator intrinsic holder"))?;
    let parent=holder.get(scope,name.into()).ok_or(V8BindingError::JavaScriptException)?;
    let prototype=v8::Object::new(scope);
    if prototype.set_prototype(scope,parent)!=Some(true){return Err(V8BindingError::JavaScriptException);}
    let next=v8::Function::builder(set_iterator_next).length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw).build(scope)
        .ok_or(V8BindingError::Allocation("font iterator next"))?;
    let name=v8::String::new(scope,"next").ok_or(V8BindingError::Allocation("font iterator next name"))?;
    next.set_name(name);
    if prototype.define_own_property(scope,name.into(),next.into(),v8::PropertyAttribute::DONT_ENUM)!=Some(true){return Err(V8BindingError::JavaScriptException);}
    let tag=v8::String::new(scope,"FontFaceSet Iterator").ok_or(V8BindingError::Allocation("font iterator tag"))?;
    let symbol=v8::Symbol::get_to_string_tag(scope);
    if prototype.define_own_property(scope,symbol.into(),tag.into(),v8::PropertyAttribute::DONT_ENUM|v8::PropertyAttribute::READ_ONLY)!=Some(true){return Err(V8BindingError::JavaScriptException);}
    if global.set_private(scope,key,prototype.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
    Ok(prototype)
}
fn set_iterator_callback<'s>(scope:&mut v8::PinScope<'s,'_>,args:v8::FunctionCallbackArguments<'s>,mut rv:v8::ReturnValue){
    let result=(||{
        let (state,id)=font_receiver(scope,args.this(),"FontFaceSet")?;
        if !state.font_loading.borrow().sets.contains_key(&id){return Err(V8BindingError::Message("TypeError".into()));}
        let data=v8::Array::new_with_elements(scope,&[
            args.this().into(),v8::BigInt::new_from_u64(scope,0).into(),args.data(),v8::Boolean::new(scope,false).into()]);
        let prototype=iterator_prototype(scope)?;
        let object=v8::Object::new(scope);
        if object.set_prototype(scope,prototype.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        let key=iterator_private(scope,"oryn.fontSetIteratorData")?;
        if object.set_private(scope,key,data.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        // A borrowed next must reject a different receiver rather than advance
        // the captured iterator. The native data keeps the exact receiver alive.
        if data.set_index(scope,4,object.into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        Ok(object)
    })();
    match result {Ok(value)=>rv.set(value.into()),Err(error)=>throw_binding_error(scope,&error)}
}
fn set_for_each<'s>(scope:&mut v8::PinScope<'s,'_>,args:v8::FunctionCallbackArguments<'s>,mut rv:v8::ReturnValue){
    let result=(||{
        let (state,id)=font_receiver(scope,args.this(),"FontFaceSet")?;
        let callback=v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_|V8BindingError::Message("TypeError".into()))?;
        let mut serial=0u64;
        loop {
            if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException);}
            let expected_owner=owner(&state);
            let entry={let fonts=state.font_loading.borrow();
                let set=fonts.sets.get(&id).ok_or(V8BindingError::Message("TypeError".into()))?;
                set.iteration_order.iter().find(|(entry,_)|*entry>serial).copied()};
            let Some((position,face))=entry else{break};
            serial=position;
            let value=new_platform_object(scope,&state,"FontFace",face)?;
            if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException);}
            if state.execution_tasks_stopped()||owner(&state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
            if !state.font_loading.borrow().sets.get(&id).is_some_and(|set|
                set.iteration_order.iter().any(|entry|*entry==(position,face))){continue;}
            // Call outside all font state borrows. Mutation insertion serials
            // are observed on the next iteration like native WebIDL setlike.
            callback.call(scope,args.get(1),&[value.into(),value.into(),args.this().into()])
                .ok_or(V8BindingError::JavaScriptException)?;
            if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException);}
        }
        Ok(())
    })();
    match result{Ok(())=>rv.set(v8::undefined(scope).into()),Err(error)=>throw_binding_error(scope,&error)}
}
fn set_iterator_next<'s>(scope:&mut v8::PinScope<'s,'_>,args:v8::FunctionCallbackArguments<'s>,mut rv:v8::ReturnValue){
    let result=(||{
        let key=iterator_private(scope,"oryn.fontSetIteratorData")?;
        let value=args.this().get_private(scope,key).ok_or_else(||V8BindingError::Message("TypeError".into()))?;
        let data=v8::Local::<v8::Array>::try_from(value).map_err(|_|V8BindingError::Message("TypeError".into()))?;
        let receiver=data.get_index(scope,4).ok_or(V8BindingError::JavaScriptException)?;
        if !receiver.strict_equals(args.this().into()){return Err(V8BindingError::Message("TypeError".into()));}
        let object=v8::Local::<v8::Object>::try_from(data.get_index(scope,0).ok_or(V8BindingError::JavaScriptException)?)
            .map_err(|_|V8BindingError::Allocation("font iterator set"))?;
        let state=binding_state_for_receiver(scope,object).ok_or(V8BindingError::Allocation("font iterator owner"))?;
        let id=stateful_id(scope,object)?;
        let expected_owner=owner(&state);
        let cursor=data.get_index(scope,1).ok_or(V8BindingError::JavaScriptException)?;
        let cursor=v8::Local::<v8::BigInt>::try_from(cursor).map_err(|_|V8BindingError::Allocation("font iterator cursor"))?;
        let (serial,lossless)=cursor.u64_value();
        if !lossless{return Err(V8BindingError::Allocation("font iterator cursor overflow"));}
        let pair=data.get_index(scope,2).and_then(|value|value.int32_value(scope))==Some(1);
        let exhausted=data.get_index(scope,3).is_some_and(|value|value.is_true());
        let entry=if exhausted {None}else{state.font_loading.borrow().sets.get(&id).and_then(|set|
            set.iteration_order.iter().find(|(entry,_)|*entry>serial).copied())};
        let face=entry.map(|(_,face)|face);
        let value=match face {
            Some(face)=>{
                let value=new_platform_object(scope,&state,"FontFace",face)?;
                if pair {v8::Array::new_with_elements(scope,&[value.into(),value.into()]).into()}else{value.into()}
            },
            None=>{
                v8::undefined(scope).into()
            },
        };
        let result=v8::Object::new(scope);
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException);}
        if state.execution_tasks_stopped()||owner(&state)!=expected_owner{return Err(V8BindingError::JavaScriptException);}
        if let Some(entry)=entry{
            if !state.font_loading.borrow().sets.get(&id).is_some_and(|set|set.iteration_order.contains(&entry)){
                return Err(V8BindingError::Allocation("font iterator entry changed during publication"));
            }
        }
        let key=v8::String::new(scope,"value").ok_or(V8BindingError::Allocation("font iterator value"))?;
        if result.set(scope,key.into(),value)!=Some(true){return Err(V8BindingError::JavaScriptException);}
        let key=v8::String::new(scope,"done").ok_or(V8BindingError::Allocation("font iterator done"))?;
        if result.set(scope,key.into(),v8::Boolean::new(scope,face.is_none()).into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        if let Some((next,_))=entry{
            if data.set_index(scope,1,v8::BigInt::new_from_u64(scope,next).into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        }else if data.set_index(scope,3,v8::Boolean::new(scope,true).into())!=Some(true){return Err(V8BindingError::JavaScriptException);}
        Ok(result)
    })();
    match result{Ok(value)=>rv.set(value.into()),Err(error)=>throw_binding_error(scope,&error)}
}
