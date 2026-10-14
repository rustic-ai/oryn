// Typed v25 native sheet state trait. This owner is either a real native sheet
// wrapper or a MediaList traced to that wrapper; no synthetic HTMLElement.

fn native_sheet_set_disabled(
    scope: &mut v8::PinScope, state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>, value: bool,
) -> Option<Result<(), V8BindingError>> {
    let handle = native_sheet_handle(scope, state, object)?;
    Some(handle.set_disabled(value).map(|_| ())
        .map_err(|error| native_css_read_error("CSSStyleSheet", error)))
}

fn wrap_native_sheet_media<'s>(
    scope: &mut v8::PinScope<'s, '_>, state: &Rc<V8BindingState>,
    sheet: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Ok(list) = private_object(scope, sheet, "oryn.native.css.sheet.media") {
        return Ok(list);
    }
    let template = state.instance_templates.borrow().get("MediaList")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("MediaList"))?;
    let list = v8::Local::new(scope, template).new_instance(scope)
        .ok_or(V8BindingError::Allocation("native sheet MediaList"))?;
    bind_native_css_owner(scope, state, list)?;
    checked_native_css_edge(scope, list, "oryn.native.css.media.sheet", sheet.into())?;
    checked_native_css_edge(scope, sheet, "oryn.native.css.sheet.media", list.into())?;
    Ok(list)
}
