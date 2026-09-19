//! Native cross-origin Location projection. These callbacks run only after
//! the creation realm's security-token check fails.
use super::*;
use crate::v8_context_bridge::{CrossContextHandlers, set_cross_context_handlers};
use v8::MapFnTo;

fn denied(scope: &mut v8::PinScope) {
    throw_binding_error(scope, &error("SecurityError"));
}
fn is_name(scope: &mut v8::PinScope, key: v8::Local<v8::Name>, name: &str) -> bool {
    key.is_string() && key.to_rust_string_lossy(scope) == name
}
fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    if args.length() < 1 {
        throw_type_error(scope, "Location operation requires a URL");
        return;
    }
    let receiver = args.this();
    if receiver.is_proxy() {
        throw_type_error(scope, "Location requires a Location receiver");
        return;
    }
    let Some(state) = binding_state_for_receiver(scope, receiver) else {
        throw_type_error(scope, "Location requires a Location receiver");
        return;
    };
    let Ok(id) = stateful_id(scope, receiver) else {
        throw_type_error(scope, "Location requires a Location receiver");
        return;
    };
    if brand(&state, id).is_err() {
        throw_type_error(scope, "Location requires a Location receiver");
        return;
    }
    let data = v8::Local::<v8::Object>::try_from(args.data()).unwrap();
    let setter = data
        .get_internal_field(scope, 1)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .is_some_and(|value| value.is_true());
    let result = if setter {
        set(scope, &state, id, "href", args.get(0)).map(|()| None)
    } else {
        operation(scope, &state, id, "replace", &args)
    };
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn callable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<v8::Object>,
    setter: bool,
) -> Option<v8::Local<'s, v8::Function>> {
    let context = holder.get_creation_context(scope)?;
    let entry = window_proxy::cache_entry(scope, context)?;
    let key = v8::String::new(
        scope,
        if setter {
            "location:set href"
        } else {
            "location:replace"
        },
    )?;
    if entry.has_own_property(scope, key.into())? {
        return v8::Local::<v8::Function>::try_from(entry.get(scope, key.into())?).ok();
    }
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(2);
    let data = template.new_instance(scope)?;
    data.set_internal_field(0, context.into());
    data.set_internal_field(1, v8::Boolean::new(scope, setter).into());
    let function = v8::Function::builder(call)
        .data(data.into())
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    function.set_name(v8::String::new(
        scope,
        if setter { "set href" } else { "replace" },
    )?);
    entry.create_data_property(scope, key.into(), function.into())?;
    Some(function)
}
fn getter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if is_name(scope, key, "replace") {
        if let Some(function) = callable(scope, args.holder(), false) {
            rv.set(function.into());
        }
    } else if window_proxy::fallback(scope, key) {
        rv.set(v8::undefined(scope).into());
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn setter(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if is_name(scope, key, "href") {
        if let Some(function) = callable(scope, args.holder(), true) {
            function.call(scope, args.holder().into(), &[value]);
        }
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn descriptor(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if is_name(scope, key, "href") {
        if let Some(function) = callable(scope, args.holder(), true)
            && let Some(record) = window_proxy::descriptor_record(scope)
        {
            for (name, value) in [
                ("get", v8::undefined(scope).into()),
                ("set", function.into()),
                ("enumerable", v8::Boolean::new(scope, false).into()),
                ("configurable", v8::Boolean::new(scope, true).into()),
            ] {
                let Some(key) = v8::String::new(scope, name) else {
                    return v8::Intercepted::kYes;
                };
                record.create_data_property(scope, key.into(), value);
            }
            rv.set(record.into());
        }
    } else if is_name(scope, key, "replace") {
        if let Some(function) = callable(scope, args.holder(), false)
            && let Some(record) = window_proxy::data_descriptor(scope, function.into(), false)
        {
            rv.set(record.into());
        }
    } else if window_proxy::fallback(scope, key) {
        let value = v8::undefined(scope).into();
        if let Some(record) = window_proxy::data_descriptor(scope, value, false) {
            rv.set(record.into());
        }
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn query(
    scope: &mut v8::PinScope,
    key: v8::Local<v8::Name>,
    _: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    if is_name(scope, key, "href") {
        rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
    } else if is_name(scope, key, "replace") || window_proxy::fallback(scope, key) {
        rv.set_int32(
            (v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY).as_u32() as i32,
        );
    } else {
        denied(scope);
    }
    v8::Intercepted::kYes
}
fn enumerate(
    scope: &mut v8::PinScope,
    _: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    let mut keys = Vec::with_capacity(6);
    for name in ["href", "replace", "then"] {
        let Some(key) = v8::String::new(scope, name) else {
            return;
        };
        keys.push(key.into());
    }
    for symbol in [
        v8::Symbol::get_to_string_tag(scope),
        v8::Symbol::get_has_instance(scope),
        v8::Symbol::get_is_concat_spreadable(scope),
    ] {
        keys.push(symbol.into());
    }
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
fn delete(
    scope: &mut v8::PinScope,
    _: v8::Local<v8::Name>,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn define(
    scope: &mut v8::PinScope,
    _: v8::Local<v8::Name>,
    _: &v8::PropertyDescriptor,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn index_get(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn index_set(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn index_query(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn index_delete(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn index_define(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    _: v8::PropertyCallbackArguments,
    _: v8::ReturnValue<()>,
) -> v8::Intercepted {
    denied(scope);
    v8::Intercepted::kYes
}
fn index_enumerate(
    scope: &mut v8::PinScope,
    _: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    rv.set(v8::Array::new(scope, 0));
}

pub(super) fn configure(scope: &mut v8::PinScope, instance: v8::Local<v8::ObjectTemplate>) {
    set_cross_context_handlers(
        scope,
        instance,
        &CrossContextHandlers {
            named_getter: Some(getter.map_fn_to()),
            named_setter: Some(setter.map_fn_to()),
            named_query: Some(query.map_fn_to()),
            named_descriptor: Some(descriptor.map_fn_to()),
            named_enumerator: Some(enumerate.map_fn_to()),
            named_deleter: Some(delete.map_fn_to()),
            named_definer: Some(define.map_fn_to()),
            indexed_getter: Some(index_get.map_fn_to()),
            indexed_setter: Some(index_set.map_fn_to()),
            indexed_query: Some(index_query.map_fn_to()),
            indexed_descriptor: Some(index_get.map_fn_to()),
            indexed_enumerator: Some(index_enumerate.map_fn_to()),
            indexed_deleter: Some(index_delete.map_fn_to()),
            indexed_definer: Some(index_define.map_fn_to()),
        },
    );
}
