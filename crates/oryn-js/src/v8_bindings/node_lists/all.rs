//! HTMLAllCollection lookup algorithms over native live collection ownership.
use super::*;

fn root(state: &V8BindingState, id: u64) -> Result<NodeId, V8BindingError> {
    let lists = state.node_lists.lists.borrow();
    match lists.get(&id).map(|list| &list.contents) {
        Some(Contents::All(root)) => Ok(*root),
        _ => Err(invalid_receiver()),
    }
}

pub(super) fn named_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    document: NodeId,
    name: String,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let nodes = {
        let dom = state.dom.borrow();
        traversal_nodes(&dom, document)?
            .into_iter()
            .skip(1)
            .filter(|node| all_named_match(&dom, *node, &name))
            .collect::<Vec<_>>()
    };
    match nodes.as_slice() {
        [] => Ok(v8::null(scope).into()),
        [node] => Ok(wrap_node(scope, state, *node)?.into()),
        _ => elements(scope, state, document, ElementFilter::AllName { name }),
    }
}

fn array_index(name: &str) -> Option<u32> {
    name.parse::<u32>()
        .ok()
        .filter(|index| *index != u32::MAX && index.to_string() == name)
}

pub(in crate::v8_bindings) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
    named_only: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    identity_as(scope, state, object, "HTMLAllCollection")?;
    // Optional DOMString arguments treat explicit undefined as absent. The
    // required namedItem argument instead converts it to the string undefined.
    if !named_only && value.is_undefined() {
        return Ok(v8::null(scope).into());
    }
    let Some(name) = value.to_string(scope) else {
        return Ok(v8::undefined(scope).into());
    };
    let name = rust_string_preserving_utf16(scope, name)?;
    let (owner, id) = owner(scope, object)?;
    let state = &owner;
    let document = root(state, id)?;
    let context = object
        .get_creation_context(scope)
        .ok_or_else(invalid_receiver)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    if !named_only && let Some(index) = array_index(&name) {
        return Ok(
            node_at(scope, state, id, index)?.map_or_else(|| v8::null(scope).into(), Into::into)
        );
    }
    named_value(scope, state, document, name)
}

fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    if args.is_construct_call() {
        throw_type_error(scope, "HTMLAllCollection is not a constructor");
        return;
    }
    // V8's object-template call handler passes the callable object itself as
    // receiver, independent of the JavaScript call's thisArgument.
    let result = owner(scope, args.this())
        .and_then(|(state, _)| operation(scope, &state, args.this(), args.get(0), false));
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub(in crate::v8_bindings) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    use v8::MapFnTo;
    super::configure(instance);
    named::configure(instance);
    crate::v8_context_bridge::configure_html_all_collection(instance, call.map_fn_to());
}
