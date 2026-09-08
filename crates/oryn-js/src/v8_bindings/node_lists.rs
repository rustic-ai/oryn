//! Native NodeList ownership, indexed properties, and live child-node views.
use super::*;
mod named;

#[derive(Clone)]
pub(super) enum ElementFilter {
    Children,
    WindowName {
        name: String,
    },
    Tag {
        name: String,
        html: bool,
    },
    Namespace {
        namespace: Option<String>,
        local: String,
    },
    Classes {
        names: Vec<String>,
        quirks: bool,
    },
}

#[derive(Clone)]
enum Contents {
    Children(NodeId),
    Named { root: NodeId, name: String },
    Snapshot(Vec<NodeId>),
    Elements { root: NodeId, filter: ElementFilter },
}

struct List {
    contents: Contents,
    interface: &'static str,
    roots: Vec<NodeId>,
    handle: v8::Weak<v8::Object>,
    // Preserve previously exposed snapshot node wrappers just as an ordinary
    // sequence did, while the collection owns their canonical native identity.
    wrappers: BTreeMap<NodeId, v8::Weak<v8::Object>>,
    roots_dirty: bool,
    bytes: i64,
}

#[derive(Default)]
pub(super) struct Store {
    next: Cell<u64>,
    lists: RefCell<BTreeMap<u64, List>>,
    children: RefCell<BTreeMap<NodeId, u64>>,
    element_children: RefCell<BTreeMap<NodeId, u64>>,
    collection_brand: RefCell<Option<v8::Global<v8::Symbol>>>,
    brand: RefCell<Option<v8::Global<v8::Symbol>>>,
    memory_delta: Cell<i64>,
    static_template: RefCell<Option<v8::Global<v8::ObjectTemplate>>>,
}

fn invalid_receiver() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "NodeList".into(),
        member: "incompatible receiver".into(),
    }
}

fn identity(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<u64, V8BindingError> {
    if object.is_proxy() || object.internal_field_count() < 3 {
        return Err(invalid_receiver());
    }
    let actual = object
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .ok_or_else(invalid_receiver)?;
    let brand = state.node_lists.brand.borrow();
    let collection_brand = state.node_lists.collection_brand.borrow();
    if ![brand.as_ref(), collection_brand.as_ref()]
        .into_iter()
        .flatten()
        .any(|brand| actual.strict_equals(v8::Local::new(scope, brand).into()))
    {
        return Err(invalid_receiver());
    }
    let id = stateful_id(scope, object)?;
    if !state.node_lists.lists.borrow().contains_key(&id) {
        return Err(invalid_receiver());
    }
    Ok(id)
}

fn identity_as(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<u64, V8BindingError> {
    let id = identity(scope, state, object)?;
    if state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .is_none_or(|list| list.interface != interface)
    {
        return Err(invalid_receiver());
    }
    Ok(id)
}

/// HTML named access includes all element IDs, plus names on four HTML types.
/// https://html.spec.whatwg.org/multipage/nav-history-apis.html#named-access-on-the-window-object
pub(super) fn window_named_match(dom: &DomRuntime, node: NodeId, wanted: &str) -> bool {
    if wanted.is_empty() {
        return false;
    }
    let Ok(record) = dom.arena().get(node) else {
        return false;
    };
    let NodeKind::Element { name } = &record.kind else {
        return false;
    };
    let matches_attribute = |key: &str| {
        record
            .attributes
            .get(key)
            .is_some_and(|value| value == wanted)
            && dom
                .arena()
                .existing_attribute_node(node, key)
                .is_none_or(|attribute| {
                    dom.arena().get(attribute).is_ok_and(|attribute| {
                        matches!(&attribute.kind,
                    NodeKind::Attribute { namespace, .. } if namespace.is_none())
                    })
                })
    };
    matches_attribute("id")
        || (name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
            && matches!(name.local.as_ref(), "embed" | "form" | "img" | "object")
            && matches_attribute("name"))
}

/// Resolve against the current light tree on every access. The same selection
/// drives Window's single-element result and its live duplicate collection.
pub(super) fn window_named_nodes(
    dom: &DomRuntime,
    root: NodeId,
    wanted: &str,
) -> Result<Vec<NodeId>, V8BindingError> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    Ok(traversal_nodes(dom, root)?
        .into_iter()
        .skip(1)
        .filter(|node| window_named_match(dom, *node, wanted))
        .collect())
}

fn contents(state: &V8BindingState, id: u64) -> Result<Vec<NodeId>, V8BindingError> {
    let value = state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .map(|list| list.contents.clone())
        .ok_or_else(invalid_receiver)?;
    let (root, nodes) = match value {
        Contents::Children(root) => (root, light_children(&state.dom.borrow(), root)?),
        Contents::Named { root, name } => {
            let dom = state.dom.borrow();
            let mut nodes = Vec::new();
            for node in traversal_nodes(&dom, root)?.into_iter().skip(1) {
                if matches!(&dom.arena().get(node)?.kind,
                    NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
                    && dom.attribute(node, "name")? == Some(name.as_str())
                {
                    nodes.push(node);
                }
            }
            (root, nodes)
        }
        Contents::Elements {
            root,
            filter: ElementFilter::WindowName { name },
        } => (root, window_named_nodes(&state.dom.borrow(), root, &name)?),
        Contents::Elements { root, filter } => {
            let dom = state.dom.borrow();
            let candidates = if matches!(&filter, ElementFilter::Children) {
                light_children(&dom, root)?
            } else {
                traversal_nodes(&dom, root)?.into_iter().skip(1).collect()
            };
            let nodes = candidates
                .into_iter()
                .filter(|id| {
                    let Ok(record) = dom.arena().get(*id) else {
                        return false;
                    };
                    let NodeKind::Element { name } = &record.kind else {
                        return false;
                    };
                    match &filter {
                        ElementFilter::Children => true,
                        ElementFilter::WindowName { name } => window_named_match(&dom, *id, name),
                        ElementFilter::Tag { name: wanted, html } => {
                            let qualified = name.prefix.as_ref().map_or_else(
                                || name.local.to_string(),
                                |prefix| format!("{prefix}:{}", name.local),
                            );
                            wanted == "*"
                                || if *html && name.ns.as_ref() == "http://www.w3.org/1999/xhtml" {
                                    qualified == wanted.to_ascii_lowercase()
                                } else {
                                    qualified == *wanted
                                }
                        }
                        ElementFilter::Namespace { namespace, local } => {
                            (namespace.as_deref() == Some("*")
                                || name.ns.as_ref() == namespace.as_deref().unwrap_or(""))
                                && (local == "*" || name.local.as_ref() == local)
                        }
                        ElementFilter::Classes {
                            names: wanted,
                            quirks,
                        } => {
                            !wanted.is_empty()
                                && wanted.iter().all(|wanted| {
                                    record.attributes.get("class").is_some_and(|classes| {
                                        classes.split_ascii_whitespace().any(|class| {
                                            if *quirks {
                                                class.eq_ignore_ascii_case(wanted)
                                            } else {
                                                class == wanted
                                            }
                                        })
                                    })
                                })
                        }
                    }
                })
                .collect();
            (root, nodes)
        }
        Contents::Snapshot(nodes) => return Ok(nodes),
    };
    let members = nodes.iter().copied().collect::<BTreeSet<_>>();
    let mut lists = state.node_lists.lists.borrow_mut();
    let list = lists.get_mut(&id).ok_or_else(invalid_receiver)?;
    let previous = list.wrappers.len();
    list.wrappers
        .retain(|node, _| *node == root || members.contains(node));
    let released =
        ((previous - list.wrappers.len()) * std::mem::size_of::<v8::Weak<v8::Object>>()) as i64;
    list.bytes -= released;
    list.roots_dirty |= released != 0;
    state
        .node_lists
        .memory_delta
        .set(state.node_lists.memory_delta.get() - released);
    Ok(nodes)
}

// The list's internal field traces its wrappers. Native strong handles would
// leak cycles such as node.savedList = node.childNodes on detached nodes.
fn sync_wrapper_roots(scope: &mut v8::PinScope, state: &V8BindingState, id: u64) {
    let (object, values) = {
        let mut lists = state.node_lists.lists.borrow_mut();
        let Some(list) = lists.get_mut(&id) else {
            return;
        };
        if !list.roots_dirty {
            return;
        }
        let Some(object) = list.handle.to_local(scope) else {
            return;
        };
        let values = list
            .wrappers
            .values()
            .filter_map(|wrapper| wrapper.to_local(scope))
            .map(Into::into)
            .collect::<Vec<v8::Local<v8::Value>>>();
        list.roots_dirty = false;
        (object, values)
    };
    let roots = v8::Array::new_with_elements(scope, &values);
    object.set_internal_field(3, roots.into());
}

// Static lists never change membership. Read their native vector in place;
// copying the whole collection for each indexed read makes iteration quadratic.
fn node_count(state: &V8BindingState, id: u64) -> Result<usize, V8BindingError> {
    {
        let lists = state.node_lists.lists.borrow();
        let list = lists.get(&id).ok_or_else(invalid_receiver)?;
        if let Contents::Snapshot(nodes) = &list.contents {
            return Ok(nodes.len());
        }
    }
    Ok(contents(state, id)?.len())
}

fn node_id_at(
    state: &V8BindingState,
    id: u64,
    index: u32,
) -> Result<Option<NodeId>, V8BindingError> {
    {
        let lists = state.node_lists.lists.borrow();
        let list = lists.get(&id).ok_or_else(invalid_receiver)?;
        if let Contents::Snapshot(nodes) = &list.contents {
            return Ok(nodes.get(index as usize).copied());
        }
    }
    Ok(contents(state, id)?.get(index as usize).copied())
}

pub(super) fn collect(state: &V8BindingState) -> Result<(), BindingError> {
    let (Ok(mut lists), Ok(mut dom)) = (
        state.node_lists.lists.try_borrow_mut(),
        state.dom.try_borrow_mut(),
    ) else {
        return Ok(());
    };
    let dead = lists
        .iter()
        .filter_map(|(id, list)| list.handle.is_empty().then_some(*id))
        .collect::<Vec<_>>();
    for id in dead {
        let list = lists.remove(&id).expect("enumerated list");
        if let Contents::Children(root) = &list.contents {
            let cached_id = state.node_lists.children.borrow().get(root).copied();
            if cached_id == Some(id) {
                state.node_lists.children.borrow_mut().remove(root);
            }
        }
        if let Contents::Elements {
            root,
            filter: ElementFilter::Children,
        } = list.contents
            && state
                .node_lists
                .element_children
                .borrow()
                .get(&root)
                .copied()
                == Some(id)
        {
            state.node_lists.element_children.borrow_mut().remove(&root);
        }
        for root in list.roots {
            dom.release_native(root)?;
        }
        state
            .node_lists
            .memory_delta
            .set(state.node_lists.memory_delta.get() - list.bytes);
    }
    Ok(())
}

pub(super) fn account_memory(scope: &mut v8::PinScope, state: &V8BindingState) {
    let delta = state.node_lists.memory_delta.replace(0);
    if delta != 0 {
        scope.adjust_amount_of_external_allocated_memory(delta);
    }
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    value: Contents,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let count = state.node_lists.lists.borrow().len();
    if count.is_multiple_of(64) {
        collect(state)?;
    }
    let child_cache = match &value {
        Contents::Children(root) => Some((*root, &state.node_lists.children)),
        Contents::Elements {
            root,
            filter: ElementFilter::Children,
        } => Some((*root, &state.node_lists.element_children)),
        _ => None,
    };
    if let Some((root, cache)) = child_cache {
        let cached_id = cache.borrow().get(&root).copied();
        let cached = cached_id.and_then(|id| {
            state
                .node_lists
                .lists
                .borrow()
                .get(&id)
                .and_then(|list| list.handle.to_local(scope))
        });
        if let Some(object) = cached {
            return Ok(object.into());
        }
    }
    let interface = if matches!(&value, Contents::Elements { .. }) {
        "HTMLCollection"
    } else {
        "NodeList"
    };
    let id = state.node_lists.next.get().saturating_add(1);
    state.node_lists.next.set(id);
    let mut object = new_stateful_object(scope, state, interface, id)?;
    if matches!(&value, Contents::Snapshot(_)) {
        // Immutable membership can use ordinary V8 indexed data properties.
        // Keep native write/delete hooks for the legacy platform-object rules,
        // without paying a Rust callback on every read or Array#indexOf step.
        let prototype = object
            .get_prototype(scope)
            .ok_or(V8BindingError::Allocation("NodeList prototype"))?;
        let template = {
            let mut cached = state.node_lists.static_template.borrow_mut();
            if cached.is_none() {
                let template = v8::ObjectTemplate::new(scope);
                template.set_internal_field_count(6);
                template.set_indexed_property_handler(
                    v8::IndexedPropertyHandlerConfiguration::new()
                        .setter(indexed_setter)
                        .definer(indexed_definer)
                        .deleter(indexed_deleter),
                );
                *cached = Some(v8::Global::new(scope, template));
            }
            v8::Local::new(scope, cached.as_ref().expect("static NodeList template"))
        };
        object = template
            .new_instance(scope)
            .ok_or(V8BindingError::Allocation("static NodeList object"))?;
        object.set_prototype(scope, prototype);
        object.set_internal_field(4, v8::null(scope).into());
        object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
        object.set_internal_field(1, v8::BigInt::new_from_u64(scope, state.instance_id).into());
    }
    let brand = {
        let mut brand = if interface == "HTMLCollection" {
            state.node_lists.collection_brand.borrow_mut()
        } else {
            state.node_lists.brand.borrow_mut()
        };
        if brand.is_none() {
            let symbol = v8::Symbol::new(scope, None);
            *brand = Some(v8::Global::new(scope, symbol));
        }
        v8::Local::new(scope, brand.as_ref().expect("list brand"))
    };
    object.set_internal_field(2, brand.into());
    let nodes = match &value {
        Contents::Children(root)
        | Contents::Named { root, .. }
        | Contents::Elements { root, .. } => vec![*root],
        Contents::Snapshot(nodes) => nodes.clone(),
    };
    let roots = nodes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut wrappers = BTreeMap::new();
    let mut wrapper_roots = Vec::new();
    // Holding a live list also holds its owner wrapper; static results hold
    // their members. Both retain expandos across detached-wrapper collection.
    for (index, node) in nodes.iter().enumerate() {
        let wrapper = wrap_node(scope, state, *node)?;
        if matches!(&value, Contents::Snapshot(_)) {
            let key = v8::String::new(scope, &index.to_string())
                .ok_or(V8BindingError::Allocation("NodeList index"))?;
            if object.define_own_property(
                scope,
                key.into(),
                wrapper.into(),
                v8::PropertyAttribute::READ_ONLY,
            ) != Some(true)
            {
                return Err(V8BindingError::Allocation("NodeList indexed value"));
            }
        }
        wrappers.insert(*node, v8::Weak::new(scope, wrapper));
        wrapper_roots.push(wrapper.into());
    }
    let traced_roots = v8::Array::new_with_elements(scope, &wrapper_roots);
    object.set_internal_field(3, traced_roots.into());
    object.set_internal_field(
        4,
        v8::Boolean::new(scope, matches!(&value, Contents::Snapshot(_))).into(),
    );
    {
        let mut dom = state.dom.borrow_mut();
        for node in &roots {
            dom.retain_native(*node)?;
        }
    }
    let bytes = (std::mem::size_of::<List>()
        + roots.len()
            * (std::mem::size_of::<NodeId>() + std::mem::size_of::<v8::Weak<v8::Object>>())
        + match &value {
            Contents::Snapshot(nodes) => nodes.len() * std::mem::size_of::<NodeId>(),
            Contents::Children(_) => 0,
            Contents::Named { name, .. } => name.len(),
            Contents::Elements { filter, .. } => match filter {
                ElementFilter::Children => 0,
                ElementFilter::WindowName { name } => name.len(),
                ElementFilter::Tag { name, .. } => name.len(),
                ElementFilter::Namespace { namespace, local } => {
                    namespace.as_ref().map_or(0, String::len) + local.len()
                }
                ElementFilter::Classes { names: classes, .. } => classes
                    .iter()
                    .map(|name| std::mem::size_of::<String>() + name.len())
                    .sum(),
            },
        }) as i64;
    if let Some((root, cache)) = child_cache {
        cache.borrow_mut().insert(root, id);
    }
    state.node_lists.lists.borrow_mut().insert(
        id,
        List {
            contents: value,
            interface,
            roots,
            handle: v8::Weak::new(scope, object),
            wrappers,
            roots_dirty: false,
            bytes,
        },
    );
    state
        .node_lists
        .memory_delta
        .set(state.node_lists.memory_delta.get() + bytes);
    account_memory(scope, state);
    Ok(object.into())
}

pub(super) fn elements<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
    filter: ElementFilter,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Elements { root, filter })
}

pub(super) fn children<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Children(root))
}

pub(super) fn named<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
    name: String,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Named { root, name })
}

pub(super) fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    nodes: &[NodeId],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Snapshot(nodes.to_vec()))
}

fn node_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    index: u32,
) -> Result<Option<v8::Local<'s, v8::Object>>, V8BindingError> {
    let node = node_id_at(state, id, index)?;
    sync_wrapper_roots(scope, state, id);
    let Some(node) = node else {
        return Ok(None);
    };
    let cached = state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .and_then(|list| list.wrappers.get(&node))
        .and_then(|object| object.to_local(scope));
    if let Some(object) = cached {
        return Ok(Some(object));
    }
    let object = wrap_node(scope, state, node)?;
    // Live children remain rooted by their parent; the cached wrapper keeps
    // identity while the corresponding child remains in this list.
    {
        let mut lists = state.node_lists.lists.borrow_mut();
        let list = lists.get_mut(&id).expect("live list");
        list.wrappers.insert(node, v8::Weak::new(scope, object));
        list.roots_dirty = true;
        let bytes = std::mem::size_of::<v8::Weak<v8::Object>>() as i64;
        list.bytes += bytes;
        state
            .node_lists
            .memory_delta
            .set(state.node_lists.memory_delta.get() + bytes);
    }
    sync_wrapper_roots(scope, state, id);
    account_memory(scope, state);
    Ok(Some(object))
}

pub(super) fn length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = identity_as(scope, state, object, interface)?;
    let count = node_count(state, id)?;
    sync_wrapper_roots(scope, state, id);
    Ok(v8::Integer::new_from_unsigned(scope, count as u32).into())
}

pub(super) fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    interface: &str,
    value: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = identity_as(scope, state, object, interface)?;
    let Some(index) = value.uint32_value(scope) else {
        return Ok(v8::undefined(scope).into());
    };
    Ok(node_at(scope, state, id, index)?.map_or_else(|| v8::null(scope).into(), Into::into))
}

fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid_receiver)?;
    let id = identity(scope, &state, object)?;
    Ok((state, id))
}

// Indexed callbacks are installed only on native NodeList instances. A static
// list can project its immutable, traced wrapper vector without resolving its
// realm and native registry on every property read. The vector is inaccessible
// to script; length shadowing still affects only the public length getter.
fn static_members<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let flag = object.get_internal_field(scope, 4)?;
    if !v8::Local::<v8::Value>::try_from(flag).ok()?.is_true() {
        return None;
    }
    v8::Local::<v8::Array>::try_from(object.get_internal_field(scope, 3)?).ok()
}

pub(super) fn static_length(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Option<u32> {
    // Only native static NodeLists carry this inaccessible boolean tag and
    // traced array. Ordinary/proxy receivers still take the full brand check.
    if object.is_proxy() || object.internal_field_count() < 5 {
        return None;
    }
    static_members(scope, object).map(|members| members.length())
}

fn indexed_getter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    if let Some(members) = static_members(scope, args.holder()) {
        if index >= members.length() {
            return v8::Intercepted::kNo;
        }
        if let Some(value) = members.get_index(scope, index) {
            rv.set(value)
        }
        return v8::Intercepted::kYes;
    }
    match owner(scope, args.holder()).and_then(|(state, id)| node_at(scope, &state, id, index)) {
        Ok(Some(node)) => {
            rv.set(node.into());
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn indexed_query(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    if let Some(members) = static_members(scope, args.holder()) {
        if index >= members.length() {
            return v8::Intercepted::kNo;
        }
        rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    match owner(scope, args.holder()).and_then(|(state, id)| {
        let count = node_count(&state, id)?;
        sync_wrapper_roots(scope, &state, id);
        Ok(count)
    }) {
        Ok(count) if (index as usize) < count => {
            rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
            v8::Intercepted::kYes
        }
        Ok(_) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}

fn indexed_enumerator(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Array>,
) {
    match owner(scope, args.holder()).and_then(|(state, id)| {
        let nodes = contents(&state, id)?;
        sync_wrapper_roots(scope, &state, id);
        Ok(nodes)
    }) {
        Ok(nodes) => {
            let values = (0..nodes.len())
                .map(|i| v8::Integer::new_from_unsigned(scope, i as u32).into())
                .collect::<Vec<_>>();
            rv.set(v8::Array::new_with_elements(scope, &values));
        }
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn reject_write(
    scope: &mut v8::PinScope,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    if args.should_throw_on_error() {
        throw_type_error(scope, "NodeList indexed properties are read-only");
    }
    v8::Intercepted::kYes
}

fn indexed_setter(
    scope: &mut v8::PinScope,
    _: u32,
    _: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if args
        .holder()
        .get_internal_field(scope, 4)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .is_some_and(|value| value.is_null())
    {
        return v8::Intercepted::kNo;
    }
    reject_write(scope, args, rv)
}
fn indexed_definer(
    scope: &mut v8::PinScope,
    _: u32,
    _: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    // Before the object is published, its immutable indexed values are
    // populated by the native constructor. The private flag is then sealed.
    if args
        .holder()
        .get_internal_field(scope, 4)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .is_some_and(|value| value.is_null())
    {
        return v8::Intercepted::kNo;
    }
    reject_write(scope, args, rv)
}
fn indexed_deleter(
    scope: &mut v8::PinScope,
    index: u32,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    match owner(scope, args.holder()).and_then(|(state, id)| {
        let count = node_count(&state, id)?;
        sync_wrapper_roots(scope, &state, id);
        Ok(count)
    }) {
        Ok(count) => {
            let allowed = index as usize >= count;
            rv.set_bool(allowed);
            if !allowed && args.should_throw_on_error() {
                throw_type_error(scope, "NodeList indexed properties cannot be deleted");
            }
        }
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}

pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_getter)
            .query(indexed_query)
            .enumerator(indexed_enumerator)
            .setter(indexed_setter)
            .definer(indexed_definer)
            .deleter(indexed_deleter),
    );
}

#[cfg(test)]
pub(super) fn retained_count(state: &V8BindingState) -> usize {
    state.node_lists.lists.borrow().len()
}

pub(super) fn named_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    named::item(scope, state, object, value)
}

pub(super) fn configure_named(instance: v8::Local<v8::ObjectTemplate>) {
    named::configure(instance);
}

#[cfg(test)]
mod window_named_tests {
    use super::*;

    #[test]
    fn window_named_selection_tracks_connection_order_and_renaming() {
        let mut dom = DomRuntime::new();
        let root = dom.document();
        let host = dom.create_element_with_local_name("http://www.w3.org/1999/xhtml", "div");
        let first = dom.create_element_with_local_name("http://www.w3.org/1999/xhtml", "img");
        let second = dom.create_element_with_local_name("http://www.w3.org/1999/xhtml", "form");
        dom.set_attribute(first, "id", "shared").unwrap();
        dom.set_attribute(first, "name", "shared").unwrap();
        dom.set_attribute(second, "name", "shared").unwrap();
        dom.append_child(host, first).unwrap();
        dom.append_child(host, second).unwrap();
        assert!(window_named_nodes(&dom, root, "shared").unwrap().is_empty());
        dom.append_child(root, host).unwrap();
        assert_eq!(
            window_named_nodes(&dom, root, "shared").unwrap(),
            vec![first, second]
        );
        dom.append_child(host, first).unwrap();
        assert_eq!(
            window_named_nodes(&dom, root, "shared").unwrap(),
            vec![second, first]
        );
        dom.set_attribute(second, "name", "renamed").unwrap();
        assert_eq!(
            window_named_nodes(&dom, root, "shared").unwrap(),
            vec![first]
        );
        assert_eq!(
            window_named_nodes(&dom, root, "renamed").unwrap(),
            vec![second]
        );
        dom.remove_child(root, host).unwrap();
        assert!(window_named_nodes(&dom, root, "shared").unwrap().is_empty());
        assert!(
            window_named_nodes(&dom, root, "renamed")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn window_named_filter_uses_current_unqualified_attributes_and_allowed_names() {
        let mut dom = DomRuntime::new();
        for tag in ["embed", "form", "img", "object", "div", "input", "iframe"] {
            let node = dom.create_element_with_local_name("http://www.w3.org/1999/xhtml", tag);
            dom.set_attribute(node, "name", "named").unwrap();
            assert_eq!(
                window_named_match(&dom, node, "named"),
                matches!(tag, "embed" | "form" | "img" | "object")
            );
            dom.set_attribute(node, "id", "identified").unwrap();
            assert!(window_named_match(&dom, node, "identified"));
            dom.remove_attribute(node, "id").unwrap();
            assert!(!window_named_match(&dom, node, "identified"));
        }
        let svg = dom.create_element_with_local_name("http://www.w3.org/2000/svg", "svg");
        dom.set_attribute(svg, "id", "vector").unwrap();
        assert!(window_named_match(&dom, svg, "vector"));
        dom.set_attribute_ns(svg, Some("urn:test".into()), "id", "foreign")
            .unwrap();
        assert!(!window_named_match(&dom, svg, "foreign"));
        assert!(!window_named_match(&dom, svg, ""));
    }
}
