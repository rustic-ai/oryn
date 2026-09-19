// Dynamic remote Window/Location cross-origin projection. These callbacks run
// only after the parent-authorized access check denies ordinary same-origin
// property access. No public marker or authored getter grants authority.
use v8::MapFnTo;
use oryn_js::v8_context_bridge::{CrossContextHandlers,set_dynamic_remote_handlers};
fn lookup(scope:&mut v8::PinScope,object:v8::Local<v8::Object>)->Option<(RemoteWindow,bool)>{
    let entries=scope.get_slot::<RemoteWindows>()?.entries.values().cloned().collect::<Vec<_>>();
    entries.into_iter().find_map(|entry|{if v8::Local::new(scope,&entry.object).strict_equals(object.into()){Some((entry,false))}else if v8::Local::new(scope,&entry.location).strict_equals(object.into()){Some((entry,true))}else{None}})
}
unsafe extern "C" fn access(context:v8::Local<v8::Context>,object:v8::Local<v8::Object>)->bool{
    // SAFETY: V8 enters this callback with rooted handles. No borrow or scope
    // escapes the synchronous native authority read.
    v8::callback_scope!(unsafe scope,context);
    let Some((entry,_))=lookup(scope,object)else{return true}; // unpublished construction
    remote_window_owner_live(&entry)&&remote_window_snapshot(scope,&entry).is_ok_and(|snapshot|snapshot.same_origin_domain)
}
fn denied(scope:&mut v8::PinScope){oryn_js::v8_bindings::window_messages::throw_external_message_error(scope,"SecurityError");}
fn fallback(scope:&mut v8::PinScope,key:v8::Local<v8::Name>)->bool{
    if key.is_string(){return key.to_rust_string_lossy(scope)=="then";}
    [v8::Symbol::get_to_string_tag(scope),v8::Symbol::get_has_instance(scope),v8::Symbol::get_is_concat_spreadable(scope)].into_iter().any(|symbol|key.strict_equals(symbol.into()))
}
fn property(name:&str,location:bool)->Option<(i32,bool)>{
    if location{return match name{"href"=>Some((101,true)),"replace"=>Some((102,false)),_=>None};}
    Some(match name{"window"|"self"|"frames"=>(11,true),"location"=>(9,true),"closed"=>(0,true),"close"=>(3,false),"focus"=>(2,false),"blur"=>(12,false),"length"=>(8,true),"top"=>(5,true),"parent"=>(6,true),"opener"=>(7,true),"postMessage"=>(4,false),_=>return None})
}
fn invoke<'s>(scope:&mut v8::PinScope<'s,'_>,args:v8::FunctionCallbackArguments<'s>,rv:v8::ReturnValue){
    let Ok(data)=v8::Local::<v8::Object>::try_from(args.data())else{denied(scope);return};
    let Some(target)=data.get_internal_field(scope,1).and_then(|v|v8::Local::<v8::Object>::try_from(v).ok())else{denied(scope);return};
    let receiver=if data.get_internal_field(scope,2).and_then(|value|v8::Local::<v8::Value>::try_from(value).ok()).is_some_and(|value|value.is_true()){args.this()}else{target};
    let Some(operation)=data.get_internal_field(scope,3).and_then(|value|v8::Local::<v8::Value>::try_from(value).ok()).and_then(|value|value.int32_value(scope))else{denied(scope);return};
    let location=data.get_internal_field(scope,4).and_then(|value|v8::Local::<v8::Value>::try_from(value).ok()).is_some_and(|value|value.is_true());
    // Enter the native algorithm directly in the wrapper's caller realm. Calling
    // an owner-realm Function would create TypeErrors in the wrong realm.
    if location{remote_location_operation_for(scope,args,rv,operation-100,receiver);}else{remote_window_operation_for(scope,args,rv,operation,receiver);}
}

fn callable<'s>(scope:&mut v8::PinScope<'s,'_>,entry:&RemoteWindow,location:bool,name:&str,operation:i32,setter:bool)->Option<v8::Local<'s,v8::Function>>{
    let holder=v8::Local::new(scope,&entry.object);let context=holder.get_creation_context(scope)?;
    let cache=oryn_js::v8_bindings::window_proxy::native_descriptor_cache(scope,context)?;
    let target_document=remote_window_snapshot(scope,entry).map(|snapshot|format!("{:?}",snapshot.creator)).unwrap_or_else(|_|"retired".into());
    // GroupToken Debug is intentionally redacted. Serialize its exact native
    // identity only inside this private cache, never into an author property.
    let group_key=serde_json::to_string(&entry.group).ok()?;
    let native_key=format!("remote:{}:{:?}:{}:{}:{group_key}:{}:{target_document}:{operation}:{name}",entry.page.0,entry.target,entry.generation,entry.epoch,u8::from(location));let key=v8::String::new(scope,&native_key)?;
    if cache.has_own_property(scope,key.into())?{return v8::Local::<v8::Function>::try_from(cache.get(scope,key.into())?).ok();}
    let function=v8::Local::new(scope,entry.functions.get(&operation)?);
    let template=v8::ObjectTemplate::new(scope);template.set_internal_field_count(5);let data=template.new_instance(scope)?;
    data.set_internal_field(0,function.into());data.set_internal_field(1,if location{v8::Local::new(scope,&entry.location).into()}else{holder.into()});
    // Location operations and Window.location setters retain ordinary native
    // receiver validation. Window cross-origin getters/methods bind the target.
    data.set_internal_field(2,v8::Boolean::new(scope,setter||location).into());
    data.set_internal_field(3,v8::Integer::new(scope,operation).into());data.set_internal_field(4,v8::Boolean::new(scope,location).into());
    let getter=property(name,location).is_some_and(|(_,getter)|getter)&&!setter;
    let label=if setter{format!("set {name}")}else if getter{format!("get {name}")}else{name.to_owned()};
    let length=i32::from(setter||name=="postMessage"||name=="replace");
    let wrapper=v8::Function::builder(invoke).data(data.into()).length(length).constructor_behavior(v8::ConstructorBehavior::Throw).build(scope)?;
    wrapper.set_name(v8::String::new(scope,&label)?);
    if cache.create_data_property(scope,key.into(),wrapper.into())!=Some(true){return None;}Some(wrapper)
}
fn record<'s>(scope:&mut v8::PinScope<'s,'_>)->Option<v8::Local<'s,v8::Object>>{let object=v8::Object::new(scope);let null=v8::null(scope).into();if object.set_prototype(scope,null)!=Some(true){return None;}Some(object)}
fn put(scope:&mut v8::PinScope,object:v8::Local<v8::Object>,name:&str,value:v8::Local<v8::Value>)->Option<()>{let name=v8::String::new(scope,name)?;(object.create_data_property(scope,name.into(),value)==Some(true)).then_some(())}
fn data<'s>(scope:&mut v8::PinScope<'s,'_>,value:v8::Local<'s,v8::Value>,enumerable:bool)->Option<v8::Local<'s,v8::Object>>{let object=record(scope)?;let no=v8::Boolean::new(scope,false).into();let yes=v8::Boolean::new(scope,true).into();let enumerable=v8::Boolean::new(scope,enumerable).into();put(scope,object,"value",value)?;put(scope,object,"writable",no)?;put(scope,object,"enumerable",enumerable)?;put(scope,object,"configurable",yes)?;Some(object)}
fn child<'s>(scope:&mut v8::PinScope<'s,'_>,entry:&RemoteWindow,key:&str)->Result<Option<v8::Local<'s,v8::Object>>,ScriptError>{
    let snapshot=match remote_window_snapshot(scope,entry){
        Ok(snapshot)=>snapshot,
        // A live named child still precedes the cross-origin fallback. When the
        // target is closed, severed or unrelated, its child inventory is not
        // accessible; `then` must still reach the permitted undefined fallback
        // so Promise resolution can proceed to native Window validation.
        Err(_) if key=="then"=>return Ok(None),
        Err(error)=>return Err(error),
    };
    let index=key.parse::<u32>().ok().filter(|index|*index<u32::MAX&&index.to_string()==key);
    let target=if let Some(index)=index{snapshot.children.get(index as usize).map(|(_,target)|target.clone())}
        else{snapshot.children.iter().find(|(name,_)|!name.is_empty()&&name==key).map(|(_,target)|target.clone())};
    target.map(|target|remote_window_object(scope,entry.page,target,false,true)).transpose()
}
fn descriptor_value<'s>(scope:&mut v8::PinScope<'s,'_>,holder:v8::Local<v8::Object>,key:v8::Local<v8::Name>)->Option<v8::Local<'s,v8::Object>>{
    let Some((entry,location))=lookup(scope,holder)else{denied(scope);return None};
    if let Ok(text)=key.try_cast::<v8::String>(){let mut units=vec![0_u16;text.length()];text.write_v2(scope,0,&mut units,v8::WriteFlags::default());let name=oryn_dom::dom_string::from_units(&units);if let Some((operation,getter))=property(&name,location){
        if !getter{let function=callable(scope,&entry,location,&name,operation,false)?;return data(scope,function.into(),false);}
        let object=record(scope)?;let get=if location{v8::undefined(scope).into()}else{callable(scope,&entry,false,&name,operation,false)?.into()};
        let set=if location||name=="location"{callable(scope,&entry,location,&name,if location{101}else{10},true)?.into()}else{v8::undefined(scope).into()};
        let no=v8::Boolean::new(scope,false).into();let yes=v8::Boolean::new(scope,true).into();put(scope,object,"get",get)?;put(scope,object,"set",set)?;put(scope,object,"enumerable",no)?;put(scope,object,"configurable",yes)?;return Some(object);
    }
    if !location{match child(scope,&entry,&name){Ok(Some(child))=>return data(scope,child.into(),name.parse::<u32>().is_ok_and(|index|index<u32::MAX&&index.to_string()==name)),Ok(None)=>{},Err(_)=>{denied(scope);return None}}}}
    if fallback(scope,key){let value=v8::undefined(scope).into();return data(scope,value,false);}denied(scope);None
}
fn descriptor(scope:&mut v8::PinScope,key:v8::Local<v8::Name>,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue)->v8::Intercepted{if let Some(value)=descriptor_value(scope,args.holder(),key){rv.set(value.into());}v8::Intercepted::kYes}
fn get(scope:&mut v8::PinScope,key:v8::Local<v8::Name>,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue)->v8::Intercepted{
    if let Some(desc)=descriptor_value(scope,args.holder(),key){let value_key=v8::String::new(scope,"value").unwrap();if desc.has_own_property(scope,value_key.into())==Some(true){if let Some(value)=desc.get(scope,value_key.into()){rv.set(value);}}
        else{let get_key=v8::String::new(scope,"get").unwrap();if let Some(function)=desc.get(scope,get_key.into()).and_then(|value|v8::Local::<v8::Function>::try_from(value).ok()){if let Some(value)=function.call(scope,args.holder().into(),&[]){rv.set(value);}}else{denied(scope);}}}v8::Intercepted::kYes
}
fn set(scope:&mut v8::PinScope,key:v8::Local<v8::Name>,value:v8::Local<v8::Value>,args:v8::PropertyCallbackArguments,_:v8::ReturnValue<()>)->v8::Intercepted{
    if let Some((entry,location))=lookup(scope,args.holder()){let name=key.to_rust_string_lossy(scope);if key.is_string()&&((location&&name=="href")||(!location&&name=="location")){if let Some(function)=callable(scope,&entry,location,&name,if location{101}else{10},true){function.call(scope,args.holder().into(),&[value]);}return v8::Intercepted::kYes;}}denied(scope);v8::Intercepted::kYes
}
fn query(scope:&mut v8::PinScope,key:v8::Local<v8::Name>,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue<v8::Integer>)->v8::Intercepted{
    if let Some(desc)=descriptor_value(scope,args.holder(),key){let enumerable=v8::String::new(scope,"enumerable").unwrap();let is_index=desc.get(scope,enumerable.into()).is_some_and(|value|value.is_true());rv.set_int32(if is_index{v8::PropertyAttribute::READ_ONLY.as_u32() as i32}else{v8::PropertyAttribute::DONT_ENUM.as_u32() as i32});}v8::Intercepted::kYes
}
fn enumerate(scope:&mut v8::PinScope,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue<v8::Array>){
    let Some((entry,location))=lookup(scope,args.holder())else{denied(scope);return};let mut keys=Vec::new();
    if !location{let snapshot=match remote_window_snapshot(scope,&entry){Ok(value)=>value,Err(_)=>{denied(scope);return}};for index in 0..snapshot.children.len(){let Some(value)=v8::String::new(scope,&index.to_string())else{return};keys.push(value.into());}}
    let names:&[&str]=if location{&["href","replace","then"]}else{&["window","self","location","close","closed","focus","blur","frames","length","top","opener","parent","postMessage","then"]};
    for name in names{let Some(value)=v8::String::new(scope,name)else{return};keys.push(value.into());}
    for symbol in [v8::Symbol::get_to_string_tag(scope),v8::Symbol::get_has_instance(scope),v8::Symbol::get_is_concat_spreadable(scope)]{keys.push(symbol.into());}
    rv.set(v8::Array::new_with_elements(scope,&keys));
}
fn delete(scope:&mut v8::PinScope,_:v8::Local<v8::Name>,_:v8::PropertyCallbackArguments,_:v8::ReturnValue<v8::Boolean>)->v8::Intercepted{denied(scope);v8::Intercepted::kYes}
fn define(scope:&mut v8::PinScope,_:v8::Local<v8::Name>,_:&v8::PropertyDescriptor,_:v8::PropertyCallbackArguments,_:v8::ReturnValue<()>)->v8::Intercepted{denied(scope);v8::Intercepted::kYes}
fn index_get(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,rv:v8::ReturnValue)->v8::Intercepted{let Some(key)=v8::String::new(scope,&index.to_string())else{return v8::Intercepted::kYes};get(scope,key.into(),args,rv)}
fn index_descriptor(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,rv:v8::ReturnValue)->v8::Intercepted{let Some(key)=v8::String::new(scope,&index.to_string())else{return v8::Intercepted::kYes};descriptor(scope,key.into(),args,rv)}
fn index_query(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,rv:v8::ReturnValue<v8::Integer>)->v8::Intercepted{let Some(key)=v8::String::new(scope,&index.to_string())else{return v8::Intercepted::kYes};query(scope,key.into(),args,rv)}
fn index_set(scope:&mut v8::PinScope,_:u32,_:v8::Local<v8::Value>,_:v8::PropertyCallbackArguments,_:v8::ReturnValue<()>)->v8::Intercepted{denied(scope);v8::Intercepted::kYes}
fn index_delete(scope:&mut v8::PinScope,_:u32,_:v8::PropertyCallbackArguments,_:v8::ReturnValue<v8::Boolean>)->v8::Intercepted{denied(scope);v8::Intercepted::kYes}
fn index_define(scope:&mut v8::PinScope,_:u32,_:&v8::PropertyDescriptor,_:v8::PropertyCallbackArguments,_:v8::ReturnValue<()>)->v8::Intercepted{denied(scope);v8::Intercepted::kYes}
fn index_enumerate(scope:&mut v8::PinScope,_:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue<v8::Array>){rv.set(v8::Array::new(scope,0));}
fn local_value<'s>(scope:&mut v8::PinScope<'s,'_>,holder:v8::Local<v8::Object>,index:u32)->Result<Option<v8::Local<'s,v8::Object>>,ScriptError>{let Some((entry,location))=lookup(scope,holder)else{return Ok(None)};if location{return Ok(None)};child(scope,&entry,&index.to_string())}
fn local_get(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue)->v8::Intercepted{match local_value(scope,args.holder(),index){Ok(Some(value))=>{rv.set(value.into());v8::Intercepted::kYes},Ok(None)=>v8::Intercepted::kNo,Err(_)=>{denied(scope);v8::Intercepted::kYes}}}
fn local_descriptor(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue)->v8::Intercepted{match local_value(scope,args.holder(),index){Ok(Some(value))=>{if let Some(value)=data(scope,value.into(),true){rv.set(value.into());}v8::Intercepted::kYes},Ok(None)=>v8::Intercepted::kNo,Err(_)=>{denied(scope);v8::Intercepted::kYes}}}
fn local_query(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue<v8::Integer>)->v8::Intercepted{match local_value(scope,args.holder(),index){Ok(Some(_))=>{rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);v8::Intercepted::kYes},Ok(None)=>v8::Intercepted::kNo,Err(_)=>{denied(scope);v8::Intercepted::kYes}}}
fn local_delete(scope:&mut v8::PinScope,index:u32,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue<v8::Boolean>)->v8::Intercepted{match local_value(scope,args.holder(),index){Ok(Some(_))=>{rv.set_bool(false);v8::Intercepted::kYes},Ok(None)=>v8::Intercepted::kNo,Err(_)=>{denied(scope);v8::Intercepted::kYes}}}
fn local_enumerate(scope:&mut v8::PinScope,args:v8::PropertyCallbackArguments,mut rv:v8::ReturnValue<v8::Array>){let Some((entry,location))=lookup(scope,args.holder())else{return};if location{return;}let snapshot=match remote_window_snapshot(scope,&entry){Ok(snapshot)=>snapshot,Err(_)=>{denied(scope);return}};let mut indices=Vec::new();for index in 0..snapshot.children.len(){let Some(value)=v8::String::new(scope,&index.to_string())else{return};indices.push(value.into());}rv.set(v8::Array::new_with_elements(scope,&indices));}
pub(super) fn configure(scope:&mut v8::PinScope,template:v8::Local<v8::ObjectTemplate>,location:bool){if !location{oryn_js::v8_context_bridge::set_window_index_handlers(scope,template,&CrossContextHandlers{indexed_getter:Some(local_get.map_fn_to()),indexed_descriptor:Some(local_descriptor.map_fn_to()),indexed_query:Some(local_query.map_fn_to()),indexed_deleter:Some(local_delete.map_fn_to()),indexed_enumerator:Some(local_enumerate.map_fn_to()),..Default::default()});}set_dynamic_remote_handlers(scope,template,access,&CrossContextHandlers{named_getter:Some(get.map_fn_to()),named_setter:Some(set.map_fn_to()),named_query:Some(query.map_fn_to()),named_descriptor:Some(descriptor.map_fn_to()),named_enumerator:Some(enumerate.map_fn_to()),named_deleter:Some(delete.map_fn_to()),named_definer:Some(define.map_fn_to()),indexed_getter:Some(index_get.map_fn_to()),indexed_setter:Some(index_set.map_fn_to()),indexed_query:Some(index_query.map_fn_to()),indexed_descriptor:Some(index_descriptor.map_fn_to()),indexed_enumerator:Some(index_enumerate.map_fn_to()),indexed_deleter:Some(index_delete.map_fn_to()),indexed_definer:Some(index_define.map_fn_to())});}

#[cfg(test)]
mod cache_namespace_tests {
    use super::*;
    #[test]
    fn exact_group_namespace_keeps_distinct_cached_callables() {
        let context=crate::Browser::in_process_with_config(Default::default()).new_context();
        let source=context.new_page();let independent=context.new_page();let target=context.new_page();
        let first_group=context.native_window_group(source.page_id(),source.page_id(),None).unwrap().0.unwrap();
        let second_group=context.native_window_group(independent.page_id(),independent.page_id(),None).unwrap().0.unwrap();
        assert_ne!(first_group,second_group);
        // Diagnostics intentionally redact tokens; the private cache must not
        // confuse two equal-looking Debug strings with equal native identities.
        assert_eq!(format!("{first_group:?}"),format!("{second_group:?}"));
        let mut host=RawV8Host::new_with_network_context_and_heap_limit(None,None,SharedContextClient::direct(context.clone(),source.page_id()),source.page_id(),256*1024*1024).unwrap();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())),1).unwrap();
        let context=host.contexts[&host.active_realm()].clone();
        v8::scope!(let handles,&mut host.isolate);
        let context=v8::Local::new(handles,&context);let scope=&mut v8::ContextScope::new(handles,context);
        let object=remote_window_object(scope,target.page_id(),oryn_js::message_wire::WindowTarget::Root,true,false).unwrap();
        let (entry,_)=lookup(scope,object).unwrap();
        // Exercise only cache identity with two real native tokens. These
        // unregistered metadata clones do not grant either token authority and
        // no callable is invoked through the synthetic entries.
        let mut first=entry.clone();first.group=Some(first_group);
        let mut second=entry;second.group=Some(second_group);
        assert!(remote_window_snapshot(scope,&first).is_err());
        assert!(remote_window_snapshot(scope,&second).is_err());
        let first_get=callable(scope,&first,false,"closed",0,false).unwrap();
        let second_get=callable(scope,&second,false,"closed",0,false).unwrap();
        let first_again=callable(scope,&first,false,"closed",0,false).unwrap();
        let second_again=callable(scope,&second,false,"closed",0,false).unwrap();
        let first_value:v8::Local<v8::Value>=first_get.into();
        let second_value:v8::Local<v8::Value>=second_get.into();
        assert!(!first_value.strict_equals(second_value));
        assert!(first_value.strict_equals(first_again.into()));
        assert!(second_value.strict_equals(second_again.into()));
    }
}
