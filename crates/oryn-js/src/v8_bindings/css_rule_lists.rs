//! Live, read-only CSS rule collections. Their backing arrays are private and
//! are mutated only by the native stylesheet binding code.
use super::*;

fn private<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    if object.is_proxy() {
        return None;
    }
    let name = v8::String::new(scope, name)?;
    let key = v8::Private::for_api(scope, Some(name));
    object.get_private(scope, key)
}

fn set_private(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, name)
        .ok_or(V8BindingError::Allocation("CSSRuleList private key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    object.set_private(scope, key, value);
    Ok(())
}

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "CSSRuleList".into(),
        member: "incompatible receiver".into(),
    }
}

fn sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    private(scope, object, "oryn.css.rule.list.sheet")
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
}

fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<Option<Vec<v8::Local<'s, v8::Value>>>, V8BindingError> {
    let Some(sheet) = sheet(scope, object) else {
        return Ok(None);
    };
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let owner = resolve_receiver(scope, &state, sheet, "CSSStyleSheet")?;
    // Refresh only this stylesheet identity. Replacing the owner element's
    // text creates a different sheet and must not change this retained list.
    // Refresh reads the private backing array, never this indexed wrapper.
    get_attribute_value(scope, &state, owner, "CSSStyleSheet", "cssRules")?;
    let array = private(scope, object, "oryn.css.rule.list.array")
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
        .ok_or_else(invalid)?;
    let mut values = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        values.push(
            array
                .get_index(scope, index)
                .ok_or(V8BindingError::JavaScriptException)?,
        );
    }
    Ok(Some(values))
}

pub(super) fn length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let length = entries(scope, object)?.ok_or_else(invalid)?.len();
    Ok(v8::Integer::new_from_unsigned(scope, length as u32).into())
}

pub(super) fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    input: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    sheet(scope, object).ok_or_else(invalid)?;
    let index = input
        .uint32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)? as usize;
    let entries = entries(scope, object)?.ok_or_else(invalid)?;
    Ok(entries
        .get(index)
        .copied()
        .unwrap_or_else(|| v8::null(scope).into()))
}

pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    owner: NodeId,
    array: v8::Local<v8::Array>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let sheet = associated_wrapper(scope, state, owner, "CSSStyleSheet")?;
    let sheet = v8::Local::<v8::Object>::try_from(sheet).map_err(|_| invalid())?;
    if let Some(list) = private(scope, sheet, "oryn.css.rule.list")
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
    {
        return Ok(list);
    }
    let template = state
        .instance_templates
        .borrow()
        .get("CSSRuleList")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("CSSRuleList"))?;
    let template = v8::Local::new(scope, template);
    let list = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("CSSRuleList instance"))?;
    set_private(scope, list, "oryn.css.rule.list.sheet", sheet.into())?;
    set_private(scope, list, "oryn.css.rule.list.array", array.into())?;
    let node = wrap_node(scope, state, owner)?;
    set_private(scope, list, "oryn.associated.owner", node.into())?;
    set_private(scope, sheet, "oryn.css.rule.list", list.into())?;
    Ok(list)
}

fn getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(Some(names)) if (index as usize) < names.len() => {
            rv.set(names[index as usize]);
            v8::Intercepted::kYes
        }
        Ok(_) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(Some(names)) if (index as usize) < names.len() => {
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(_) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn enumerator(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match entries(scope, args.holder()) {
        Ok(Some(names)) => {
            let indices = (0..names.len())
                .map(|i| v8::Integer::new_from_unsigned(scope, i as u32).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &indices));
        }
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn reject_write(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(None) => return v8::Intercepted::kNo,
        Err(error) => throw_binding_error(scope, &error),
        Ok(Some(_)) => {
            rv.set_bool(false);
            if args.should_throw_on_error() {
                throw_type_error(scope, "CSSRuleList indexed properties are read-only");
            }
        }
    }
    v8::Intercepted::kYes
}
fn setter(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject_write(scope, args, rv)
}
fn definer(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject_write(scope, args, rv)
}
fn deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match entries(scope, args.holder()) {
        Ok(None) => return v8::Intercepted::kNo,
        Err(error) => throw_binding_error(scope, &error),
        Ok(Some(names)) => {
            let allowed = index as usize >= names.len();
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(scope, "CSSRuleList indexed properties cannot be deleted");
            }
        }
    }
    v8::Intercepted::kYes
}

pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(getter)
            .query(query)
            .enumerator(enumerator)
            .setter(setter)
            .definer(definer)
            .deleter(deleter),
    );
}
