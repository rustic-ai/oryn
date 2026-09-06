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
        match dom.parent(current).ok()? {
            Some(parent) => current = parent,
            None => break current,
        }
    };
    for node in path {
        roots.insert(node, root);
    }
    Some(root)
}

pub(super) fn refresh(scope: &mut v8::PinScope, state: &V8BindingState) -> usize {
    let Ok(dom) = state.dom.try_borrow() else {
        state.wrapper_graph_revision.set(None);
        return 0;
    };
    let revision = dom.mutation_cursor().0;
    if state.wrapper_graph_revision.get() == Some(revision) {
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
    state.wrapper_graph_revision.set(Some(revision));
    let mut owners = BTreeMap::new();
    let mut released = 0;
    let mut nodes = BTreeMap::new();
    let mut groups = BTreeMap::<NodeId, Vec<(NodeId, v8::Local<v8::Object>)>>::new();
    {
        let mut roots = BTreeMap::new();
        for (id, object) in objects.iter_mut() {
            let Some(metadata) = wrappers.metadata(*id) else {
                continue;
            };
            let Some(local) = object.handle.to_local(scope) else {
                continue;
            };
            let connected = dom.is_connected(metadata.node).unwrap_or(false);
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
