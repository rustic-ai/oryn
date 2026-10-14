// The final neutral rule trait supplies layer_name_count()/layer_name_at(index)
// from the original CssRule::LayerStatement native names. No text splitting.
fn native_layer_name_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &Rc<dyn BindingNativeRule>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let count = handle
        .layer_name_count()
        .map_err(|error| native_css_read_error("CSSLayerStatementRule", error))?
        .ok_or_else(|| V8BindingError::Unsupported {
            interface: "CSSLayerStatementRule".into(),
            member: "incompatible receiver".into(),
        })?;
    let count = i32::try_from(count)
        .map_err(|_| V8BindingError::Allocation("CSS layer names length overflow"))?;
    let result = v8::Array::new(scope, count);
    for index in 0..count {
        let name = handle
            .layer_name_at(index as usize)
            .map_err(|error| native_css_read_error("CSSLayerStatementRule", error))?
            .ok_or(V8BindingError::Allocation(
                "native CSS layer name disappeared",
            ))?;
        let value = v8_string(scope, &name)?;
        if result.set_index(scope, index as u32, value) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    if result.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(result.into())
}
