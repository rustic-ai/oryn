//! CSSOM sheet adoption and its WebIDL observable array. The proxy owns the
//! JavaScript list; the layout snapshot reads it without exposing its target.
use super::*;

fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "CSSStyleSheet".into(),
        member: name.into(),
    }
}

pub(super) fn capture_reflect_set(
    scope: &mut v8::PinScope,
) -> Result<v8::Global<v8::Function>, V8BindingError> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "Reflect").unwrap();
    let reflect = global
        .get(scope, key.into())
        .and_then(|v| v.to_object(scope))
        .ok_or(error("TypeError"))?;
    let key = v8::String::new(scope, "set").unwrap();
    let function = reflect
        .get(scope, key.into())
        .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
        .ok_or(error("TypeError"))?;
    Ok(v8::Global::new(scope, function))
}

fn index(scope: &mut v8::PinScope, key: v8::Local<v8::Value>) -> Option<u32> {
    if !key.is_string() {
        return None;
    }
    let text = key.to_rust_string_lossy(scope);
    let n = text.parse::<u32>().ok()?;
    (n != u32::MAX && n.to_string() == text).then_some(n)
}

fn sheet_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| error("TypeError"))?;
    let key = v8::String::new(scope, "oryn.cssStyleSheet.owner").unwrap();
    let key = v8::Private::for_api(scope, Some(key));
    let owner = object.get_private(scope, key).ok_or(error("TypeError"))?;
    if owner.is_undefined() {
        return Err(error("TypeError"));
    }
    Ok(owner)
}

pub(super) fn mark_sheet(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    constructed: bool,
) -> Result<(), V8BindingError> {
    let owner = if constructed {
        wrap_node(scope, state, state.document)?.into()
    } else {
        v8::null(scope).into()
    };
    let key = v8::String::new(scope, "oryn.cssStyleSheet.owner").unwrap();
    let key = v8::Private::for_api(scope, Some(key));
    object.set_private(scope, key, owner);
    Ok(())
}

fn validate(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    root: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<NodeId, V8BindingError> {
    let owner = sheet_owner(scope, value)?;
    let owner = v8::Local::<v8::Object>::try_from(owner).map_err(|_| error("NotAllowedError"))?;
    let (owner_node, realm, _) =
        embedded_node_identity(scope, owner).ok_or(error("NotAllowedError"))?;
    let document = if matches!(
        state.dom.borrow().arena().get(root)?.kind,
        NodeKind::Document
    ) {
        root
    } else {
        state
            .node_documents
            .borrow()
            .get(&root)
            .copied()
            .unwrap_or(state.document)
    };
    if realm != state.realm || owner_node != document {
        return Err(error("NotAllowedError"));
    }
    resolve_receiver(
        scope,
        state,
        value.try_into().map_err(|_| error("TypeError"))?,
        "CSSStyleSheet",
    )
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<v8::Local<'s, v8::Proxy>, V8BindingError> {
    if let Some(proxy) = state
        .adopted_stylesheets
        .borrow()
        .get(&root)
        .and_then(|weak| weak.to_local(scope))
    {
        return Ok(proxy);
    }
    let target = v8::Array::new(scope, 0);
    let sheets = state
        .dom
        .borrow()
        .arena()
        .get(root)?
        .adopted_stylesheets
        .clone();
    for (index, sheet) in sheets.into_iter().enumerate() {
        let value = associated_wrapper(scope, state, sheet, "CSSStyleSheet")?;
        let key = v8::String::new(scope, &index.to_string()).unwrap();
        target.create_data_property(scope, key.into(), value);
    }
    let handler = v8::Object::new(scope);
    let null = v8::null(scope);
    handler.set_prototype(scope, null.into());
    for (name, mode) in [
        ("set", 0),
        ("defineProperty", 1),
        ("deleteProperty", 2),
        ("preventExtensions", 3),
        ("get", 4),
        ("has", 5),
    ] {
        let data = v8::Array::new_with_elements(
            scope,
            &[
                v8::BigInt::new_from_u64(scope, node_key(root)).into(),
                v8::Integer::new(scope, mode).into(),
            ],
        );
        let callback = v8::Function::builder(trap)
            .data(data.into())
            .build(scope)
            .ok_or(error("allocation"))?;
        let name = v8::String::new(scope, name).ok_or(error("allocation"))?;
        handler.create_data_property(scope, name.into(), callback.into());
    }
    let proxy = v8::Proxy::new(scope, target.into(), handler).ok_or(error("allocation"))?;
    // A GC-traced owner/list cycle keeps detached roots and adopted sheets alive
    // when referenced, while permitting collection of unreachable shadow roots.
    let owner = wrap_node(scope, state, root)?;
    let key = v8::String::new(scope, "oryn.adoptedStyleSheets").unwrap();
    let key = v8::Private::for_api(scope, Some(key));
    owner.set_private(scope, key, proxy.into());
    let key = v8::String::new(scope, "oryn.adoptedStyleSheets.owner").unwrap();
    let key = v8::Private::for_api(scope, Some(key));
    target.set_private(scope, key, owner.into());
    state
        .adopted_stylesheets
        .borrow_mut()
        .insert(root, v8::Weak::new(scope, proxy));
    Ok(proxy)
}

fn sync(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    root: NodeId,
    target: v8::Local<v8::Array>,
) -> Result<(), V8BindingError> {
    let mut sheets = Vec::new();
    for index in 0..target.length() {
        let value = target.get_index(scope, index).ok_or(error("TypeError"))?;
        sheets.push(validate(scope, state, root, value)?);
    }
    state
        .dom
        .borrow_mut()
        .set_adopted_stylesheets(root, sheets)?;
    Ok(())
}

fn set_value(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    root: NodeId,
    target: v8::Local<v8::Array>,
    key: v8::Local<v8::Value>,
    value: v8::Local<v8::Value>,
) -> Result<bool, V8BindingError> {
    if let Some(index) = index(scope, key) {
        if index > target.length() {
            return Ok(false);
        }
        validate(scope, state, root, value)?;
        return Ok(target
            .create_data_property(
                scope,
                key.try_into().map_err(|_| error("TypeError"))?,
                value,
            )
            .unwrap_or(false));
    }
    if key.is_string() && key.to_rust_string_lossy(scope) == "length" {
        let uint = value.uint32_value(scope).ok_or(error("TypeError"))?;
        let number = value.number_value(scope).ok_or(error("TypeError"))?;
        if f64::from(uint) != number {
            return Err(error("RangeError"));
        }
        if uint > target.length() {
            return Ok(false);
        }
        return Ok(target
            .set(
                scope,
                key,
                v8::Integer::new_from_unsigned(scope, uint).into(),
            )
            .unwrap_or(false));
    }
    unreachable!("only indexed and length writes use set_value")
}

fn trap(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(error("TypeError"))?;
        let data = v8::Local::<v8::Array>::try_from(args.data()).map_err(|_| error("TypeError"))?;
        let packed = exact_u64(data.get_index(scope, 0).ok_or(error("TypeError"))?)
            .ok_or(error("TypeError"))?;
        let root = NodeId {
            slot: packed as u32,
            generation: (packed >> 32) as u32,
        };
        let mode = data
            .get_index(scope, 1)
            .and_then(|v| v.int32_value(scope))
            .unwrap_or(-1);
        let target =
            v8::Local::<v8::Array>::try_from(args.get(0)).map_err(|_| error("TypeError"))?;
        let key = args.get(1);
        let idx = index(scope, key);
        let length = key.is_string() && key.to_rust_string_lossy(scope) == "length";
        let success = match mode {
            0 if idx.is_some() || length => {
                set_value(scope, &state, root, target, key, args.get(2))?
            }
            0 => {
                let receiver = v8::Local::<v8::Object>::try_from(args.get(3))
                    .map_err(|_| error("TypeError"))?;
                // Preserve receiver/proxy semantics without exposing the target
                // to inherited setters. Capture the intrinsic before page code runs.
                let set = v8::Local::new(scope, &state.intrinsic_reflect_set);
                let undefined = v8::undefined(scope);
                set.call(
                    scope,
                    undefined.into(),
                    &[target.into(), key, args.get(2), receiver.into()],
                )
                .is_some_and(|value| value.boolean_value(scope))
            }
            1 => {
                let desc = args.get(2).to_object(scope).ok_or(error("TypeError"))?;
                let mut fields = BTreeMap::new();
                for name in [
                    "value",
                    "get",
                    "set",
                    "writable",
                    "enumerable",
                    "configurable",
                ] {
                    let key = v8::String::new(scope, name).unwrap();
                    if desc.has_own_property(scope, key.into()) == Some(true) {
                        fields.insert(name, desc.get(scope, key.into()).ok_or(error("TypeError"))?);
                    }
                }
                if idx.is_some() || length {
                    let wrong = fields.contains_key("get")
                        || fields.contains_key("set")
                        || fields
                            .get("writable")
                            .is_some_and(|v| !v.boolean_value(scope))
                        || ["configurable", "enumerable"].iter().any(|name| {
                            fields
                                .get(name)
                                .is_some_and(|v| v.boolean_value(scope) == length)
                        });
                    if wrong {
                        false
                    } else if let Some(value) = fields.get("value") {
                        set_value(scope, &state, root, target, key, *value)?
                    } else {
                        true
                    }
                } else {
                    let undefined = v8::undefined(scope).into();
                    let mut descriptor = if fields.contains_key("get") || fields.contains_key("set")
                    {
                        v8::PropertyDescriptor::new_from_get_set(
                            *fields.get("get").unwrap_or(&undefined),
                            *fields.get("set").unwrap_or(&undefined),
                        )
                    } else if let Some(writable) = fields.get("writable") {
                        v8::PropertyDescriptor::new_from_value_writable(
                            *fields.get("value").unwrap_or(&undefined),
                            writable.boolean_value(scope),
                        )
                    } else if let Some(value) = fields.get("value") {
                        v8::PropertyDescriptor::new_from_value(*value)
                    } else {
                        v8::PropertyDescriptor::new()
                    };
                    if let Some(value) = fields.get("configurable") {
                        descriptor.set_configurable(value.boolean_value(scope));
                    }
                    if let Some(value) = fields.get("enumerable") {
                        descriptor.set_enumerable(value.boolean_value(scope));
                    }
                    target
                        .define_property(
                            scope,
                            key.try_into().map_err(|_| error("TypeError"))?,
                            &descriptor,
                        )
                        .unwrap_or(false)
                }
            }
            2 if length => false,
            2 if idx.is_some() => {
                let idx = idx.unwrap();
                if idx.checked_add(1) != Some(target.length()) {
                    false
                } else {
                    let key = v8::String::new(scope, "length").unwrap();
                    let value = v8::Integer::new_from_unsigned(scope, idx);
                    target.set(scope, key.into(), value.into()).unwrap_or(false)
                }
            }
            2 => target.delete(scope, key).unwrap_or(false),
            3 => false,
            4 => {
                if idx.is_some_and(|i| i >= target.length()) {
                    return Ok(v8::undefined(scope).into());
                }
                let receiver = v8::Local::<v8::Object>::try_from(args.get(2))
                    .map_err(|_| error("TypeError"))?;
                return target
                    .get_with_receiver(scope, key, receiver)
                    .ok_or(error("TypeError"));
            }
            5 if idx.is_some() => idx.unwrap() < target.length(),
            5 => target.has(scope, key).unwrap_or(false),
            _ => false,
        };
        if success && matches!(mode, 0..=2) {
            sync(scope, &state, root, target)?;
        }
        Ok(v8::Boolean::new(scope, success).into())
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub(super) fn assign(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| error("TypeError"))?;
    let key = v8::Symbol::get_iterator(scope);
    let Some(method) = object.get(scope, key.into()) else {
        return Ok(());
    };
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| error("TypeError"))?;
    let Some(iterator) = method.call(scope, object.into(), &[]) else {
        return Ok(());
    };
    let iterator = v8::Local::<v8::Object>::try_from(iterator).map_err(|_| error("TypeError"))?;
    let key = v8::String::new(scope, "next").unwrap();
    let Some(next) = iterator.get(scope, key.into()) else {
        return Ok(());
    };
    let next = v8::Local::<v8::Function>::try_from(next).map_err(|_| error("TypeError"))?;
    let mut values = Vec::new();
    loop {
        let Some(entry) = next.call(scope, iterator.into(), &[]) else {
            return Ok(());
        };
        let entry = v8::Local::<v8::Object>::try_from(entry).map_err(|_| error("TypeError"))?;
        let key = v8::String::new(scope, "done").unwrap();
        let Some(done) = entry.get(scope, key.into()) else {
            return Ok(());
        };
        if done.boolean_value(scope) {
            break;
        }
        let key = v8::String::new(scope, "value").unwrap();
        let Some(value) = entry.get(scope, key.into()) else {
            return Ok(());
        };
        if let Err(error) = sheet_owner(scope, value) {
            let key = v8::String::new(scope, "return").unwrap();
            if let Some(close) = iterator
                .get(scope, key.into())
                .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
            {
                close.call(scope, iterator.into(), &[]);
            }
            return Err(error);
        }
        values.push(value);
    }
    let proxy = get(scope, state, root)?;
    let target = v8::Local::<v8::Array>::try_from(proxy.get_target(scope))
        .map_err(|_| error("TypeError"))?;
    let key = v8::String::new(scope, "length").unwrap();
    let zero = v8::Integer::new(scope, 0);
    target.set(scope, key.into(), zero.into());
    sync(scope, state, root, target)?;
    for (index, value) in values.into_iter().enumerate() {
        validate(scope, state, root, value)?;
        let key = v8::String::new(scope, &index.to_string()).unwrap();
        target.create_data_property(scope, key.into(), value);
        sync(scope, state, root, target)?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn styles(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<BTreeMap<NodeId, Vec<String>>, V8BindingError> {
    let mut result = BTreeMap::new();
    for (root, proxy) in state.adopted_stylesheets.borrow().iter() {
        let Some(proxy) = proxy.to_local(scope) else {
            continue;
        };
        let target = v8::Local::<v8::Array>::try_from(proxy.get_target(scope))
            .map_err(|_| error("TypeError"))?;
        let mut sheets = Vec::new();
        for i in 0..target.length() {
            let value = target.get_index(scope, i).ok_or(error("TypeError"))?;
            let node = validate(scope, state, *root, value)?;
            sheets.push(state.dom.borrow().arena().text_content(node)?);
        }
        result.insert(*root, sheets);
    }
    Ok(result)
}
