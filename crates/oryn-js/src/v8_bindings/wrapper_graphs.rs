//! Trace detached DOM wrapper relationships through V8, without native Global cycles.
use super::*;

fn native_root(
    dom: &DomRuntime,
    node: NodeId,
    roots: &mut BTreeMap<NodeId, NodeId>,
) -> Option<NodeId> {
    let mut current = node;
    let mut path = Vec::new();
    let root = loop {
        if let Some(root) = roots.get(&current) {
            break *root;
        }
        path.push(current);
        match dom.retention_parent(current).ok()? {
            Some(parent) => current = parent,
            None => break current,
        }
    };
    for node in path {
        roots.insert(node, root);
    }
    Some(root)
}

pub(super) fn retained_by_document(dom: &DomRuntime, node: NodeId) -> bool {
    let owner = match dom.arena().get(node).map(|record| &record.kind) {
        Ok(NodeKind::Attribute {
            owner: Some(owner), ..
        }) => *owner,
        _ => node,
    };
    dom.is_connected(owner).unwrap_or(false)
}

pub(super) fn refresh(scope: &mut v8::PinScope, state: &V8BindingState) -> usize {
    let Ok(dom) = state.dom.try_borrow() else {
        state.wrapper_graph_revision.set(None);
        return 0;
    };
    let revision = dom.mutation_cursor().0;
    if state.wrapper_graph_revision.get() == Some(revision)
        && state.wrapper_graph_new_nodes.borrow().is_empty()
    {
        return 0;
    }
    let (Ok(node_documents), Ok(wrappers), Ok(mut objects)) = (
        state.node_documents.try_borrow(),
        state.wrappers.try_borrow(),
        state.objects.try_borrow_mut(),
    ) else {
        // Some wrapper factories initialize ownership under a native borrow.
        // Keep the old traced graph until the outer binding checkpoint.
        state.wrapper_graph_revision.set(None);
        return 0;
    };
    let mut affected = BTreeSet::new();
    let mut roots = BTreeMap::new();
    let mut complete_rebuild = state.wrapper_graph_revision.get().is_none();
    let new_nodes = state.wrapper_graph_new_nodes.borrow().clone();
    let mut changed = new_nodes.clone();
    if let Some(previous) = state.wrapper_graph_revision.get() {
        let batch = dom.mutations_since(oryn_dom::MutationCursor(previous));
        complete_rebuild |= batch.truncated;
        for mutation in batch.mutations {
            match mutation {
                Mutation::Inserted { node, parent, .. } => {
                    changed.insert(node);
                    changed.extend(parent);
                }
                Mutation::Removed {
                    node,
                    former_parent,
                    ..
                } => {
                    changed.insert(node);
                    changed.extend(former_parent);
                }
                Mutation::ShadowAttached { host, root, .. } => {
                    changed.extend([host, root]);
                }
                Mutation::Attribute { node, .. } => {
                    changed.insert(node);
                }
                Mutation::ChildrenReplaced { .. }
                | Mutation::CharacterData { .. }
                | Mutation::StyleSheets { .. }
                | Mutation::FormState { .. }
                | Mutation::EventListener { .. }
                | Mutation::Focus { .. } => {}
            }
        }
    }
    // A newly materialized owner-document wrapper must become reachable from
    // every existing detached component that exposes it through ownerDocument.
    for (node, owner) in node_documents.iter().filter(|_| !new_nodes.is_empty()) {
        if new_nodes.contains(owner)
            && let Some(object) = wrappers
                .lookup(state.realm, *node)
                .and_then(|id| objects.get(&id))
        {
            affected.extend(object.retention_component);
        }
    }
    for node in changed {
        // Both the former and current component matter when a subtree moves.
        if let Some(object) = wrappers
            .lookup(state.realm, node)
            .and_then(|id| objects.get(&id))
        {
            affected.extend(object.retention_component);
        } else {
            // An unwrapped structural endpoint has no cached former component.
            // Fall back rather than risk leaving an old traced edge behind.
            complete_rebuild = true;
        }
        affected.extend(native_root(&dom, node, &mut roots));
    }
    state.wrapper_graph_new_nodes.borrow_mut().clear();
    state.wrapper_graph_revision.set(Some(revision));
    let mut owners = BTreeMap::new();
    let mut released = 0;
    let mut nodes = BTreeMap::new();
    let mut groups = BTreeMap::<NodeId, Vec<(NodeId, v8::Local<v8::Object>)>>::new();
    {
        for (id, object) in objects.iter_mut() {
            if !complete_rebuild
                && object
                    .retention_component
                    .is_some_and(|root| !affected.contains(&root))
            {
                continue;
            }
            let Some(metadata) = wrappers.metadata(*id) else {
                continue;
            };
            object.retention_component = native_root(&dom, metadata.node, &mut roots);
            let Some(local) = object.handle.to_local(scope) else {
                continue;
            };
            let connected = retained_by_document(&dom, metadata.node);
            if connected {
                if object.connected_root.is_none() {
                    object.connected_root = Some(v8::Global::new(scope, local));
                }
                local.set_internal_field(5, v8::undefined(scope).into());
            } else {
                released += usize::from(object.connected_root.take().is_some());
                if let Some(root) = native_root(&dom, metadata.node, &mut roots) {
                    groups.entry(root).or_default().push((metadata.node, local));
                } else {
                    local.set_internal_field(5, v8::undefined(scope).into());
                }
            }
            if let Some(owner) = node_documents.get(&metadata.node) {
                owners.insert(metadata.node, *owner);
            }
            nodes.insert(metadata.node, local);
        }
    }
    for owner in owners.values() {
        if let Some(local) = wrappers
            .lookup(state.realm, *owner)
            .and_then(|id| objects.get(&id))
            .and_then(|object| object.handle.to_local(scope))
        {
            nodes.insert(*owner, local);
        }
    }
    drop(objects);
    drop(wrappers);
    drop(node_documents);
    drop(dom);
    for members in groups.values() {
        let mut seen = members
            .iter()
            .map(|(node, _)| *node)
            .collect::<BTreeSet<_>>();
        let mut references = members
            .iter()
            .map(|(_, object)| (*object).into())
            .collect::<Vec<v8::Local<v8::Value>>>();
        // Detached nodes still expose their explicitly assigned owner document.
        // Trace that existing wrapper as well; the mapping remains Rust-owned.
        for (node, _) in members {
            if let Some(owner) = owners.get(node).copied()
                && seen.insert(owner)
                && let Some(object) = nodes.get(&owner)
            {
                references.push((*object).into());
            }
        }
        let references: v8::Local<v8::Value> = if references.len() > 1 {
            v8::Array::new_with_elements(scope, &references).into()
        } else {
            v8::undefined(scope).into()
        };
        // Every wrapper in a detached tree can reach every other one through
        // DOM parent/child traversal. One shared traced array preserves those
        // identities and expandos, and an unreachable cycle remains collectable.
        for (_, object) in members {
            object.set_internal_field(5, references.into());
        }
    }
    released
}

// A single fresh detached leaf cannot change any existing traced tree.
pub(super) fn attach_new_leaf(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
    object: v8::Local<v8::Object>,
) -> bool {
    let Some(previous) = state.wrapper_graph_revision.get() else {
        return false;
    };
    let Ok(dom) = state.dom.try_borrow() else {
        return false;
    };
    let revision = dom.mutation_cursor().0;
    if previous.checked_add(1) != Some(revision) {
        return false;
    }
    let batch = dom.mutations_since(oryn_dom::MutationCursor(previous));
    if batch.truncated
        || !matches!(batch.mutations.as_slice(), [Mutation::Inserted { node: inserted, parent: None, .. }] if *inserted == node)
    {
        return false;
    }
    let Ok(record) = dom.arena().get(node) else {
        return false;
    };
    if record.parent.is_some()
        || !record.children.is_empty()
        || record.template_content.is_some()
        || !matches!(
            record.kind,
            NodeKind::Element { .. }
                | NodeKind::Attribute { owner: None, .. }
                | NodeKind::DocumentType { .. }
                | NodeKind::Text { .. }
                | NodeKind::Comment { .. }
                | NodeKind::CDataSection { .. }
                | NodeKind::ProcessingInstruction { .. }
        )
    {
        return false;
    }
    let (Ok(documents), Ok(wrappers), Ok(objects)) = (
        state.node_documents.try_borrow(),
        state.wrappers.try_borrow(),
        state.objects.try_borrow(),
    ) else {
        return false;
    };
    let owner = documents
        .get(&node)
        .and_then(|owner| wrappers.lookup(state.realm, *owner))
        .and_then(|id| objects.get(&id))
        .and_then(|owner| owner.handle.to_local(scope));
    drop(objects);
    drop(wrappers);
    drop(documents);
    drop(dom);
    let reference = owner.map_or_else(|| v8::undefined(scope).into(), |owner| owner.into());
    object.set_internal_field(5, reference);
    if let Some(id) = state.wrappers.borrow().lookup(state.realm, node)
        && let Some(wrapper) = state.objects.borrow_mut().get_mut(&id)
    {
        wrapper.retention_component = Some(node);
    }
    state.wrapper_graph_revision.set(Some(revision));
    true
}
