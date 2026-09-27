//! Traced host/content edges survive adoption into independent native arenas.
use super::*;
fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: bool,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(
        scope,
        if host {
            "oryn.template.host"
        } else {
            "oryn.template.content"
        },
    )
    .ok_or(V8BindingError::Allocation("template adoption edge"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
pub(super) struct Prepared {
    edges: Vec<(v8::Global<v8::Object>, v8::Global<v8::Object>)>,
}
pub(super) fn capture(
    scope: &mut v8::PinScope,
    source: &Rc<V8BindingState>,
    nodes: &BTreeSet<NodeId>,
) -> Result<Prepared, V8BindingError> {
    let edges = {
        let dom = source.dom.borrow();
        dom.arena()
            .iter()
            .filter_map(|(host, record)| {
                record
                    .template_content
                    .filter(|content| nodes.contains(&host) || nodes.contains(content))
                    .map(|content| (host, content))
            })
            .collect::<Vec<_>>()
    };
    let mut result = Vec::new();
    for (host, content) in edges {
        let host = wrap_node(scope, source, host)?;
        let content = wrap_node(scope, source, content)?;
        result.push((
            v8::Global::new(scope, host),
            v8::Global::new(scope, content),
        ));
    }
    Ok(Prepared { edges: result })
}
impl Prepared {
    pub(super) fn publish(&mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        while let Some((host, content)) = self.edges.last() {
            let host = v8::Local::new(scope, host);
            let content = v8::Local::new(scope, content);
            let content_key = key(scope, false)?;
            host.set_private(scope, content_key, content.into())
                .filter(|set| *set)
                .ok_or(V8BindingError::JavaScriptException)?;
            let host_key = key(scope, true)?;
            content
                .set_private(scope, host_key, host.into())
                .filter(|set| *set)
                .ok_or(V8BindingError::JavaScriptException)?;
            self.edges.pop();
        }
        Ok(())
    }
}
pub(super) fn content<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    host: NodeId,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let host = wrap_node(scope, state, host)?;
    let key = key(scope, false)?;
    Ok(host
        .get_private(scope, key)
        .filter(|value| value.is_object()))
}
pub(super) fn host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    content: NodeId,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let content = wrap_node(scope, state, content)?;
    let key = key(scope, true)?;
    Ok(content
        .get_private(scope, key)
        .filter(|value| value.is_object()))
}
/// Follow native parent, shadow host and preserved template host edges using
/// arena-qualified identities. A numeric NodeId in another arena never aliases.
pub(super) fn validate(
    scope: &mut v8::PinScope,
    destination: &Rc<V8BindingState>,
    parent: NodeId,
    source: &Rc<V8BindingState>,
    child: NodeId,
) -> Result<(), V8BindingError> {
    let mut current = Some((destination.clone(), parent));
    let mut seen = BTreeSet::new();
    while let Some((owner, node)) = current {
        if Rc::ptr_eq(&owner.dom, &source.dom) && node == child {
            return Err(hierarchy_request_error());
        }
        if !seen.insert((owner.dom.borrow().arena().id(), node)) {
            return Err(hierarchy_request_error());
        }
        let (native_parent, template_host) = {
            let dom = owner.dom.borrow();
            let record = dom.arena().get(node)?;
            (record.parent, record.template_host)
        };
        if let Some(parent) = native_parent.or(template_host) {
            current = Some((owner, parent));
            continue;
        }
        let wrapper = wrap_node(scope, &owner, node)?;
        let key = key(scope, true)?;
        current = match wrapper
            .get_private(scope, key)
            .filter(|value| value.is_object())
        {
            Some(value) => node_identity::argument(scope, value)?,
            None => None,
        };
    }
    Ok(())
}

pub(super) fn inner_html<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    host: NodeId,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let Some(content) = content(scope, state, host)? else {
        return Ok(None);
    };
    let (owner, node) = node_identity::argument(scope, content)?
        .ok_or(V8BindingError::Allocation("template content owner"))?;
    let html = serialize_children(scope, &owner, node)?;
    Ok(Some(v8_string(scope, &html)?))
}
/// Native cloning copies arena-local edges. Complete any template edges held
/// across arenas by the native wrapper graph, without modifying the source.
/// The iterative worklist also reaches external links in nested templates and
/// in clonable shadow trees (which clone deeply even for shallow host clones).
pub(super) fn complete_clone(
    scope: &mut v8::PinScope,
    source: &Rc<V8BindingState>,
    node: NodeId,
    destination: &Rc<V8BindingState>,
    clone: NodeId,
    deep: bool,
) -> Result<(), V8BindingError> {
    let mut pending = vec![(source.clone(), node, clone, deep)];
    while let Some((owner, node, clone, deep)) = pending.pop() {
        let is_template = {
            let dom = owner.dom.borrow();
            dom.is_element(node, "http://www.w3.org/1999/xhtml", "template")
        };
        if deep && is_template {
            let retained = content(scope, &owner, node)?;
            let (source_owner, source_content, external) = if let Some(content) = retained {
                let (content_owner, content) = node_identity::argument(scope, content)?
                    .ok_or(V8BindingError::Allocation("template clone owner"))?;
                let external = !Rc::ptr_eq(&content_owner.dom, &owner.dom)
                    || owner.dom.borrow().arena().get(node)?.template_content != Some(content);
                (content_owner, content, external)
            } else {
                let content = owner.dom.borrow_mut().template_content(node)?;
                (owner.clone(), content, false)
            };
            let target = destination.dom.borrow_mut().template_content(clone)?;
            if external {
                let old = light_children(&destination.dom.borrow(), target)?;
                for child in old {
                    destination.dom.borrow_mut().remove_child(target, child)?;
                }
                let children = light_children(&source_owner.dom.borrow(), source_content)?;
                for child in children {
                    let copied = if Rc::ptr_eq(&source_owner.dom, &destination.dom) {
                        destination.dom.borrow_mut().clone_node(child, true)?
                    } else {
                        let source = source_owner.dom.borrow();
                        destination.dom.borrow_mut().import_node_from(
                            source.arena(),
                            child,
                            true,
                        )?
                    };
                    destination.dom.borrow_mut().append_child(target, copied)?;
                }
            }
            pending.push((source_owner, source_content, target, true));
        }
        let children = {
            let dom = owner.dom.borrow();
            let mut children = Vec::new();
            for child in dom.arena().get(node)?.children.iter().copied() {
                let kind = &dom.arena().get(child)?.kind;
                let shadow = matches!(kind, NodeKind::ShadowRoot { clonable: true, .. });
                if shadow
                    || (deep && !matches!(kind, NodeKind::Document | NodeKind::ShadowRoot { .. }))
                {
                    children.push((child, deep || shadow));
                }
            }
            children
        };
        let targets = {
            let dom = destination.dom.borrow();
            dom.arena()
                .get(clone)?
                .children
                .iter()
                .copied()
                .filter(|child| {
                    matches!(
                        dom.arena().get(*child).map(|node| &node.kind),
                        Ok(NodeKind::ShadowRoot { clonable: true, .. })
                    ) || (deep
                        && !matches!(
                            dom.arena().get(*child).map(|node| &node.kind),
                            Ok(NodeKind::Document | NodeKind::ShadowRoot { .. })
                        ))
                })
                .collect::<Vec<_>>()
        };
        if children.len() != targets.len() {
            return Err(V8BindingError::Allocation("template clone topology"));
        }
        for ((child, deep), target) in children.into_iter().zip(targets) {
            pending.push((owner.clone(), child, target, deep));
        }
    }
    Ok(())
}

struct Resolver {
    arenas: BTreeMap<oryn_dom::ArenaId, SharedDom>,
    templates: BTreeMap<oryn_dom::NodeRef, oryn_dom::NodeRef>,
}
pub(super) fn get_html(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<v8::Value>,
) -> Result<String, V8BindingError> {
    let receiver = wrap_node(scope, state, node)?;
    let invalid = || V8BindingError::Unsupported {
        interface: "GetHTMLOptions".into(),
        member: "TypeError".into(),
    };
    let mut options = oryn_dom::HtmlSerializationOptions::default();
    // Keep wrappers until all iterator calls finish: later calls may adopt a
    // root that an earlier iteration already converted.
    let mut roots = Vec::new();
    if !value.is_null_or_undefined() {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
        let key = v8_string(scope, "serializableShadowRoots")?;
        options.serializable_shadow_roots = object
            .get(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?
            .boolean_value(scope);
        let key = v8_string(scope, "shadowRoots")?;
        let sequence = object
            .get(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?;
        if !sequence.is_undefined() {
            let iterable = v8::Local::<v8::Object>::try_from(sequence).map_err(|_| invalid())?;
            let key = v8::Symbol::get_iterator(scope);
            let method = iterable
                .get(scope, key.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| invalid())?;
            iterable_conversion::consume(scope, iterable, method, |scope, value| {
                let (owner, node) = node_identity::argument(scope, value)?.ok_or_else(invalid)?;
                if !matches!(
                    owner.dom.borrow().arena().get(node)?.kind,
                    NodeKind::ShadowRoot { .. }
                ) {
                    return Err(invalid());
                }
                roots
                    .try_reserve(1)
                    .map_err(|_| V8BindingError::Allocation("getHTML shadow roots"))?;
                roots.push(v8::Global::new(scope, value));
                Ok(())
            })?;
        }
    }
    for root in roots {
        let value = v8::Local::new(scope, &root);
        let (owner, node) = node_identity::argument(scope, value)?.ok_or_else(invalid)?;
        options
            .shadow_roots
            .insert(owner.dom.borrow().arena().node_ref(node)?);
    }
    let (owner, node) = node_identity::argument(scope, receiver.into())?.ok_or_else(invalid)?;
    let (root, resolver) = resolver(scope, &owner, node, true)?;
    Ok(oryn_dom::serialize_html_with_options(
        root, &resolver, &options, true,
    )?)
}
impl oryn_dom::SerializationResolver for Resolver {
    fn non_global_shadow_registry(&self, node: oryn_dom::NodeRef) -> Result<bool, DomError> {
        self.arenas
            .get(&node.arena)
            .ok_or(DomError::StaleNode(node.node))?
            .borrow()
            .has_non_global_shadow_registry(node.node)
    }
    fn node(&self, node: oryn_dom::NodeRef) -> Result<oryn_dom::Node, DomError> {
        let dom = self
            .arenas
            .get(&node.arena)
            .ok_or(DomError::StaleNode(node.node))?
            .borrow();
        Ok(dom.arena().get(node.node)?.clone())
    }
    fn template_content(
        &self,
        node: oryn_dom::NodeRef,
    ) -> Result<Option<oryn_dom::NodeRef>, DomError> {
        if let Some(content) = self.templates.get(&node) {
            return Ok(Some(*content));
        }
        let dom = self
            .arenas
            .get(&node.arena)
            .ok_or(DomError::StaleNode(node.node))?
            .borrow();
        Ok(dom
            .arena()
            .get(node.node)?
            .template_content
            .map(|content| oryn_dom::NodeRef {
                arena: node.arena,
                node: content,
            }))
    }
}
fn resolver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    include_shadow: bool,
) -> Result<(oryn_dom::NodeRef, Resolver), V8BindingError> {
    let root_ref = state.dom.borrow().arena().node_ref(root)?;
    let mut result = Resolver {
        arenas: BTreeMap::new(),
        templates: BTreeMap::new(),
    };
    let mut pending = vec![(state.clone(), root)];
    let mut seen = BTreeSet::new();
    while let Some((owner, node)) = pending.pop() {
        let reference = owner.dom.borrow().arena().node_ref(node)?;
        if !seen.insert(reference) {
            continue;
        }
        result.arenas.insert(reference.arena, owner.dom.clone());
        let (children, template, local_content) = {
            let dom = owner.dom.borrow();
            let record = dom.arena().get(node)?;
            (
                record.children.clone(),
                dom.is_element(node, "http://www.w3.org/1999/xhtml", "template"),
                record.template_content,
            )
        };
        for child in children {
            if !matches!(
                owner.dom.borrow().arena().get(child)?.kind,
                NodeKind::Document
            ) && (include_shadow
                || !matches!(
                    owner.dom.borrow().arena().get(child)?.kind,
                    NodeKind::ShadowRoot { .. }
                ))
            {
                pending.push((owner.clone(), child));
            }
        }
        if template {
            let content = if let Some(value) = content(scope, &owner, node)? {
                node_identity::argument(scope, value)?
            } else {
                local_content.map(|node| (owner.clone(), node))
            };
            if let Some((content_owner, content)) = content {
                let content_ref = content_owner.dom.borrow().arena().node_ref(content)?;
                result.templates.insert(reference, content_ref);
                pending.push((content_owner, content));
            }
        }
    }
    Ok((root_ref, result))
}
pub(super) fn serialize_html(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<String, V8BindingError> {
    let (root, resolver) = resolver(scope, state, node, false)?;
    Ok(oryn_dom::serialize_html_with_resolver(root, &resolver)?)
}
pub(super) fn serialize_xml(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<String, V8BindingError> {
    let (root, resolver) = resolver(scope, state, node, false)?;
    Ok(oryn_dom::serialize_xml_with_resolver(root, &resolver)?)
}
fn html_document(state: &V8BindingState, node: NodeId) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    let document = dom.node_document(node)?.ok_or(DomError::StaleNode(node))?;
    Ok(dom
        .arena()
        .get(document)?
        .document_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_html))
}
fn strict_xml(
    root: oryn_dom::NodeRef,
    resolver: &impl oryn_dom::SerializationResolver,
) -> Result<String, V8BindingError> {
    oryn_dom::serialize_xml_well_formed_with_resolver(root, resolver).map_err(|error| match error {
        oryn_dom::WellFormedError::Dom(error) => error.into(),
        oryn_dom::WellFormedError::InvalidState => V8BindingError::Unsupported {
            interface: "Element".into(),
            member: "InvalidStateError".into(),
        },
    })
}
pub(super) fn serialize_legacy_outer(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<String, V8BindingError> {
    if html_document(state, node)? {
        serialize_html(scope, state, node)
    } else {
        let (root, resolver) = resolver(scope, state, node, false)?;
        strict_xml(root, &resolver)
    }
}
pub(super) fn serialize_children(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<String, V8BindingError> {
    let html = html_document(state, node)?;
    let (root, resolver) = resolver(scope, state, node, false)?;
    let children = light_children(&state.dom.borrow(), node)?;
    let mut units = Vec::new();
    for node in children {
        let reference = oryn_dom::NodeRef {
            arena: root.arena,
            node,
        };
        let text = if html {
            oryn_dom::serialize_html_with_resolver(reference, &resolver)?
        } else {
            strict_xml(reference, &resolver)?
        };
        units.extend(oryn_dom::dom_string::units(&text));
    }
    Ok(oryn_dom::dom_string::from_units(&units))
}
