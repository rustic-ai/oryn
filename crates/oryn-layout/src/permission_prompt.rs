//! Browser-owned permission choices, painted outside authored element content.
use super::*;
use oryn_common::v4::{PaintNodeSnapshot, TopLayerKind};
pub(super) fn append(
    input: &LayoutInput,
    limits: &LayoutLimits,
    output: &mut LayoutOutput,
) -> Result<(), LayoutError> {
    let mut used = input
        .nodes
        .iter()
        .map(|node| node.id)
        .chain(output.geometry.keys().copied())
        .collect::<BTreeSet<_>>();
    for (order, entry) in input.top_layer.iter().enumerate() {
        if entry.kind != TopLayerKind::PermissionPrompt {
            continue;
        }
        let Some(prompt) = input.permission_prompts.get(&entry.node_id) else {
            continue;
        };
        let Some(owner) = input.nodes.iter().find(|node| node.id == entry.node_id) else {
            continue;
        };
        if used.len() + 6 > limits.max_nodes {
            return Err(LayoutError::StyleTraversal(
                "permission prompt node budget exceeded".into(),
            ));
        }
        let width = 360f64.min(input.viewport.width.max(1.));
        let height = 144f64.min(input.viewport.height.max(1.));
        let left = ((input.viewport.width - width) / 2.).max(0.);
        let top = ((input.viewport.height - height) / 2.).max(0.);
        let mut next = u64::MAX;
        let mut allocate = || {
            while used.contains(&next) {
                next -= 1
            }
            let id = next;
            used.insert(id);
            next -= 1;
            id
        };
        let wrapper = allocate();
        let mut nodes = Vec::new();
        let mut tags = BTreeMap::new();
        let mut styles = BTreeMap::new();
        let style = |x: f64, y: f64, w: f64, h: f64, background: &str| {
            Arc::new(BTreeMap::from([
                ("display".into(), "block".into()),
                ("position".into(), "fixed".into()),
                ("left".into(), format!("{x}px")),
                ("top".into(), format!("{y}px")),
                ("width".into(), format!("{w}px")),
                ("height".into(), format!("{h}px")),
                ("font-size".into(), "14px".into()),
                ("font-family".into(), "sans-serif".into()),
                ("line-height".into(), "28px".into()),
                ("color".into(), "black".into()),
                ("background-color".into(), background.into()),
                ("box-sizing".into(), "border-box".into()),
                ("padding".into(), "4px".into()),
                ("overflow".into(), "hidden".into()),
                ("text-align".into(), "center".into()),
            ]))
        };
        let mut root = owner.clone();
        root.id = wrapper;
        root.tag = "oryn-native-permission-prompt".into();
        root.parent = None;
        root.children.clear();
        root.content = Some(Vec::new());
        root.text.clear();
        root.attributes.clear();
        root.namespaced_attributes.clear();
        root.validity = None;
        root.disabled = None;
        root.checked = None;
        styles.insert(wrapper, style(left, top, width, height, "#eeeeee"));
        tags.insert(wrapper, "::picker(permission)".to_string());
        let mut entries = vec![
            (
                "title",
                if let Some(scheme) = &prompt.protocol {
                    format!("Open {scheme}: links with this site?")
                } else if prompt.geolocation {
                    "Share your location?".to_owned()
                } else if owner.tag == "microphone" {
                    "Use your microphone?".to_owned()
                } else if owner.tag == "camera" {
                    "Use your camera?".to_owned()
                } else {
                    "Use your camera and microphone?".to_owned()
                },
                left,
                top,
                width,
            ),
            ("origin", prompt.origin.clone(), left, top + 36., width),
        ];
        for (index, (tag, label)) in [
            ("allow", "Allow"),
            ("deny", "Don't allow"),
            ("dismiss", "Not now"),
        ]
        .into_iter()
        .enumerate()
        {
            entries.push((
                tag,
                label.to_owned(),
                left + index as f64 * width / 3.,
                top + height - 36.,
                width / 3.,
            ));
        }
        for (tag, label, x, y, w) in entries {
            let id = allocate();
            let mut node = root.clone();
            node.id = id;
            node.parent = Some(wrapper);
            node.content = Some(vec![LayoutContent::Text {
                node_id: id,
                text: label.clone(),
            }]);
            node.text = label;
            let active = match tag {
                "allow" => prompt.active == 0,
                "deny" => prompt.active == 1,
                "dismiss" => prompt.active == 2,
                _ => false,
            };
            styles.insert(
                id,
                style(x, y, w, 36., if active { "#99bbff" } else { "white" }),
            );
            tags.insert(id, format!("::permission-{tag}"));
            root.children.push(id);
            root.content
                .as_mut()
                .unwrap()
                .push(LayoutContent::Element(id));
            nodes.push(node);
        }
        nodes.insert(0, root);
        let mut popup = input.clone();
        popup.nodes = nodes;
        popup.top_layer.clear();
        popup.temporal_pickers.clear();
        popup.permission_prompts.clear();
        popup.rendered_tree = None;
        popup.scroll = ScrollState::default();
        let layout = compute_with_styles(&popup, limits, styles)?;
        for node in &popup.nodes {
            let Some(geometry) = layout.geometry.get(&node.id).cloned() else {
                continue;
            };
            output.generated_nodes.push(PaintNodeSnapshot {
                builtin_image: None,
                backdrop: None,
                control: None,
                backdrop_style: None,
                top_layer: Some(order),
                node_id: node.id,
                parent_id: Some(owner.id),
                tag: tags[&node.id].clone(),
                geometry,
                projection: layout.paint_projections.get(&node.id).cloned(),
                style: layout
                    .computed_styles
                    .get(&node.id)
                    .map(|style| (**style).clone())
                    .unwrap_or_default(),
                text: layout
                    .text_fragments
                    .get(&node.id)
                    .cloned()
                    .unwrap_or_default(),
                image_resource: None,
            });
        }
        for (family, characters) in layout.font_demands {
            output
                .font_demands
                .entry(family)
                .or_default()
                .extend(characters);
        }
        output.resolved_text.extend(layout.resolved_text);
        output.geometry.extend(layout.geometry);
        output.paint_projections.extend(layout.paint_projections);
        output.computed_styles.extend(layout.computed_styles);
        output.text_fragments.extend(layout.text_fragments);
        output.text_clusters.extend(layout.text_clusters);
        output.caret_stops.extend(layout.caret_stops);
        output.paint_fonts.extend(layout.paint_fonts);
        output.loaded_fonts.extend(layout.loaded_fonts);
        output.diagnostics.extend(layout.diagnostics);
        if output
            .paint_fonts
            .values()
            .fold(0usize, |sum, font| sum.saturating_add(font.bytes.len()))
            > limits.max_page_font_bytes
        {
            return Err(LayoutError::StyleTraversal(
                "permission prompt font budget exceeded".into(),
            ));
        }
    }
    Ok(())
}
