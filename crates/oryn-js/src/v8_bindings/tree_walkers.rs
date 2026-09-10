//! Live traversal of the canonical light tree, with callback state kept out of
//! DOM borrows. Algorithms: https://dom.spec.whatwg.org/#interface-treewalker
use super::*;

const ACCEPT: u16 = 1;
const REJECT: u16 = 2;
const SKIP: u16 = 3;

pub(super) struct Record {
    root: NodeId,
    current: Cell<NodeId>,
    what_to_show: u32,
    active: Cell<bool>,
    _wrapper: v8::Weak<v8::Object>,
}

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "TreeWalker".into(),
        member: "incompatible receiver".into(),
    }
}

fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name =
        v8::String::new(scope, name).ok_or(V8BindingError::Allocation("TreeWalker private key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

fn record(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<Rc<Record>, V8BindingError> {
    if object.is_proxy() {
        return Err(invalid());
    }
    let key = key(scope, "oryn.treewalker.id")?;
    let id = object
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
        .ok_or_else(invalid)?;
    state
        .tree_walkers
        .borrow()
        .get(&id)
        .cloned()
        .ok_or_else(invalid)
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
    what_to_show: u32,
    filter: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !filter.is_null_or_undefined() && !filter.is_object() {
        return Err(V8BindingError::Unsupported {
            interface: "NodeFilter".into(),
            member: "filter must be an object".into(),
        });
    }
    let id = state.next_traversal_id.get();
    state.next_traversal_id.set(id.saturating_add(1));
    let object = new_stateful_object(scope, state, "TreeWalker", id)?;
    let id_key = key(scope, "oryn.treewalker.id")?;
    let id_value = v8::BigInt::new_from_u64(scope, id);
    object
        .set_private(scope, id_key, id_value.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let root_object = wrap_node(scope, state, root)?;
    for name in ["oryn.treewalker.root", "oryn.treewalker.current"] {
        let key = key(scope, name)?;
        object
            .set_private(scope, key, root_object.into())
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    let filter_key = key(scope, "oryn.treewalker.filter")?;
    let filter = if filter.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        filter
    };
    object
        .set_private(scope, filter_key, filter)
        .ok_or(V8BindingError::JavaScriptException)?;
    let weak_state = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.tree_walkers.borrow_mut().remove(&id);
            }
        }),
    );
    state.tree_walkers.borrow_mut().insert(
        id,
        Rc::new(Record {
            root,
            current: Cell::new(root),
            what_to_show,
            active: Cell::new(false),
            _wrapper: wrapper,
        }),
    );
    Ok(object)
}

pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let record = record(scope, state, object)?;
    let name = match member {
        "root" => "oryn.treewalker.root",
        "currentNode" => "oryn.treewalker.current",
        "filter" => "oryn.treewalker.filter",
        "whatToShow" => {
            return Ok(v8::Integer::new_from_unsigned(scope, record.what_to_show).into());
        }
        _ => return Err(invalid()),
    };
    let key = key(scope, name)?;
    object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}

pub(super) fn set_current(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let record = record(scope, state, object)?;
    let node = argument_node(scope, state, value)?;
    let key = key(scope, "oryn.treewalker.current")?;
    object
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    record.current.set(node);
    Ok(())
}

fn parent(state: &V8BindingState, node: NodeId) -> Result<Option<NodeId>, V8BindingError> {
    let dom = state.dom.borrow();
    if matches!(
        dom.arena().get(node)?.kind,
        NodeKind::Document | NodeKind::ShadowRoot { .. }
    ) {
        return Ok(None);
    }
    Ok(dom.parent(node)?)
}

fn child(
    state: &V8BindingState,
    node: NodeId,
    reverse: bool,
) -> Result<Option<NodeId>, V8BindingError> {
    let nodes = light_children(&state.dom.borrow(), node)?;
    Ok(if reverse { nodes.last() } else { nodes.first() }.copied())
}

fn sibling(
    state: &V8BindingState,
    node: NodeId,
    reverse: bool,
) -> Result<Option<NodeId>, V8BindingError> {
    let Some(parent) = parent(state, node)? else {
        return Ok(None);
    };
    let nodes = light_children(&state.dom.borrow(), parent)?;
    let Some(index) = nodes.iter().position(|candidate| *candidate == node) else {
        return Ok(None);
    };
    Ok(if reverse {
        index.checked_sub(1)
    } else {
        index.checked_add(1)
    }
    .and_then(|index| nodes.get(index).copied()))
}

fn filter(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    record: &Record,
    node: NodeId,
) -> Result<u16, V8BindingError> {
    if record.active.get() {
        return Err(V8BindingError::Unsupported {
            interface: "TreeWalker".into(),
            member: "InvalidStateError".into(),
        });
    }
    let node_type = traversal_node_type(&state.dom.borrow(), node)?;
    if record.what_to_show & (1 << (node_type - 1)) == 0 {
        return Ok(SKIP);
    }
    let filter_key = key(scope, "oryn.treewalker.filter")?;
    let callback = object
        .get_private(scope, filter_key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if callback.is_null() {
        return Ok(ACCEPT);
    }
    record.active.set(true);
    let result = (|| {
        let callback_object = v8::Local::<v8::Object>::try_from(callback).map_err(|_| invalid())?;
        let argument = wrap_node(scope, state, node)?;
        let context = callback_object
            .get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("NodeFilter callback realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let (function, receiver) = if let Ok(function) =
            v8::Local::<v8::Function>::try_from(callback)
        {
            (function, v8::undefined(scope).into())
        } else {
            let receiver = v8::Local::<v8::Object>::try_from(callback).map_err(|_| invalid())?;
            let name = v8::String::new(scope, "acceptNode")
                .ok_or(V8BindingError::Allocation("NodeFilter operation"))?;
            let value = receiver
                .get(scope, name.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            let function = v8::Local::<v8::Function>::try_from(value).map_err(|_| {
                throw_type_error(scope, "NodeFilter.acceptNode must be callable");
                V8BindingError::JavaScriptException
            })?;
            (function, receiver.into())
        };
        let value = function
            .call(scope, receiver, &[argument.into()])
            .ok_or(V8BindingError::JavaScriptException)?;
        Ok(value
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as u16)
    })();
    record.active.set(false);
    result
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let record = record(scope, state, object)?;
    let found = walk(scope, state, object, &record, member)?;
    let Some(node) = found else {
        return Ok(Some(v8::null(scope).into()));
    };
    let value = wrap_node(scope, state, node)?;
    let key = key(scope, "oryn.treewalker.current")?;
    object
        .set_private(scope, key, value.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    record.current.set(node);
    Ok(Some(value.into()))
}

fn walk(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    record: &Record,
    member: &str,
) -> Result<Option<NodeId>, V8BindingError> {
    let mut node = record.current.get();
    match member {
        "parentNode" => {
            while node != record.root {
                let Some(next) = parent(state, node)? else {
                    break;
                };
                node = next;
                if filter(scope, state, object, record, node)? == ACCEPT {
                    return Ok(Some(node));
                }
            }
        }
        "firstChild" | "lastChild" => {
            let reverse = member == "lastChild";
            let Some(first) = child(state, node, reverse)? else {
                return Ok(None);
            };
            node = first;
            loop {
                match filter(scope, state, object, record, node)? {
                    ACCEPT => return Ok(Some(node)),
                    SKIP => {
                        if let Some(next) = child(state, node, reverse)? {
                            node = next;
                            continue;
                        }
                    }
                    _ => {}
                }
                loop {
                    if let Some(next) = sibling(state, node, reverse)? {
                        node = next;
                        break;
                    }
                    let Some(next) = parent(state, node)? else {
                        return Ok(None);
                    };
                    if next == record.root || next == record.current.get() {
                        return Ok(None);
                    }
                    node = next;
                }
            }
        }
        "nextSibling" | "previousSibling" => {
            if node == record.root {
                return Ok(None);
            }
            let reverse = member == "previousSibling";
            loop {
                let mut next = sibling(state, node, reverse)?;
                while let Some(candidate) = next {
                    node = candidate;
                    let result = filter(scope, state, object, record, node)?;
                    if result == ACCEPT {
                        return Ok(Some(node));
                    }
                    next = child(state, node, reverse)?;
                    if result == REJECT || next.is_none() {
                        next = sibling(state, node, reverse)?;
                    }
                }
                let Some(next) = parent(state, node)? else {
                    return Ok(None);
                };
                node = next;
                if node == record.root || filter(scope, state, object, record, node)? == ACCEPT {
                    return Ok(None);
                }
            }
        }
        "previousNode" => {
            while node != record.root {
                let mut next = sibling(state, node, true)?;
                while let Some(candidate) = next {
                    node = candidate;
                    let mut result = filter(scope, state, object, record, node)?;
                    while result != REJECT {
                        let Some(next) = child(state, node, true)? else {
                            break;
                        };
                        node = next;
                        result = filter(scope, state, object, record, node)?;
                    }
                    if result == ACCEPT {
                        return Ok(Some(node));
                    }
                    next = sibling(state, node, true)?;
                }
                if node == record.root {
                    return Ok(None);
                }
                let Some(next) = parent(state, node)? else {
                    return Ok(None);
                };
                node = next;
                if filter(scope, state, object, record, node)? == ACCEPT {
                    return Ok(Some(node));
                }
            }
        }
        "nextNode" => {
            let mut result = ACCEPT;
            loop {
                while result != REJECT {
                    let Some(next) = child(state, node, false)? else {
                        break;
                    };
                    node = next;
                    result = filter(scope, state, object, record, node)?;
                    if result == ACCEPT {
                        return Ok(Some(node));
                    }
                }
                let mut ancestor = node;
                node = loop {
                    if ancestor == record.root {
                        return Ok(None);
                    }
                    if let Some(next) = sibling(state, ancestor, false)? {
                        break next;
                    }
                    let Some(next) = parent(state, ancestor)? else {
                        return Ok(None);
                    };
                    ancestor = next;
                };
                result = filter(scope, state, object, record, node)?;
                if result == ACCEPT {
                    return Ok(Some(node));
                }
            }
        }
        _ => return Err(invalid()),
    }
    Ok(None)
}
