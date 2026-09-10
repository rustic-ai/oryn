//! Readonly, live CSS declarations backed by the layout provider.
use super::*;

pub(super) fn readonly_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "CSSStyleDeclaration".into(),
        member: "NoModificationAllowedError".into(),
    }
}

pub(super) fn is_computed(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> bool {
    if object.is_proxy() || object.internal_field_count() < 4 {
        return false;
    }
    let Some(brand) = state.computed_style_brand.borrow().clone() else {
        return false;
    };
    object
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .is_some_and(|value| value.strict_equals(v8::Local::new(scope, &brand).into()))
}

fn node(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<NodeId, V8BindingError> {
    if !is_computed(scope, state, object) {
        return Err(V8BindingError::Unsupported {
            interface: "CSSStyleDeclaration".into(),
            member: "incompatible receiver".into(),
        });
    }
    let target = object
        .get_internal_field(scope, 3)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("computed style element"))?;
    resolve_receiver(scope, state, target, "Element")
}

fn styles(
    state: &V8BindingState,
    node: NodeId,
) -> Result<BTreeMap<String, String>, V8BindingError> {
    refresh_layout(state)?;
    if !state.dom.borrow().is_connected(node)? {
        return Ok(BTreeMap::new());
    }
    let mut styles = state
        .layout
        .borrow()
        .styles
        .get(&node_key(node))
        .cloned()
        .unwrap_or_default();
    // Preserve the existing layout bridge fallback until all resolved longhands
    // are exported by Stylo. Never override a value supplied by the provider.
    if let Some(inline) = state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .attributes
        .get("style")
    {
        for (name, value) in inline_style_entries(inline) {
            // Custom values come from Stylo, including substitution and
            // guaranteed-invalid values which must remain absent here.
            if !name.starts_with("--") {
                styles.entry(name).or_insert(value);
            }
        }
    }
    if let (Some(x), Some(y)) = (styles.get("overflow-x"), styles.get("overflow-y"))
        && x == y
    {
        styles.insert("overflow".into(), x.clone());
    }
    Ok(styles)
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !matches!(
        state.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { .. }
    ) {
        return Err(V8BindingError::Unsupported {
            interface: "getComputedStyle".into(),
            member: "argument is not an Element".into(),
        });
    }
    let values = styles(state, node)?;
    let template = state
        .instance_templates
        .borrow()
        .get("CSSStyleDeclaration")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("CSSStyleDeclaration"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("computed style declaration"))?;
    let brand = if let Some(brand) = state.computed_style_brand.borrow().as_ref() {
        v8::Local::new(scope, brand)
    } else {
        v8::Symbol::new(scope, None)
    };
    *state.computed_style_brand.borrow_mut() = Some(v8::Global::new(scope, brand));
    object.set_internal_field(0, v8::Integer::new(scope, 0).into());
    object.set_internal_field(1, v8::BigInt::new_from_u64(scope, state.instance_id).into());
    object.set_internal_field(2, brand.into());
    // A traced wrapper retains the target element without a permanent Rust root.
    let target = wrap_node(scope, state, node)?;
    object.set_internal_field(3, target.into());
    for property in values.keys() {
        let mut idl = String::new();
        let mut uppercase = false;
        for character in property.trim_start_matches('-').chars() {
            if character == '-' {
                uppercase = true;
            } else if uppercase {
                idl.extend(character.to_uppercase());
                uppercase = false;
            } else {
                idl.push(character);
            }
        }
        for name in [property.as_str(), idl.as_str()] {
            if generated_interface("CSSStyleDeclaration")
                .is_some_and(|interface| interface.members.iter().any(|member| member.name == name))
            {
                continue;
            }
            let data = v8_string(scope, property)?;
            let getter = v8::Function::builder(property_getter)
                .data(data)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .ok_or(V8BindingError::Allocation("computed property getter"))?;
            let setter = v8::Function::builder(property_setter)
                .length(1)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope)
                .ok_or(V8BindingError::Allocation("computed property setter"))?;
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let name = v8::String::new(scope, name)
                .ok_or(V8BindingError::Allocation("computed property name"))?;
            object.define_property(scope, name.into(), &descriptor);
        }
    }
    Ok(object)
}

/// CSSOM lists ordinary properties before custom properties. Keep index lookup
/// live, using the same style flush as named property access and length.
pub(super) fn property_names(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<Vec<String>, V8BindingError> {
    let node = node(scope, state, object)?;
    let styles = styles(state, node)?;
    let mut names = styles
        .keys()
        .filter(|name| !name.starts_with("--"))
        .cloned()
        .collect::<Vec<_>>();
    names.extend(styles.keys().filter(|name| name.starts_with("--")).cloned());
    Ok(names)
}

pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let node = node(scope, state, object)?;
    match member {
        "parentRule" => Ok(v8::null(scope).into()),
        "cssText" => v8_string(scope, ""),
        "length" => {
            Ok(v8::Integer::new_from_unsigned(scope, styles(state, node)?.len() as u32).into())
        }
        _ => {
            let values = styles(state, node)?;
            let property = if member == "webkitTransition" {
                "transition"
            } else {
                css_property_name(member)
            };
            v8_string(scope, values.get(property).map_or("", String::as_str))
        }
    }
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let node = node(scope, state, args.this())?;
    if member == "item" {
        let Some(index) = args.get(0).uint32_value(scope) else {
            return Ok(None);
        };
        let names = property_names(scope, state, args.this())?;
        return Ok(Some(v8_string(
            scope,
            names.get(index as usize).map_or("", String::as_str),
        )?));
    }
    let Some(property) = args.get(0).to_string(scope) else {
        return Ok(None);
    };
    let mut property = property.to_rust_string_lossy(scope);
    if !property.starts_with("--") {
        property.make_ascii_lowercase();
    }
    match member {
        "getPropertyValue" => Ok(Some(v8_string(
            scope,
            styles(state, node)?
                .get(&property)
                .map_or("", String::as_str),
        )?)),
        "getPropertyPriority" => Ok(Some(v8_string(scope, "")?)),
        "setProperty" | "removeProperty" => {
            if member == "setProperty" {
                if args.get(1).to_string(scope).is_none() {
                    return Ok(None);
                }
                if args.length() > 2 && args.get(2).to_string(scope).is_none() {
                    return Ok(None);
                }
            }
            Err(readonly_error())
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "CSSStyleDeclaration".into(),
            member: member.into(),
        }),
    }
}

fn property_getter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        return;
    };
    let property = args.data().to_rust_string_lossy(scope);
    match attribute(scope, &state, args.this(), &property) {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn property_setter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        return;
    };
    if let Err(error) = node(scope, &state, args.this()) {
        throw_binding_error(scope, &error);
        return;
    }
    if args.get(0).to_string(scope).is_some() {
        throw_binding_error(scope, &readonly_error());
    }
}
