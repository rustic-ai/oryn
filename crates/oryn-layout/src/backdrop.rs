//! Independently formatted viewport-containing backdrop boxes. Source owner
//! ancestry never contributes transforms or clips to a top-layer backdrop.
use super::*;
use oryn_common::v4::{PaintNodeSnapshot, TopLayerKind};
pub(super) fn append(
    input: &LayoutInput,
    limits: &LayoutLimits,
    output: &mut LayoutOutput,
) -> Result<(), LayoutError> {
    append_reserved(input, limits, output, &BTreeSet::new())
}
pub(super) fn append_reserved(
    input: &LayoutInput,
    limits: &LayoutLimits,
    output: &mut LayoutOutput,
    reserved: &BTreeSet<u64>,
) -> Result<(), LayoutError> {
    let mut used = input
        .nodes
        .iter()
        .map(|node| node.id)
        .chain(output.geometry.keys().copied())
        .collect::<BTreeSet<_>>();
    used.extend(reserved.iter().copied());
    for entry in &input.top_layer {
        if matches!(
            entry.kind,
            TopLayerKind::SelectPicker
                | TopLayerKind::TemporalPicker
                | TopLayerKind::PermissionPrompt
        ) {
            continue;
        }
        let Some(style) = output
            .pseudo_styles
            .get(&entry.node_id)
            .and_then(|styles| styles.get("backdrop"))
            .cloned()
        else {
            continue;
        };
        if style_value(&style, "display", "block") == "none" {
            continue;
        }
        if !output
            .geometry
            .get(&entry.node_id)
            .is_some_and(|geometry| geometry.availability == GeometryAvailability::Available)
        {
            continue;
        }
        if used.len() >= limits.max_nodes || output.backdrops.len() >= 1024 {
            return Err(LayoutError::StyleTraversal(
                "native backdrop count exceeds layout bound".into(),
            ));
        }
        let mut id = u64::MAX;
        while used.contains(&id) {
            id = id
                .checked_sub(1)
                .ok_or_else(|| LayoutError::StyleTraversal("backdrop identity exhausted".into()))?;
        }
        used.insert(id);
        let Some(owner) = input.nodes.iter().find(|node| node.id == entry.node_id) else {
            continue;
        };
        let mut node = owner.clone();
        node.id = id;
        node.parent = None;
        node.children.clear();
        node.content = Some(Vec::new());
        node.text.clear();
        node.tag = "oryn-native-backdrop".into();
        node.attributes.clear();
        node.namespaced_attributes.clear();
        node.checked = None;
        node.validity = None;
        node.disabled = None;
        let mut local = input.clone();
        local.nodes = vec![node];
        local.top_layer.clear();
        local.rendered_tree = None;
        local.scroll = ScrollState::default();
        local.element_scrolls.clear();
        let layout = compute_with_styles(&local, limits, [(id, style.clone())].into())?;
        let Some(geometry) = layout.geometry.get(&id).cloned() else {
            continue;
        };
        output.backdrops.insert(
            entry.node_id,
            PaintNodeSnapshot {
                builtin_image: None,
                backdrop: None,
                control: None,
                backdrop_style: None,
                top_layer: None,
                node_id: id,
                parent_id: Some(entry.node_id),
                tag: "oryn-native-backdrop".into(),
                geometry,
                projection: layout.paint_projections.get(&id).cloned(),
                style: (*style).clone(),
                text: Vec::new(),
                image_resource: None,
            },
        );
        output.diagnostics.extend(layout.diagnostics);
    }
    Ok(())
}
