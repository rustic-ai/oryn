//! A successfully decoded object image replaces its fallback formatting tree.
//! The DOM and computed styles remain available; no fallback box, including a
//! top-layer descendant, can escape this replacement boundary.
use super::*;
/// Apply natural dimensions from the SVG root's computed geometry, before any
/// used percentage sizes are resolved against the embedding object's viewport.
pub(super) fn project_styled<'a>(
    input: &'a LayoutInput,
    styles: &ComputedStyles,
) -> (Cow<'a, LayoutInput>, BTreeSet<u64>) {
    let needs_projection = input.nodes.iter().any(|node| {
        (node.image_intrinsics.is_some()
            && node
                .image_document_root
                .is_some_and(|root| styles.contains_key(&root)))
            || (node.namespace == "http://www.w3.org/1999/xhtml"
                && (node.tag == "embed"
                    || (node.tag == "object"
                        && (node.intrinsic_image_size.is_some() || node.document_replaced))))
    });
    if !needs_projection {
        return (Cow::Borrowed(input), BTreeSet::new());
    }
    let mut input = input.clone();
    for node in &mut input.nodes {
        let Some(root) = node.image_document_root else {
            continue;
        };
        let Some(intrinsics) = node.image_intrinsics else {
            continue;
        };
        let Some(style) = styles.get(&root) else {
            continue;
        };
        let absolute = |name: &str| {
            style
                .get(name)
                .and_then(|value| value.strip_suffix("px"))
                .and_then(|value| value.trim().parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value >= 0.)
        };
        let intrinsics = intrinsics.with_dimensions(absolute("width"), absolute("height"));
        node.image_intrinsics = Some(intrinsics);
        node.intrinsic_image_size = Some(intrinsics.default_size());
    }
    let (projected, hidden) = project(&input);
    (Cow::Owned(projected), hidden)
}
pub(super) fn project(input: &LayoutInput) -> (LayoutInput, BTreeSet<u64>) {
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let replaced = input
        .nodes
        .iter()
        .filter(|node| {
            node.namespace == "http://www.w3.org/1999/xhtml"
                && matches!(node.tag.as_str(), "object" | "embed")
                && (node.intrinsic_image_size.is_some() || node.document_replaced)
        })
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    // An embed without a selected image or Document represents nothing, even
    // when authored dimensions/background would otherwise create a box.
    let empty_embeds = input
        .nodes
        .iter()
        .filter(|node| {
            node.namespace == "http://www.w3.org/1999/xhtml"
                && node.tag == "embed"
                && node.intrinsic_image_size.is_none()
                && !node.document_replaced
        })
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    if replaced.is_empty() && empty_embeds.is_empty() {
        return (input.clone(), BTreeSet::new());
    }
    let flat = project_rendered_tree(input);
    let flat_by_id = flat
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut hidden = BTreeSet::new();
    let mut pending = empty_embeds.iter().copied().collect::<Vec<_>>();
    pending.extend(
        replaced
            .iter()
            .filter_map(|id| by_id.get(id))
            .flat_map(|node| node.children.iter().copied()),
    );
    for id in &replaced {
        if let Some(node) = flat_by_id.get(id) {
            pending.extend(node.children.iter().copied());
        }
    }
    while let Some(id) = pending.pop() {
        if hidden.insert(id) {
            if let Some(node) = by_id.get(&id) {
                pending.extend(node.children.iter().copied());
            }
            if let Some(node) = flat_by_id.get(&id) {
                pending.extend(node.children.iter().copied());
            }
            for tree in input
                .shadow_trees
                .roots
                .values()
                .filter(|tree| tree.host == id)
            {
                pending.extend(tree.content.iter().filter_map(|item| {
                    if let LayoutContent::Element(id) = item {
                        Some(*id)
                    } else {
                        None
                    }
                }));
            }
        }
    }
    let mut result = input.clone();
    result.nodes.retain(|node| !hidden.contains(&node.id));
    for node in &mut result.nodes {
        if replaced.contains(&node.id) {
            node.children.clear();
            node.content = Some(Vec::new());
            node.text.clear();
        } else {
            node.children.retain(|id| !hidden.contains(id));
            if let Some(content) = node.content.as_mut() {
                content.retain(
                    |entry| !matches!(entry,LayoutContent::Element(id)if hidden.contains(id)),
                );
            }
        }
    }
    result
        .shadow_trees
        .roots
        .retain(|_, tree| !hidden.contains(&tree.host) && !replaced.contains(&tree.host));
    for tree in result.shadow_trees.roots.values_mut() {
        tree.content
            .retain(|entry| !matches!(entry,LayoutContent::Element(id)if hidden.contains(id)));
    }
    for (index, scope) in &input.shadow_trees.stylesheet_scopes {
        if !result.shadow_trees.roots.contains_key(scope)
            && let Some(sheet) = result.stylesheets.get_mut(*index)
        {
            sheet.clear();
        }
    }
    result
        .shadow_trees
        .node_scopes
        .retain(|id, scope| !hidden.contains(id) && result.shadow_trees.roots.contains_key(scope));
    result
        .shadow_trees
        .stylesheet_scopes
        .retain(|_, scope| result.shadow_trees.roots.contains_key(scope));
    result
        .shadow_trees
        .assigned_slots
        .retain(|id, slot| !hidden.contains(id) && !hidden.contains(slot));
    result
        .top_layer
        .retain(|entry| !hidden.contains(&entry.node_id));
    if let Some(tree) = result.rendered_tree.as_mut() {
        tree.retain(|id, _| !hidden.contains(id));
        for (id, content) in tree.iter_mut() {
            if replaced.contains(id) {
                content.clear();
            } else {
                content.retain(
                    |entry| !matches!(entry,LayoutContent::Element(id)if hidden.contains(id)),
                );
            }
        }
    }
    (result, hidden)
}
pub(super) fn finish(output: &mut LayoutOutput, hidden: &BTreeSet<u64>) {
    for id in hidden {
        output.geometry.insert(
            *id,
            NodeGeometry {
                availability: GeometryAvailability::Hidden,
                box_model: None,
                client_rects: Vec::new(),
                in_viewport: false,
                interactable: false,
                diagnostic: None,
            },
        );
        output.paint_projections.remove(id);
        output.text_fragments.remove(id);
        output.backdrops.remove(id);
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn ordinary_and_fallback_documents_borrow_but_empty_embeds_project() {
        let mut input: LayoutInput = serde_json::from_value(serde_json::json!({
            "nodes": [{"id": 1, "parent": null, "tag": "object", "text": "fallback"}],
            "stylesheets": [],
            "viewport": {"width": 800., "height": 600., "device_pixel_ratio": 1.},
            "scroll": ScrollState::default()
        }))
        .unwrap();
        let styles = ComputedStyles::new();
        for tag in ["body", "object"] {
            input.nodes[0].tag = tag.into();
            let (projected, hidden) = project_styled(&input, &styles);
            assert!(matches!(projected, Cow::Borrowed(_)));
            assert!(std::ptr::eq(projected.as_ref(), &input));
            assert!(hidden.is_empty());
            assert_eq!(projected.nodes[0].text, "fallback");
        }
        input.nodes[0].tag = "embed".into();
        let (projected, hidden) = project_styled(&input, &styles);
        assert!(matches!(projected, Cow::Owned(_)));
        assert!(hidden.contains(&1));
        assert!(projected.nodes.is_empty());
        assert_eq!(input.nodes.len(), 1);
    }
}
