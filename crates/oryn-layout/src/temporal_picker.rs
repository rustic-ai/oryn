//! Private UA calendar/clock boxes use the production text/layout pipeline.
//! Synthetic IDs never enter the author DOM; hit testing retargets to the owner.
use super::*;
use oryn_common::v4::{PaintNodeSnapshot, TopLayerKind};
// Keep the active native row visible without making offscreen candidates into
// invisible pointer targets. Keyboard navigation changes the active row and
// therefore the next production snapshot's visible window.
fn row_window(
    items: usize,
    columns: usize,
    active: usize,
    available: f64,
    row_height: f64,
) -> (usize, usize) {
    let total = items.div_ceil(columns);
    let visible = ((available / row_height).floor().max(1.) as usize).min(total);
    let active_row = (active / columns).min(total.saturating_sub(1));
    let first = active_row
        .saturating_sub(visible.saturating_sub(1))
        .min(total.saturating_sub(visible));
    (first, visible)
}
fn popup_position(
    anchor: oryn_common::v4::CssRect,
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
    mode: &str,
    rtl: bool,
) -> (f64, f64) {
    let vertical = matches!(
        mode,
        "vertical-rl" | "vertical-lr" | "sideways-rl" | "sideways-lr"
    );
    let reverse = matches!(mode, "vertical-rl" | "sideways-rl");
    let x = if vertical {
        let preferred = if reverse {
            anchor.x - width
        } else {
            anchor.right()
        };
        if preferred < 0. || preferred + width > viewport_width {
            if reverse {
                anchor.right()
            } else {
                anchor.x - width
            }
        } else {
            preferred
        }
    } else if rtl {
        anchor.right() - width
    } else {
        anchor.x
    };
    let y = if vertical {
        if rtl {
            anchor.bottom() - height
        } else {
            anchor.y
        }
    } else if anchor.bottom() + height <= viewport_height {
        anchor.bottom()
    } else {
        anchor.y - height
    };
    (
        x.clamp(0., (viewport_width - width).max(0.)),
        y.clamp(0., (viewport_height - height).max(0.)),
    )
}
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
        if entry.kind != TopLayerKind::TemporalPicker {
            continue;
        }
        let Some(state) = input.temporal_pickers.get(&entry.node_id) else {
            continue;
        };
        let Some(owner) = input.nodes.iter().find(|node| node.id == entry.node_id) else {
            continue;
        };
        let Some(anchor) = output
            .geometry
            .get(&owner.id)
            .and_then(|geometry| geometry.box_model.as_ref())
            .map(|model| model.border_box)
        else {
            continue;
        };
        if state.items.len() > 256
            || used
                .len()
                .saturating_add(state.items.len() + state.headings.len() + 5)
                > limits.max_nodes
        {
            return Err(LayoutError::StyleTraversal(
                "temporal picker node budget exceeded".into(),
            ));
        }
        let columns = state.columns.clamp(1, 16);
        let width = 280f64.min(input.viewport.width.max(1.));
        let cell = width / columns as f64;
        let viewport_height = input.viewport.height.max(1.);
        let row_height =
            32f64.min(viewport_height / if state.headings.is_empty() { 3. } else { 4. });
        let header = row_height * if state.headings.is_empty() { 1. } else { 2. };
        let (first_row, visible_rows) = row_window(
            state.items.len(),
            columns,
            state.active,
            viewport_height - header - row_height,
            row_height,
        );
        let height = header + row_height + visible_rows as f64 * row_height;
        let arrow = 32f64.min(width / 2.);
        // Geometry already includes ancestor and owner transforms in compute_with_styles.
        let owner_style = output.computed_styles.get(&owner.id);
        let mode = owner_style
            .map(|style| style_value(style, "writing-mode", "horizontal-tb"))
            .unwrap_or("horizontal-tb");
        let rtl = owner_style.is_some_and(|style| style_value(style, "direction", "ltr") == "rtl");
        let (left, top) = popup_position(
            anchor,
            width,
            height,
            input.viewport.width,
            input.viewport.height,
            mode,
            rtl,
        );
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
                ("width".into(), format!("{}px", w.max(0.))),
                ("height".into(), format!("{h}px")),
                ("font-size".into(), "14px".into()),
                ("font-family".into(), "sans-serif".into()),
                ("line-height".into(), "28px".into()),
                ("color".into(), "black".into()),
                ("background-color".into(), background.into()),
                ("box-sizing".into(), "border-box".into()),
                ("padding".into(), "2px".into()),
                ("text-align".into(), "center".into()),
            ]))
        };
        let mut root = owner.clone();
        root.id = wrapper;
        root.tag = "oryn-native-temporal-picker".into();
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
        tags.insert(wrapper, "::picker(temporal)".to_string());
        let mut entries = vec![
            (
                "previous".to_string(),
                "‹".to_string(),
                left,
                top,
                arrow,
                "#eeeeee",
            ),
            (
                "title".to_string(),
                state.title.clone(),
                left + arrow,
                top,
                width - 2. * arrow,
                "#eeeeee",
            ),
            (
                "next".to_string(),
                "›".to_string(),
                left + width - arrow,
                top,
                arrow,
                "#eeeeee",
            ),
        ];
        for (index, label) in state.headings.iter().enumerate() {
            entries.push((
                format!("heading-{index}"),
                label.clone(),
                left + index as f64 * cell,
                top + row_height,
                cell,
                "#eeeeee",
            ));
        }
        for (index, item) in state
            .items
            .iter()
            .enumerate()
            .skip(first_row * columns)
            .take(visible_rows * columns)
        {
            entries.push((
                format!("item-{index}"),
                item.label.clone(),
                left + (index % columns) as f64 * cell,
                top + header + (index / columns - first_row) as f64 * row_height,
                cell,
                if !item.enabled {
                    "#cccccc"
                } else if index == state.active {
                    "#99bbff"
                } else if state.kind == "color" {
                    item.value.as_str()
                } else {
                    "white"
                },
            ));
        }
        entries.push((
            "clear".into(),
            "×".into(),
            left,
            top + height - row_height,
            width,
            "#eeeeee",
        ));
        for (tag, label, x, y, w, background) in entries {
            let native_color = if state.kind == "color"
                && tag.starts_with("item-")
                && !matches!(background, "#cccccc" | "#99bbff")
            {
                let rgba = oryn_dom::input_color_rgba8(background);
                Some(format!(
                    "rgba({}, {}, {}, {})",
                    rgba[0],
                    rgba[1],
                    rgba[2],
                    f64::from(rgba[3]) / 255.
                ))
            } else {
                None
            };
            let background = native_color.as_deref().unwrap_or(background);
            let id = allocate();
            let mut node = root.clone();
            node.id = id;
            node.parent = Some(wrapper);
            node.children.clear();
            node.content = Some(vec![LayoutContent::Text {
                node_id: id,
                text: label.clone(),
            }]);
            node.text = label;
            styles.insert(id, style(x, y, w, row_height, background));
            tags.insert(id, format!("::temporal-{tag}"));
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
                "temporal picker font budget exceeded".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{popup_position, row_window};
    #[test]
    fn popup_follows_logical_block_side_and_inline_direction() {
        let anchor = oryn_common::v4::CssRect {
            x: 200.,
            y: 150.,
            width: 80.,
            height: 40.,
        };
        assert_eq!(
            popup_position(anchor, 100., 80., 500., 400., "horizontal-tb", true),
            (180., 190.)
        );
        assert_eq!(
            popup_position(anchor, 100., 80., 500., 400., "vertical-rl", false),
            (100., 150.)
        );
        assert_eq!(
            popup_position(anchor, 100., 80., 500., 400., "vertical-lr", true),
            (280., 110.)
        );
        assert_eq!(
            popup_position(anchor, 100., 80., 250., 200., "vertical-lr", false),
            (100., 120.)
        );
    }
    #[test]
    fn candidate_window_tracks_native_active_row_with_bounded_height() {
        assert_eq!(row_window(256, 16, 0, 96., 32.), (0, 3));
        assert_eq!(row_window(256, 16, 255, 96., 32.), (13, 3));
        assert_eq!(row_window(64, 1, 32, 64., 32.), (31, 2));
        assert_eq!(row_window(0, 1, 0, 64., 32.), (0, 0));
    }
}
