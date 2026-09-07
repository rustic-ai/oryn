//! Live indexed CSS declaration access. Parsing and order are owned by Stylo.
use super::*;

fn names(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Option<Vec<String>>, V8BindingError> {
    let Some(state) = binding_state_for_receiver(scope, object) else {
        return Ok(None);
    };
    if computed_styles::is_computed(scope, &state, object) {
        return computed_styles::property_names(scope, &state, object).map(Some);
    }
    let node = resolve_receiver(scope, &state, object, "CSSStyleDeclaration")?;
    Ok(Some(
        match read_inline_declaration(scope, &state, node, "", None)? {
            Some(view) => view.names,
            None => inline_style_entries(
                state
                    .dom
                    .borrow()
                    .arena()
                    .get(node)?
                    .attributes
                    .get("style")
                    .map_or("", String::as_str),
            )
            .into_keys()
            .collect(),
        },
    ))
}

fn getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match names(scope, args.holder()) {
        Ok(Some(names)) if (index as usize) < names.len() => {
            match v8_string(scope, &names[index as usize]) {
                Ok(value) => rv.set(value),
                Err(error) => throw_binding_error(scope, &error),
            }
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
    match names(scope, args.holder()) {
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
    match names(scope, args.holder()) {
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
    match names(scope, args.holder()) {
        Ok(None) => return v8::Intercepted::kNo,
        Err(error) => throw_binding_error(scope, &error),
        Ok(Some(_)) => {
            rv.set_bool(false);
            if args.should_throw_on_error() {
                throw_type_error(
                    scope,
                    "CSSStyleDeclaration indexed properties are read-only",
                );
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
    match names(scope, args.holder()) {
        Ok(None) => return v8::Intercepted::kNo,
        Err(error) => throw_binding_error(scope, &error),
        Ok(Some(names)) => {
            let allowed = index as usize >= names.len();
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(
                    scope,
                    "CSSStyleDeclaration indexed properties cannot be deleted",
                );
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
