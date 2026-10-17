//! Native typed EditContext event payloads retained by their V8 wrapper.
use super::*;
fn bad()->V8BindingError{V8BindingError::Unsupported{interface:"EditContext event".into(),member:"TypeError".into()}}
fn key<'s>(scope:&mut v8::PinScope<'s,'_>,interface:&str)->Result<v8::Local<'s,v8::Private>,V8BindingError>{
    let label=match interface{"TextUpdateEvent"=>"oryn.ec.textupdate","TextFormat"=>"oryn.ec.format","TextFormatUpdateEvent"=>"oryn.ec.formats","CharacterBoundsUpdateEvent"=>"oryn.ec.characterbounds",_=>return Err(bad())};
    let text=v8::String::new(scope,label).ok_or(V8BindingError::Allocation("EditContext payload key"))?;
    Ok(v8::Private::for_api(scope,Some(text)))
}
fn put(scope:&mut v8::PinScope,object:v8::Local<v8::Object>,name:&str,value:v8::Local<v8::Value>)->Result<(),V8BindingError>{
    let name=v8::String::new(scope,name).ok_or(V8BindingError::Allocation("EditContext payload member"))?;
    if object.create_data_property(scope,name.into(),value)!=Some(true){return Err(V8BindingError::JavaScriptException)}Ok(())
}
fn format_brand(scope:&mut v8::PinScope,object:v8::Local<v8::Object>)->Result<(),V8BindingError>{
    if object.is_proxy(){return Err(bad())}
    let brand=key(scope,"TextFormat")?;
    let payload=object.get_private(scope,brand).ok_or(V8BindingError::JavaScriptException)?;
    // This private key cannot be authored or inherited through a prototype;
    // require the constructor-published payload, not a public prototype brand.
    v8::Local::<v8::Object>::try_from(payload).map_err(|_|bad())?;
    Ok(())
}
/// Convert flattened dictionaries once, in lexical order. EventInit's
/// bubbles/cancelable/composed are converted by the generic event constructor.
/// Invoke this AFTER composed for all event types; TextFormat uses its sole init.
pub(super) fn initialize<'s>(scope:&mut v8::PinScope<'s,'_>,object:v8::Local<'s,v8::Object>,interface:&str,init:Option<v8::Local<'s,v8::Object>>)->Result<(),V8BindingError>{
    let payload=v8::Object::new(scope);
    let null = v8::null(scope);
    if payload.set_prototype(scope,null.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
    let members:&[&str]=match interface{
        "TextUpdateEvent"=>&["selectionEnd","selectionStart","text","updateRangeEnd","updateRangeStart"],
        "TextFormat"=>&["rangeEnd","rangeStart","underlineStyle","underlineThickness"],
        "CharacterBoundsUpdateEvent"=>&["rangeEnd","rangeStart"],
        "TextFormatUpdateEvent"=>&["textFormats"],_=>return Err(bad())};
    for &name in members{
        let value=event_init_member(scope,init,name).ok_or(V8BindingError::JavaScriptException)?;
        let converted:v8::Local<v8::Value>=if name=="textFormats"{
            let array=v8::Array::new(scope,0);
            if !value.is_undefined(){
                let source=v8::Local::<v8::Object>::try_from(value).map_err(|_|bad())?;
                let symbol=v8::Symbol::get_iterator(scope);
                let method=source.get(scope,symbol.into()).ok_or(V8BindingError::JavaScriptException)?;
                let method=v8::Local::<v8::Function>::try_from(method).map_err(|_|bad())?;
                let mut index=0u32;
                iterable_conversion::consume(scope,source,method,|scope,value|{
                    if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
                    let object=v8::Local::<v8::Object>::try_from(value).map_err(|_|bad())?;
                    format_brand(scope,object)?;
                    if array.set_index(scope,index,object.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
                    index=index.checked_add(1).ok_or(V8BindingError::Allocation("TextFormat sequence length"))?;Ok(())
                })?;
            }array.into()
        }else if matches!(name,"text"|"underlineStyle"|"underlineThickness"){
            let text=if value.is_undefined(){v8::String::new(scope,if name=="text"{""}else{"none"}).ok_or(V8BindingError::Allocation("EditContext default"))?}else{value.to_string(scope).ok_or(V8BindingError::JavaScriptException)?};
            if name!="text"{
                let valid:&[&str]=if name=="underlineStyle"{&["none","solid","dotted","dashed","wavy"]}else{&["none","thin","thick"]};
                if !valid.iter().any(|v|v8::String::new(scope,v).is_some_and(|candidate|candidate.strict_equals(text.into()))){return Err(bad())}
            }text.into()
        }else{let number=if value.is_undefined(){0}else{value.uint32_value(scope).ok_or(V8BindingError::JavaScriptException)?};v8::Integer::new_from_unsigned(scope,number).into()};
        put(scope,payload,name,converted)?;
    }
    let brand=key(scope,interface)?;
    if object.set_private(scope,brand,payload.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}Ok(())
}
pub(super) fn get<'s>(scope:&mut v8::PinScope<'s,'_>,object:v8::Local<'s,v8::Object>,interface:&str,member:&str)->Result<v8::Local<'s,v8::Value>,V8BindingError>{
    if object.is_proxy(){return Err(bad())}
    let brand=key(scope,interface)?;
    let value=object.get_private(scope,brand).ok_or(V8BindingError::JavaScriptException)?;
    let payload=v8::Local::<v8::Object>::try_from(value).map_err(|_|bad())?;
    let name=v8::String::new(scope,member).ok_or(V8BindingError::Allocation("EditContext getter key"))?;
    payload.get(scope,name.into()).ok_or(V8BindingError::JavaScriptException)
}
/// getTextFormats returns a new sequence array, preserving original element
/// identity and wrapper-reachable GC ownership without persistent native Globals.
pub(super) fn formats<'s>(scope:&mut v8::PinScope<'s,'_>,object:v8::Local<'s,v8::Object>)->Result<v8::Local<'s,v8::Value>,V8BindingError>{
    let value=get(scope,object,"TextFormatUpdateEvent","textFormats")?;
    let source=v8::Local::<v8::Array>::try_from(value).map_err(|_|bad())?;
    let result=v8::Array::new(scope,0);
    for index in 0..source.length(){if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}let value=source.get_index(scope,index).ok_or(V8BindingError::JavaScriptException)?;if result.set_index(scope,index,value)!=Some(true){return Err(V8BindingError::JavaScriptException)}}Ok(result.into())
}

pub(super) fn publish_native_formats(scope:&mut v8::PinScope,owner:&V8BindingState,event:v8::Local<v8::Object>,origin:u32,formats:&[crate::composition::NativeTextFormat])->Result<(),V8BindingError>{
    use crate::composition::{NativeUnderlineStyle as Style,NativeUnderlineThickness as Thickness};
    let array=v8::Array::new(scope,0);
    for (index,format) in formats.iter().enumerate(){
        if !crate::v8_termination::checkpoint(scope){return Err(V8BindingError::JavaScriptException)}
        let start=origin.checked_add(format.range.start).ok_or(V8BindingError::Allocation("native format range"))?;
        let end=start.checked_add(format.range.length).ok_or(V8BindingError::Allocation("native format range"))?;
        let object=new_stateful_object(scope,owner,"TextFormat",0)?;
        let payload=v8::Object::new(scope);let null=v8::null(scope);
        if payload.set_prototype(scope,null.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
        let style=match format.style{Style::None=>"none",Style::Solid=>"solid",Style::Dotted=>"dotted",Style::Dashed=>"dashed"};
        let thickness=match format.thickness{Thickness::None=>"none",Thickness::Thin=>"thin",Thickness::Thick=>"thick"};
        let start=v8::Integer::new_from_unsigned(scope,start);
        put(scope,payload,"rangeStart",start.into())?;
        let end=v8::Integer::new_from_unsigned(scope,end);
        put(scope,payload,"rangeEnd",end.into())?;
        let style=v8::String::new(scope,style).ok_or(V8BindingError::Allocation("native underline style"))?;
        put(scope,payload,"underlineStyle",style.into())?;
        let thickness=v8::String::new(scope,thickness).ok_or(V8BindingError::Allocation("native underline thickness"))?;
        put(scope,payload,"underlineThickness",thickness.into())?;
        let brand=key(scope,"TextFormat")?;
        if object.set_private(scope,brand,payload.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
        let index=u32::try_from(index).map_err(|_|V8BindingError::Allocation("native formats length"))?;
        if array.set_index(scope,index,object.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
    }
    let payload=v8::Object::new(scope);let null=v8::null(scope);
    if payload.set_prototype(scope,null.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}
    put(scope,payload,"textFormats",array.into())?;
    let brand=key(scope,"TextFormatUpdateEvent")?;
    if event.set_private(scope,brand,payload.into())!=Some(true){return Err(V8BindingError::JavaScriptException)}Ok(())
}
