//! Live native FormData pair iterators; the owner link keeps File values reachable.
use super::*;
type Result<T> = std::result::Result<T, V8BindingError>;
pub(super) struct IteratorRecord {
    owner: u64,
    index: usize,
    kind: i32,
}
fn type_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "FormData".into(),
        member: "TypeError: incompatible receiver".into(),
    }
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
            .ok_or(V8BindingError::Allocation("FormData iterator method"))?;
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
fn entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    index: usize,
) -> Result<Option<(String, v8::Local<'s, v8::Value>)>> {
    let record = platform_object(state, id)?;
    let PlatformObjectRecord::FormData(params) = &*record else {
        return Err(type_error());
    };
    Ok(params
        .entries
        .get(index)
        .map(|(key, value)| (key.clone(), v8::Local::new(scope, value))))
}
fn iterable_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    let object = args.this();
    let state = binding_state_for_receiver(scope, object).ok_or_else(type_error)?;
    let id = stateful_id(scope, object)?;
    entry(scope, &state, id, 0)?; // Validate the receiver even for an empty list.
    let kind = args.data().int32_value(scope).ok_or_else(type_error)?;
    if kind == 3 {
        let callback =
            v8::Local::<v8::Function>::try_from(args.get(0)).map_err(|_| type_error())?;
        let mut index = 0;
        while let Some((key, value)) = entry(scope, &state, id, index)? {
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
        PlatformObjectRecord::FormDataIterator(IteratorRecord {
            owner: id,
            index: 0,
            kind,
        }),
    );
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(1);
    let iterator = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("FormData iterator"))?;
    let realm_state = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .ok_or_else(type_error)?;
    let prototype = iterator_prototype(scope, &realm_state)?;
    iterator
        .set_prototype(scope, prototype.into())
        .ok_or(V8BindingError::Allocation("FormData iterator prototype"))?;
    bind_platform_object(scope, &state, iterator_id, iterator);
    let name = v8::String::new(scope, "oryn.formdata.iteratorOwner")
        .ok_or(V8BindingError::Allocation("FormData iterator owner key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    iterator
        .set_private(scope, key, object.into())
        .ok_or(V8BindingError::Allocation("FormData iterator owner link"))?;
    Ok(Some(iterator.into()))
}
fn iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(prototype) = state.form_data_iterator_prototype.borrow().as_ref() {
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
    let value = v8_string(scope, "FormData Iterator")?;
    prototype
        .define_own_property(
            scope,
            tag.into(),
            value,
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        )
        .ok_or(V8BindingError::Allocation("iterator tag"))?;
    state
        .form_data_iterator_prototype
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
    let name = v8::String::new(scope, "oryn.formdata.iteratorOwner")
        .ok_or(V8BindingError::Allocation("iterator owner key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let owner = object.get_private(scope, key).ok_or_else(type_error)?;
    let owner = v8::Local::<v8::Object>::try_from(owner).map_err(|_| type_error())?;
    let state = binding_state_for_receiver(scope, owner).ok_or_else(type_error)?;
    let id = stateful_id(scope, object)?;
    let (owner, index, kind) = {
        let record = platform_object(&state, id)?;
        let PlatformObjectRecord::FormDataIterator(iterator) = &*record else {
            return Err(type_error());
        };
        (iterator.owner, iterator.index, iterator.kind)
    };
    let pair = if index == usize::MAX {
        None
    } else {
        entry(scope, &state, owner, index)?
    };
    let done = pair.is_none();
    if done {
        let mut record = platform_object_mut(&state, id)?;
        let PlatformObjectRecord::FormDataIterator(iterator) = &mut *record else {
            return Err(type_error());
        };
        iterator.index = usize::MAX;
    }
    let value = if let Some((key, value)) = pair {
        {
            let mut record = platform_object_mut(&state, id)?;
            let PlatformObjectRecord::FormDataIterator(iterator) = &mut *record else {
                return Err(type_error());
            };
            iterator.index += 1;
        }
        match kind {
            1 => v8_string(scope, &key)?,
            2 => value,
            _ => {
                let key = v8_string(scope, &key)?;
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
