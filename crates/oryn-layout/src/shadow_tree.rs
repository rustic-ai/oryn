//! Selector and formatting trees share identities, never ancestry.
use super::*;
pub(super) fn validate(input: &LayoutInput) -> Result<(), LayoutError> {
    let fail = |message: &str| LayoutError::InvalidRenderedTree(message.into());
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut hosts = BTreeSet::new();
    let mut direct = BTreeSet::new();
    for (root, tree) in &input.shadow_trees.roots {
        if nodes.contains_key(root) || !nodes.contains_key(&tree.host) || !hosts.insert(tree.host) {
            return Err(fail("invalid or duplicate shadow host"));
        }
        for item in &tree.content {
            if let LayoutContent::Element(id) = item {
                if !nodes.contains_key(id)
                    || input.shadow_trees.node_scopes.get(id) != Some(root)
                    || !direct.insert(*id)
                    || nodes[id].parent.is_some()
                {
                    return Err(fail("invalid shadow root child ownership"));
                }
            }
        }
        let mut visited = BTreeSet::new();
        let mut current = Some(*root);
        while let Some(root) = current {
            if !visited.insert(root) {
                return Err(fail("cyclic shadow host ancestry"));
            }
            let tree = input
                .shadow_trees
                .roots
                .get(&root)
                .ok_or_else(|| fail("unknown ancestor shadow scope"))?;
            current = input.shadow_trees.node_scopes.get(&tree.host).copied();
        }
    }
    for (id, root) in &input.shadow_trees.node_scopes {
        let node = nodes
            .get(id)
            .ok_or_else(|| fail("unknown scoped element"))?;
        if !input.shadow_trees.roots.contains_key(root) {
            return Err(fail("unknown shadow root"));
        }
        if let Some(parent) = node.parent {
            if input.shadow_trees.node_scopes.get(&parent) != Some(root) {
                return Err(fail("selector parent crosses shadow boundary"));
            }
        } else if !direct.contains(id) {
            return Err(fail("orphaned shadow member"));
        }
    }
    for (index, root) in &input.shadow_trees.stylesheet_scopes {
        if *index >= input.stylesheets.len() || !input.shadow_trees.roots.contains_key(root) {
            return Err(fail("unknown shadow stylesheet owner"));
        }
    }
    for (id, slot) in &input.shadow_trees.assigned_slots {
        let target = nodes
            .get(slot)
            .ok_or_else(|| fail("unknown assigned slot"))?;
        let root = input
            .shadow_trees
            .node_scopes
            .get(slot)
            .and_then(|root| input.shadow_trees.roots.get(root))
            .ok_or_else(|| fail("slot outside shadow tree"))?;
        if target.tag != "slot" || target.namespace != "http://www.w3.org/1999/xhtml" {
            return Err(fail("assignment target is not an HTML slot"));
        }
        let owner = nodes.values().find(|node| {
            node.children.contains(id)
                || node.content.as_ref().is_some_and(|content| {
                    content
                        .iter()
                        .any(|item| matches!(item,LayoutContent::Text{node_id,..} if node_id==id))
                })
        });
        if owner.map(|node| node.id) != Some(root.host) {
            return Err(fail("slot assignment crosses native host"));
        }
    }
    Ok(())
}
fn content(node: &LayoutNode) -> Vec<LayoutContent> {
    node.content.clone().unwrap_or_else(|| {
        let mut content = Vec::new();
        if !node.text.is_empty() {
            content.push(LayoutContent::StyledText {
                owner: node.id,
                node_id: None,
                text: node.text.clone(),
            });
        }
        content.extend(node.children.iter().copied().map(LayoutContent::Element));
        content
    })
}
pub(super) fn rendered(input: &LayoutInput) -> BTreeMap<u64, Vec<LayoutContent>> {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let hosts = input
        .shadow_trees
        .roots
        .values()
        .map(|tree| (tree.host, tree))
        .collect::<BTreeMap<_, _>>();
    let mut assigned = BTreeMap::<u64, Vec<LayoutContent>>::new();
    for node in &input.nodes {
        for item in content(node) {
            let id = match &item {
                LayoutContent::Element(id) | LayoutContent::Text { node_id: id, .. } => Some(*id),
                _ => None,
            };
            if let Some(slot) = id.and_then(|id| input.shadow_trees.assigned_slots.get(&id)) {
                assigned.entry(*slot).or_default().push(item);
            }
        }
    }
    let mut pending = input
        .nodes
        .iter()
        .filter(|node| {
            node.parent.is_none() && !input.shadow_trees.node_scopes.contains_key(&node.id)
        })
        .map(|node| node.id)
        .collect::<Vec<_>>();
    let mut result = BTreeMap::new();
    while let Some(id) = pending.pop() {
        if result.contains_key(&id) {
            continue;
        }
        let Some(node) = nodes.get(&id) else { continue };
        let children = if let Some(tree) = hosts.get(&id) {
            tree.content.clone()
        } else {
            assigned.remove(&id).unwrap_or_else(|| content(node))
        };
        pending.extend(children.iter().filter_map(|item| {
            if let LayoutContent::Element(id) = item {
                Some(*id)
            } else {
                None
            }
        }));
        result.insert(id, children);
    }
    result
}

/// Focus-within follows the flat tree, unlike selector combinator ancestry.
pub(super) fn focus_within(input: &LayoutInput, focused: u64) -> BTreeSet<u64> {
    let native;
    let tree = if let Some(tree) = &input.rendered_tree {
        tree
    } else {
        native = rendered(input);
        &native
    };
    let mut parents = BTreeMap::new();
    for (parent, children) in tree {
        for child in children {
            if let LayoutContent::Element(child) = child {
                parents.insert(*child, *parent);
            }
        }
    }
    let mut result = BTreeSet::new();
    let mut current = Some(focused);
    while let Some(node) = current.filter(|node| result.insert(*node)) {
        current = parents.get(&node).copied();
    }
    result
}
