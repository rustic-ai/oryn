// Preserve the WebIDL DOMString code units. The scalar value is the native
// CSS parser input: CSS Syntax preprocessing replaces surrogate code points.
// This is not an implicit USVString conversion in the generated binding.
struct FontOwnedInput { value:String, _units:Vec<u16>, _allocation:Option<std::sync::Arc<dyn Send+Sync>> }
fn font_owned_input(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,value:v8::Local<v8::Value>)
    ->Result<FontOwnedInput,V8BindingError>{
    let expected_owner=owner(state);
    let value=value.to_string(scope).ok_or(V8BindingError::JavaScriptException)?;
    if state.execution_tasks_stopped()||owner(state)!=expected_owner{return Err(V8BindingError::Message("InvalidStateError".into()));}
    let count=value.length();
    let bytes=count.checked_mul(3).ok_or(V8BindingError::Allocation("font input scalar capacity"))?;
    let allocation=reserve_font_container(scope,state,count.checked_mul(std::mem::size_of::<u16>()).and_then(|n|n.checked_add(bytes))
        .ok_or(V8BindingError::Allocation("font input capacity overflow"))?)?;
    let mut units=Vec::new();units.try_reserve_exact(count).map_err(|_|V8BindingError::Allocation("font input units"))?;
    if units.capacity()>count{return Err(V8BindingError::Allocation("font input unit admission"));}units.resize(count,0);
    // Pinned write_v2 returns (), writing min(current length, buffer length).
    // V8 strings are immutable; checking both sides proves the exact count.
    if value.length()!=count||units.len()!=count{return Err(V8BindingError::Allocation("font input unit count"));}
    value.write_v2(scope,0,&mut units,v8::WriteFlags::default());
    if value.length()!=count{return Err(V8BindingError::Allocation("font input unit count changed"));}
    let mut value=String::new();value.try_reserve_exact(bytes).map_err(|_|V8BindingError::Allocation("font scalar input"))?;
    if value.capacity()>bytes{return Err(V8BindingError::Allocation("font scalar admission"));}
    for (index,character) in char::decode_utf16(units.iter().copied()).enumerate(){
        // Cancellation polling granularity, not an input or execution limit.
        if index%256==0&&(!crate::v8_termination::checkpoint(scope)||state.execution_tasks_stopped()){
            return Err(V8BindingError::JavaScriptException);
        }
        value.push(character.unwrap_or('\u{fffd}'));
    }
    if !crate::v8_termination::checkpoint(scope)||state.execution_tasks_stopped()||owner(state)!=expected_owner{
        return Err(V8BindingError::Message("InvalidStateError".into()));
    }
    Ok(FontOwnedInput{value,_units:units,_allocation:allocation})
}
