fn font_source_input(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,value:v8::Local<v8::Value>,
)->Result<font_loading_transport::FontFaceSourceInput,V8BindingError>{
    if value.is_shared_array_buffer(){return Err(V8BindingError::Message("TypeError".into()));}
    let view=v8::Local::<v8::ArrayBufferView>::try_from(value).ok();
    let buffer=if let Some(view)=view {view.buffer(scope)}else{v8::Local::<v8::ArrayBuffer>::try_from(value).ok()};
    let Some(buffer)=buffer else {
        let value=font_owned_input(scope,state,value)?;
        return Ok(font_loading_transport::FontFaceSourceInput::Css{text:value.value,units:value._units,allocation:value._allocation});
    };
    if buffer.was_detached(){return Err(V8BindingError::Message("TypeError".into()));}
    let length=view.map_or_else(||buffer.byte_length(),|view|view.byte_length());
    let admission=state.retained_allocation.borrow().clone().ok_or(V8BindingError::Allocation("native font source admission"))?;
    let allocation=admission.reserve(length).map_err(V8BindingError::EventCallback)?;
    let mut bytes=Vec::new();
    bytes.try_reserve_exact(length).map_err(|_|V8BindingError::Allocation("native font source bytes"))?;
    if bytes.capacity()>length{return Err(V8BindingError::Allocation("native font source admission"));}
    bytes.resize(length,0);
    if let Some(view)=view {
        if view.copy_contents(&mut bytes)!=length{return Err(V8BindingError::Allocation("native font source copy"));}
    }else {
        let backing=buffer.get_backing_store();
        for (target,source) in bytes.iter_mut().zip(backing.iter()){*target=source.get();}
    }
    Ok(font_loading_transport::FontFaceSourceInput::Binary{bytes,allocation})
}
