fn wrap_native_rule_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    rule: v8::Local<v8::Object>,
    handle: &Rc<dyn BindingNativeRule>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Ok(style) = private_object(scope, rule, "oryn.native.css.declaration") {
        return Ok(style);
    }
    let view = handle
        .view()
        .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
    let interface = match view.rule_type {
        1 | 24 => "CSSStyleProperties",
        5 => "CSSFontFaceDescriptors",
        6 => "CSSPageDescriptors",
        _ => "CSSStyleDeclaration",
    };
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let style = v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("native CSS declaration"))?;
    bind_native_css_owner(scope, state, style)?;
    checked_native_css_edge(
        scope,
        style,
        "oryn.native.css.declaration.rule",
        rule.into(),
    )?;
    checked_native_css_edge(scope, rule, "oryn.native.css.declaration", style.into())?;
    Ok(style)
}

fn native_declaration_rule(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Option<Rc<dyn BindingNativeRule>> {
    let rule = private_object(scope, object, "oryn.native.css.declaration.rule").ok()?;
    let identity = native_css_identity(scope, rule, "oryn.native.css.rule")?;
    state
        .native_stylesheets
        .borrow()
        .rule_records
        .get(&identity)
        .cloned()
}



fn native_declaration_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let handle = native_declaration_rule(scope, state, object)?;
    Some((|| {
        if member == "parentRule" {
            return Ok(private_object(scope, object, "oryn.native.css.declaration.rule")?.into());
        }
        let property = if matches!(member, "cssText" | "length") {
            ""
        } else {
            css_property_name(member)
        };
        let view = handle
            .declaration(property)
            .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
        match member {
            "cssText" => v8_string(scope, &view.css_text),
            "length" => {
                let length = u32::try_from(view.names.len())
                    .map_err(|_| V8BindingError::Allocation("native declaration length"))?;
                Ok(v8::Integer::new_from_unsigned(scope, length).into())
            }
            _ => v8_string(scope, &view.value),
        }
    })())
}

fn native_declaration_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let handle = native_declaration_rule(scope, state, args.this())?;
    Some((|| {
        if method == "item" {
            let index = args
                .get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let view = handle
                .declaration("")
                .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
            return v8_string(
                scope,
                view.names.get(index as usize).map_or("", String::as_str),
            );
        }
        let property = cssom_string(scope, args.get(0), false)?;
        match method {
            "getPropertyValue" | "getPropertyPriority" => {
                let view = handle
                    .declaration(&property)
                    .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
                v8_string(
                    scope,
                    if method == "getPropertyValue" {
                        &view.value
                    } else if view.important {
                        "important"
                    } else {
                        ""
                    },
                )
            }
            "removeProperty" => {
                let mutation = handle
                    .set_property(&property, "", "")
                    .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
                v8_string(scope, &mutation.previous_value)
            }
            "setProperty" => {
                let value = cssom_string(scope, args.get(1), true)?;
                let priority = if args.length() < 3 || args.get(2).is_undefined() {
                    String::new()
                } else {
                    cssom_string(scope, args.get(2), true)?
                };
                handle
                    .set_property(&property, &value, &priority)
                    .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
                Ok(v8::undefined(scope).into())
            }
            _ => Err(V8BindingError::Unsupported {
                interface: "CSSStyleDeclaration".into(),
                member: method.into(),
            }),
        }
    })())
}

fn native_declaration_set_attribute(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    input: v8::Local<v8::Value>,
) -> Option<Result<(), V8BindingError>> {
    let handle = native_declaration_rule(scope, state, object)?;
    Some((|| {
        let replace = member == "cssText";
        let value = cssom_string(scope, input, !replace)?;
        if replace {
            handle
                .set_declaration_text(&value)
                .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
        } else {
            handle
                .set_property(css_property_name(member), &value, "")
                .map_err(|error| native_css_read_error("CSSStyleDeclaration", error))?;
        }
        Ok(())
    })())
}
