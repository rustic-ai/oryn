fn wrap_native_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    handle: Rc<dyn BindingNativeRule>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Some(import) = handle
        .as_import()
        .map_err(|error| native_css_read_error("CSSRule", error))?
    {
        return wrap_native_import(scope, state, import);
    }
    let identity = handle.identity();
    let existing = state
        .native_stylesheets
        .borrow()
        .rules
        .get(&identity)
        .and_then(|wrapper| wrapper.to_local(scope));
    if let Some(object) = existing {
        return Ok(object);
    }
    let view = handle
        .view()
        .map_err(|error| native_css_read_error("CSSRule", error))?;
    let interface = match view.rule_type {
        1 => "CSSStyleRule",
        4 => "CSSMediaRule",
        5 => "CSSFontFaceRule",
        6 => "CSSPageRule",
        10 => "CSSNamespaceRule",
        12 => "CSSSupportsRule",
        17 => "CSSLayerStatementRule",
        18 => "CSSContainerRule",
        16 => "CSSLayerBlockRule",
        21 => "CSSScopeRule",
        22 => "CSSStartingStyleRule",
        24 => "CSSNestedDeclarations",
        13 | 26 => "CSSGroupingRule",
        _ => "CSSRule",
    };
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let object = v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("native CSS rule wrapper"))?;
    bind_native_css_owner(scope, state, object)?;
    let value = v8::BigInt::new_from_u64(scope, identity);
    checked_native_css_edge(scope, object, "oryn.native.css.rule", value.into())?;
    let weak_state = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak_state.upgrade() {
                let removed = {
                    let mut wrappers = state.native_stylesheets.borrow_mut();
                    wrappers.rules.remove(&identity);
                    wrappers.rule_records.remove(&identity)
                };
                drop(removed);
            }
        }),
    );
    let mut wrappers = state.native_stylesheets.borrow_mut();
    wrappers.rule_records.insert(identity, handle);
    wrappers.rules.insert(identity, wrapper);
    Ok(object)
}

fn native_rule_relationship<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let invalid = || V8BindingError::Unsupported {
        interface: "CSSRule".into(),
        member: "incompatible receiver".into(),
    };
    let identity =
        native_css_identity(scope, object, "oryn.native.css.rule").ok_or_else(invalid)?;
    let handle = state
        .native_stylesheets
        .borrow()
        .rule_records
        .get(&identity)
        .cloned()
        .ok_or_else(invalid)?;
    let kind=handle.view().map_err(|e|native_css_read_error(interface,e))?.rule_type;
    if !page_rules::supports_interface(kind,interface) {return Err(invalid());}
    // Query original list membership; no stale positional or captured parent.
    match member {
        "media" => Ok(wrap_native_import_media(scope, state, object)?.into()),
        "matches" => {
            let view = handle.view().map_err(|e| native_css_read_error(interface, e))?;
            Ok(v8::Boolean::new(scope, view.properties.get("matches")
                .is_some_and(|value| value == "true")).into())
        }
        "containerName" | "containerQuery" => {
            let view = handle.view().map_err(|e| native_css_read_error(interface, e))?;
            v8_string(scope, view.properties.get(member).map_or("", String::as_str))
        }
        "start" | "end" => {
            let view = handle.view().map_err(|e| native_css_read_error(interface, e))?;
            match view.properties.get(member) {
                Some(value) => v8_string(scope, value),
                None => Ok(v8::null(scope).into()),
            }
        }
        "conditions" => {
            let view = handle.view().map_err(|e| native_css_read_error(interface, e))?;
            let mut values = Vec::new();
            values.try_reserve_exact(view.conditions.len())
                .map_err(|_| V8BindingError::Allocation("native CSS conditions"))?;
            for (name, query) in &view.conditions {
                let condition = v8::Object::new(scope);
                for (key, value) in [("name", name), ("query", query)] {
                    let key = v8::String::new(scope, key)
                        .ok_or(V8BindingError::Allocation("native CSS condition key"))?;
                    let value = v8_string(scope, value)?;
                    if condition.create_data_property(scope, key.into(), value) != Some(true) {
                        return Err(V8BindingError::JavaScriptException);
                    }
                }
                values.push(condition.into());
            }
            let array = v8::Array::new_with_elements(scope, &values);
            if array.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
                return Err(V8BindingError::JavaScriptException);
            }
            Ok(array.into())
        }
        "nameList" => native_layer_name_list(scope, &handle),
        "style" => Ok(wrap_native_rule_declaration(scope, state, object, &handle)?.into()),
        "type" => {
            let view = handle
                .view()
                .map_err(|error| native_css_read_error("CSSRule", error))?;
            Ok(v8::Integer::new_from_unsigned(scope, u32::from(if view.rule_type == 24 { 0 } else { view.rule_type })).into())
        }
        "cssText" | "selectorText" | "conditionText" | "name" | "namespaceURI" | "prefix" => {
            let view = handle
                .view()
                .map_err(|error| native_css_read_error("CSSRule", error))?;
            let value = match member {
                "cssText" => view.css_text,
                "selectorText" => view.selector_text.unwrap_or_default(),
                _ => view
                    .properties
                    .get(member)
                    .cloned()
                    .or_else(|| {
                        view.conditions
                            .iter()
                            .find(|(name, _)| name == member)
                            .map(|(_, value)| value.clone())
                    })
                    .unwrap_or_default(),
            };
            v8_string(scope, &value)
        }
        "parentStyleSheet" => match handle
            .sheet()
            .map_err(|error| native_css_read_error("CSSRule", error))?
        {
            Some(sheet) => Ok(wrap_native_stylesheet(scope, state, sheet)?.into()),
            None => Ok(v8::null(scope).into()),
        },
        "parentRule" => match handle
            .parent_rule()
            .map_err(|error| native_css_read_error("CSSRule", error))?
        {
            Some(rule) => Ok(wrap_native_rule(scope, state, rule)?.into()),
            None => Ok(v8::null(scope).into()),
        },
        "cssRules" => {
            handle
                .child_count()
                .map_err(|error| native_css_read_error("CSSRule", error))?;
            Ok(wrap_native_rule_list(scope, state, object, false)?.into())
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "CSSRule".into(),
            member: member.into(),
        }),
    }
}
