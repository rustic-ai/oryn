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
    // Query original list membership; no stale positional or captured parent.
    match member {
        "nameList" => native_layer_name_list(scope, &handle),
        "style" => Ok(wrap_native_rule_declaration(scope, state, object, &handle)?.into()),
        "type" => {
            let view = handle
                .view()
                .map_err(|error| native_css_read_error("CSSRule", error))?;
            Ok(v8::Integer::new_from_unsigned(scope, u32::from(view.rule_type)).into())
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
