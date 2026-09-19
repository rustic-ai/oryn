//! CSS script registrations use the document's native Stylo registry contract.
use super::*;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "CSS".into(),
        member: name.into(),
    }
}
fn register(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let result = (|| -> Result<(), V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("CSS realm"))?;
        let input = args.get(0);
        let mut name = None;
        let mut syntax = "*".to_owned();
        let mut inherits = None;
        let mut initial_value = None;
        // Dictionary conversion is observable, including callbacks which may
        // themselves register this name. Recheck the live document afterwards.
        for key in ["inherits", "initialValue", "name", "syntax"] {
            if let Some(value) = checked_dictionary_member(scope, input, key)? {
                match key {
                    "inherits" => inherits = Some(value.boolean_value(scope)),
                    _ => {
                        let value = value
                            .to_string(scope)
                            .ok_or(V8BindingError::JavaScriptException)?
                            .to_rust_string_lossy(scope);
                        match key {
                            "initialValue" => initial_value = Some(value),
                            "name" => name = Some(value),
                            _ => syntax = value,
                        }
                    }
                }
            } else if matches!(key, "inherits" | "name") {
                throw_type_error(scope, "CSS.registerProperty requires name and inherits");
                return Err(V8BindingError::JavaScriptException);
            }
        }
        let name = name.ok_or_else(|| error("TypeError"))?;
        let document = state.document_root();
        if state
            .dom
            .borrow()
            .arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .is_some_and(|metadata| {
                metadata
                    .registered_custom_properties
                    .iter()
                    .any(|entry| entry.name == name)
            })
        {
            return Err(error("InvalidModificationError"));
        }
        let registration = oryn_dom::RegisteredCustomProperty {
            name,
            syntax,
            inherits: inherits.unwrap_or(false),
            initial_value,
            base_url: state.native_document_base_url(true)?,
        };
        let provider = state
            .layout_provider
            .borrow()
            .clone()
            .ok_or_else(|| error("NotSupportedError"))?;
        provider
            .validate_custom_property(&registration)
            .map_err(|name| error(&name))?;
        if !state
            .dom
            .borrow_mut()
            .register_custom_property(document, registration)?
        {
            return Err(error("InvalidModificationError"));
        }
        state.layout_cache_key.borrow_mut().take();
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or(V8BindingError::Allocation("CSS realm"))?;
    if state.binding_global != BindingGlobal::Window {
        return Ok(());
    }
    let namespace = v8::Object::new(scope);
    let function = v8::Function::builder(register)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("registerProperty"))?;
    set_object_property(scope, namespace, "registerProperty", function.into())?;
    let function = v8::Function::builder(supports)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("CSS supports"))?;
    set_object_property(scope, namespace, "supports", function.into())?;
    let function = v8::Function::builder(escape)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("CSS escape"))?;
    set_object_property(scope, namespace, "escape", function.into())?;

    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "CSS").ok_or(V8BindingError::Allocation("CSS tag"))?;
    if namespace.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    ) != Some(true)
    {
        return Err(V8BindingError::Allocation("CSS tag"));
    }
    let global = context.global(scope);
    set_object_property(scope, global, "CSS", namespace.into())
}

fn supports(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<bool, V8BindingError> {
        if args.length() == 0 {
            throw_type_error(scope, "CSS.supports requires a condition");
            return Err(V8BindingError::JavaScriptException);
        }
        let condition = args
            .get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope);
        let value = if args.length() > 1 {
            Some(
                args.get(1)
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope),
            )
        } else {
            None
        };
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or(V8BindingError::Allocation("CSS realm"))?;
        let base = state.native_document_base_url(true)?;
        let provider = state
            .layout_provider
            .borrow()
            .clone()
            .ok_or_else(|| error("NotSupportedError"))?;
        Ok(provider.css_supports(&condition, value.as_deref(), &base))
    })();
    match result {
        Ok(value) => rv.set_bool(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn escape(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        if args.length() == 0 {
            throw_type_error(scope, "CSS.escape requires a string");
            return Err(V8BindingError::JavaScriptException);
        }
        let string = args
            .get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        let mut units = vec![0u16; string.length()];
        string.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
        let mut output = Vec::new();
        for (index, unit) in units.iter().copied().enumerate() {
            if unit == 0 {
                output.push(0xfffd);
            } else if unit <= 31
                || unit == 127
                || (index == 0 && (48..=57).contains(&unit))
                || (index == 1 && units[0] == 45 && (48..=57).contains(&unit))
            {
                output.push(92);
                output.extend(format!("{unit:x} ").encode_utf16());
            } else if unit >= 128
                || unit == 95
                || unit == 45 && !(index == 0 && units.len() == 1)
                || (48..=57).contains(&unit)
                || (65..=90).contains(&unit)
                || (97..=122).contains(&unit)
            {
                output.push(unit);
            } else {
                output.push(92);
                output.push(unit);
            }
        }
        Ok(
            v8::String::new_from_two_byte(scope, &output, v8::NewStringType::Normal)
                .ok_or(V8BindingError::Allocation("CSS escape"))?
                .into(),
        )
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
