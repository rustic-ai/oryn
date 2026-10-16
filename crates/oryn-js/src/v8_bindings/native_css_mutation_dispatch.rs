fn native_rule_error(error: BindingNativeRuleError) -> V8BindingError {
    let member = match error {
        BindingNativeRuleError::IndexSize => "IndexSizeError",
        BindingNativeRuleError::HierarchyRequest => "HierarchyRequestError",
        BindingNativeRuleError::Syntax => "SyntaxError",
        BindingNativeRuleError::InvalidState => "InvalidStateError",
        BindingNativeRuleError::Security => "SecurityError",
        BindingNativeRuleError::Native(message) => return V8BindingError::EventCallback(message),
    };
    V8BindingError::Unsupported {
        interface: "CSSStyleSheet".into(),
        member: member.into(),
    }
}
fn native_css_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
    interface: &str,
    member: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let sheet = native_sheet_handle(scope, state, args.this());
    let rule =
        native_css_identity(scope, args.this(), "oryn.native.css.rule").and_then(|identity| {
            state
                .native_stylesheets
                .borrow()
                .rule_records
                .get(&identity)
                .cloned()
        });
    if sheet.is_none() && rule.is_none() {
        return None;
    }
    Some((|| {
        match member {
            "insertRule" => {
                let source = cssom_string(scope, args.get(0), false)?;
                let index = if args.length() < 2 || args.get(1).is_undefined() {
                    0
                } else {
                    args.get(1)
                        .uint32_value(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                };
                // WebIDL conversions can reenter CSSOM and invalidate the
                // enclosing receipt; native validation needs a current scope.
                let _retained_scope = state.enter_retained_allocation(scope)?;
                let result = if let Some(sheet) = &sheet {
                    sheet.insert_rule(index as usize, &source)
                } else {
                    rule.as_ref()
                        .expect("native mutation owner")
                        .insert_rule(index as usize, &source)
                };
                result.map_err(native_rule_error)?;
                Ok(v8::Integer::new_from_unsigned(scope, index).into())
            }
            "deleteRule" | "removeRule" => {
                let index = args
                    .get(0)
                    .uint32_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                let _retained_scope = state.enter_retained_allocation(scope)?;
                let result = if let Some(sheet) = &sheet {
                    sheet.delete_rule(index as usize)
                } else {
                    rule.as_ref()
                        .expect("native mutation owner")
                        .delete_rule(index as usize)
                };
                result.map_err(native_rule_error)?;
                Ok(v8::undefined(scope).into())
            }
            // Retained regular sheets are not constructable sheets. Conversion
            // still precedes the native CSSOM NotAllowedError category.
            "replaceSync" | "replace" => {
                let _source = cssom_string(scope, args.get(0), false)?;
                if member == "replace" {
                    let resolver = v8::PromiseResolver::new(scope).ok_or(
                        V8BindingError::Allocation("native stylesheet replacement promise"),
                    )?;
                    let exception = dom_exception_value(scope, "NotAllowedError")?;
                    if resolver.reject(scope, exception) != Some(true) {
                        return Err(V8BindingError::JavaScriptException);
                    }
                    return Ok(resolver.get_promise(scope).into());
                }
                Err(V8BindingError::Unsupported {
                    interface: interface.into(),
                    member: "NotAllowedError".into(),
                })
            }
            _ => Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            }),
        }
    })())
}
