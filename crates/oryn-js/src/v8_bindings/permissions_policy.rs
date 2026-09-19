//! Policy objects read native declaration snapshots; script receives no mutation
//! handle to the actual security policy or its document authority.
use super::*;
pub(super) fn object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if let Some(object) = state.permissions_policy_objects.borrow().get(&node) {
        return object
            .to_local(scope)
            .map(Into::into)
            .ok_or(V8BindingError::Allocation("policy object"));
    }
    if state.permissions_policy_objects.borrow().len() >= 1024 {
        return Err(V8BindingError::Allocation("policy object limit"));
    }
    let id = register_platform_object(state, PlatformObjectRecord::PermissionsPolicy(node));
    let object = new_platform_object(scope, state, "PermissionsPolicy", id)?;
    let retained = context_intrinsics::retain_object_for(
        scope,
        state,
        &format!("permissions-policy-{node:?}"),
        object,
    )?;
    state
        .permissions_policy_objects
        .borrow_mut()
        .insert(node, retained);
    Ok(object.into())
}
fn snapshot(
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<crate::permissions_policy::Snapshot, V8BindingError> {
    let value = state.native_permissions_policy_snapshot()?;
    if node == state.document_root() {
        return Ok(value);
    }
    let (allow, source, srcdoc) = {
        let dom = state.dom.borrow();
        if !dom.is_element(node, "http://www.w3.org/1999/xhtml", "iframe")
            || dom.node_document(node)? != Some(state.document_root())
        {
            return Err(V8BindingError::Unsupported {
                interface: "PermissionsPolicy".into(),
                member: "InvalidStateError".into(),
            });
        }
        (
            dom.frame_permissions_policy_allow(node)?,
            dom.attribute_in_null_namespace(node, "src")?
                .unwrap_or("about:blank")
                .to_owned(),
            dom.attribute_in_null_namespace(node, "srcdoc")?.is_some(),
        )
    };
    let url = if srcdoc {
        url::Url::parse("about:srcdoc").unwrap()
    } else {
        url::Url::parse(&state.native_document_base_url(true)?)
            .ok()
            .and_then(|base| base.join(&source).ok())
            .unwrap_or_else(|| url::Url::parse("about:blank").unwrap())
    };
    let origin = if url.scheme() == "about" {
        value.origin.clone()
    } else {
        url.origin().ascii_serialization()
    };
    Ok(value.iframe(origin, url.to_string(), allow))
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let node = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::PermissionsPolicy(node) = &*record else {
            return Err(V8BindingError::Allocation("policy receiver"));
        };
        *node
    };
    let feature = if matches!(member, "allowsFeature" | "getAllowlistForFeature") {
        Some(required_string(scope, args, 0)?)
    } else {
        None
    };
    let origin = if member == "allowsFeature" && args.length() > 1 && !args.get(1).is_undefined() {
        Some(required_string(scope, args, 1)?)
    } else {
        None
    };
    let snapshot = snapshot(state, node)?;
    if member == "allowsFeature" {
        return Ok(Some(
            v8::Boolean::new(
                scope,
                snapshot.allows(feature.as_deref().unwrap_or(""), origin.as_deref()),
            )
            .into(),
        ));
    }
    let values = match member {
        "features" => crate::permissions_policy::FEATURES
            .iter()
            .map(|value| value.to_string())
            .collect(),
        "allowedFeatures" => crate::permissions_policy::FEATURES
            .iter()
            .filter(|feature| snapshot.allows(feature, None))
            .map(|value| value.to_string())
            .collect(),
        "getAllowlistForFeature" => snapshot.allowlist(feature.as_deref().unwrap_or("")),
        _ => return Err(V8BindingError::Allocation("policy operation")),
    };
    let array = v8::Array::new(scope, values.len() as i32);
    for (index, value) in values.iter().enumerate() {
        let value = v8_string(scope, value)?;
        let key = v8::String::new(scope, &index.to_string())
            .ok_or(V8BindingError::Allocation("policy list index"))?;
        array
            .create_data_property(scope, key.into(), value)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(Some(array.into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn policy_wrappers_keep_identity_and_observe_native_and_container_changes() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<iframe id=f src='https://child.test/frame' allowfullscreen></iframe>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let native = Rc::new(RefCell::new(crate::permissions_policy::Snapshot {
            origin: "https://top.test".into(),
            container: false,
            constraints: vec![crate::permissions_policy::Constraint::Header {
                origin: "https://top.test".into(),
                headers: Vec::new(),
            }],
        }));
        let snapshot = native.clone();
        state
            .set_native_permissions_policy_snapshot(Rc::new(move || Ok(snapshot.borrow().clone())));
        assert!(eval(scope,"globalThis.policy=document.permissionsPolicy;globalThis.f=document.getElementById('f');policy===document.permissionsPolicy && f.permissionsPolicy===f.permissionsPolicy && policy.features().includes('picture-in-picture') && policy.allowsFeature('picture-in-picture','https://child.test') && f.permissionsPolicy.allowsFeature('fullscreen')").is_true());
        assert!(eval(scope,"f.allow=\"fullscreen 'none'; picture-in-picture 'none'\";!f.permissionsPolicy.allowsFeature('fullscreen') && !f.permissionsPolicy.allowsFeature('picture-in-picture') && policy.allowsFeature('picture-in-picture')").is_true());
        if let crate::permissions_policy::Constraint::Header { headers, .. } =
            &mut native.borrow_mut().constraints[0]
        {
            headers.push(("permissions-policy".into(), "picture-in-picture=()".into()));
        }
        assert!(eval(scope,"policy===document.permissionsPolicy && !policy.allowsFeature('picture-in-picture') && !policy.allowedFeatures().includes('picture-in-picture') && policy.getAllowlistForFeature('picture-in-picture').length===0 && !policy.allowsFeature('not-a-feature')").is_true());
    }
}
