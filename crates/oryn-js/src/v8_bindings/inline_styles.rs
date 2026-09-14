//! Live indexed CSS declaration access. Parsing and order are owned by Stylo.
use super::*;

fn names(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Option<Vec<String>>, V8BindingError> {
    if let Some(names) = page_rules::declaration_names(scope, object)? {
        return Ok(Some(names));
    }
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
                    .get_null("style")
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

pub(super) fn attribute_names(property: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::from([property.to_owned()]);
    let mut camel = String::new();
    let mut uppercase = false;
    for character in property.chars() {
        if character == '-' {
            uppercase = true;
        } else if uppercase {
            camel.extend(character.to_uppercase());
            uppercase = false;
        } else {
            camel.push(character);
        }
    }
    if let Some(suffix) = camel.strip_prefix("Webkit") {
        names.insert(format!("webkit{suffix}"));
    }
    names.insert(camel);
    names
}

fn property_getter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let state = binding_state_for_receiver(scope, args.this())
            .ok_or(V8BindingError::Allocation("CSS property receiver"))?;
        let property = args.data().to_rust_string_lossy(scope);
        if page_rules::is_declaration(scope, args.this()) {
            return page_rules::declaration_attribute(scope, args.this(), &property);
        }
        if computed_styles::is_computed(scope, &state, args.this()) {
            return computed_styles::attribute(scope, &state, args.this(), &property);
        }
        let node = resolve_receiver(scope, &state, args.this(), "CSSStyleDeclaration")?;
        get_attribute_value(scope, &state, node, "CSSStyleDeclaration", &property)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn property_setter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let result = (|| {
        let state = binding_state_for_receiver(scope, args.this())
            .ok_or(V8BindingError::Allocation("CSS property receiver"))?;
        let property = args.data().to_rust_string_lossy(scope);
        if page_rules::is_declaration(scope, args.this()) {
            return page_rules::set_declaration_attribute(
                scope,
                args.this(),
                &property,
                args.get(0),
            );
        }
        if computed_styles::is_computed(scope, &state, args.this()) {
            cssom_string(scope, args.get(0), true)?;
            return Err(computed_styles::readonly_error());
        }
        let node = resolve_receiver(scope, &state, args.this(), "CSSStyleDeclaration")?;
        let value = cssom_string(scope, args.get(0), true)?;
        mutate_inline_declaration(scope, &state, node, &property, &value, "")?;
        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error)
    }
}

pub(super) fn install_properties(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let properties = state
        .layout_provider
        .borrow()
        .as_ref()
        .map(|provider| provider.supported_css_properties())
        .unwrap_or_default();
    if properties.is_empty() {
        return Ok(());
    }
    let template = state
        .instance_templates
        .borrow()
        .get("CSSStyleDeclaration")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("CSSStyleDeclaration"))?;
    // Obtain the canonical prototype from the realm's template rather than a
    // replaceable global constructor, including when the provider is refreshed.
    let object =
        v8::Local::new(scope, &template)
            .new_instance(scope)
            .ok_or(V8BindingError::Allocation(
                "CSS property prototype instance",
            ))?;
    let prototype = object
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("CSS property prototype"))?;
    for (property, canonical) in properties {
        for name in attribute_names(&property) {
            let key = v8::String::new(scope, &name)
                .ok_or(V8BindingError::Allocation("CSS property name"))?;
            if prototype
                .has_own_property(scope, key.into())
                .unwrap_or(false)
            {
                continue;
            }
            let data = v8_string(scope, &canonical)?;
            let getter = v8::Function::builder(property_getter)
                .data(data)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .ok_or(V8BindingError::Allocation("CSS property getter"))?;
            let setter = v8::Function::builder(property_setter)
                .data(data)
                .length(1)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .ok_or(V8BindingError::Allocation("CSS property setter"))?;
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            if !prototype
                .define_property(scope, key.into(), &descriptor)
                .unwrap_or(false)
            {
                return Err(V8BindingError::Allocation("CSS property descriptor"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn property_attribute_names_preserve_css_and_idl_spellings() {
        assert_eq!(
            attribute_names("padding-top"),
            BTreeSet::from(["padding-top".into(), "paddingTop".into()])
        );
        assert_eq!(
            attribute_names("-webkit-transform"),
            BTreeSet::from([
                "-webkit-transform".into(),
                "WebkitTransform".into(),
                "webkitTransform".into()
            ])
        );
        assert_eq!(attribute_names("float"), BTreeSet::from(["float".into()]));
    }
}
