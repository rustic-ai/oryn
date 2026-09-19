//! Native snapshot lists of layout rectangles.
use super::*;

fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let name = v8::String::new(scope, "oryn.rectList")?;
    let key = v8::Private::for_api(scope, Some(name));
    object
        .get_private(scope, key)
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
}
pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    values: &[v8::Local<v8::Value>],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get("DOMRectList")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("DOMRectList"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("DOMRectList"))?;
    let array = v8::Array::new_with_elements(scope, values);
    let name = v8::String::new(scope, "oryn.rectList")
        .ok_or(V8BindingError::Allocation("rect list storage"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if object.set_private(scope, key, array.into()) != Some(true) {
        return Err(V8BindingError::Allocation("rect list storage"));
    }
    Ok(object)
}
fn length(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(array) = entries(scope, args.this()) else {
        throw_type_error(scope, "DOMRectList receiver required");
        return;
    };
    rv.set_uint32(array.length());
}
fn item(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(array) = entries(scope, args.this()) else {
        throw_type_error(scope, "DOMRectList receiver required");
        return;
    };
    if args.length() == 0 {
        throw_type_error(scope, "DOMRectList.item requires an index");
        return;
    }
    let Some(index) = args.get(0).uint32_value(scope) else {
        return;
    };
    if index >= array.length() {
        rv.set(v8::null(scope).into());
    } else if let Some(value) = array.get_index(scope, index) {
        rv.set(value);
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let getter = v8::Function::builder(length)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("rect list length"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key = v8::String::new(scope, "length")
        .ok_or(V8BindingError::Allocation("rect list length key"))?;
    if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
        return Err(V8BindingError::Allocation("rect list length"));
    }
    let method = v8::Function::builder(item)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("rect list item"))?;
    set_object_property(scope, prototype, "item", method.into())?;
    let symbol = v8::Symbol::get_iterator(scope);
    let array = v8::Array::new(scope, 0);
    let iterator = array
        .get(scope, symbol.into())
        .ok_or(V8BindingError::Allocation("rect list iterator"))?;
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(iterator, true);
    descriptor.set_enumerable(false);
    descriptor.set_configurable(true);
    if prototype.define_property(scope, symbol.into(), &descriptor) != Some(true) {
        return Err(V8BindingError::Allocation("rect list iterator"));
    }
    Ok(())
}
fn getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    let Some(array) = entries(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= array.length() {
        return v8::Intercepted::kNo;
    }
    if let Some(value) = array.get_index(scope, index) {
        rv.set(value);
    }
    v8::Intercepted::kYes
}
fn query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    let Some(array) = entries(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= array.length() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}
fn enumerator(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    if let Some(array) = entries(scope, args.holder()) {
        let indices = (0..array.length())
            .map(|i| v8::Integer::new_from_unsigned(scope, i).into())
            .collect::<Vec<_>>();
        rv.set(v8::Array::new_with_elements(scope, &indices));
    }
}
fn reject(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if entries(scope, args.holder()).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "DOMRectList indices are readonly");
    }
    v8::Intercepted::kYes
}
fn setter(
    scope: &mut v8::PinScope,
    _index: u32,
    _value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject(scope, args, rv)
}
fn definer(
    scope: &mut v8::PinScope,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    reject(scope, args, rv)
}
fn deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    let Some(array) = entries(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let allowed = index >= array.length();
    rv.set_bool(allowed);
    if !allowed && args.should_throw_on_error() {
        throw_type_error(scope, "DOMRectList indices cannot be deleted");
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
