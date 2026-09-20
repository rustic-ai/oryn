//! Native document stylesheet-set selection; no authored property lookup.
use super::*;
pub(super) fn install(
    scope: &mut v8::PinScope,
    constructor: v8::Local<v8::Function>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, "prototype")
        .ok_or(V8BindingError::Allocation("Document prototype"))?;
    let prototype = constructor
        .get(scope, key.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("Document prototype"))?;
    for member in [
        "selectedStyleSheetSet",
        "preferredStyleSheetSet",
        "lastStyleSheetSet",
        "styleSheetSets",
    ] {
        let key = v8::String::new(scope, member)
            .ok_or(V8BindingError::Allocation("stylesheet set name"))?;
        let getter = v8::Function::builder(get)
            .data(key.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("stylesheet set getter"))?;
        let setter = if member == "selectedStyleSheetSet" {
            Some(
                v8::Function::new(scope, set)
                    .ok_or(V8BindingError::Allocation("stylesheet set setter"))?,
            )
        } else {
            None
        };
        let setter: v8::Local<v8::Value> = setter
            .map(Into::into)
            .unwrap_or_else(|| v8::undefined(scope).into());
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter);
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
            return Err(V8BindingError::Allocation("stylesheet set accessor"));
        }
    }
    let name = v8::String::new(scope, "enableStyleSheetsForSet")
        .ok_or(V8BindingError::Allocation("stylesheet set method"))?;
    let method = v8::Function::new(scope, enable)
        .ok_or(V8BindingError::Allocation("stylesheet set method"))?;
    prototype.create_data_property(scope, name.into(), method.into());
    Ok(())
}
fn get(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        return;
    };
    let member = args.data().to_rust_string_lossy(scope);
    if member == "styleSheetSets" {
        match resolve_receiver(scope, &state, args.this(), "Document")
            .and_then(|_| list(scope, args.this()))
        {
            Ok(value) => rv.set(value.into()),
            Err(error) => throw_binding_error(scope, &error),
        }
        return;
    }
    let result = resolve_receiver(scope, &state, args.this(), "Document").and_then(|document| {
        let dom = state.dom.borrow();
        if member == "selectedStyleSheetSet" {
            return Ok(dom.arena().selected_stylesheet_set(document)?);
        }
        let metadata = dom.arena().get(document)?.document_metadata.as_ref();
        Ok(if member == "preferredStyleSheetSet" {
            Some(
                metadata
                    .and_then(|m| m.preferred_style_set.clone())
                    .unwrap_or_default(),
            )
        } else {
            metadata.and_then(|m| m.last_style_set.clone())
        })
    });
    match result {
        Ok(Some(value)) => {
            if let Some(value) = v8::String::new(scope, &value) {
                rv.set(value.into());
            }
        }
        Ok(None) => rv.set(v8::null(scope).into()),
        Err(e) => throw_binding_error(scope, &e),
    }
}
fn select(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, last: bool) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        return;
    };
    let document = match resolve_receiver(scope, &state, args.this(), "Document") {
        Ok(node) => node,
        Err(e) => {
            throw_binding_error(scope, &e);
            return;
        }
    };
    if args.get(0).is_null() {
        return;
    }
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let result = state
        .dom
        .borrow_mut()
        .select_stylesheet_set(document, &value, last);
    if let Err(error) = result {
        throw_binding_error(scope, &error.into());
    }
}
fn set(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    select(scope, args, true)
}
fn enable(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    select(scope, args, false)
}

fn list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let key = v8::String::new(scope, "oryn.document.styleSheetSets")
        .ok_or(V8BindingError::Allocation("stylesheet sets cache"))?;
    let key = v8::Private::for_api(scope, Some(key));
    if let Some(value) = document
        .get_private(scope, key)
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
    {
        return Ok(value);
    }
    let state = binding_state_for_receiver(scope, document)
        .ok_or(V8BindingError::Allocation("stylesheet sets realm"))?;
    let template = state
        .instance_templates
        .borrow()
        .get("DOMStringList")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::Allocation("DOMStringList template"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("stylesheet sets object"))?;
    object.set_internal_field(0, document.into());
    let tag = v8::String::new(scope, "StyleSheetSets")
        .ok_or(V8BindingError::Allocation("stylesheet sets brand"))?;
    object.set_internal_field(1, tag.into());
    document.set_private(scope, key, object.into());
    Ok(object)
}
pub(super) fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    names: &[String],
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get("DOMStringList")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::Allocation("DOMStringList template"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("DOMStringList snapshot"))?;
    let values = names
        .iter()
        .map(|name| v8_string(scope, name))
        .collect::<Result<Vec<_>, _>>()?;
    let array = v8::Array::new_with_elements(scope, &values);
    object.set_internal_field(0, array.into());
    let tag = v8::String::new(scope, "NativeStringSnapshot")
        .ok_or(V8BindingError::Allocation("DOMStringList brand"))?;
    object.set_internal_field(1, tag.into());
    Ok(object)
}
fn names(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Vec<String>, V8BindingError> {
    if !object.is_proxy()
        && object.internal_field_count() >= 2
        && object
            .get_internal_field(scope, 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .is_some_and(|value| value.to_rust_string_lossy(scope) == "NativeStringSnapshot")
    {
        let values = object
            .get_internal_field(scope, 0)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .ok_or(V8BindingError::Allocation("DOMStringList snapshot backing"))?;
        return (0..values.length())
            .map(|index| {
                values
                    .get_index(scope, index)
                    .map(|value| value.to_rust_string_lossy(scope))
                    .ok_or(V8BindingError::Allocation("DOMStringList snapshot item"))
            })
            .collect();
    }
    if object.is_proxy()
        || object.internal_field_count() < 2
        || !object
            .get_internal_field(scope, 1)
            .and_then(|v| v8::Local::<v8::String>::try_from(v).ok())
            .is_some_and(|v| v.to_rust_string_lossy(scope) == "StyleSheetSets")
    {
        return Err(V8BindingError::Allocation("stylesheet sets receiver"));
    }
    let document = object
        .get_internal_field(scope, 0)
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("stylesheet sets document"))?;
    let state = binding_state_for_receiver(scope, document)
        .ok_or(V8BindingError::Allocation("stylesheet sets realm"))?;
    let node = resolve_receiver(scope, &state, document, "Document")?;
    let names = state.dom.borrow().arena().stylesheet_set_names(node)?;
    Ok(names)
}
pub(super) fn length(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match names(scope, args.this()) {
        Ok(names) => rv.set_uint32(names.len() as u32),
        Err(e) => throw_binding_error(scope, &e),
    }
}
pub(super) fn item(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if args.length() < 1 {
        throw_type_error(scope, "DOMStringList.item requires an index");
        return;
    }
    let Some(index) = args.get(0).uint32_value(scope) else {
        return;
    };
    match names(scope, args.this()) {
        Ok(names) => match names.get(index as usize) {
            Some(name) => {
                if let Some(value) = v8::String::new(scope, name) {
                    rv.set(value.into());
                }
            }
            None => rv.set(v8::null(scope).into()),
        },
        Err(e) => throw_binding_error(scope, &e),
    }
}
pub(super) fn contains(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if args.length() < 1 {
        throw_type_error(scope, "DOMStringList.contains requires a string");
        return;
    }
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    match names(scope, args.this()) {
        Ok(names) => rv.set_bool(names.contains(&value)),
        Err(e) => throw_binding_error(scope, &e),
    }
}
fn index(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match names(scope, args.holder()) {
        Ok(names) => {
            let Some(value) = names.get(index as usize) else {
                return v8::Intercepted::kNo;
            };
            if let Some(value) = v8::String::new(scope, value) {
                rv.set(value.into());
            }
        }
        Err(e) => throw_binding_error(scope, &e),
    }
    v8::Intercepted::kYes
}
fn query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match names(scope, args.holder()) {
        Ok(names) => {
            if index as usize >= names.len() {
                return v8::Intercepted::kNo;
            }
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
        }
        Err(e) => throw_binding_error(scope, &e),
    }
    v8::Intercepted::kYes
}
fn enumerate(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match names(scope, args.holder()) {
        Ok(names) => {
            let values = (0..names.len())
                .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &values));
        }
        Err(e) => throw_binding_error(scope, &e),
    }
}

pub(super) fn configure(template: v8::Local<v8::ObjectTemplate>) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(index)
            .query(query)
            .enumerator(enumerate)
            .setter(reject_set)
            .definer(reject_define),
    );
}
fn reject_set(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "DOMStringList indices are readonly");
    }
    v8::Intercepted::kYes
}
fn reject_define(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "DOMStringList indices are readonly");
    }
    v8::Intercepted::kYes
}
