//! Native NodeList ownership, indexed properties, and live child-node views.
use super::*;
pub(super) mod all;
pub(super) mod attributes;
mod migration;
mod named;
pub(super) use migration::{Prepared as MigrationPrepared, capture_migration};

#[derive(Clone)]
pub(super) enum ElementFilter {
    Children,
    SelectedOptions,
    Options,
    Links,
    WindowName {
        name: String,
    },
    AllName {
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
    StyleSheets(NodeId),
    Attributes(NodeId),
    Fieldset(NodeId),
    Form(NodeId),
    Radios { form: NodeId, name: String },
    All(NodeId),
    Children(NodeId),
    Labels(NodeId),
    Named { root: NodeId, name: String },
    Snapshot(Vec<NodeId>),
    Elements { root: NodeId, filter: ElementFilter },
}

#[derive(Clone)]
struct ForeignNode {
    owner: std::rc::Weak<V8BindingState>,
    dom: SharedDom,
    node: NodeId,
}

struct List {
    foreign: BTreeMap<NodeId, ForeignNode>,
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
    attributes: RefCell<BTreeMap<NodeId, u64>>,
    stylesheets: RefCell<BTreeMap<NodeId, u64>>,
    children: RefCell<BTreeMap<NodeId, u64>>,
    labels: RefCell<BTreeMap<NodeId, u64>>,
    forms: RefCell<BTreeMap<NodeId, u64>>,
    fieldsets: RefCell<BTreeMap<NodeId, u64>>,
    element_children: RefCell<BTreeMap<NodeId, u64>>,
    selected_options: RefCell<BTreeMap<NodeId, u64>>,
    options: RefCell<BTreeMap<NodeId, u64>>,
    collection_brand: RefCell<Option<v8::Global<v8::Symbol>>>,
    all_brand: RefCell<Option<v8::Global<v8::Symbol>>>,
    all: RefCell<BTreeMap<NodeId, u64>>,
    brand: RefCell<Option<v8::Global<v8::Symbol>>>,
    memory_delta: Cell<i64>,
    static_template: RefCell<Option<v8::Weak<v8::ObjectTemplate>>>,
}

thread_local! {static ROOT_RELEASES:RefCell<Vec<(std::rc::Weak<RefCell<DomRuntime>>,NodeId)>>=const{RefCell::new(Vec::new())};}
fn drain_root_releases() -> Result<(), BindingError> {
    ROOT_RELEASES.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut index = 0;
        while index < pending.len() {
            let Some(dom) = pending[index].0.upgrade() else {
                pending.swap_remove(index);
                continue;
            };
            let Ok(mut dom) = dom.try_borrow_mut() else {
                index += 1;
                continue;
            };
            dom.release_native(pending[index].1)?;
            pending.swap_remove(index);
        }
        Ok(())
    })
}
pub(super) fn release_root(dom: &SharedDom, node: NodeId) {
    let released = dom
        .try_borrow_mut()
        .is_ok_and(|mut dom| dom.release_native(node).is_ok());
    if !released {
        ROOT_RELEASES.with(|pending| pending.borrow_mut().push((Rc::downgrade(dom), node)));
    }
}
fn retire_list(list: List, default: &SharedDom) {
    for key in list.roots {
        let (dom, node) = list
            .foreign
            .get(&key)
            .map(|reference| (&reference.dom, reference.node))
            .unwrap_or((default, key));
        release_root(dom, node);
    }
}
pub(super) fn retire_store(state: &mut V8BindingState) {
    let lists = std::mem::take(state.node_lists.lists.get_mut());
    for (_, list) in lists {
        retire_list(list, &state.dom);
    }
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
    let all_brand = state.node_lists.all_brand.borrow();
    if ![
        brand.as_ref(),
        collection_brand.as_ref(),
        all_brand.as_ref(),
    ]
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
    if state.node_lists.lists.borrow().get(&id).is_none_or(|list| {
        list.interface != interface
            && !matches!(
                (interface, list.interface),
                (
                    "HTMLCollection",
                    "HTMLFormControlsCollection" | "HTMLOptionsCollection"
                ) | ("NodeList", "RadioNodeList")
            )
    }) {
        return Err(invalid_receiver());
    }
    Ok(id)
}

/// HTML named access includes all element IDs, plus names on four HTML types.
/// https://html.spec.whatwg.org/multipage/nav-history-apis.html#named-access-on-the-window-object
pub(super) fn window_named_match(dom: &DomRuntime, node: NodeId, wanted: &str) -> bool {
    named_element_match(dom, node, wanted, false)
}

pub(super) fn all_named_match(dom: &DomRuntime, node: NodeId, wanted: &str) -> bool {
    named_element_match(dom, node, wanted, true)
}

fn named_element_match(dom: &DomRuntime, node: NodeId, wanted: &str, all: bool) -> bool {
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
            .namespaced(None, key)
            .is_some_and(|entry| entry.value == wanted)
    };
    matches_attribute("id")
        || (name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
            && (matches!(name.local.as_ref(), "embed" | "form" | "img" | "object")
                || (all
                    && matches!(
                        name.local.as_ref(),
                        "a" | "button"
                            | "frame"
                            | "frameset"
                            | "iframe"
                            | "input"
                            | "map"
                            | "meta"
                            | "select"
                            | "textarea"
                    )))
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
        Contents::StyleSheets(root) => {
            let candidates = {
                let dom = state.dom.borrow();
                let mut candidates = dom
                    .arena()
                    .get(root)?
                    .children
                    .iter()
                    .copied()
                    .filter(|node| dom.arena().xml_stylesheet(*node).ok().flatten().is_some())
                    .collect::<Vec<_>>();
                candidates.extend(dom.query_selector_all(root, "style, link")?);
                candidates
            };
            let mut sheets = Vec::new();
            for candidate in candidates {
                if stylesheet_ownership::available(state, candidate)? {
                    sheets.push(candidate);
                }
            }
            (root, sheets)
        }
        Contents::Attributes(root) => {
            let keys = state
                .dom
                .borrow()
                .arena()
                .get(root)?
                .attributes
                .records()
                .map(|entry| (entry.namespace.clone(), entry.local_name.clone()))
                .collect::<Vec<_>>();
            let mut dom = state.dom.borrow_mut();
            let nodes = keys
                .into_iter()
                .map(|(namespace, local)| {
                    dom.attribute_node_ns(root, namespace.as_deref(), &local)
                        .and_then(|node| node.ok_or(DomError::StaleNode(root)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            (root, nodes)
        }
        Contents::Fieldset(root) => (root, state.dom.borrow().built_in_fieldset_controls(root)?),
        Contents::Form(form) => (form, state.dom.borrow().built_in_form_controls(form)?),
        Contents::Radios { form, name } => (
            form,
            state
                .dom
                .borrow()
                .named_built_in_form_controls(form, &name)?,
        ),
        Contents::All(root) => {
            let dom = state.dom.borrow();
            let nodes = traversal_nodes(&dom, root)?
                .into_iter()
                .skip(1)
                .filter(|node| {
                    dom.arena()
                        .get(*node)
                        .is_ok_and(|record| matches!(record.kind, NodeKind::Element { .. }))
                })
                .collect();
            (root, nodes)
        }
        Contents::Children(root) => (root, light_children(&state.dom.borrow(), root)?),
        Contents::Labels(control) => {
            let dom = state.dom.borrow();
            let mut nodes = Vec::new();
            if dom.is_labelable(control) {
                let root = dom.control_tree_root(control)?;
                for node in traversal_nodes(&dom, root)? {
                    if dom.is_element(node, "http://www.w3.org/1999/xhtml", "label")
                        && dom.label_control(node)? == Some(control)
                    {
                        nodes.push(node);
                    }
                }
            }
            (control, nodes)
        }
        Contents::Named { root, name } => {
            let dom = state.dom.borrow();
            let mut nodes = Vec::new();
            for node in traversal_nodes(&dom, root)?.into_iter().skip(1) {
                if matches!(&dom.arena().get(node)?.kind,
                    NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
                    && dom.attribute_in_null_namespace(node, "name")? == Some(name.as_str())
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
            } else if matches!(
                &filter,
                ElementFilter::SelectedOptions | ElementFilter::Options
            ) {
                dom.select_options(root)?
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
                        ElementFilter::SelectedOptions => record.selected(),
                        ElementFilter::Options => true,
                        ElementFilter::Links => {
                            name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                                && matches!(name.local.as_ref(), "a" | "area")
                                && record.attributes.namespaced(None, "href").is_some()
                        }
                        ElementFilter::WindowName { name } => window_named_match(&dom, *id, name),
                        ElementFilter::AllName { name } => all_named_match(&dom, *id, name),
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
                                    record.attributes.get_null("class").is_some_and(|classes| {
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
        // Immutable indexed properties already trace every snapshot member.
        // A second member array adds storage without extending any lifetime.
        if matches!(&list.contents, Contents::Snapshot(_)) {
            list.roots_dirty = false;
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

pub(super) fn checkpoint_releases_pending(state: &V8BindingState) -> bool {
    ROOT_RELEASES.with(|queue| queue.try_borrow().map_or(true, |queue| !queue.is_empty()))
        || state.node_lists.lists.try_borrow().map_or(true, |lists| {
            lists.values().any(|list| list.handle.is_empty())
        })
}

pub(super) fn collect(state: &V8BindingState) -> Result<(), BindingError> {
    drain_root_releases()?;
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
        // Release one lease at a time and retain the rest of the record on
        // borrow/authority failure. Migration may split a snapshot across DOMs.
        let list = lists.get_mut(&id).expect("enumerated list");
        let mut blocked = false;
        while let Some(root) = list.roots.last().copied() {
            if let Some(reference) = list.foreign.get(&root) {
                if Rc::ptr_eq(&reference.dom, &state.dom) {
                    dom.release_native(reference.node)?;
                } else {
                    let Ok(mut target) = reference.dom.try_borrow_mut() else {
                        blocked = true;
                        break;
                    };
                    target.release_native(reference.node)?;
                }
            } else {
                dom.release_native(root)?;
            }
            list.roots.pop();
        }
        if blocked {
            continue;
        }
        let list = lists.remove(&id).expect("enumerated list");
        if let Contents::Elements {
            root,
            filter: ElementFilter::Options,
        } = &list.contents
            && state.node_lists.options.borrow().get(root).copied() == Some(id)
        {
            state.node_lists.options.borrow_mut().remove(root);
        }
        if let Contents::Elements {
            root,
            filter: ElementFilter::SelectedOptions,
        } = &list.contents
            && state
                .node_lists
                .selected_options
                .borrow()
                .get(root)
                .copied()
                == Some(id)
        {
            state.node_lists.selected_options.borrow_mut().remove(root);
        }
        if let Contents::Fieldset(root) = &list.contents
            && state.node_lists.fieldsets.borrow().get(root).copied() == Some(id)
        {
            state.node_lists.fieldsets.borrow_mut().remove(root);
        }
        if let Contents::Form(root) = &list.contents
            && state.node_lists.forms.borrow().get(root).copied() == Some(id)
        {
            state.node_lists.forms.borrow_mut().remove(root);
        }
        if let Contents::All(root) = &list.contents {
            let cached_id = state.node_lists.all.borrow().get(root).copied();
            if cached_id == Some(id) {
                state.node_lists.all.borrow_mut().remove(root);
            }
        }
        if let Contents::Children(root) = &list.contents {
            let cached_id = state.node_lists.children.borrow().get(root).copied();
            if cached_id == Some(id) {
                state.node_lists.children.borrow_mut().remove(root);
            }
        }
        if let Contents::StyleSheets(root) = &list.contents
            && state.node_lists.stylesheets.borrow().get(root).copied() == Some(id)
        {
            state.node_lists.stylesheets.borrow_mut().remove(root);
        }
        if let Contents::Attributes(root) = &list.contents
            && state.node_lists.attributes.borrow().get(root).copied() == Some(id)
        {
            state.node_lists.attributes.borrow_mut().remove(root);
        }
        if let Contents::Labels(root) = &list.contents {
            let cached_id = state.node_lists.labels.borrow().get(root).copied();
            if cached_id == Some(id) {
                state.node_lists.labels.borrow_mut().remove(root);
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

pub(super) fn style_sheets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::StyleSheets(document))
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
        Contents::StyleSheets(root) => Some((*root, &state.node_lists.stylesheets)),
        Contents::Attributes(root) => Some((*root, &state.node_lists.attributes)),
        Contents::Elements {
            root,
            filter: ElementFilter::Options,
        } => Some((*root, &state.node_lists.options)),
        Contents::Elements {
            root,
            filter: ElementFilter::SelectedOptions,
        } => Some((*root, &state.node_lists.selected_options)),
        Contents::Fieldset(root) => Some((*root, &state.node_lists.fieldsets)),
        Contents::Form(root) => Some((*root, &state.node_lists.forms)),
        Contents::All(root) => Some((*root, &state.node_lists.all)),
        Contents::Children(root) => Some((*root, &state.node_lists.children)),
        Contents::Labels(root) => Some((*root, &state.node_lists.labels)),
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
    let interface = match &value {
        Contents::StyleSheets(_) => "StyleSheetList",
        Contents::Attributes(_) => "NamedNodeMap",
        Contents::Elements {
            filter: ElementFilter::Options,
            ..
        } => "HTMLOptionsCollection",
        Contents::Form(_) => "HTMLFormControlsCollection",
        Contents::Radios { .. } => "RadioNodeList",
        Contents::All(_) => "HTMLAllCollection",
        Contents::Elements { .. } | Contents::Fieldset(_) => "HTMLCollection",
        _ => "NodeList",
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
            if let Some(template) = cached
                .as_ref()
                .and_then(|template| template.to_local(scope))
            {
                template
            } else {
                let template = v8::ObjectTemplate::new(scope);
                template.set_internal_field_count(6);
                template.set_indexed_property_handler(
                    v8::IndexedPropertyHandlerConfiguration::new()
                        .setter(indexed_setter)
                        .definer(indexed_definer)
                        .deleter(indexed_deleter),
                );
                *cached = Some(v8::Weak::new(scope, template));
                template
            }
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
        let mut brand = if interface == "HTMLAllCollection" {
            state.node_lists.all_brand.borrow_mut()
        } else if matches!(
            interface,
            "HTMLCollection" | "HTMLFormControlsCollection" | "HTMLOptionsCollection"
        ) {
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
        Contents::StyleSheets(root)
        | Contents::Attributes(root)
        | Contents::All(root)
        | Contents::Form(root)
        | Contents::Fieldset(root)
        | Contents::Radios { form: root, .. }
        | Contents::Children(root)
        | Contents::Labels(root)
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
        if !matches!(&value, Contents::Snapshot(_)) {
            wrapper_roots.push(wrapper.into());
        }
    }
    if matches!(&value, Contents::Snapshot(_)) {
        object.set_internal_field(3, v8::null(scope).into());
    } else {
        let traced_roots = v8::Array::new_with_elements(scope, &wrapper_roots);
        object.set_internal_field(3, traced_roots.into());
    }
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
            Contents::StyleSheets(_)
            | Contents::Attributes(_)
            | Contents::Children(_)
            | Contents::All(_)
            | Contents::Labels(_)
            | Contents::Form(_)
            | Contents::Fieldset(_) => 0,
            Contents::Radios { name, .. } => name.len(),
            Contents::Named { name, .. } => name.len(),
            Contents::Elements { filter, .. } => match filter {
                ElementFilter::Children
                | ElementFilter::Links
                | ElementFilter::SelectedOptions
                | ElementFilter::Options => 0,
                ElementFilter::WindowName { name } | ElementFilter::AllName { name } => name.len(),
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
            foreign: BTreeMap::new(),
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

pub(super) fn all<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::All(document))
}

pub(super) fn elements<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
    filter: ElementFilter,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Elements { root, filter })
}

pub(super) fn labels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    control: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Labels(control))
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
    if state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .is_some_and(|list| list.interface == "StyleSheetList")
    {
        // Resolve the current generation every time. Caching by owner element
        // would incorrectly return an old sheet after a source replacement.
        let value = associated_wrapper(scope, state, node, "CSSStyleSheet")?;
        return Ok(v8::Local::<v8::Object>::try_from(value).ok());
    }
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
    let foreign = state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .and_then(|list| list.foreign.get(&node))
        .cloned();
    let object = if let Some(reference) = foreign {
        let owner = reference.owner.upgrade().ok_or_else(invalid_receiver)?;
        wrap_node(scope, &owner, reference.node)?
    } else {
        wrap_node(scope, state, node)?
    };
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
    identity_as(scope, state, object, interface)?;
    let index = value
        .uint32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let (state, id) = owner(scope, object)?;
    Ok(node_at(scope, &state, id, index)?.map_or_else(|| v8::null(scope).into(), Into::into))
}

fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, u64), V8BindingError> {
    let state = binding_state_for_receiver(scope, object).ok_or_else(invalid_receiver)?;
    let id = match identity(scope, &state, object) {
        Ok(id) => id,
        Err(_) => {
            let node = argument_node(scope, &state, object.into())?;
            if !state
                .dom
                .borrow()
                .is_element(node, "http://www.w3.org/1999/xhtml", "select")
            {
                return Err(invalid_receiver());
            }
            let collection = elements(scope, &state, node, ElementFilter::Options)?;
            let collection =
                v8::Local::<v8::Object>::try_from(collection).map_err(|_| invalid_receiver())?;
            identity(scope, &state, collection)?
        }
    };
    Ok((state, id))
}

fn indexed_options_select(state: &V8BindingState, id: u64) -> Option<NodeId> {
    match state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .map(|list| &list.contents)
    {
        Some(Contents::Elements {
            root,
            filter: ElementFilter::Options,
        }) => Some(*root),
        _ => None,
    }
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
        let writable = matches!(
            state
                .node_lists
                .lists
                .borrow()
                .get(&id)
                .map(|list| &list.contents),
            Some(Contents::Elements {
                filter: ElementFilter::Options,
                ..
            })
        );
        Ok((count, writable))
    }) {
        Ok((count, writable)) if (index as usize) < count => {
            rv.set_int32(if writable {
                0
            } else {
                v8::PropertyAttribute::READ_ONLY.as_u32() as i32
            });
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
    index: u32,
    value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if let Ok((state, id)) = owner(scope, args.holder())
        && let Some(select) = indexed_options_select(&state, id)
    {
        match set_option_index_with_reactions(scope, &state, select, index, value) {
            Ok(()) => rv.set_bool(true),
            Err(error) => throw_binding_error(scope, &error),
        }
        queue_mutation_observer_microtask(scope);
        return v8::Intercepted::kYes;
    }
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
    index: u32,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if let Ok((state, id)) = owner(scope, args.holder())
        && let Some(select) = indexed_options_select(&state, id)
    {
        if !descriptor.has_value() && !descriptor.has_writable() {
            return reject_write(scope, args, rv);
        }
        let value = if descriptor.has_value() {
            descriptor.value()
        } else {
            v8::undefined(scope).into()
        };
        match set_option_index_with_reactions(scope, &state, select, index, value) {
            Ok(()) => rv.set_bool(true),
            Err(error) => throw_binding_error(scope, &error),
        }
        queue_mutation_observer_microtask(scope);
        return v8::Intercepted::kYes;
    }
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

fn set_option_index_with_reactions(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    select: NodeId,
    index: u32,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let boundary = custom_elements::ReactionBoundary::begin(scope)?;
    let mutation = set_option_index(scope, state, select, index, value);
    let completion = boundary.complete_result(scope);
    mutation.and(completion)
}

fn set_option_index(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    select: NodeId,
    index: u32,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if value.is_null_or_undefined() {
        let option = state
            .dom
            .borrow()
            .select_options(select)?
            .get(index as usize)
            .copied();
        if let Some(option) = option {
            state.dom.borrow_mut().remove(option)?;
        }
        return Ok(());
    }
    let (source, option) = node_identity::argument(scope, value)?.ok_or_else(invalid_receiver)?;
    if !source
        .dom
        .borrow()
        .is_element(option, "http://www.w3.org/1999/xhtml", "option")
    {
        return Err(V8BindingError::Unsupported {
            interface: "HTMLOptionsCollection".into(),
            member: "indexed value must be an option".into(),
        });
    }
    let options = state.dom.borrow().select_options(select)?;
    if let Some(replaced) = options.get(index as usize).copied() {
        let parent = state
            .dom
            .borrow()
            .parent(replaced)?
            .ok_or_else(invalid_receiver)?;
        let insertion = node_migration::insertion_argument(
            scope,
            state,
            parent,
            value,
            Some(replaced),
            Some(replaced),
        )?;
        if insertion.replacement_removed {
            insertion.owner.dom.borrow_mut().insert_before(
                insertion.parent,
                insertion.node,
                insertion.reference,
            )?;
        } else {
            insertion.owner.dom.borrow_mut().replace_child(
                insertion.parent,
                insertion.node,
                insertion.replaced.ok_or_else(invalid_receiver)?,
            )?;
        }
    } else {
        // Indexed padding has no length-setter ceiling in the HTML algorithm.
        if !state.dom.borrow_mut().append_new_options(
            select,
            index - options.len() as u32,
            || !scope.is_execution_terminating(),
        )? {
            return Err(V8BindingError::JavaScriptException);
        }
        let insertion =
            node_migration::insertion_argument(scope, state, select, value, None, None)?;
        insertion
            .owner
            .dom
            .borrow_mut()
            .append_child(insertion.parent, insertion.node)?;
    }
    let (owner, option) = node_identity::argument(scope, value)?.ok_or_else(invalid_receiver)?;
    prepare_dynamic_script_subtree(scope, &owner, option)?;
    Ok(())
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

pub(super) fn fieldset_elements<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    fieldset: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Fieldset(fieldset))
}

fn options_select(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<NodeId, V8BindingError> {
    let id = identity_as(scope, state, object, "HTMLOptionsCollection")?;
    match state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .map(|list| &list.contents)
    {
        Some(Contents::Elements {
            root,
            filter: ElementFilter::Options,
        }) => Ok(*root),
        _ => Err(invalid_receiver()),
    }
}

pub(super) fn options_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    value: Option<v8::Local<v8::Value>>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    options_select(scope, state, object)?; // brand before authored conversion
    let converted = match value {
        Some(value) => Some(if member == "selectedIndex" {
            value
                .int32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)? as u32
        } else {
            value
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?
        }),
        None => None,
    };
    let (owner, _) = owner(scope, object)?;
    let state = &owner;
    let select = options_select(scope, state, object)?;
    if member == "selectedIndex" {
        if let Some(index) = converted {
            state
                .dom
                .borrow_mut()
                .set_select_selected_index(select, index as i32)?;
        }
        return Ok(
            v8::Integer::new(scope, state.dom.borrow().select_selected_index(select)?).into(),
        );
    }
    if let Some(length) = converted {
        state.dom.borrow_mut().set_select_length(select, length)?;
    }
    Ok(v8::Integer::new_from_unsigned(
        scope,
        state.dom.borrow().select_options(select)?.len() as u32,
    )
    .into())
}

/// Shared native select/options add conversion. Author numeric conversion may
/// move the receiver and either argument into another live Document.
pub(super) fn select_add<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    select: NodeId,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let receiver = wrap_node(scope, state, select)?;
    let (source, element) =
        node_identity::argument(scope, args.get(0))?.ok_or_else(invalid_receiver)?;
    if !["option", "optgroup"].iter().any(|tag| {
        source
            .dom
            .borrow()
            .is_element(element, "http://www.w3.org/1999/xhtml", tag)
    }) {
        return Err(V8BindingError::Unsupported {
            interface: "HTMLSelectElement".into(),
            member: "TypeError: add requires an option or optgroup".into(),
        });
    }
    let before = args.get(1);
    let before_is_element = if before.is_null_or_undefined() {
        false
    } else {
        node_identity::canonical(scope,before)?.is_some_and(|(owner,node)|{let dom=owner.dom.borrow();matches!(dom.arena().get(node).map(|record|&record.kind),Ok(NodeKind::Element{name})if name.ns.as_ref()=="http://www.w3.org/1999/xhtml")})
    };
    let index = if !before_is_element && !before.is_null_or_undefined() {
        Some(
            before
                .int32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?,
        )
    } else {
        None
    };
    let (state, select) =
        node_identity::argument(scope, receiver.into())?.ok_or_else(invalid_receiver)?;
    let (source, element) =
        node_identity::argument(scope, args.get(0))?.ok_or_else(invalid_receiver)?;
    let before_node = if before_is_element {
        let (owner, node) = node_identity::argument(scope, before)?.ok_or_else(invalid_receiver)?;
        if !Rc::ptr_eq(&owner.dom, &state.dom) {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLSelectElement".into(),
                member: "NotFoundError".into(),
            });
        }
        Some(node)
    } else {
        None
    };
    let (parent, reference) = {
        let dom = state.dom.borrow();
        if Rc::ptr_eq(&source.dom, &state.dom) && dom.arena().ancestors(select)?.contains(&element)
        {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLSelectElement".into(),
                member: "HierarchyRequestError".into(),
            });
        }
        if let Some(before) = before_node {
            if !dom.arena().ancestors(before)?.contains(&select) {
                return Err(V8BindingError::Unsupported {
                    interface: "HTMLSelectElement".into(),
                    member: "NotFoundError".into(),
                });
            }
            if Rc::ptr_eq(&source.dom, &state.dom) && before == element {
                return Ok(None);
            }
        }
        let reference = match before_node {
            Some(node) => Some(node),
            None => index.filter(|index| *index >= 0).and_then(|index| {
                dom.select_options(select)
                    .ok()?
                    .get(index as usize)
                    .copied()
            }),
        };
        (
            match reference {
                Some(node) => dom.parent(node)?.unwrap_or(select),
                None => select,
            },
            reference,
        )
    };
    let insertion =
        node_migration::insertion_argument(scope, &state, parent, args.get(0), reference, None)?;
    insertion.owner.dom.borrow_mut().insert_before(
        insertion.parent,
        insertion.node,
        insertion.reference,
    )?;
    prepare_dynamic_script_subtree(scope, &insertion.owner, insertion.node)?;
    Ok(None)
}

pub(super) fn options_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let select = options_select(scope, state, object)?;
    if member == "add" {
        return select_add(scope, state, select, args);
    }
    if args.length() == 0 {
        return Err(V8BindingError::Unsupported {
            interface: "HTMLOptionsCollection".into(),
            member: "remove requires an index".into(),
        });
    }
    let index = args
        .get(0)
        .int32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let (owner, _) = owner(scope, object)?;
    let state = &owner;
    let select = options_select(scope, state, object)?;
    state.dom.borrow_mut().remove_select_option(select, index)?;
    Ok(None)
}

pub(super) fn form_elements<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    form: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    create(scope, state, Contents::Form(form))
}

fn form_named_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    form: NodeId,
    name: String,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let nodes = state
        .dom
        .borrow()
        .named_built_in_form_controls(form, &name)?;
    match nodes.as_slice() {
        [] => Ok(v8::null(scope).into()),
        [node] => Ok(wrap_node(scope, state, *node)?.into()),
        _ => create(scope, state, Contents::Radios { form, name }),
    }
}

pub(super) fn form_named_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    identity_as(scope, state, object, "HTMLFormControlsCollection")?;
    let name = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let (owner, id) = owner(scope, object)?;
    let state = &owner;
    let form = match state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .map(|list| &list.contents)
    {
        Some(Contents::Form(form)) => *form,
        _ => return Err(invalid_receiver()),
    };
    let name = rust_string_preserving_utf16(scope, name)?;
    form_named_value(scope, state, form, name)
}

pub(super) fn radio_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: Option<v8::Local<v8::Value>>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    identity_as(scope, state, object, "RadioNodeList")?;
    let desired = match value {
        Some(value) => {
            let Some(value) = value.to_string(scope) else {
                return Ok(v8::undefined(scope).into());
            };
            Some(rust_string_preserving_utf16(scope, value)?)
        }
        None => None,
    };
    let (owner, id) = owner(scope, object)?;
    let state = &owner;
    let nodes = contents(state, id)?;
    sync_wrapper_roots(scope, state, id);
    let mut selected = None;
    let mut result = String::new();
    {
        let dom = state.dom.borrow();
        for node in nodes {
            if !dom.is_radio(node) {
                continue;
            }
            let value = dom
                .attribute_in_null_namespace(node, "value")?
                .unwrap_or("on");
            if let Some(desired) = &desired {
                if value == desired {
                    selected = Some(node);
                    break;
                }
            } else if dom.checked(node)? {
                result = value.to_owned();
                break;
            }
        }
    }
    if let Some(node) = selected {
        state.dom.borrow_mut().set_checked(node, true)?;
    }
    v8_string(scope, &result)
}

pub(super) fn configure_named(instance: v8::Local<v8::ObjectTemplate>) {
    named::configure(instance);
}

#[cfg(test)]
mod window_named_tests {
    use super::*;

    #[test]
    fn all_named_selection_includes_legacy_names_without_expanding_window_names() {
        let mut dom = DomRuntime::new();
        for tag in [
            "a", "button", "embed", "form", "frame", "frameset", "iframe", "img", "input", "map",
            "meta", "object", "select", "textarea", "div", "span", "area",
        ] {
            let node = dom.create_element_with_local_name("http://www.w3.org/1999/xhtml", tag);
            dom.set_attribute(node, "name", "legacy").unwrap();
            assert_eq!(
                all_named_match(&dom, node, "legacy"),
                !matches!(tag, "div" | "span" | "area")
            );
            assert_eq!(
                window_named_match(&dom, node, "legacy"),
                matches!(tag, "embed" | "form" | "img" | "object")
            );
            dom.set_attribute(node, "id", "identified").unwrap();
            assert!(all_named_match(&dom, node, "identified"));
        }
    }

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

/// Snapshot only genuinely branded DOM node collections, using native contents
/// rather than author-replaceable length, indices, or iterator properties.
pub(super) fn native_snapshot(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Result<Option<RealmNodes>, V8BindingError> {
    if object.is_proxy() || object.internal_field_count() < 3 {
        return Ok(None);
    }
    let Some(actual) = object
        .get_internal_field(scope, 2)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
    else {
        return Ok(None);
    };
    let branded = [
        state.node_lists.brand.borrow().clone(),
        state.node_lists.collection_brand.borrow().clone(),
        state.node_lists.all_brand.borrow().clone(),
    ]
    .into_iter()
    .flatten()
    .any(|brand| actual.strict_equals(v8::Local::new(scope, &brand).into()));
    if !branded {
        return Ok(None);
    }
    let id = identity(scope, state, object)?;
    let valid_interface = state
        .node_lists
        .lists
        .borrow()
        .get(&id)
        .is_some_and(|list| {
            matches!(
                list.interface,
                "NodeList"
                    | "RadioNodeList"
                    | "HTMLCollection"
                    | "HTMLFormControlsCollection"
                    | "HTMLOptionsCollection"
                    | "HTMLAllCollection"
            )
        });
    if !valid_interface {
        return Ok(None);
    }
    let nodes = contents(state, id)?;
    let lists = state.node_lists.lists.borrow();
    let list = lists.get(&id).ok_or_else(invalid_receiver)?;
    nodes
        .into_iter()
        .map(|node| {
            if let Some(reference) = list.foreign.get(&node) {
                Ok((
                    reference.owner.upgrade().ok_or_else(invalid_receiver)?,
                    reference.node,
                ))
            } else {
                Ok((state.clone(), node))
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

#[cfg(test)]
mod conversion_migration_tests;

/// Snapshot keys may still name the source arena after adoption. Native leases
/// have already moved, so account against each committed foreign identity.
pub(super) fn owned_native_roots(
    state: &V8BindingState,
    roots: &mut traced_native_roots::Snapshot,
) -> Result<(), V8BindingError> {
    for list in state.node_lists.lists.borrow().values() {
        for node in &list.roots {
            if let Some(foreign) = list.foreign.get(node) {
                roots.add(&foreign.dom, foreign.node)?;
            } else {
                roots.add(&state.dom, *node)?;
            }
        }
    }
    Ok(())
}
