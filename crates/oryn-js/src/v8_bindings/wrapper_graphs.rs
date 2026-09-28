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

// Enumerate the inverse of retention_parent only in the selected tree.
// Owner-document and template references are separate directed graph edges;
// merging them here would incorrectly join otherwise independent components.
fn retention_component_nodes(dom: &DomRuntime, root: NodeId) -> Vec<NodeId> {
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    while let Some(node) = pending.pop() {
        let Ok(record) = dom.arena().get(node) else {
            continue;
        };
        if !seen.insert(node) {
            continue;
        }
        for child in record
            .children
            .iter()
            .copied()
            .chain(dom.arena().attribute_node_ids(node))
            .chain(dom.arena().hosted_document(node))
        {
            if dom.retention_parent(child).ok().flatten() == Some(node) {
                pending.push(child);
            }
        }
    }
    seen.into_iter().collect()
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

// A complete insertion-only journal can join an isolated wrapped leaf to an
// existing detached fragment without rewriting every edge in that component.
// All other mutations retain the general former/current-component rebuild.
fn grow_detached_fragment(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<bool, V8BindingError> {
    let Some(previous) = state.wrapper_graph_revision.get() else {
        return Ok(false);
    };
    if state.execution_retired()
        || node_migration::has_publications(scope)
        || !state.wrapper_graph_new_nodes.borrow().is_empty()
        || scope
            .get_slot::<BindingStateRegistry>()
            .is_some_and(|registry| registry.cross_realm_wrappers.get())
    {
        return Ok(false);
    }
    let Ok(dom) = state.dom.try_borrow() else {
        return Ok(false);
    };
    let revision = dom.mutation_cursor().0;
    if previous == revision {
        return Ok(false);
    }
    let batch = dom.mutations_since(oryn_dom::MutationCursor(previous));
    if batch.truncated {
        return Ok(false);
    }
    let (node, parent) = match batch.mutations.as_slice() {
        [
            Mutation::Inserted {
                node,
                parent: Some(parent),
                ..
            },
        ] => (*node, *parent),
        [
            Mutation::Inserted {
                node,
                parent: Some(parent),
                ..
            },
            Mutation::ChildrenReplaced {
                node: target,
                added_nodes,
                removed_nodes,
                ..
            },
        ] if target == parent && added_nodes.as_slice() == [*node] && removed_nodes.is_empty() => {
            (*node, *parent)
        }
        _ => return Ok(false),
    };
    let (Ok(leaf), Ok(fragment)) = (dom.arena().get(node), dom.arena().get(parent)) else {
        return Ok(false);
    };
    if !matches!(fragment.kind, NodeKind::DocumentFragment)
        || fragment.parent.is_some()
        || fragment.template_host.is_some()
        || leaf.parent != Some(parent)
        || !leaf.children.is_empty()
        || leaf.template_content.is_some()
        || leaf.template_host.is_some()
        || !dom.arena().attribute_node_ids(node).is_empty()
        || dom.arena().hosted_document(node).is_some()
        || leaf.owner_document.is_none()
        || leaf.owner_document != fragment.owner_document
        || !matches!(
            leaf.kind,
            NodeKind::Element { .. }
                | NodeKind::Text { .. }
                | NodeKind::Comment { .. }
                | NodeKind::CDataSection { .. }
                | NodeKind::ProcessingInstruction { .. }
        )
    {
        return Ok(false);
    }
    let (leaf_id, leaf_object, array, expected_length) = {
        let (Ok(wrappers), Ok(objects), Ok(components)) = (
            state.wrappers.try_borrow(),
            state.objects.try_borrow(),
            state.wrapper_graph_components.try_borrow(),
        ) else {
            return Ok(false);
        };
        let Some(leaf_id) = wrappers.lookup(dom.arena().id(), state.realm, node) else {
            return Ok(false);
        };
        let Some(parent_id) = wrappers.lookup(dom.arena().id(), state.realm, parent) else {
            return Ok(false);
        };
        let (Some(leaf), Some(fragment)) = (objects.get(&leaf_id), objects.get(&parent_id)) else {
            return Ok(false);
        };
        let (Some(old_members), Some(members)) = (components.get(&node), components.get(&parent))
        else {
            return Ok(false);
        };
        if leaf.retention_component != Some(node)
            || fragment.retention_component != Some(parent)
            || leaf.connected_root.is_some()
            || fragment.connected_root.is_some()
            || old_members.len() != 1
            || !old_members.contains(&leaf_id)
            || !members.contains(&parent_id)
            || members.contains(&leaf_id)
        {
            return Ok(false);
        }
        let (Some(leaf_object), Some(parent_object)) =
            (leaf.handle.to_local(scope), fragment.handle.to_local(scope))
        else {
            return Ok(false);
        };
        if leaf_object.internal_field_count() <= 5 {
            return Ok(false);
        }
        let Some(array) = parent_object
            .get_internal_field(scope, 5)
            .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        else {
            return Ok(false);
        };
        // The completed prior rebuild has one entry per wrapper plus its
        // common ownerDocument. Refuse stale/dead or more complex owner sets.
        let Some(expected_length) = members
            .len()
            .checked_add(1)
            .and_then(|length| u32::try_from(length).ok())
            .filter(|length| *length < u32::MAX)
        else {
            return Ok(false);
        };
        if array.length() != expected_length {
            return Ok(false);
        }
        (leaf_id, leaf_object, array, expected_length)
    };
    drop(dom);
    // Refresh only the new leaf's registry edge. Existing members retain their
    // component-registry edges, including registries on unwrapped descendants.
    // Mutual reachability through the shared array retains the union of both.
    custom_elements::trace_node_registry(scope, state, node, leaf_object)?;
    custom_elements::trace_component_registries(scope, state, &[node], &[(node, leaf_object)])?;
    let Some(key) = v8::String::new(scope, &expected_length.to_string()) else {
        return Ok(false);
    };
    let (Ok(mut objects), Ok(mut components)) = (
        state.objects.try_borrow_mut(),
        state.wrapper_graph_components.try_borrow_mut(),
    ) else {
        return Ok(false);
    };
    let (Some(leaf), Some(members)) = (objects.get_mut(&leaf_id), components.get_mut(&parent))
    else {
        return Ok(false);
    };
    // Prepare the only allocating native index update before V8 publication.
    // No wrapper-registry borrow survives allocation: weak finalizers may queue
    // releases there. Locals keep the new leaf and existing component alive.
    if !members.insert(leaf_id) {
        return Ok(false);
    }
    // CreateDataProperty bypasses indexed Array.prototype setters. This private
    // ordinary array cannot execute authored code, so our checked internal field
    // and the native bookkeeping remain valid across allocation/GC.
    if array.create_data_property(scope, key.into(), leaf_object.into()) != Some(true) {
        members.remove(&leaf_id);
        // Do not consume the journal. The general rebuild repairs even a
        // partially published edge; it is only tracing, never native authority.
        return Ok(false);
    }
    // The prechecked SetInternalField has no allocating or fallible branch.
    leaf_object.set_internal_field(5, array.into());
    leaf.retention_component = Some(parent);
    components.remove(&node);
    state.wrapper_graph_revision.set(Some(revision));
    Ok(true)
}

pub(super) fn refresh(scope: &mut v8::PinScope, state: &V8BindingState) -> usize {
    v8::scope!(let scope, scope);
    match grow_detached_fragment(scope, state) {
        Ok(true) => 0,
        Ok(false) => rebuild(scope, state),
        Err(error) => {
            // Repair the whole graph before surfacing the allocation error.
            // Neither a partial registry edge nor its journal becomes cached.
            state.wrapper_graph_revision.set(None);
            let released = rebuild(scope, state);
            state.wrapper_graph_revision.set(None);
            throw_binding_error(scope, &error);
            released
        }
    }
}

fn rebuild(scope: &mut v8::PinScope, state: &V8BindingState) -> usize {
    // Graph observation handles must end with this refresh, not the caller task.
    v8::scope!(let scope, scope);
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
    let (Ok(wrappers), Ok(mut objects)) =
        (state.wrappers.try_borrow(), state.objects.try_borrow_mut())
    else {
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
                Mutation::HostedDocument { .. } => complete_rebuild = true,
                Mutation::DocumentOwnership { node, .. } => {
                    // Ownership journals identify every changed descendant,
                    // including attributes and template contents individually.
                    changed.insert(node);
                }
                Mutation::ChildrenReplaced { .. }
                | Mutation::CharacterData { .. }
                | Mutation::StyleSheets { .. }
                | Mutation::FormState { .. }
                | Mutation::ImageResource { .. }
                | Mutation::EventListener { .. }
                | Mutation::Focus { .. } => {}
            }
        }
    }
    // A newly materialized owner-document wrapper must become reachable from
    // every existing detached component that exposes it through ownerDocument.
    // Ordinary new node wrappers cannot be an ownerDocument target. Avoid
    // scanning every ownership entry for each newly materialized text/element.
    let new_owner_document = new_nodes.iter().any(|node| {
        dom.arena()
            .get(*node)
            .map_or(true, |record| matches!(record.kind, NodeKind::Document))
    });
    if new_owner_document {
        for (node, record) in dom.arena().iter() {
            if record
                .owner_document
                .is_some_and(|owner| new_nodes.contains(&owner))
                && let Some(object) = wrappers
                    .lookup(dom.arena().id(), state.realm, node)
                    .and_then(|id| objects.get(&id))
            {
                affected.extend(object.retention_component);
            }
        }
    }
    for node in changed {
        // Materializing a connected wrapper does not change existing roots.
        // Still select the new wrapper below so its component membership is
        // recorded for a later detach. Documents can add ownerDocument edges,
        // and any intervening mutation must retain the general repair path.
        if state.wrapper_graph_revision.get() == Some(revision)
            && new_nodes.contains(&node)
            && retained_by_document(&dom, node)
            && !matches!(
                dom.arena().get(node).map(|node| &node.kind),
                Ok(NodeKind::Document)
            )
        {
            continue;
        }
        // Both the former and current component matter when a subtree moves.
        if let Some(object) = wrappers
            .lookup(dom.arena().id(), state.realm, node)
            .and_then(|id| objects.get(&id))
        {
            affected.extend(object.retention_component);
        } else {
            // An unwrapped endpoint can still contain wrapped descendants.
            // Their cached components identify the former trees without
            // rebuilding every unrelated wrapper in this realm.
            let mut pending = vec![node];
            let mut seen = BTreeSet::new();
            while let Some(descendant) = pending.pop() {
                if !seen.insert(descendant) {
                    continue;
                }
                if let Some(object) = wrappers
                    .lookup(dom.arena().id(), state.realm, descendant)
                    .and_then(|id| objects.get(&id))
                {
                    affected.extend(object.retention_component);
                }
                let Ok(record) = dom.arena().get(descendant) else {
                    complete_rebuild = true;
                    break;
                };
                pending.extend(record.children.iter().copied());
                pending.extend(dom.arena().attribute_node_ids(descendant));
            }
        }
        affected.extend(native_root(&dom, node, &mut roots));
    }
    let mut components = state.wrapper_graph_components.borrow_mut();
    let mut selected = BTreeSet::new();
    if complete_rebuild {
        selected.extend(objects.keys().copied());
        components.clear();
    } else {
        for root in &affected {
            if let Some(members) = components.get(root) {
                selected.extend(members);
            }
        }
        for node in &new_nodes {
            selected.extend(wrappers.lookup(dom.arena().id(), state.realm, *node));
        }
    }
    state.wrapper_graph_new_nodes.borrow_mut().clear();
    state.wrapper_graph_revision.set(Some(revision));
    let mut owners = BTreeMap::new();
    let mut released = 0;
    let mut nodes = BTreeMap::new();
    let mut groups = BTreeMap::<NodeId, Vec<(NodeId, v8::Local<v8::Object>)>>::new();
    {
        for id in selected {
            let Some(object) = objects.get_mut(&id) else {
                continue;
            };
            let Some(metadata) = wrappers.metadata(id) else {
                continue;
            };
            if let Some(previous) = object.retention_component
                && let Some(members) = components.get_mut(&previous)
            {
                members.remove(&id);
                if members.is_empty() {
                    components.remove(&previous);
                }
            }
            object.retention_component = native_root(&dom, metadata.node, &mut roots);
            if let Some(root) = object.retention_component {
                components.entry(root).or_default().insert(id);
            }
            let Some(local) = object.handle.to_local(scope) else {
                continue;
            };
            let connected = !state.execution_retired() && retained_by_document(&dom, metadata.node);
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
            if let Ok(Some(owner)) = dom.arena().node_document(metadata.node) {
                owners.insert(metadata.node, owner);
            }
            nodes.insert(metadata.node, local);
        }
    }
    for owner in owners.values() {
        if let Some(local) = wrappers
            .lookup(dom.arena().id(), state.realm, *owner)
            .and_then(|id| objects.get(&id))
            .and_then(|object| object.handle.to_local(scope))
        {
            nodes.insert(*owner, local);
        }
    }
    // Detached descendants may have observable registries without wrappers.
    // Visit only affected retention trees rather than every node in the arena.
    let component_nodes = groups
        .keys()
        .map(|root| (*root, retention_component_nodes(&dom, *root)))
        .collect::<BTreeMap<_, _>>();
    drop(components);
    drop(objects);
    drop(wrappers);
    drop(dom);
    for (node, object) in &nodes {
        let result =
            custom_elements::trace_node_registry(scope, state, *node, *object).and_then(|()| {
                custom_elements::trace_component_registries(scope, state, &[], &[(*node, *object)])
            });
        if let Err(error) = result {
            state.wrapper_graph_revision.set(None);
            throw_binding_error(scope, &error);
        }
    }
    for (root, members) in &groups {
        if let Some(component) = component_nodes.get(root)
            && let Err(error) =
                custom_elements::trace_component_registries(scope, state, component, members)
        {
            state.wrapper_graph_revision.set(None);
            throw_binding_error(scope, &error);
        }
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
    refresh_agent(scope, state);
    released
}

/// V8-traced edges for nodes that have crossed document realms in one arena.
/// Only weak native handles are read; the arrays are ordinary collectable V8
/// cycles. No native Global cycle or cross-arena wrapper lookup is introduced.
pub(super) fn refresh_agent(scope: &mut v8::PinScope, state: &V8BindingState) {
    // Graph observation handles must end with this refresh, not the caller task.
    v8::scope!(let scope, scope);
    if !scope
        .get_slot::<BindingStateRegistry>()
        .is_some_and(|registry| registry.cross_realm_wrappers.get())
    {
        return;
    }
    let states = shared_document_states(scope, state);
    if states.len() < 2 {
        return;
    }
    let Ok(dom) = state.dom.try_borrow() else {
        return;
    };
    let mut nodes = BTreeMap::new();
    let mut roots = BTreeMap::new();
    let mut groups = BTreeMap::<NodeId, Vec<(NodeId, v8::Local<v8::Object>)>>::new();
    for other in states {
        let (Ok(wrappers), Ok(objects)) = (other.wrappers.try_borrow(), other.objects.try_borrow())
        else {
            continue;
        };
        for (id, object) in objects.iter() {
            let Some(metadata) = wrappers.metadata(*id) else {
                continue;
            };
            let Some(local) = object.handle.to_local(scope) else {
                continue;
            };
            if let Some(root) = native_root(&dom, metadata.node, &mut roots) {
                groups.entry(root).or_default().push((metadata.node, local));
                nodes.insert(metadata.node, local);
            }
        }
    }
    for members in groups.values() {
        let mut references = BTreeMap::new();
        for (node, object) in members {
            references.insert(*node, *object);
            if let Ok(Some(owner)) = dom.arena().node_document(*node)
                && let Some(object) = nodes.get(&owner)
            {
                references.insert(owner, *object);
            }
        }
        let values = references
            .values()
            .map(|object| (*object).into())
            .collect::<Vec<v8::Local<v8::Value>>>();
        let array: v8::Local<v8::Value> = v8::Array::new_with_elements(scope, &values).into();
        for (_, object) in members {
            object.set_internal_field(5, array.into());
        }
    }
}

// Materializing another wrapper in an unchanged detached tree only adds one
// traced member. Reuse its private reference array instead of walking the tree
// and rewriting every existing wrapper for each member of a query result.
pub(super) fn attach_existing_component(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
    object: v8::Local<v8::Object>,
) -> bool {
    if state.execution_retired()
        || node_migration::has_publications(scope)
        || !state.wrapper_graph_new_nodes.borrow().is_empty()
        || scope
            .get_slot::<BindingStateRegistry>()
            .is_some_and(|registry| registry.cross_realm_wrappers.get())
    {
        return false;
    }
    let Ok(dom) = state.dom.try_borrow() else {
        return false;
    };
    if state.wrapper_graph_revision.get() != Some(dom.mutation_cursor().0)
        || retained_by_document(&dom, node)
        || object.internal_field_count() <= 5
    {
        return false;
    }
    let Some(root) = native_root(&dom, node, &mut BTreeMap::new()) else {
        return false;
    };
    let Ok(Some(owner)) = dom.arena().node_document(node) else {
        return false;
    };
    let (id, array, length) = {
        let (Ok(wrappers), Ok(objects), Ok(components)) = (
            state.wrappers.try_borrow(),
            state.objects.try_borrow(),
            state.wrapper_graph_components.try_borrow(),
        ) else {
            return false;
        };
        let Some(id) = wrappers.lookup(dom.arena().id(), state.realm, node) else {
            return false;
        };
        let Some(members) = components.get(&root) else {
            return false;
        };
        let Some(representative) = members.first() else {
            return false;
        };
        let Some(metadata) = wrappers.metadata(*representative) else {
            return false;
        };
        // An existing member must already trace this node's ownerDocument.
        // New documents and complex cross-document components use rebuild.
        if members.contains(&id)
            || dom.arena().node_document(metadata.node).ok().flatten() != Some(owner)
            || wrappers
                .lookup(dom.arena().id(), state.realm, owner)
                .and_then(|owner| objects.get(&owner))
                .and_then(|owner| owner.handle.to_local(scope))
                .is_none()
        {
            return false;
        }
        let Some(existing) = objects.get(representative) else {
            return false;
        };
        if existing.retention_component != Some(root) || existing.connected_root.is_some() {
            return false;
        }
        let Some(array) = existing
            .handle
            .to_local(scope)
            .and_then(|object| object.get_internal_field(scope, 5))
            .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        else {
            return false;
        };
        let Some(length) = members
            .len()
            .checked_add(1)
            .and_then(|length| u32::try_from(length).ok())
            .filter(|length| *length < u32::MAX)
        else {
            return false;
        };
        if array.length() != length {
            return false;
        }
        (id, array, length)
    };
    drop(dom);
    let Some(key) = v8::String::new(scope, &length.to_string()) else {
        return false;
    };
    let (Ok(mut objects), Ok(mut components)) = (
        state.objects.try_borrow_mut(),
        state.wrapper_graph_components.try_borrow_mut(),
    ) else {
        return false;
    };
    let (Some(wrapper), Some(members)) = (objects.get_mut(&id), components.get_mut(&root)) else {
        return false;
    };
    if wrapper.retention_component.is_some()
        || wrapper.connected_root.is_some()
        || !members.insert(id)
    {
        return false;
    }
    // No wrapper-registry borrow crosses allocation. The private array cannot
    // invoke authored setters; Locals retain its members across a collection.
    if array.create_data_property(scope, key.into(), object.into()) != Some(true) {
        members.remove(&id);
        return false;
    }
    object.set_internal_field(5, array.into());
    wrapper.retention_component = Some(root);
    // Existing members already trace registries throughout the native tree,
    // including its previously unwrapped nodes. The shared array retains them.
    true
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
    let (Ok(wrappers), Ok(objects)) = (state.wrappers.try_borrow(), state.objects.try_borrow())
    else {
        return false;
    };
    let owner = dom
        .arena()
        .node_document(node)
        .ok()
        .flatten()
        .and_then(|owner| wrappers.lookup(dom.arena().id(), state.realm, owner))
        .and_then(|id| objects.get(&id))
        .and_then(|owner| owner.handle.to_local(scope));
    drop(objects);
    drop(wrappers);
    let arena = dom.arena().id();
    drop(dom);
    let reference = owner.map_or_else(|| v8::undefined(scope).into(), |owner| owner.into());
    object.set_internal_field(5, reference);
    if let Some(id) = state.wrappers.borrow().lookup(arena, state.realm, node)
        && let Some(wrapper) = state.objects.borrow_mut().get_mut(&id)
    {
        wrapper.retention_component = Some(node);
        state
            .wrapper_graph_components
            .borrow_mut()
            .entry(node)
            .or_default()
            .insert(id);
    }
    state.wrapper_graph_revision.set(Some(revision));
    true
}

#[cfg(test)]
mod component_traversal_tests {
    use super::*;

    fn compare_with_arena_partition(dom: &DomRuntime) {
        let mut roots = BTreeMap::new();
        let mut expected = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
        for (node, _) in dom.arena().iter() {
            if let Some(root) = native_root(dom, node, &mut roots) {
                expected.entry(root).or_default().insert(node);
            }
        }
        for (root, members) in expected {
            assert_eq!(
                retention_component_nodes(dom, root)
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                members,
                "retention tree rooted at {root:?}"
            );
        }
    }

    #[test]
    fn affected_tree_traversal_matches_arena_partition_across_native_edges() {
        let mut dom = oryn_dom::html::parse_inert_with_declarative_shadow(
            "<body><main id=parent><span id=child data-value=kept></span></main><iframe id=frame></iframe><template id=template><b data-value=inert></b></template><div id=shadowhost><template shadowrootmode=open><i>shadow</i></template></div></body>"
        ).into_runtime().unwrap();
        let shadow_host = dom.find_by_id("shadowhost").unwrap();
        let shadow = dom
            .attached_shadow_root(shadow_host, false)
            .unwrap()
            .unwrap();
        let template = dom.find_by_id("template").unwrap();
        let content = dom.arena().get(template).unwrap().template_content.unwrap();
        assert!(!dom.arena().get(content).unwrap().children.is_empty());
        assert!(retention_component_nodes(&dom, dom.document()).contains(&shadow));
        assert!(!retention_component_nodes(&dom, dom.document()).contains(&content));
        compare_with_arena_partition(&dom);
        let child = dom.find_by_id("child").unwrap();
        dom.attribute_node(child, "data-value").unwrap().unwrap();
        let frame = dom.find_by_id("frame").unwrap();
        let hosted = dom.create(NodeKind::Document);
        let hosted_child = dom.create_element("section");
        dom.append_child(hosted, hosted_child).unwrap();
        dom.set_hosted_document(frame, Some(hosted)).unwrap();
        compare_with_arena_partition(&dom);

        let first = dom.create_fragment();
        let second = dom.create_fragment();
        dom.retain_native(first).unwrap();
        dom.retain_native(second).unwrap();
        dom.append_child(first, child).unwrap();
        compare_with_arena_partition(&dom);
        dom.append_child(second, child).unwrap();
        compare_with_arena_partition(&dom);
        dom.append_child(second, frame).unwrap();
        compare_with_arena_partition(&dom);
        assert!(!retention_component_nodes(&dom, first).contains(&child));
        assert!(retention_component_nodes(&dom, second).contains(&hosted_child));
        dom.release_native(first).unwrap();
        dom.release_native(second).unwrap();
    }
}

#[cfg(test)]
mod fragment_growth_tests {
    use super::*;

    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }

    fn graph<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<v8::Object>,
    ) -> v8::Local<'s, v8::Array> {
        let field = object.get_internal_field(scope, 5).unwrap();
        let value = v8::Local::<v8::Value>::try_from(field).unwrap();
        v8::Local::<v8::Array>::try_from(value).unwrap()
    }

    #[test]
    fn materializing_detached_nodes_reuses_graph_and_split_remains_collectable() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(309), 1).unwrap();
        let old_root = {
            v8::scope!(let scope, scope);
            let root = v8::Local::<v8::Object>::try_from(eval(
                scope,
                r#"
                globalThis.root=document.createElement('section');
                root.innerHTML='<b>first</b><i>last</i>';root.saved='root';root
            "#,
            ))
            .unwrap();
            refresh(scope, &state);
            let before = graph(scope, root);
            let length = before.length();
            eval(
                scope,
                &format!(
                    "globalThis.setterCalls=0;Object.defineProperty(Array.prototype,'{length}',{{configurable:true,set(){{setterCalls++}}}})"
                ),
            );
            let child = v8::Local::<v8::Object>::try_from(eval(scope,
                "globalThis.held=root.firstChild;held.saved='child';root.lastChild.saved='sibling';held"
            )).unwrap();
            let after = graph(scope, child);
            assert!(before.strict_equals(after.into()));
            assert_eq!(after.length(), length + 2);
            assert!(eval(scope, "setterCalls===0 && held.ownerDocument===document").is_true());
            eval(
                scope,
                &format!("delete Array.prototype['{length}'];root=null"),
            );
            v8::Weak::new(scope, root)
        };
        scope.low_memory_notification();
        {
            v8::scope!(let scope, scope);
            assert!(old_root.to_local(scope).is_some());
            assert!(eval(scope,
                "held.parentNode.saved==='root' && held.nextSibling.saved==='sibling' && held.saved==='child'"
            ).is_true());
            eval(scope, "held.remove()");
        }
        scope.low_memory_notification();
        assert!(old_root.is_empty());
        assert!(
            eval(
                scope,
                "held.parentNode===null && held.saved==='child' && held.ownerDocument===document"
            )
            .is_true()
        );
    }

    #[test]
    fn materializing_node_rebuilds_when_existing_graph_cannot_grow() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(310), 1).unwrap();
        let root = v8::Local::<v8::Object>::try_from(eval(
            scope,
            "globalThis.root=document.createElement('section');root.innerHTML='<b></b>';root",
        ))
        .unwrap();
        refresh(scope, &state);
        let before = graph(scope, root);
        assert_eq!(
            before.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
            Some(true)
        );
        let child = v8::Local::<v8::Object>::try_from(eval(scope, "root.firstChild")).unwrap();
        let after = graph(scope, child);
        assert!(!before.strict_equals(after.into()));
        assert!(after.strict_equals(graph(scope, root).into()));
        assert!(eval(scope, "root.firstChild.parentNode===root").is_true());
    }

    #[test]
    fn fragment_growth_reuses_graph_without_setters_and_split_releases_old_component() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(306), 1).unwrap();
        let old_fragment = {
            v8::scope!(let scope, scope);
            let fragment = v8::Local::<v8::Object>::try_from(eval(
                scope,
                r#"
                (()=>{globalThis.fragment=document.createDocumentFragment();
                const seed=document.createElement('i');seed.saved='seed';fragment.appendChild(seed);
                globalThis.held=document.createElement('b');held.saved='leaf';return fragment;})()
            "#,
            ))
            .unwrap();
            let leaf = v8::Local::<v8::Object>::try_from(eval(scope, "held")).unwrap();
            let parent = resolve_receiver(scope, &state, fragment, "Node").unwrap();
            let node = resolve_receiver(scope, &state, leaf, "Node").unwrap();
            let before = graph(scope, fragment);
            let length = before.length();
            eval(
                scope,
                &format!(
                    "globalThis.setterCalls=0;Object.defineProperty(Array.prototype,'{length}',{{configurable:true,set(){{setterCalls++}}}})"
                ),
            );
            state
                .dom
                .borrow_mut()
                .insert_dom_node(parent, node, None)
                .unwrap();
            assert!(grow_detached_fragment(scope, &state).unwrap());
            let after = graph(scope, leaf);
            assert!(before.strict_equals(after.into()));
            assert_eq!(after.length(), length + 1);
            assert!(
                eval(
                    scope,
                    "setterCalls===0 && held.parentNode===fragment && held.ownerDocument===document"
                )
                .is_true()
            );
            eval(
                scope,
                &format!("delete Array.prototype['{length}'];fragment=null"),
            );
            v8::Weak::new(scope, fragment)
        };
        scope.low_memory_notification();
        {
            v8::scope!(let scope, scope);
            assert!(old_fragment.to_local(scope).is_some());
            assert!(
                eval(
                    scope,
                    "held.parentNode.firstChild.saved==='seed' && held.saved==='leaf'"
                )
                .is_true()
            );
            eval(scope, "held.remove()");
        }
        scope.low_memory_notification();
        assert!(old_fragment.is_empty());
        assert!(
            eval(
                scope,
                "held.parentNode===null && held.saved==='leaf' && held.ownerDocument===document"
            )
            .is_true()
        );
    }

    #[test]
    fn failed_graph_append_preserves_singleton_index_and_pending_journal_for_rebuild() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(307), 1).unwrap();
        let fragment = v8::Local::<v8::Object>::try_from(eval(scope, "globalThis.fragment=document.createDocumentFragment();fragment.appendChild(document.createTextNode('seed'));globalThis.leaf=document.createElement('b');fragment")).unwrap();
        let leaf = v8::Local::<v8::Object>::try_from(eval(scope, "leaf")).unwrap();
        let parent = resolve_receiver(scope, &state, fragment, "Node").unwrap();
        let node = resolve_receiver(scope, &state, leaf, "Node").unwrap();
        let old = graph(scope, fragment);
        assert_eq!(
            old.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
            Some(true)
        );
        let previous = state.wrapper_graph_revision.get();
        state
            .dom
            .borrow_mut()
            .insert_dom_node(parent, node, None)
            .unwrap();
        assert!(!grow_detached_fragment(scope, &state).unwrap());
        assert_eq!(state.wrapper_graph_revision.get(), previous);
        let id = state
            .wrappers
            .borrow()
            .lookup(state.dom.borrow().arena().id(), state.realm, node)
            .unwrap();
        assert_eq!(
            state.objects.borrow().get(&id).unwrap().retention_component,
            Some(node)
        );
        assert!(
            state
                .wrapper_graph_components
                .borrow()
                .get(&node)
                .unwrap()
                .contains(&id)
        );
        assert!(
            !state
                .wrapper_graph_components
                .borrow()
                .get(&parent)
                .unwrap()
                .contains(&id)
        );
        refresh(scope, &state);
        let repaired = graph(scope, fragment);
        let leaf_graph = graph(scope, leaf);
        assert!(!old.strict_equals(repaired.into()));
        assert!(repaired.strict_equals(leaf_graph.into()));
        assert_eq!(
            state.wrapper_graph_revision.get(),
            Some(state.dom.borrow().mutation_cursor().0)
        );
        assert!(eval(scope, "leaf.parentNode===fragment && fragment.lastChild===leaf && leaf.ownerDocument===document").is_true());
    }
    #[test]
    fn growth_unites_new_leaf_registry_with_existing_unwrapped_descendant_registry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(308), 1).unwrap();
        {
            v8::scope!(let scope, scope);
            let fragment = v8::Local::<v8::Object>::try_from(eval(scope, r#"(()=>{
                const oldRegistry=new CustomElementRegistry();
                oldRegistry.define('x-old',class extends HTMLElement{ping(){return 42}});
                const source=document.createElement('section');
                source.appendChild(document.createElement('x-old',{customElementRegistry:oldRegistry}));
                globalThis.fragment=document.createDocumentFragment();
                fragment.appendChild(source.cloneNode(true));
                const newRegistry=new CustomElementRegistry();
                newRegistry.define('x-new',class extends HTMLElement{ping(){return 73}});
                globalThis.held=document.createElement('x-new',{customElementRegistry:newRegistry});
                return fragment;
            })()"#)).unwrap();
            let leaf = v8::Local::<v8::Object>::try_from(eval(scope, "held")).unwrap();
            let parent = resolve_receiver(scope, &state, fragment, "Node").unwrap();
            let node = resolve_receiver(scope, &state, leaf, "Node").unwrap();
            let old = graph(scope, fragment);
            state
                .dom
                .borrow_mut()
                .insert_dom_node(parent, node, None)
                .unwrap();
            assert!(grow_detached_fragment(scope, &state).unwrap());
            let current = graph(scope, leaf);
            assert!(old.strict_equals(current.into()));
            eval(scope, "fragment=null");
        }
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let scope, scope);
            assert!(
                eval(
                    scope,
                    r#"(()=>{
                const old=held.parentNode.firstChild.firstChild;
                return old.ping()===42 && held.ping()===73
                    && old.customElementRegistry.get('x-old')===old.constructor
                    && held.customElementRegistry.get('x-new')===held.constructor;
            })()"#
                )
                .is_true()
            );
        }
    }
}
