//! NamedNodeMap shares the native live-list owner and traced wrapper graph.
use super::*;

pub(in crate::v8_bindings) fn map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    element: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Attributes(element))
}

fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "NamedNodeMap".into(),
        member: name.into(),
    }
}

fn dom_error(value: DomError) -> V8BindingError {
    match value {
        DomError::InUseAttribute => error("InUseAttributeError"),
        DomError::AttributeNotFound => error("NotFoundError"),
        value => value.into(),
    }
}

pub(in crate::v8_bindings) fn element_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    element: NodeId,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let receiver = wrap_node(scope, state, element)?;
    let receiver = v8::Global::new(scope, receiver);
    let (mut source, mut attribute) =
        node_identity::argument(scope, args.get(0))?.ok_or_else(|| error("TypeError"))?;
    let (data, owner) = match &source.dom.borrow().arena().get(attribute)?.kind {
        NodeKind::Attribute { data, owner } => (data.clone(), *owner),
        _ => return Err(error("TypeError")),
    };
    if member == "removeAttributeNode" && !Rc::ptr_eq(&source.dom, &state.dom) {
        return Err(error("NotFoundError"));
    }
    if owner.is_some() && (!Rc::ptr_eq(&source.dom, &state.dom) || owner != Some(element)) {
        return Err(error(if member == "removeAttributeNode" {
            "NotFoundError"
        } else {
            "InUseAttributeError"
        }));
    }
    let mut converted = None;
    if member != "removeAttributeNode"
        && let Some(kind) = native_trusted_attribute_type(
            state,
            element,
            &data.local_name,
            data.namespace.as_deref(),
        )?
    {
        let value = v8_string(scope, &data.value)?;
        converted = Some(trusted_string(
            scope,
            state,
            value,
            kind,
            &native_trusted_attribute_sink(state, element, &data.local_name)?,
        )?);
    }
    // Default policy conversion can adopt the receiver or the Attr. Resolve
    // both original objects again and retain exact native ownership.
    let receiver = v8::Local::new(scope, &receiver);
    let (state, element) =
        node_identity::argument(scope, receiver.into())?.ok_or_else(|| error("TypeError"))?;
    let state = &state;
    (source, attribute) =
        node_identity::argument(scope, args.get(0))?.ok_or_else(|| error("TypeError"))?;
    let owner = match source.dom.borrow().arena().get(attribute)?.kind {
        NodeKind::Attribute { owner, .. } => owner,
        _ => return Err(error("TypeError")),
    };
    if owner.is_some() && (!Rc::ptr_eq(&source.dom, &state.dom) || owner != Some(element)) {
        return Err(error(if member == "removeAttributeNode" {
            "NotFoundError"
        } else {
            "InUseAttributeError"
        }));
    }
    let result = if member == "removeAttributeNode" {
        if !Rc::ptr_eq(&source.dom, &state.dom) {
            return Err(error("NotFoundError"));
        }
        state
            .dom
            .borrow_mut()
            .remove_attribute_node(element, attribute)
            .map_err(dom_error)?;
        Some(attribute)
    } else {
        if let Some(value) = converted {
            source
                .dom
                .borrow_mut()
                .set_attribute_node_value(attribute, value)?;
        }
        if !Rc::ptr_eq(&source.dom, &state.dom) {
            attribute = node_migration::adopt(
                scope,
                state,
                owner_document_for_node(state, element)?,
                &source,
                attribute,
            )?;
        }
        state
            .dom
            .borrow_mut()
            .set_attribute_node(element, attribute)
            .map_err(dom_error)?
    };
    let data = match &state.dom.borrow().arena().get(attribute)?.kind {
        NodeKind::Attribute { data, .. } => data.clone(),
        _ => return Err(error("TypeError: expected Attr")),
    };
    attribute_resource_changed(
        scope,
        state,
        element,
        data.namespace.as_deref(),
        &data.local_name,
    )?;
    match result {
        Some(attribute) => Ok(wrap_node(scope, state, attribute)?.into()),
        None => Ok(v8::null(scope).into()),
    }
}

pub(in crate::v8_bindings) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = identity_as(scope, state, object, "NamedNodeMap")?;
    let element = match state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .map(|list| &list.contents)
    {
        Some(Contents::Attributes(element)) => *element,
        _ => return Err(invalid_receiver()),
    };
    if matches!(member, "setNamedItem" | "setNamedItemNS") {
        return element_operation(scope, state, element, "setAttributeNode", args);
    }
    let (namespace, name) = if member.ends_with("NS") {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        (namespace, required_string(scope, args, 1)?)
    } else {
        (None, required_string(scope, args, 0)?)
    };
    let (owner, id) = owner(scope, object)?;
    let state = &owner;
    let element = match state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .map(|list| &list.contents)
    {
        Some(Contents::Attributes(element)) => *element,
        _ => return Err(invalid_receiver()),
    };
    let attribute = if member.ends_with("NS") {
        state
            .dom
            .borrow_mut()
            .attribute_node_ns(element, namespace.as_deref(), &name)?
    } else {
        let name = normalized_attribute_name(state, element, name)?;
        state.dom.borrow_mut().attribute_node(element, &name)?
    };
    if member.starts_with("remove") {
        let attribute = attribute.ok_or_else(|| error("NotFoundError"))?;
        let data = match &state.dom.borrow().arena().get(attribute)?.kind {
            NodeKind::Attribute { data, .. } => data.clone(),
            _ => return Err(error("TypeError: expected Attr")),
        };
        state
            .dom
            .borrow_mut()
            .remove_attribute_node(element, attribute)
            .map_err(dom_error)?;
        attribute_resource_changed(
            scope,
            state,
            element,
            data.namespace.as_deref(),
            &data.local_name,
        )?;
    }
    match attribute {
        Some(attribute) => Ok(wrap_node(scope, state, attribute)?.into()),
        None => Ok(v8::null(scope).into()),
    }
}
