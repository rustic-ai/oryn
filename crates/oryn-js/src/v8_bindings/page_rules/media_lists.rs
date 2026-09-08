//! Live media lists retain their native CSS rule through private traced edges.
use super::*;

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "MediaList".into(),
        member: "incompatible receiver".into(),
    }
}

fn rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if object.is_proxy() {
        return Err(invalid());
    }
    private_object(scope, object, "oryn.css.media.rule").map_err(|_| invalid())
}

pub(crate) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    rule: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Ok(list) = private_object(scope, rule, "oryn.css.media.list") {
        return Ok(list);
    }
    let identity = id(scope, rule).ok_or_else(invalid)?;
    let owner = state
        .page_rules
        .borrow()
        .get(&identity)
        .ok_or_else(invalid)?
        .owner;
    let template = state
        .instance_templates
        .borrow()
        .get("MediaList")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("MediaList"))?;
    let template = v8::Local::new(scope, template);
    let list = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("MediaList instance"))?;
    let owner = wrap_node(scope, state, owner)?;
    for (object, name, value) in [
        (list, "oryn.css.media.rule", rule),
        (list, "oryn.associated.owner", owner),
        (rule, "oryn.css.media.list", list),
    ] {
        let name =
            v8::String::new(scope, name).ok_or(V8BindingError::Allocation("MediaList edge"))?;
        let key = v8::Private::for_api(scope, Some(name));
        object.set_private(scope, key, value.into());
    }
    Ok(list)
}

pub(crate) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let rule = rule(scope, object)?;
    let identity = id(scope, rule).ok_or_else(invalid)?;
    let records = state.page_rules.borrow();
    let view = &records.get(&identity).ok_or_else(invalid)?.view;
    match member {
        "length" => {
            Ok(v8::Integer::new_from_unsigned(scope, view.media_queries.len() as u32).into())
        }
        "mediaText" | "toString" => v8_string(scope, &view.media_queries.join(", ")),
        _ => Err(invalid()),
    }
}

fn mutate(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    edit: BindingMediaListEdit<'_>,
) -> Result<(), V8BindingError> {
    let rule = rule(scope, object)?;
    let identity = id(scope, rule).ok_or_else(invalid)?;
    let (source, path) = mutation_context(scope, state, identity)?;
    let (base_url, quirks) = {
        let records = state.page_rules.borrow();
        let record = records.get(&identity).ok_or_else(invalid)?;
        (record.base_url.clone(), record.quirks)
    };
    let provider = state
        .layout_provider
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("MediaList provider"))?;
    let result = provider
        .mutate_media_list(&source, &path, edit, &base_url, quirks)
        .ok_or(V8BindingError::Allocation("MediaList mutation"))?
        .map_err(V8BindingError::EventCallback)?;
    if result.not_found {
        return Err(V8BindingError::Unsupported {
            interface: "MediaList".into(),
            member: "NotFoundError".into(),
        });
    }
    if let Some(rules) = result.rules {
        publish_rule_transaction(
            scope,
            state,
            identity,
            tree::stylesheet_owner(state, identity),
            path,
            rules,
        )?
    }
    Ok(())
}

pub(crate) fn set_text(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    input: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    rule(scope, object)?;
    let text = cssom_string(scope, input, true)?;
    mutate(scope, state, object, BindingMediaListEdit::Replace(&text))
}

pub(crate) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    rule(scope, args.this())?;
    if member == "toString" {
        return attribute(scope, state, args.this(), member);
    }
    if member == "item" {
        let index = args
            .get(0)
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as usize;
        let entries = entries(scope, args.this())?.ok_or_else(invalid)?;
        return Ok(entries
            .get(index)
            .copied()
            .unwrap_or_else(|| v8::null(scope).into()));
    }
    let text = cssom_string(scope, args.get(0), false)?;
    let edit = match member {
        "appendMedium" => BindingMediaListEdit::Append(&text),
        "deleteMedium" => BindingMediaListEdit::Delete(&text),
        _ => return Err(invalid()),
    };
    mutate(scope, state, args.this(), edit)?;
    Ok(v8::undefined(scope).into())
}

fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<Option<Vec<v8::Local<'s, v8::Value>>>, V8BindingError> {
    let Ok(rule) = rule(scope, object) else {
        return Ok(None);
    };
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let identity = id(scope, rule).ok_or_else(invalid)?;
    let queries = state
        .page_rules
        .borrow()
        .get(&identity)
        .ok_or_else(invalid)?
        .view
        .media_queries
        .clone();
    queries
        .iter()
        .map(|text| v8_string(scope, text))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
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
                throw_type_error(scope, "MediaList indexed properties are read-only");
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
                throw_type_error(scope, "MediaList indexed properties cannot be deleted");
            }
        }
    }
    v8::Intercepted::kYes
}

pub(crate) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
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
