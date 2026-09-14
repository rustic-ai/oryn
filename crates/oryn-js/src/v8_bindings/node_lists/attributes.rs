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
    let attribute = argument_node(scope, state, args.get(0))?;
    if !matches!(
        state.dom.borrow().arena().get(attribute)?.kind,
        NodeKind::Attribute { .. }
    ) {
        return Err(error("TypeError: expected Attr"));
    }
    let result = if member == "removeAttributeNode" {
        state
            .dom
            .borrow_mut()
            .remove_attribute_node(element, attribute)
            .map_err(dom_error)?;
        Some(attribute)
    } else {
        state
            .dom
            .borrow_mut()
            .set_attribute_node(element, attribute)
            .map_err(dom_error)?
    };
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
    let attribute = if member.ends_with("NS") {
        let namespace = if args.get(0).is_null_or_undefined() {
            None
        } else {
            Some(required_string(scope, args, 0)?)
        };
        let local = required_string(scope, args, 1)?;
        state
            .dom
            .borrow_mut()
            .attribute_node_ns(element, namespace.as_deref(), &local)?
    } else {
        let name = normalized_attribute_name(state, element, required_string(scope, args, 0)?)?;
        state.dom.borrow_mut().attribute_node(element, &name)?
    };
    if member.starts_with("remove") {
        let attribute = attribute.ok_or_else(|| error("NotFoundError"))?;
        state
            .dom
            .borrow_mut()
            .remove_attribute_node(element, attribute)
            .map_err(dom_error)?;
    }
    match attribute {
        Some(attribute) => Ok(wrap_node(scope, state, attribute)?.into()),
        None => Ok(v8::null(scope).into()),
    }
}
