fn native_sheet_handle(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Option<Rc<dyn BindingNativeStylesheet>> {
    let identity = native_css_identity(scope, object, "oryn.native.css.sheet")?;
    state
        .native_stylesheets
        .borrow()
        .records
        .get(&identity)
        .cloned()
}

fn native_sheet_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let handle = native_sheet_handle(scope, state, object)?;
    Some((|| match member {
        "disabled" => Ok(v8::Boolean::new(
            scope,
            handle
                .disabled()
                .map_err(|error| native_css_read_error("StyleSheet", error))?,
        )
        .into()),
        "media" => Ok(wrap_native_sheet_media(scope, state, object)?.into()),
        "title" => match handle
            .title()
            .map_err(|error| native_css_read_error("StyleSheet", error))?
        {
            Some(title) => v8_string(scope, &title),
            None => Ok(v8::null(scope).into()),
        },
        "cssRules" | "rules" => {
            // Always perform the current native security check even if the
            // SameObject wrapper already exists.
            if !handle.origin_clean() {
                return Err(V8BindingError::Unsupported {
                    interface: "CSSStyleSheet".into(),
                    member: "SecurityError".into(),
                });
            }
            Ok(wrap_native_rule_list(scope, state, object, true)?.into())
        }
        "href" => match handle.href() {
            Some(href) => v8_string(scope, &href),
            None => Ok(v8::null(scope).into()),
        },
        "type" => v8_string(scope, "text/css"),
        "ownerNode" => match handle
            .owner_node()
            .map_err(|error| native_css_read_error("CSSStyleSheet", error))?
        {
            Some(owner) => wrap_node(scope, state, owner).map(Into::into),
            None => Ok(v8::null(scope).into()),
        },
        "parentStyleSheet" => match handle
            .parent()
            .map_err(|error| native_css_read_error("CSSStyleSheet", error))?
        {
            Some(parent) => {
                let parent = wrap_native_stylesheet(scope, state, parent)?;
                checked_native_css_edge(scope, object, "oryn.native.css.parent", parent.into())?;
                Ok(parent.into())
            }
            None => {
                clear_native_css_edge(scope, object, "oryn.native.css.parent")?;
                Ok(v8::null(scope).into())
            }
        },
        // Imported sheets have no DOM owner element. A top-level native owner
        // requires the provider's actual owner-node field; do not invent one.
        _ => Err(V8BindingError::Unsupported {
            interface: "CSSStyleSheet".into(),
            member: member.into(),
        }),
    })())
}
