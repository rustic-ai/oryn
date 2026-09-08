//! Native URL query lists, shared with their associated URL record.
use super::*;

type Result<T> = std::result::Result<T, V8BindingError>;

pub(super) struct SearchParams {
    pub entries: Vec<(String, String)>,
    pub url: Option<u64>,
}
pub(super) struct IteratorRecord {
    owner: u64,
    index: usize,
    kind: i32,
}
fn type_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "URLSearchParams".into(),
        member: "TypeError".into(),
    }
}
fn string(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String> {
    Ok(value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope))
}
fn field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("URL iterator key"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn parse(input: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(input.as_bytes())
        .into_owned()
        .collect()
}
fn serialize(entries: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(entries.iter())
        .finish()
}

// Convert each item while the iterator is active, so conversion side effects,
// abrupt completions, and IteratorClose retain their specified ordering.
fn consume<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    method: v8::Local<v8::Function>,
    mut convert: impl FnMut(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Value>) -> Result<()>,
) -> Result<()> {
    let iterator = method
        .call(scope, object.into(), &[])
        .ok_or(V8BindingError::JavaScriptException)?;
    let iterator = v8::Local::<v8::Object>::try_from(iterator).map_err(|_| type_error())?;
    let next = v8::Local::<v8::Function>::try_from(field(scope, iterator, "next")?)
        .map_err(|_| type_error())?;
    loop {
        let step = next
            .call(scope, iterator.into(), &[])
            .ok_or(V8BindingError::JavaScriptException)?;
        let step = v8::Local::<v8::Object>::try_from(step).map_err(|_| type_error())?;
        if field(scope, step, "done")?.boolean_value(scope) {
            return Ok(());
        }
        let value = field(scope, step, "value")?;
        let (result, exception) = {
            v8::tc_scope!(let tc, scope);
            let result = convert(tc, value);
            let exception = tc.exception().map(|e| v8::Global::new(tc, e));
            tc.reset();
            (result, exception)
        };
        if let Err(error) = result {
            {
                v8::tc_scope!(let tc, scope);
                if let Ok(value) = field(tc, iterator, "return")
                    && let Ok(close) = v8::Local::<v8::Function>::try_from(value)
                {
                    close.call(tc, iterator.into(), &[]);
                }
            }
            if let Some(exception) = exception {
                let exception = v8::Local::new(scope, exception);
                scope.throw_exception(exception);
                return Err(V8BindingError::JavaScriptException);
            }
            return Err(error);
        }
    }
}

pub(super) fn initialize(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<SearchParams> {
    let mut entries = Vec::new();
    if value.is_undefined() {
        return Ok(SearchParams { entries, url: None });
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        let key = v8::Symbol::get_iterator(scope);
        let method = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !method.is_null_or_undefined() {
            let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| type_error())?;
            consume(scope, object, method, |scope, value| {
                let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| type_error())?;
                let key = v8::Symbol::get_iterator(scope);
                let method = object
                    .get(scope, key.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
                let method =
                    v8::Local::<v8::Function>::try_from(method).map_err(|_| type_error())?;
                let mut pair = Vec::new();
                consume(scope, object, method, |scope, value| {
                    pair.push(string(scope, value)?);
                    Ok(())
                })?;
                if pair.len() != 2 {
                    return Err(type_error());
                }
                entries.push((pair.remove(0), pair.remove(0)));
                Ok(())
            })?;
        } else {
            let names = object
                .get_own_property_names(
                    scope,
                    v8::GetPropertyNamesArgs {
                        mode: v8::KeyCollectionMode::OwnOnly,
                        property_filter: v8::PropertyFilter::SKIP_SYMBOLS,
                        key_conversion: v8::KeyConversionMode::ConvertToString,
                        ..Default::default()
                    },
                )
                .ok_or(V8BindingError::JavaScriptException)?;
            for index in 0..names.length() {
                let key = names
                    .get_index(scope, index)
                    .ok_or(V8BindingError::JavaScriptException)?;
                let key_name = v8::Local::<v8::Name>::try_from(key).map_err(|_| type_error())?;
                let descriptor = object
                    .get_own_property_descriptor(scope, key_name)
                    .ok_or(V8BindingError::JavaScriptException)?;
                let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) else {
                    continue;
                };
                if !field(scope, descriptor, "enumerable")?.boolean_value(scope) {
                    continue;
                }
                let name = string(scope, key)?;
                let value = object
                    .get(scope, key)
                    .ok_or(V8BindingError::JavaScriptException)?;
                let value = string(scope, value)?;
                if let Some((_, old)) = entries.iter_mut().find(|(old, _)| old == &name) {
                    *old = value;
                } else {
                    entries.push((name, value));
                }
            }
        }
    } else {
        let input = string(scope, value)?;
        entries = parse(input.strip_prefix('?').unwrap_or(&input));
    }
    Ok(SearchParams { entries, url: None })
}

fn private_link(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<()> {
    let name = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("URL private key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    object
        .set_private(scope, key, value)
        .ok_or(V8BindingError::Allocation("URL private edge"))?;
    Ok(())
}
pub(super) fn search_params<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    id: u64,
) -> Result<v8::Local<'s, v8::Value>> {
    let existing = state.url_search_params.borrow().get(&id).copied();
    let params_id = match existing {
        Some(id) => id,
        None => {
            let entries = {
                let record = platform_object(state, id)?;
                let PlatformObjectRecord::Url(url) = &*record else {
                    return Err(type_error());
                };
                parse(url.query().unwrap_or(""))
            };
            let params = register_platform_object(
                state,
                PlatformObjectRecord::UrlSearchParams(SearchParams {
                    entries,
                    url: Some(id),
                }),
            );
            state.url_search_params.borrow_mut().insert(id, params);
            params
        }
    };
    let context = object.get_creation_context(scope).ok_or_else(type_error)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let params = new_platform_object(scope, state, "URLSearchParams", params_id)?;
    private_link(scope, object, "oryn.url.searchParams", params.into())?;
    private_link(scope, params, "oryn.searchParams.url", object.into())?;
    Ok(params.into())
}
pub(super) fn refresh(state: &V8BindingState, url_id: u64) -> Result<()> {
    let Some(id) = state.url_search_params.borrow().get(&url_id).copied() else {
        return Ok(());
    };
    let entries = {
        let record = platform_object(state, url_id)?;
        let PlatformObjectRecord::Url(url) = &*record else {
            return Err(type_error());
        };
        parse(url.query().unwrap_or(""))
    };
    let mut record = platform_object_mut(state, id)?;
    let PlatformObjectRecord::UrlSearchParams(params) = &mut *record else {
        return Err(type_error());
    };
    params.entries = entries;
    Ok(())
}
fn update(state: &V8BindingState, id: u64) -> Result<()> {
    let (url_id, query) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::UrlSearchParams(params) = &*record else {
            return Err(type_error());
        };
        (params.url, serialize(&params.entries))
    };
    if let Some(id) = url_id {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::Url(url) = &mut *record else {
            return Err(type_error());
        };
        url.set_query((!query.is_empty()).then_some(query.as_str()));
    }
    Ok(())
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    if !matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::UrlSearchParams(_)
    ) {
        return Err(type_error());
    }
    if member == "toString" {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::UrlSearchParams(params) = &*record else {
            return Err(type_error());
        };
        return Ok(Some(v8_string(scope, &serialize(&params.entries))?));
    }
    let name = if member == "sort" {
        String::new()
    } else {
        string(scope, args.get(0))?
    };
    let value = if matches!(member, "append" | "set")
        || (matches!(member, "delete" | "has") && !args.get(1).is_undefined())
    {
        Some(string(scope, args.get(1))?)
    } else {
        None
    };
    {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::UrlSearchParams(params) = &mut *record else {
            return Err(type_error());
        };
        let matches = |(key, entry): &(String, String)| {
            key == &name && value.as_ref().is_none_or(|value| entry == value)
        };
        match member {
            "append" => params.entries.push((name, value.unwrap_or_default())),
            "delete" => params.entries.retain(|entry| !matches(entry)),
            "get" => {
                return Ok(Some(
                    match params.entries.iter().find(|(key, _)| key == &name) {
                        Some((_, value)) => v8_string(scope, value)?,
                        None => v8::null(scope).into(),
                    },
                ));
            }
            "getAll" => {
                let values = params
                    .entries
                    .iter()
                    .filter(|(key, _)| key == &name)
                    .map(|(_, value)| v8_string(scope, value))
                    .collect::<Result<Vec<_>>>()?;
                return Ok(Some(v8::Array::new_with_elements(scope, &values).into()));
            }
            "has" => {
                return Ok(Some(
                    v8::Boolean::new(scope, params.entries.iter().any(matches)).into(),
                ));
            }
            "set" => {
                let value = value.unwrap_or_default();
                let mut found = false;
                params.entries.retain_mut(|(key, entry)| {
                    if key != &name {
                        return true;
                    }
                    if found {
                        return false;
                    }
                    *entry = value.clone();
                    found = true;
                    true
                });
                if !found {
                    params.entries.push((name, value));
                }
            }
            "sort" => params
                .entries
                .sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16())),
            _ => return Err(type_error()),
        }
    }
    update(state, id)?;
    Ok(None)
}

pub(super) fn install_iterable(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::ObjectTemplate>,
) -> Result<()> {
    for (kind, name) in [(0, "entries"), (1, "keys"), (2, "values"), (3, "forEach")] {
        let data = v8::Integer::new(scope, kind);
        let function = v8::FunctionTemplate::builder(iterable_callback)
            .data(data.into())
            .length(if kind == 3 { 1 } else { 0 })
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope);
        let name = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("URL iterator method"))?;
        function.set_class_name(name);
        prototype.set(name.into(), function.into());
        if kind == 0 {
            prototype.set_with_attr(
                v8::Symbol::get_iterator(scope).into(),
                function.into(),
                v8::PropertyAttribute::DONT_ENUM,
            );
        }
    }
    Ok(())
}
fn iterable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    match iterable_operation(scope, &args) {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn entry(state: &V8BindingState, id: u64, index: usize) -> Result<Option<(String, String)>> {
    let record = platform_object(state, id)?;
    let PlatformObjectRecord::UrlSearchParams(params) = &*record else {
        return Err(type_error());
    };
    Ok(params.entries.get(index).cloned())
}
fn iterable_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    let object = args.this();
    let state = binding_state_for_receiver(scope, object).ok_or_else(type_error)?;
    let id = stateful_id(scope, object)?;
    entry(&state, id, 0)?; // Validate the receiver even for an empty list.
    let kind = args.data().int32_value(scope).ok_or_else(type_error)?;
    if kind == 3 {
        let callback =
            v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_| type_error())?;
        let mut index = 0;
        while let Some((key, value)) = entry(&state, id, index)? {
            let value = v8_string(scope, &value)?;
            let key = v8_string(scope, &key)?;
            callback
                .call(scope, args.get(1), &[value, key, object.into()])
                .ok_or(V8BindingError::JavaScriptException)?;
            index += 1;
        }
        return Ok(None);
    }
    let iterator_id = register_platform_object(
        &state,
        PlatformObjectRecord::UrlSearchParamsIterator(IteratorRecord {
            owner: id,
            index: 0,
            kind,
        }),
    );
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(1);
    let iterator = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("URL iterator"))?;
    let realm_state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or_else(type_error)?;
    let prototype = iterator_prototype(scope, &realm_state)?;
    iterator
        .set_prototype(scope, prototype.into())
        .ok_or(V8BindingError::Allocation("URL iterator prototype"))?;
    bind_platform_object(scope, &state, iterator_id, iterator);
    private_link(scope, iterator, "oryn.url.iteratorOwner", object.into())?;
    Ok(Some(iterator.into()))
}
fn iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(prototype) = state.url_iterator_prototype.borrow().as_ref() {
        return Ok(v8::Local::new(scope, prototype));
    }
    let holder = v8::ObjectTemplate::new(scope);
    let key = v8::String::new(scope, "parent")
        .ok_or(V8BindingError::Allocation("iterator parent key"))?;
    holder.set_intrinsic_data_property(
        key.into(),
        v8::Intrinsic::IteratorPrototype,
        v8::PropertyAttribute::NONE,
    );
    let holder = holder
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("iterator intrinsic holder"))?;
    let parent = holder
        .get(scope, key.into())
        .ok_or(V8BindingError::Allocation("iterator intrinsic"))?;
    let prototype = v8::Object::new(scope);
    prototype
        .set_prototype(scope, parent)
        .ok_or(V8BindingError::Allocation("iterator prototype parent"))?;
    let function = v8::FunctionTemplate::builder(next_callback)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope);
    let name =
        v8::String::new(scope, "next").ok_or(V8BindingError::Allocation("iterator next name"))?;
    function.set_class_name(name);
    let function = function
        .get_function(scope)
        .ok_or(V8BindingError::Allocation("iterator next"))?;
    prototype
        .create_data_property(scope, name.into(), function.into())
        .ok_or(V8BindingError::Allocation("iterator next property"))?;
    let tag = v8::Symbol::get_to_string_tag(scope);
    let value = v8_string(scope, "URLSearchParams Iterator")?;
    prototype
        .define_own_property(
            scope,
            tag.into(),
            value,
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        )
        .ok_or(V8BindingError::Allocation("iterator tag"))?;
    state
        .url_iterator_prototype
        .borrow_mut()
        .replace(v8::Global::new(scope, prototype));
    Ok(prototype)
}
fn next_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match next(scope, args.this()) {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>> {
    let name = v8::String::new(scope, "oryn.url.iteratorOwner")
        .ok_or(V8BindingError::Allocation("iterator owner key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let owner = object.get_private(scope, key).ok_or_else(type_error)?;
    let owner = v8::Local::<v8::Object>::try_from(owner).map_err(|_| type_error())?;
    let state = binding_state_for_receiver(scope, owner).ok_or_else(type_error)?;
    let id = stateful_id(scope, object)?;
    let (owner, index, kind) = {
        let record = platform_object(&state, id)?;
        let PlatformObjectRecord::UrlSearchParamsIterator(iterator) = &*record else {
            return Err(type_error());
        };
        (iterator.owner, iterator.index, iterator.kind)
    };
    let pair = entry(&state, owner, index)?;
    let done = pair.is_none();
    let value = if let Some((key, value)) = pair {
        {
            let mut record = platform_object_mut(&state, id)?;
            let PlatformObjectRecord::UrlSearchParamsIterator(iterator) = &mut *record else {
                return Err(type_error());
            };
            iterator.index += 1;
        }
        match kind {
            1 => v8_string(scope, &key)?,
            2 => v8_string(scope, &value)?,
            _ => {
                let key = v8_string(scope, &key)?;
                let value = v8_string(scope, &value)?;
                v8::Array::new_with_elements(scope, &[key, value]).into()
            }
        }
    } else {
        v8::undefined(scope).into()
    };
    let result = v8::Object::new(scope);
    set_object_property(scope, result, "value", value)?;
    let done = v8::Boolean::new(scope, done);
    set_object_property(scope, result, "done", done.into())?;
    Ok(result.into())
}
