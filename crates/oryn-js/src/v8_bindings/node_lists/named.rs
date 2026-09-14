//! HTMLCollection named properties over the shared live collection owner.
use super::*;

fn supported_index(name: &str) -> bool {
    name.parse::<u32>()
        .is_ok_and(|index| index != u32::MAX && index.to_string() == name)
}

fn names(state: &V8BindingState, id: u64) -> Result<Vec<(String, usize)>, V8BindingError> {
    let nodes = contents(state, id)?;
    let all = state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .is_some_and(|list| matches!(list.contents, Contents::All(_)));
    let dom = state.dom.borrow();
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for (index, node) in nodes.into_iter().enumerate() {
        let record = dom.arena().get(node)?;
        if let NodeKind::Attribute {
            data,
            owner: Some(owner),
        } = &record.kind
        {
            let name = &data.name;
            if normalized_attribute_name(state, *owner, name.clone())? == *name
                && seen.insert(name.clone())
            {
                names.push((name.clone(), index));
            }
            continue;
        }
        let html = matches!(&record.kind, NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml");
        for attribute in ["id", "name"] {
            if attribute == "name" && !html {
                continue;
            }
            if let Some(name) = record.attributes.get_null(attribute)
                && !name.is_empty()
                && (!all || all_named_match(&dom, node, name))
                && seen.insert(name.clone())
            {
                names.push((name.clone(), index));
            }
        }
    }
    Ok(names)
}

fn visible(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    key: v8::Local<v8::Name>,
) -> bool {
    let mut current = Some(object);
    while let Some(object) = current {
        if object.has_real_named_property(scope, key) == Some(true) {
            return false;
        }
        current = object
            .get_prototype(scope)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    }
    true
}

fn resolve(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    key: v8::Local<v8::Name>,
    check_visibility: bool,
) -> Result<Option<(Rc<V8BindingState>, u64, usize)>, V8BindingError> {
    let Ok(name) = v8::Local::<v8::String>::try_from(key) else {
        return Ok(None);
    };
    let name = rust_string_preserving_utf16(scope, name)?;
    if name.is_empty()
        || supported_index(&name)
        || (check_visibility && !visible(scope, object, key))
    {
        return Ok(None);
    }
    let (state, id) = owner(scope, object)?;
    let index = names(&state, id)?
        .into_iter()
        .find_map(|(candidate, index)| (candidate == name).then_some(index));
    Ok(index.map(|index| (state, id, index)))
}

pub(super) fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = identity_as(scope, state, object, "HTMLCollection")?;
    let Some(name) = value.to_string(scope) else {
        return Ok(v8::undefined(scope).into());
    };
    let name = rust_string_preserving_utf16(scope, name)?;
    let index = names(state, id)?
        .into_iter()
        .find_map(|(candidate, index)| (candidate == name).then_some(index));
    match index {
        Some(index) => Ok(node_at(scope, state, id, index as u32)?
            .map_or_else(|| v8::null(scope).into(), Into::into)),
        None => Ok(v8::null(scope).into()),
    }
}

fn value_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    index: usize,
    key: v8::Local<v8::Name>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let form = {
        let lists = state.node_lists.lists.borrow();
        lists.get(&id).and_then(|list| match list.contents {
            Contents::Form(form) => Some(form),
            _ => None,
        })
    };
    if let Some(form) = form {
        let key = v8::Local::<v8::String>::try_from(key).map_err(|_| invalid_receiver())?;
        let name = rust_string_preserving_utf16(scope, key)?;
        return form_named_value(scope, state, form, name).map(Some);
    }
    let document = {
        let lists = state.node_lists.lists.borrow();
        lists.get(&id).and_then(|list| match list.contents {
            Contents::All(root) => Some(root),
            _ => None,
        })
    };
    if let Some(document) = document {
        let key = v8::Local::<v8::String>::try_from(key).map_err(|_| invalid_receiver())?;
        let name = rust_string_preserving_utf16(scope, key)?;
        let global = state
            .global_object
            .to_local(scope)
            .ok_or_else(invalid_receiver)?;
        let context = global
            .get_creation_context(scope)
            .ok_or_else(invalid_receiver)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        return all::named_value(scope, state, document, name).map(Some);
    }
    Ok(node_at(scope, state, id, index as u32)?.map(Into::into))
}

fn getter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key, true).and_then(|found| {
        found
            .map(|(state, id, index)| value_at(scope, &state, id, index, key))
            .transpose()
            .map(Option::flatten)
    }) {
        Ok(Some(node)) => {
            rv.set(node);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn query(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key, true) {
        Ok(Some(_)) => {
            // V8's assignment path consults query attributes before trying a
            // receiver's own definition. WebIDL [[Set]] ignores virtual named
            // properties here; the explicit descriptor hook reports readonly
            // to reflection, and the definer protects the collection itself.
            rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn descriptor(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key, true).and_then(|found| {
        found
            .map(|(state, id, index)| value_at(scope, &state, id, index, key))
            .transpose()
            .map(Option::flatten)
    }) {
        Ok(Some(node)) => {
            let descriptor = v8::Object::new(scope);
            for (name, value) in [
                ("value", node),
                ("writable", v8::Boolean::new(scope, false).into()),
                ("enumerable", v8::Boolean::new(scope, false).into()),
                ("configurable", v8::Boolean::new(scope, true).into()),
            ] {
                let key = v8::String::new(scope, name).unwrap();
                descriptor.create_data_property(scope, key.into(), value);
            }
            rv.set(descriptor.into());
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
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
    let result = owner(scope, args.holder()).and_then(|(state, id)| names(&state, id));
    match result {
        Ok(names) => {
            let mut values = Vec::new();
            for (name, _) in names {
                if supported_index(&name) {
                    continue;
                }
                let key = match v8_string(scope, &name) {
                    Ok(key) => v8::Local::<v8::String>::try_from(key).unwrap(),
                    Err(error) => {
                        throw_binding_error(scope, &error);
                        return;
                    }
                };
                if visible(scope, args.holder(), key.into()) {
                    values.push(key.into());
                }
            }
            rv.set(v8::Array::new_with_elements(scope, &values));
        }
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn reject(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "HTMLCollection named properties are read-only");
    }
    v8::Intercepted::kYes
}

fn definer(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if args.holder().has_real_named_property(scope, key) == Some(true) {
        return v8::Intercepted::kNo;
    }
    match resolve(scope, args.holder(), key, false) {
        Ok(Some(_)) => reject(scope, args, rv),
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn deleter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match resolve(scope, args.holder(), key, true) {
        Ok(Some(_)) => {
            rv.set_bool(false);
            if args.should_throw_on_error() {
                throw_type_error(scope, "HTMLCollection named properties cannot be deleted");
            }
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(getter)
            .query(query)
            .descriptor(descriptor)
            .enumerator(enumerator)
            .definer(definer)
            .deleter(deleter),
    );
}
