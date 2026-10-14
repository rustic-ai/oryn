use super::*;
#[derive(Clone, Copy)]
pub(super) struct LayoutRequest {
    pub(super) id: u64,
    pub(super) containing: FlowBox,
    pub(super) allocated: Option<TaffySize<f64>>,
    pub(super) ancestor_hidden: bool,
}

#[cfg(test)]
mod continuation_tests {
    use super::*;

    #[test]
    fn aborted_column_trial_restores_exact_previous_overrides() {
        let mut engine = TextEngine::new(&[]);
        engine.column_float_trials.insert(7, 23.);
        engine
            .column_float_section_heights
            .insert(7, vec![23., 29.]);
        engine.out_of_flow.push((
            99,
            FlowBox {
                x: 1.,
                y: 2.,
                width: 3.,
                height: None,
            },
        ));
        let request = LayoutRequest {
            id: 7,
            containing: FlowBox {
                x: 0.,
                y: 0.,
                width: 100.,
                height: None,
            },
            allocated: None,
            ancestor_hidden: false,
        };
        let columns = columns::Columns {
            count: 2,
            inline: 45.,
            gap: 10.,
            available: 100.,
            limit: None,
            balance: true,
            reverse: false,
        };
        let mut geometry = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let trial = match ColumnTrial::begin(
            request,
            columns,
            80.,
            1,
            &mut engine,
            &BTreeMap::new(),
            &mut geometry,
            &mut diagnostics,
        ) {
            ColumnAdvance::Child(_, trial) => trial,
            ColumnAdvance::Complete(_) => panic!("first replay must suspend"),
        };
        engine.out_of_flow.push((100, request.containing));
        trial.unwind(&mut engine);
        assert_eq!(engine.column_float_trials.get(&7), Some(&23.));
        assert_eq!(
            engine.column_float_section_heights.get(&7),
            Some(&vec![23., 29.])
        );
        assert_eq!(engine.out_of_flow.len(), 1);
        assert_eq!(engine.out_of_flow[0].0, 99);
    }
}

struct PreInlineCapture {
    id: u64,
    containing: FlowBox,
    allocated: Option<TaffySize<f64>>,
    ancestor_hidden: bool,
    formatting_style: Option<BTreeMap<String, String>>,
    font_size: f64,
    margin: EdgeSizes,
    padding: EdgeSizes,
    authored_padding: EdgeSizes,
    border: EdgeSizes,
    scrollbar_gutter: EdgeSizes,
    absorbed: [f64; 2],
    available: f64,
    intrinsic_widths: Option<(f64, f64)>,
    horizontal_edges: f64,
    content_width: f64,
    outer_width: f64,
    specified_width: Option<f64>,
    specified_height: Option<f64>,
    height_basis: Option<f64>,
    border_box_sizing: bool,
    out_of_flow: bool,
    invisible: bool,
    x: f64,
    y: f64,
    axes: writing_mode::Axes,
    intrinsic_text: String,
    inline_size: f64,
    text_inline: f64,
    column_format: Option<columns::Columns>,
    float_scope: inline_formatting::FloatScope,
    column_has_floats: bool,
    column_float_height: Option<f64>,
    column_deferred_start: usize,
    previous_columns: Option<(u64, f64)>,
}

mod pre_child_production {
    //! Actual production pre-child extraction draft. Inline/column child requests
    //! must be suspended before this becomes the final iterative layout entry.
    use super::*;
    pub(super) enum PreChildResult {
        Complete(f64),
        Ready(PreChildFrame),
        Inline(LayoutRequest, SuspendedInline),
        Replay(LayoutRequest, Box<ColumnTrial>),
        Failed(&'static str),
    }
    pub(super) struct PreChildFrame {
        pub(super) id: u64,
        pub(super) containing: FlowBox,
        pub(super) allocated: Option<TaffySize<f64>>,
        pub(super) formatting_style: Option<BTreeMap<String, String>>,
        pub(super) margin: EdgeSizes,
        pub(super) padding: EdgeSizes,
        pub(super) authored_padding: EdgeSizes,
        pub(super) border: EdgeSizes,
        pub(super) scrollbar_gutter: EdgeSizes,
        pub(super) absorbed: [f64; 2],
        pub(super) font_size: f64,
        pub(super) x: f64,
        pub(super) y: f64,
        pub(super) axes: writing_mode::Axes,
        pub(super) content_width: f64,
        pub(super) outer_width: f64,
        pub(super) horizontal_edges: f64,
        pub(super) available: f64,
        pub(super) intrinsic_widths: Option<(f64, f64)>,
        pub(super) vertical_edges: f64,
        pub(super) height_basis: Option<f64>,
        pub(super) specified_width: Option<f64>,
        pub(super) specified_height: Option<f64>,
        pub(super) border_box_sizing: bool,
        pub(super) out_of_flow: bool,
        pub(super) invisible: bool,
        pub(super) intrinsic_text: String,
        pub(super) intrinsic_text_height: f64,
        pub(super) inline_context: Option<inline_formatting::Paragraph>,
        pub(super) text_layout: TextLayout,
        pub(super) column_format: Option<columns::Columns>,
        pub(super) column_block_flow: bool,
        pub(super) column_float_height: Option<f64>,
        pub(super) float_bfc_extent: Option<f64>,
        pub(super) ruby_baseline: Option<f64>,
        pub(super) text_inline: f64,
        pub(super) inline_size: f64,
        pub(super) definite_content_height: Option<f64>,
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_node(
        id: u64,
        containing: FlowBox,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &mut BTreeMap<u64, NodeGeometry>,
        offsets: &mut BTreeMap<u64, (f64, f64)>,
        text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
        input: &LayoutInput,
        ancestor_hidden: bool,
        diagnostics: &mut Vec<String>,
        engine: &mut TextEngine,
        allocated: Option<TaffySize<f64>>,
    ) -> PreChildResult {
        let by_id = nodes;
        let offset_dimensions = offsets;
        let text_fragments = text;
        let text_engine = engine;
        let Some(node) = by_id.get(&id).copied() else {
            return PreChildResult::Complete(0.0);
        };
        let formatting_style = tables::formatting_style(id, by_id, styles);
        let style = formatting_style.as_ref().unwrap_or(styles[&id].as_ref());
        let no_box = ancestor_hidden || style.get("display").is_some_and(|value| value == "none");
        let invisible = style
            .get("visibility")
            .is_some_and(|value| value == "hidden" || value == "collapse");
        if no_box {
            let mut pending = vec![id];
            let mut visited = BTreeSet::new();
            while let Some(hidden_id) = pending.pop() {
                if !visited.insert(hidden_id) {
                    continue;
                }
                let Some(node) = by_id.get(&hidden_id).copied() else {
                    continue;
                };
                if let Some(content) = &node.content {
                    for item in content {
                        match item {
                            LayoutContent::Text { node_id, .. }
                            | LayoutContent::StyledText {
                                node_id: Some(node_id),
                                ..
                            } => {
                                text_engine.source_geometry.remove(node_id);
                            }
                            _ => {}
                        }
                    }
                }
                geometry.insert(hidden_id, hidden_geometry());
                offset_dimensions.insert(hidden_id, (0.0, 0.0));
                pending.extend(node.children.iter().rev().copied());
            }
            return PreChildResult::Complete(0.0);
        }
        let containing = if let Some((root, inline)) = text_engine
            .column_context
            .filter(|(root, _)| columns::eligible_spanner(*root, id, by_id, styles))
        {
            let vertical =
                style_value(&styles[&root], "writing-mode", "horizontal-tb") != "horizontal-tb";
            if vertical {
                FlowBox {
                    height: Some(inline),
                    ..containing
                }
            } else {
                FlowBox {
                    width: inline,
                    ..containing
                }
            }
        } else {
            containing
        };
        let out_of_flow = matches!(
            style_value(style, "position", "static"),
            "absolute" | "fixed"
        );
        if out_of_flow && text_engine.active_out_of_flow != Some(id) {
            text_engine.out_of_flow.push((id, containing));
            return PreChildResult::Complete(0.);
        }
        let font_size = length(
            style_value(style, "font-size", "16px"),
            16.0,
            16.0,
            input.viewport,
        );
        let parent_vertical = node.parent.and_then(|id| styles.get(&id)).is_some_and(|s| {
            if matches!(
                style_value(s, "display", "block"),
                "table-row" | "table-row-group" | "table-header-group" | "table-footer-group"
            ) {
                tables::containing_axes(id, by_id, styles).vertical
            } else {
                writing_mode::Axes::new(style_value(s, "writing-mode", "horizontal-tb"), false)
                    .vertical
            }
        });
        let edge_basis = if parent_vertical {
            containing.height.unwrap_or(input.viewport.height)
        } else {
            containing.width
        };
        let mut margin = edge(style, "margin", edge_basis, font_size, input.viewport);
        let padding = edge(style, "padding", edge_basis, font_size, input.viewport);
        let border = text_engine
            .table_borders
            .get(&id)
            .copied()
            .unwrap_or_else(|| border_edge(style, containing.width, font_size, input.viewport));
        let resolve_inset = |name, basis| {
            optional_length(
                style_value(style, name, "auto"),
                basis,
                font_size,
                input.viewport,
            )
        };
        let left = out_of_flow
            .then(|| resolve_inset("left", containing.width))
            .flatten();
        let right = out_of_flow
            .then(|| resolve_inset("right", containing.width))
            .flatten();
        let available = text_engine
            .bfc_available_width
            .filter(|(owner, _)| *owner == id && !parent_vertical)
            .map(|(_, width)| width)
            .unwrap_or(
                containing.width
                    - margin.left
                    - margin.right
                    - left.unwrap_or(0.)
                    - right.unwrap_or(0.),
            )
            .max(0.0);
        let width_value = style_value(style, "width", "auto");
        let mut specified_width = (!is_intrinsic_width(width_value))
            .then(|| optional_length(width_value, containing.width, font_size, input.viewport))
            .flatten();
        let height_basis = containing.height;
        let mut specified_height = block_height_length(
            style_value(style, "height", "auto"),
            height_basis,
            font_size,
            input.viewport,
        );
        let border_box_sizing = style_value(style, "box-sizing", "content-box") == "border-box";
        if style_value(style, "height", "auto") == "auto"
            && writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical
            && !is_inline(style)
            && !is_replaced_box(node)
            && !out_of_flow
        {
            specified_height = Some(
                (text_engine
                    .bfc_available_width
                    .filter(|(owner, _)| *owner == id && parent_vertical)
                    .map(|(_, width)| width)
                    .unwrap_or(
                        containing.height.unwrap_or(input.viewport.height)
                            - margin.top
                            - margin.bottom,
                    )
                    - if border_box_sizing {
                        0.
                    } else {
                        padding.top + padding.bottom + border.top + border.bottom
                    })
                .max(0.),
            );
        }
        if out_of_flow && !is_replaced_box(node) {
            if width_value == "auto" && left.is_some() && right.is_some() {
                specified_width = Some(
                    (available
                        - if border_box_sizing {
                            0.
                        } else {
                            padding.left + padding.right + border.left + border.right
                        })
                    .max(0.),
                );
            }
            if style_value(style, "height", "auto") == "auto"
                && let Some((top, bottom)) = resolve_inset("top", height_basis.unwrap_or(0.))
                    .zip(resolve_inset("bottom", height_basis.unwrap_or(0.)))
            {
                specified_height = height_basis.map(|height| {
                    (height
                        - top
                        - bottom
                        - margin.top
                        - margin.bottom
                        - if border_box_sizing {
                            0.
                        } else {
                            padding.top + padding.bottom + border.top + border.bottom
                        })
                    .max(0.)
                });
            }
        }
        let descendant_source = (node.text.is_empty()
            && is_inline(style)
            && !matches!(node.tag.as_str(), "input" | "button" | "option"))
        .then(|| text_source::TextSource::collect_visible(node, by_id, false, Some(styles)));
        let direct_source = text_source::TextSource::direct_with_breaks(node, by_id, styles);
        let intrinsic_text =
            if node.text.is_empty() && matches!(node.tag.as_str(), "input" | "button" | "option") {
                node.attributes.get("value").cloned().unwrap_or_default()
            } else if let Some(source) = &descendant_source {
                source.text.clone()
            } else {
                direct_source.text.clone()
            };
        let shaped_source = descendant_source
            .or(Some(direct_source))
            // Control values do not describe DOM text
            // ranges. Keep those synthetic runs separate from native provenance.
            .filter(|source| source.text == intrinsic_text);
        let horizontal_edges = padding.left + padding.right + border.left + border.right;
        let needs_intrinsic = [
            width_value,
            style_value(style, "min-width", "auto"),
            style_value(style, "max-width", "none"),
        ]
        .iter()
        .any(|value| is_intrinsic_width(value))
            || ((is_inline(style) || out_of_flow || style_value(style, "float", "none") != "none")
                && specified_width.is_none());
        let intrinsic_widths = needs_intrinsic.then(|| {
            if let Some((width, _)) = text_engine.select_sizes.get(&node.id) {
                return (*width, *width);
            }
            if let Some((width, _)) = node
                .intrinsic_image_size
                .or(node.document_replaced.then_some((300, 150)))
            {
                return (f64::from(width), f64::from(width));
            }
            if !node.children.is_empty() {
                return text_engine.intrinsic_node(
                    node,
                    by_id,
                    styles,
                    input.viewport,
                    &mut BTreeSet::new(),
                );
            }
            let synthetic;
            let source = if let Some(source) = &shaped_source {
                source
            } else {
                synthetic = text_source::TextSource::synthetic(&intrinsic_text);
                &synthetic
            };
            text_engine.intrinsic_source(source, font_size, style, input.viewport, styles)
        });
        let intrinsic_width = intrinsic_widths.and_then(|widths| {
            resolve_intrinsic_width(
                width_value,
                (available - horizontal_edges).max(0.),
                widths,
                font_size,
                input.viewport,
            )
        });
        let mut content_width = intrinsic_width.or(specified_width).unwrap_or_else(|| {
            if let Some((width, _)) = node
                .intrinsic_image_size
                .or(node.document_replaced.then_some((300, 150)))
            {
                if let Some((specified_height, ratio)) =
                    specified_height.zip(preferred_ratio(node, style))
                {
                    let content_height = if border_box_sizing {
                        (specified_height
                            - padding.top
                            - padding.bottom
                            - border.top
                            - border.bottom)
                            .max(0.0)
                    } else {
                        specified_height.max(0.0)
                    };
                    ratio.content_width(
                        content_height,
                        horizontal_edges,
                        padding.top + padding.bottom + border.top + border.bottom,
                    )
                } else {
                    f64::from(width)
                }
            } else if is_inline(style)
                || out_of_flow
                || style_value(style, "float", "none") != "none"
            {
                let (min, max) = intrinsic_widths.unwrap_or_default();
                max.min((available - horizontal_edges).max(min))
            } else {
                (available - padding.left - padding.right - border.left - border.right).max(0.0)
            }
        });
        if border_box_sizing && specified_width.is_some() && intrinsic_width.is_none() {
            content_width =
                (content_width - padding.left - padding.right - border.left - border.right)
                    .max(0.0);
        }
        let bound = |property, default| {
            let value = style_value(style, property, default);
            if is_intrinsic_width(value) {
                intrinsic_widths.and_then(|widths| {
                    resolve_intrinsic_width(
                        value,
                        (available - horizontal_edges).max(0.),
                        widths,
                        font_size,
                        input.viewport,
                    )
                })
            } else {
                optional_length(value, containing.width, font_size, input.viewport).map(|value| {
                    if border_box_sizing {
                        (value - horizontal_edges).max(0.)
                    } else {
                        value.max(0.)
                    }
                })
            }
        };
        // CSS resolves a conflicting minimum in favor of min-width.
        if let Some(max) = bound("max-width", "none") {
            content_width = content_width.min(max);
        }
        if let Some(min) = bound("min-width", "auto") {
            content_width = content_width.max(min);
        }
        if node
            .intrinsic_image_size
            .or(node.document_replaced.then_some((300, 150)))
            .is_some()
            && width_value == "auto"
            && specified_height.is_none()
            && let Some(ratio) = preferred_ratio(node, style)
        {
            let vertical = padding.top + padding.bottom + border.top + border.bottom;
            let tentative_height = ratio.content_height(content_width, horizontal_edges, vertical);
            let constrained_height = clamp_content_height(
                style,
                tentative_height,
                height_basis,
                font_size,
                input.viewport,
                vertical,
            );
            // Transfer the height constraint back through the ratio, then
            // reapply width constraints. Incompatible axis bounds necessarily
            // break the ratio; final height clamping below honors those bounds.
            content_width = ratio.content_width(constrained_height, horizontal_edges, vertical);
            if let Some(maximum) = bound("max-width", "none") {
                content_width = content_width.min(maximum);
            }
            if let Some(minimum) = bound("min-width", "auto") {
                content_width = content_width.max(minimum);
            }
        }
        if let Some(size) = allocated {
            content_width = (size.width - horizontal_edges).max(0.);
        }
        if matches!(
            style_value(style, "display", "block"),
            "table" | "inline-table"
        ) {
            let vertical =
                writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                    .vertical;
            let table_inline = if vertical {
                specified_height.unwrap_or(containing.height.unwrap_or(input.viewport.height))
            } else {
                content_width
            };
            tables::prepare(
                node,
                table_inline,
                by_id,
                styles,
                input.viewport,
                text_engine,
            );
            if vertical {
                specified_height = Some(
                    text_engine
                        .table_widths
                        .get(&id)
                        .copied()
                        .unwrap_or(table_inline),
                );
                if width_value == "auto" {
                    content_width = text_engine
                        .table_block_sizes
                        .get(&id)
                        .copied()
                        .unwrap_or(content_width);
                }
            } else {
                content_width = text_engine
                    .table_widths
                    .get(&id)
                    .copied()
                    .unwrap_or(content_width);
            }
        }
        if let Some(row) = text_engine.table_rows.get(&id).filter(|row| row.vertical) {
            content_width = row.height;
            specified_height = Some(row.inline_size);
        }
        let scrollbar_gutter = text_engine
            .scrollbar_gutters
            .get(&id)
            .copied()
            .unwrap_or_default();
        let authored_padding = padding;
        content_width = (content_width - scrollbar_gutter.left - scrollbar_gutter.right).max(0.);
        let dimension_bound = bound;
        let bound = |property, default| {
            dimension_bound(property, default)
                .map(|value| (value - scrollbar_gutter.left - scrollbar_gutter.right).max(0.))
        };
        if !border_box_sizing {
            specified_height = specified_height
                .map(|height| (height - scrollbar_gutter.top - scrollbar_gutter.bottom).max(0.));
        }
        let padding = EdgeSizes {
            left: padding.left + scrollbar_gutter.left,
            right: padding.right + scrollbar_gutter.right,
            top: padding.top + scrollbar_gutter.top,
            bottom: padding.bottom + scrollbar_gutter.bottom,
        };
        let mut outer_width =
            content_width + padding.left + padding.right + border.left + border.right;
        let position = style_value(style, "position", "static");
        let containing_rtl = node
            .parent
            .and_then(|id| styles.get(&id))
            .is_some_and(|parent| style_value(parent, "direction", "ltr") == "rtl");
        if allocated.is_none()
            && !matches!(position, "absolute" | "fixed")
            && !writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical
            && style_value(style, "float", "none") == "none"
            && matches!(
                style_value(style, "display", "block"),
                "block" | "flow-root" | "flex" | "grid" | "list-item"
            )
        {
            let margin_space = text_engine
                .bfc_available_width
                .filter(|(owner, _)| *owner == id)
                .map(|(_, width)| width + margin.left + margin.right)
                .unwrap_or(containing.width);
            let (left, right) = block_inline_margins(
                margin_space,
                outer_width,
                margin.left,
                margin.right,
                style_value(style, "margin-left", "0px") == "auto",
                style_value(style, "margin-right", "0px") == "auto",
                containing_rtl,
            );
            margin.left = left;
            margin.right = right;
        }
        if allocated.is_none() && !out_of_flow {
            for child in &node.children {
                text_engine.clearance_nodes.remove(child);
                text_engine.absorbed_margins.remove(child);
                text_engine.absorbed_vertical_margins.remove(child);
            }
            if let Some((child, strut)) =
                margin_collapse::child_edge(crate::margin_collapse::ChildMarginEdgeInputs {
                    id,
                    bottom: false,
                    width: content_width,
                    nodes: by_id,
                    styles,
                    viewport: input.viewport,
                    clear_obstacles: margin_collapse::clear_obstacles(
                        text_engine.float_context.as_ref(),
                        text_engine.float_block_offset,
                    ),
                    cache: Some(&text_engine.margin_suffixes),
                    own_margin: margin.top,
                })
            {
                margin.top = strut.append(margin.top).value();
                text_engine.absorbed_margins.entry(child).or_insert([0.; 2])[0] = strut.value();
            }
            // Trailing margins use actual child clearance after native placement.
            let axes =
                writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false);
            if axes.vertical {
                let inline = specified_height
                    .map(|value| {
                        (value
                            - if border_box_sizing {
                                padding.top + padding.bottom + border.top + border.bottom
                            } else {
                                0.
                            })
                        .max(0.)
                    })
                    .or(containing.height)
                    .unwrap_or(input.viewport.height);
                for after in [false] {
                    if let Some((child, strut)) = margin_collapse::vertical_child_edge(
                        crate::margin_collapse::VerticalChildMarginInputs {
                            id,
                            after,
                            reverse: axes.x_reverse,
                            basis: inline,
                            nodes: by_id,
                            styles,
                            viewport: input.viewport,
                            clear_obstacles: margin_collapse::clear_obstacles(
                                text_engine.float_context.as_ref(),
                                text_engine.float_block_offset,
                            ),
                            own_margin: if after == axes.x_reverse {
                                margin.left
                            } else {
                                margin.right
                            },
                        },
                    ) {
                        let left = after == axes.x_reverse;
                        if left {
                            margin.left = strut.append(margin.left).value();
                        } else {
                            margin.right = strut.append(margin.right).value();
                        }
                        text_engine
                            .absorbed_vertical_margins
                            .entry(child)
                            .or_insert([0.; 2])[if left { 0 } else { 1 }] = strut.value();
                    }
                }
            }
        }
        let absorbed = text_engine
            .absorbed_margins
            .get(&id)
            .copied()
            .unwrap_or([0.; 2]);
        let absorbed_vertical = text_engine
            .absorbed_vertical_margins
            .get(&id)
            .copied()
            .unwrap_or([0.; 2]);
        let mut x = containing.x
            + if allocated.is_none() {
                margin.left - absorbed_vertical[0]
            } else {
                0.
            };
        let mut y = containing.y
            + if allocated.is_none() {
                margin.top - absorbed[0]
            } else {
                0.
            };
        if position == "fixed" {
            x = margin.left
                + optional_length(
                    style_value(style, "left", "auto"),
                    input.viewport.width,
                    font_size,
                    input.viewport,
                )
                .unwrap_or(0.0);
            y = margin.top
                + optional_length(
                    style_value(style, "top", "auto"),
                    input.viewport.height,
                    font_size,
                    input.viewport,
                )
                .unwrap_or(0.0);
        } else if position == "relative" {
            let (dx, dy) = relative_position_offset(
                style,
                containing.width,
                containing.height,
                containing_rtl,
                node.parent
                    .and_then(|id| styles.get(&id))
                    .map(|s| style_value(s, "writing-mode", "horizontal-tb"))
                    .unwrap_or("horizontal-tb"),
                font_size,
                input.viewport,
            );
            x += dx;
            y += dy;
        } else if position == "absolute" {
            x += optional_length(
                style_value(style, "left", "auto"),
                containing.width,
                font_size,
                input.viewport,
            )
            .unwrap_or(0.0);
            y += optional_length(
                style_value(style, "top", "auto"),
                input.viewport.height,
                font_size,
                input.viewport,
            )
            .unwrap_or(0.0);
        }
        let axes = writing_mode::Axes::new(
            style_value(style, "writing-mode", "horizontal-tb"),
            style_value(style, "direction", "ltr") == "rtl",
        );
        let inline_size = if axes.vertical {
            allocated
                .map(|size| {
                    (size.height - padding.top - padding.bottom - border.top - border.bottom)
                        .max(0.)
                })
                .or_else(|| {
                    specified_height.map(|h| {
                        (h - if border_box_sizing {
                            padding.top + padding.bottom + border.top + border.bottom
                        } else {
                            0.
                        })
                        .max(0.)
                    })
                })
                .or_else(|| text_engine.select_sizes.get(&node.id).map(|size| size.1))
                .or(containing.height)
                .unwrap_or(input.viewport.height)
        } else {
            content_width
        };
        let column_limit = if axes.vertical {
            specified_width.map(|value| {
                (value
                    - if border_box_sizing {
                        horizontal_edges
                    } else {
                        0.
                    })
                .max(0.)
            })
        } else {
            specified_height.map(|value| {
                (value
                    - if border_box_sizing {
                        padding.top + padding.bottom + border.top + border.bottom
                    } else {
                        0.
                    })
                .max(0.)
            })
        };
        let column_format = match columns::Columns::resolve(
            style,
            inline_size,
            column_limit,
            font_size,
            input.viewport,
        ) {
            Ok(columns) => columns,
            Err(error) => {
                diagnostics.push(format!("node {id}: {error}"));
                None
            }
        };
        let text_inline = column_format
            .map(|columns| columns.inline)
            .unwrap_or(inline_size);
        let float_scope = inline_formatting::enter_float_scope(
            text_engine,
            style,
            text_inline,
            if axes.vertical {
                y + border.top + padding.top
            } else {
                x + border.left + padding.left
            },
            if axes.vertical {
                if axes.x_reverse {
                    margin.right + border.right + padding.right
                } else {
                    x + border.left + padding.left
                }
            } else {
                y + border.top + padding.top
            },
            node.parent.is_none() || column_format.is_some(),
            axes.vertical,
        );
        let column_has_floats = column_format.is_some()
            && inline_formatting::has_flow_floats(
                id,
                by_id,
                styles,
                &mut text_engine.float_descendants,
            );
        let column_float_height = if column_has_floats {
            text_engine
                .column_float_trials
                .get(&id)
                .copied()
                .or_else(|| {
                    column_format
                        .filter(|columns| !columns.balance)
                        .and_then(|columns| columns.limit)
                })
        } else {
            None
        };
        let column_deferred_start = text_engine.out_of_flow.len();
        if let Some(height) = column_float_height {
            text_engine.float_fragment = Some((height, 0.));
            text_engine.column_float_heights.insert(id, height);
        } else {
            text_engine.column_float_heights.remove(&id);
            text_engine.column_float_section_heights.remove(&id);
            text_engine.column_float_section_counts.remove(&id);
            text_engine.column_float_section_sizes.remove(&id);
        }
        let mut inline_context = None;
        let mut prepared_inline = None;
        let column_block_flow = false;
        let mut ruby_baseline = None;
        if matches!(
            style_value(style, "display", "block"),
            "ruby" | "block ruby"
        ) {
            let (context, layout, baseline) =
                ruby::layout(node, by_id, styles, font_size, input.viewport, text_engine);
            inline_context = Some(context);
            prepared_inline = Some(layout);
            ruby_baseline = Some(baseline);
        }
        if ruby_baseline.is_none()
            && !matches!(
                style_value(style, "display", "block"),
                "flex"
                    | "inline-flex"
                    | "grid"
                    | "inline-grid"
                    | "table"
                    | "inline-table"
                    | "table-row"
                    | "ruby"
                    | "block ruby"
            )
        {
            let flow = inline_formatting::collect(node, by_id, styles);
            let active=(axes.vertical&&matches!(style_value(style,"display","block"),"block"|"flow-root"|"list-item")&&!flow.is_empty())||text_engine.float_context.as_ref().is_some_and(|context|context.has_floats())||inline_formatting::has_flow_floats(id,by_id,styles,&mut text_engine.float_descendants)||column_format.is_some()&&!flow.is_empty()||flow.iter().any(|item|matches!(item,inline_formatting::FlowItem::Paragraph(paragraph) if !paragraph.inline_nodes.is_empty()||!paragraph.atomic.is_empty()))
            ||flow.iter().any(|item|matches!(item,inline_formatting::FlowItem::Block(child) if styles.get(child).is_some_and(|style|style_value(style,"clear","none")!="none")))
            ||(flow.len()>1&&flow.iter().any(|item|matches!(item,inline_formatting::FlowItem::Paragraph(paragraph) if has_text_line_content(&paragraph.source,styles,style))));
            if active {
                let previous_columns = text_engine.column_context;
                if column_format.is_some() {
                    text_engine.column_context = Some((id, inline_size));
                    text_engine.column_float_section_index.insert(id, 0);
                }
                let advance = Box::new(InlineFlowFrame::begin(
                    flow,
                    style,
                    text_inline,
                    inline_size,
                    content_width,
                    specified_height,
                    font_size,
                    text_engine,
                ))
                .advance(
                    by_id,
                    styles,
                    geometry,
                    text_fragments,
                    input,
                    text_engine,
                    style,
                );
                let capture = PreInlineCapture {
                    id,
                    containing,
                    allocated,
                    ancestor_hidden,
                    formatting_style,
                    font_size,
                    margin,
                    padding,
                    authored_padding,
                    border,
                    scrollbar_gutter,
                    absorbed,
                    available,
                    intrinsic_widths,
                    horizontal_edges,
                    content_width,
                    outer_width,
                    specified_width,
                    specified_height,
                    height_basis,
                    border_box_sizing,
                    out_of_flow,
                    invisible,
                    x,
                    y,
                    axes,
                    intrinsic_text,
                    inline_size,
                    text_inline,
                    column_format,
                    float_scope,
                    column_has_floats,
                    column_float_height,
                    column_deferred_start,
                    previous_columns,
                };
                return match advance {
                    Ok(InlineAdvance::Child(request, flow)) => {
                        PreChildResult::Inline(request, SuspendedInline { capture, flow })
                    }
                    Ok(InlineAdvance::Complete(context, layout, has_blocks)) => {
                        let (capture, context, layout, blocks, extent) =
                            capture.receive_inline(context, layout, has_blocks, text_engine);
                        match capture.continue_layout(
                            context,
                            layout,
                            blocks,
                            extent,
                            by_id,
                            styles,
                            geometry,
                            input,
                            text_engine,
                            diagnostics,
                        ) {
                            PostInlineAdvance::Ready(frame) => PreChildResult::Ready(*frame),
                            PostInlineAdvance::Replay(request, trial) => {
                                PreChildResult::Replay(request, trial)
                            }
                            PostInlineAdvance::Complete(used) => PreChildResult::Complete(used),
                        }
                    }
                    Err(error) => {
                        text_engine.column_context = capture.previous_columns;
                        text_engine
                            .out_of_flow
                            .truncate(capture.column_deferred_start);
                        inline_formatting::leave_float_scope(text_engine, capture.float_scope);
                        PreChildResult::Failed(error)
                    }
                };
            }
        }
        let mut text_layout = if let Some(layout) = prepared_inline {
            layout
        } else if let Some(source) = &shaped_source {
            text_engine.shape_source(
                source,
                text_inline.max(1.0),
                font_size,
                style,
                input.viewport,
                styles,
            )
        } else {
            text_engine.shape_styled(
                &intrinsic_text,
                text_inline.max(1.0),
                font_size,
                style,
                input.viewport,
            )
        };
        let float_bfc_extent = inline_formatting::leave_float_scope(text_engine, float_scope);
        if column_has_floats {
            text_layout.height = text_layout.height.max(float_bfc_extent.unwrap_or(0.));
            text_engine
                .column_float_extents
                .insert(id, text_layout.height);
        }
        if let Some(columns) = column_format.filter(|columns| {
            (columns.balance || columns.limit.is_none())
                && column_has_floats
                && column_float_height.is_none()
        }) {
            return match ColumnTrial::begin(
                LayoutRequest {
                    id,
                    containing,
                    allocated,
                    ancestor_hidden,
                },
                columns,
                text_layout.height,
                column_deferred_start,
                text_engine,
                by_id,
                geometry,
                diagnostics,
            ) {
                ColumnAdvance::Child(request, trial) => PreChildResult::Replay(request, trial),
                ColumnAdvance::Complete(used) => PreChildResult::Complete(used),
            };
        }
        if ruby_baseline.is_some() && width_value == "auto" && allocated.is_none() {
            content_width = if axes.vertical {
                text_layout.height
            } else {
                text_layout.width
            };
            if let Some(max) = bound("max-width", "none") {
                content_width = content_width.min(max);
            }
            if let Some(min) = bound("min-width", "auto") {
                content_width = content_width.max(min);
            }
            outer_width =
                content_width + horizontal_edges + scrollbar_gutter.left + scrollbar_gutter.right;
        }
        if let Some(columns) = column_format.filter(|_| !column_block_flow) {
            columns.place(&mut text_layout);
            let step = (columns.inline + columns.gap).max(1.);
            let count = text_layout
                .lines
                .iter()
                .map(|line| {
                    ((if columns.reverse {
                        columns.available - columns.inline - line.4
                    } else {
                        line.4
                    }) / step)
                        .round()
                        .max(0.) as usize
                        + 1
                })
                .max()
                .unwrap_or(0);
            text_engine.column_runs.insert(
                id,
                vec![columns::ColumnRun {
                    columns,
                    count,
                    start: 0.,
                    block: text_layout.height,
                }],
            );
        }
        if axes.vertical {
            if allocated.is_none()
                && !out_of_flow
                && inline_context.is_some()
                && let Some((child, value)) = margin_collapse::used_child_vertical_end(
                    crate::margin_collapse::VerticalChildEndInputs {
                        id,
                        reverse: axes.x_reverse,
                        basis: inline_size,
                        nodes: by_id,
                        styles,
                        viewport: input.viewport,
                        geometry,
                        clearance: &text_engine.clearance_nodes,
                    },
                )
            {
                text_layout.height = (text_layout.height - value).max(0.);
                for item in &mut text_layout.inline_boxes {
                    if item.node == child {
                        item.rect.height = (item.rect.height - value).max(0.);
                    }
                }
                if axes.x_reverse {
                    let previous = margin.left;
                    margin.left = inline_formatting::MarginStrut::default()
                        .append(previous)
                        .append(value)
                        .value();
                    x += margin.left - previous;
                } else {
                    margin.right = inline_formatting::MarginStrut::default()
                        .append(margin.right)
                        .append(value)
                        .value();
                }
                text_engine
                    .absorbed_vertical_margins
                    .entry(child)
                    .or_insert([0.; 2])[if axes.x_reverse { 0 } else { 1 }] = value;
            }
            if width_value == "auto"
                && allocated.is_none()
                && (node.children.is_empty() || inline_context.is_some())
                && specified_width.is_none()
            {
                content_width = text_layout.height.max(float_bfc_extent.unwrap_or(0.));
                if let Some(max) = bound("max-width", "none") {
                    content_width = content_width.min(max);
                }
                if let Some(min) = bound("min-width", "auto") {
                    content_width = content_width.max(min);
                }
                outer_width = content_width
                    + horizontal_edges
                    + scrollbar_gutter.left
                    + scrollbar_gutter.right;
            }
            vertical_text::physicalize(
                &mut text_layout,
                content_width,
                inline_size,
                style_value(style, "writing-mode", "horizontal-tb"),
            );
        }
        let mut resolved = text_layout.resolved_text.clone();
        for run in &mut resolved {
            run.layout_node = id;
        }
        text_engine.resolved_text.insert(id, resolved);
        if text_engine.font_demand_error.is_none() {
            match text_pseudos::font_demands(&text_layout) {
                Ok(demands) => {
                    let old = text_engine
                        .font_demands
                        .get(&id)
                        .map(text_pseudos::demand_bytes)
                        .unwrap_or(0);
                    let bytes = text_engine
                        .font_demand_bytes
                        .saturating_sub(old)
                        .saturating_add(text_pseudos::demand_bytes(&demands));
                    if bytes > 64 * 1024 * 1024 {
                        text_engine.font_demand_error = Some(LayoutError::StyleTraversal(
                            "page font demand exceeds native 64-MiB bound".into(),
                        ));
                    } else {
                        text_engine.font_demand_bytes = bytes;
                        text_engine.font_demands.insert(id, demands);
                    }
                }
                Err(error) => text_engine.font_demand_error = Some(error),
            }
        }
        for line_break in &text_layout.breaks {
            let mut line_break = line_break.clone();
            line_break.layout_node = id;
            let shift = |rect: &mut CssRect| {
                rect.x += x + border.left + padding.left
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.x
                    };
                rect.y += y + border.top + padding.top
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.y
                    };
            };
            shift(&mut line_break.before);
            if let Some(after) = &mut line_break.after {
                shift(after);
            }
            text_engine.break_boxes.insert(line_break.node, line_break);
        }
        let mut source_geometry = BTreeMap::<u64, Vec<oryn_common::v4::TextClusterGeometry>>::new();
        for cluster in &text_layout.source_clusters {
            let rect = CssRect {
                x: x + border.left + padding.left + cluster.rect.x
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.x
                    },
                y: y + border.top + padding.top + cluster.rect.y
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.y
                    },
                ..cluster.rect
            };
            source_geometry.entry(cluster.text_node).or_default().push(
                oryn_common::v4::TextClusterGeometry {
                    layout_node: if inline_context
                        .as_ref()
                        .is_some_and(|paragraph| paragraph.inline_nodes.contains(&cluster.owner))
                    {
                        cluster.owner
                    } else {
                        id
                    },
                    start: cluster.utf16.start,
                    end: cluster.utf16.end,
                    rect,
                    matrix: transforms::Projection::IDENTITY.matrix(),
                    line: cluster.line,
                    rtl: cluster.rtl,
                },
            );
        }
        // Deeper direct-text layout supersedes ancestor aggregate measurements.
        text_engine.source_geometry.extend(source_geometry);
        let intrinsic_text_height = text_layout.height;
        let vertical_edges = padding.top + padding.bottom + border.top + border.bottom;
        let clamp_height = |value| {
            let gutter = scrollbar_gutter.top + scrollbar_gutter.bottom;
            (clamp_content_height(
                style,
                value + gutter,
                height_basis,
                font_size,
                input.viewport,
                vertical_edges - gutter,
            ) - gutter)
                .max(0.)
        };
        // An auto height remains indefinite even when min-height gives the box a
        // used height. Percentage descendants must not use that minimum as a basis.
        let definite_content_height = allocated
            .map(|size| (size.height - vertical_edges).max(0.))
            .or_else(|| {
                specified_height.map(|value| {
                    clamp_height(
                        (value
                            - if border_box_sizing {
                                vertical_edges
                            } else {
                                0.
                            })
                        .max(0.),
                    )
                })
            });
        PreChildResult::Ready(PreChildFrame {
            id,
            containing,
            allocated,
            formatting_style,
            margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            font_size,
            x,
            y,
            axes,
            content_width,
            outer_width,
            horizontal_edges,
            available,
            intrinsic_widths,
            vertical_edges,
            height_basis,
            specified_width,
            specified_height,
            border_box_sizing,
            out_of_flow,
            invisible,
            intrinsic_text,
            intrinsic_text_height,
            inline_context,
            text_layout,
            column_format,
            column_block_flow,
            column_float_height,
            float_bfc_extent,
            ruby_baseline,
            text_inline,
            inline_size,
            definite_content_height,
        })
    }
}

struct SuspendedInline {
    capture: PreInlineCapture,
    flow: InlineChildResume,
}

impl SuspendedInline {
    fn unwind(self, engine: &mut TextEngine) {
        match self.flow {
            InlineChildResume::Block {
                flow,
                previous_available,
                deferred_start,
                ..
            } => {
                engine.bfc_available_width = previous_available;
                engine.out_of_flow.truncate(deferred_start);
                engine.float_block_offset = flow.float_origin;
            }
            InlineChildResume::Atomic { flow, .. } => engine.float_block_offset = flow.float_origin,
        }
        engine.column_context = self.capture.previous_columns;
        engine
            .out_of_flow
            .truncate(self.capture.column_deferred_start);
        inline_formatting::leave_float_scope(engine, self.capture.float_scope);
    }
}

impl PreInlineCapture {
    fn receive_inline(
        self,
        context: inline_formatting::Paragraph,
        layout: TextLayout,
        has_blocks: bool,
        engine: &mut TextEngine,
    ) -> (
        PostInlineCapture,
        Option<inline_formatting::Paragraph>,
        TextLayout,
        bool,
        Option<f64>,
    ) {
        engine.column_context = self.previous_columns;
        let Self {
            id,
            containing,
            allocated,
            ancestor_hidden,
            formatting_style,
            font_size,
            margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            available,
            intrinsic_widths,
            horizontal_edges,
            content_width,
            outer_width,
            specified_width,
            specified_height,
            height_basis,
            border_box_sizing,
            out_of_flow,
            invisible,
            x,
            y,
            axes,
            intrinsic_text,
            inline_size,
            text_inline,
            column_format,
            float_scope,
            column_has_floats,
            column_float_height,
            column_deferred_start,
            previous_columns: _,
        } = self;
        let extent = inline_formatting::leave_float_scope(engine, float_scope);
        let capture = PostInlineCapture {
            id,
            containing,
            allocated,
            ancestor_hidden,
            formatting_style,
            font_size,
            margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            available,
            intrinsic_widths,
            horizontal_edges,
            content_width,
            outer_width,
            specified_width,
            specified_height,
            height_basis,
            border_box_sizing,
            out_of_flow,
            invisible,
            x,
            y,
            axes,
            intrinsic_text,
            inline_size,
            text_inline,
            column_format,
            column_has_floats,
            column_float_height,
            column_deferred_start,
        };
        (
            capture,
            Some(context),
            layout,
            has_blocks || column_has_floats,
            extent,
        )
    }
}

struct PostInlineCapture {
    id: u64,
    containing: FlowBox,
    allocated: Option<TaffySize<f64>>,
    ancestor_hidden: bool,
    formatting_style: Option<BTreeMap<String, String>>,
    font_size: f64,
    margin: EdgeSizes,
    padding: EdgeSizes,
    authored_padding: EdgeSizes,
    border: EdgeSizes,
    scrollbar_gutter: EdgeSizes,
    absorbed: [f64; 2],
    available: f64,
    intrinsic_widths: Option<(f64, f64)>,
    horizontal_edges: f64,
    content_width: f64,
    outer_width: f64,
    specified_width: Option<f64>,
    specified_height: Option<f64>,
    height_basis: Option<f64>,
    border_box_sizing: bool,
    out_of_flow: bool,
    invisible: bool,
    x: f64,
    y: f64,
    axes: writing_mode::Axes,
    intrinsic_text: String,
    inline_size: f64,
    text_inline: f64,
    column_format: Option<columns::Columns>,
    column_has_floats: bool,
    column_float_height: Option<f64>,
    column_deferred_start: usize,
}

enum PostInlineAdvance {
    Ready(Box<pre_child_production::PreChildFrame>),
    Replay(LayoutRequest, Box<ColumnTrial>),
    Complete(f64),
}

impl PostInlineCapture {
    #[allow(clippy::too_many_arguments)]
    fn continue_layout(
        self,
        context: Option<inline_formatting::Paragraph>,
        mut layout: TextLayout,
        has_blocks: bool,
        extent: Option<f64>,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &mut BTreeMap<u64, NodeGeometry>,
        input: &LayoutInput,
        engine: &mut TextEngine,
        diagnostics: &mut Vec<String>,
    ) -> PostInlineAdvance {
        if self.column_has_floats {
            layout.height = layout.height.max(extent.unwrap_or(0.));
            engine.column_float_extents.insert(self.id, layout.height);
        }
        if let Some(columns) = self.column_format.filter(|columns| {
            (columns.balance || columns.limit.is_none())
                && self.column_has_floats
                && self.column_float_height.is_none()
        }) {
            let request = LayoutRequest {
                id: self.id,
                containing: self.containing,
                allocated: self.allocated,
                ancestor_hidden: self.ancestor_hidden,
            };
            return match ColumnTrial::begin(
                request,
                columns,
                layout.height,
                self.column_deferred_start,
                engine,
                nodes,
                geometry,
                diagnostics,
            ) {
                ColumnAdvance::Child(request, trial) => PostInlineAdvance::Replay(request, trial),
                ColumnAdvance::Complete(used) => PostInlineAdvance::Complete(used),
            };
        }
        match self.finish(
            context, layout, has_blocks, extent, None, nodes, styles, geometry, input, engine,
        ) {
            pre_child_production::PreChildResult::Ready(frame) => {
                PostInlineAdvance::Ready(Box::new(frame))
            }
            pre_child_production::PreChildResult::Complete(used) => {
                PostInlineAdvance::Complete(used)
            }
            _ => unreachable!("post-inline tail cannot schedule child work"),
        }
    }
}

struct InlineFlowFrame {
    remaining: std::vec::IntoIter<inline_formatting::FlowItem>,
    context: inline_formatting::Paragraph,
    combined: TextLayout,
    cursor: f64,
    byte_offset: usize,
    has_blocks: bool,
    preceding_margin: Option<inline_formatting::MarginStrut>,
    float_origin: f64,
    axes: writing_mode::Axes,
    inline: f64,
    full_inline: f64,
    block_available: f64,
    block_limit: Option<f64>,
    font_size: f64,
}

enum InlineChildResume {
    /// BFC slot selection and re-layout are not complete until the child's
    /// actual border box fits the selected segment. Preserve previous state
    /// across every candidate, restoring it only when the block completes.
    Block {
        flow: Box<InlineFlowFrame>,
        node: u64,
        incoming_margin: inline_formatting::MarginStrut,
        compatible: bool,
        spanning: bool,
        previous_available: Option<(u64, f64)>,
        block_x: f64,
        deferred_start: usize,
        slot: Option<taffy::compute::BfcSlot>,
        insets: [f32; 2],
        inline_margins: [f64; 2],
        direction: taffy::Direction,
        used_inline: f64,
        block_margin: f64,
    },
    Atomic {
        flow: Box<InlineFlowFrame>,
        paragraph: inline_formatting::Paragraph,
        pending: u64,
        next: usize,
    },
}

enum BfcResume {
    Retry(taffy::compute::BfcSlot),
    Complete,
}
enum InlineAdvance {
    Child(LayoutRequest, InlineChildResume),
    Complete(inline_formatting::Paragraph, TextLayout, bool),
}

impl InlineChildResume {
    // Explicit native continuation inputs preserve the existing formatter boundary.
    #[allow(clippy::too_many_arguments)]
    fn resume_block(
        self,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &BTreeMap<u64, NodeGeometry>,
        paint: &BTreeMap<u64, Vec<PaintTextFragment>>,
        input: &LayoutInput,
        engine: &mut TextEngine,
        style: &BTreeMap<String, String>,
    ) -> Result<InlineAdvance, &'static str> {
        let Self::Block {
            mut flow,
            node,
            incoming_margin,
            compatible,
            spanning,
            previous_available,
            mut block_x,
            mut deferred_start,
            slot,
            insets,
            inline_margins,
            direction,
            used_inline,
            block_margin,
        } = self
        else {
            return Err("atomic continuation requires atomic resume");
        };
        let candidate = match resume_bfc_candidate(
            node,
            slot,
            flow.axes,
            insets,
            inline_margins,
            direction,
            deferred_start,
            geometry,
            engine,
        ) {
            Ok(candidate) => candidate,
            Err(error) => {
                engine.bfc_available_width = previous_available;
                engine.float_block_offset = flow.float_origin;
                engine.out_of_flow.truncate(deferred_start);
                return Err(error);
            }
        };
        match candidate {
            BfcResume::Retry(slot) => {
                flow.cursor = flow
                    .cursor
                    .max(f64::from(slot.y) - flow.float_origin - block_margin);
                engine.float_block_offset = flow.float_origin + flow.cursor;
                engine.bfc_available_width = Some((node, f64::from(slot.stretch_width).max(0.)));
                block_x = f64::from(slot.x) - f64::from(insets[0]) - inline_margins[0];
                deferred_start = engine.out_of_flow.len();
                let request = LayoutRequest {
                    id: node,
                    containing: FlowBox {
                        x: 0.,
                        y: 0.,
                        width: if flow.axes.vertical {
                            flow.block_available
                        } else {
                            used_inline
                        },
                        height: if flow.axes.vertical {
                            Some(used_inline)
                        } else {
                            flow.block_limit
                        },
                    },
                    allocated: None,
                    ancestor_hidden: false,
                };
                Ok(InlineAdvance::Child(
                    request,
                    Self::Block {
                        flow,
                        node,
                        incoming_margin,
                        compatible,
                        spanning,
                        previous_available,
                        block_x,
                        deferred_start,
                        slot: Some(slot),
                        insets,
                        inline_margins,
                        direction,
                        used_inline,
                        block_margin,
                    },
                ))
            }
            BfcResume::Complete => {
                engine.bfc_available_width = previous_available;
                flow.append_block_placement(
                    node,
                    block_x,
                    incoming_margin,
                    compatible,
                    geometry,
                    styles,
                    engine,
                );
                if spanning {
                    let mut next = taffy::compute::FloatContext::new();
                    next.set_width(engine.float_context_width as f32);
                    engine.float_context = Some(next);
                    let height = engine.column_context.and_then(|(root, _)| {
                        let index = engine.column_float_section_index.entry(root).or_default();
                        *index += 1;
                        engine
                            .column_float_section_heights
                            .get(&root)
                            .and_then(|heights| heights.get(*index))
                            .copied()
                    });
                    engine.float_fragment = engine.float_fragment.map(|(previous, _)| {
                        (height.unwrap_or(previous), flow.float_origin + flow.cursor)
                    });
                    flow.preceding_margin = None;
                }
                flow.advance(nodes, styles, geometry, paint, input, engine, style)
            }
        }
    }

    // Explicit native continuation inputs preserve the existing formatter boundary.
    #[allow(clippy::too_many_arguments)]
    fn resume_atomic(
        self,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &BTreeMap<u64, NodeGeometry>,
        paint: &BTreeMap<u64, Vec<PaintTextFragment>>,
        input: &LayoutInput,
        engine: &mut TextEngine,
        style: &BTreeMap<String, String>,
    ) -> Result<InlineAdvance, &'static str> {
        match self {
            Self::Atomic {
                mut flow,
                paragraph,
                pending,
                next,
            } => {
                if geometry.get(&pending).is_some_and(|geometry| {
                    geometry.availability == GeometryAvailability::Unsupported
                }) {
                    engine.float_block_offset = flow.float_origin;
                    return Err("native atomic child layout incomplete");
                }
                flow.resume_atomic_measurement(pending, nodes, styles, geometry, paint, engine);
                if let Some(&id) = paragraph.atomic.iter().nth(next) {
                    let request = flow.atomic_request(id);
                    Ok(InlineAdvance::Child(
                        request,
                        Self::Atomic {
                            flow,
                            paragraph,
                            pending: id,
                            next: next + 1,
                        },
                    ))
                } else {
                    let layout =
                        flow.shape_ready_paragraph(&paragraph, style, styles, input, engine);
                    flow.append_ready_paragraph(paragraph, layout);
                    flow.advance(nodes, styles, geometry, paint, input, engine, style)
                }
            }
            Self::Block { .. } => Err("block continuation requires BFC-specific resume"),
        }
    }
}

// Explicit native continuation inputs preserve the existing formatter boundary.
#[allow(clippy::too_many_arguments)]
fn resume_bfc_candidate(
    node: u64,
    slot: Option<taffy::compute::BfcSlot>,
    axes: writing_mode::Axes,
    insets: [f32; 2],
    margins: [f64; 2],
    direction: taffy::Direction,
    deferred_start: usize,
    geometry: &BTreeMap<u64, NodeGeometry>,
    engine: &mut TextEngine,
) -> Result<BfcResume, &'static str> {
    let Some(current) = slot else {
        return Ok(BfcResume::Complete);
    };
    let model = geometry
        .get(&node)
        .and_then(|value| value.box_model.as_ref());
    let fits = current.segment_id.is_none()
        || model.is_none_or(|model| {
            (if axes.vertical {
                model.border_box.height
            } else {
                model.border_box.width
            }) <= f64::from(current.border_width) + 0.0001
        });
    let collision = engine.float_context.as_ref().and_then(|context| {
        model.and_then(|model| {
            float_lines::intersecting_bottom(
                context,
                engine.float_context_width,
                CssRect {
                    x: f64::from(current.x)
                        + if axes.vertical {
                            model.margin.top
                        } else {
                            model.margin.left
                        }
                        - margins[0],
                    y: f64::from(current.y),
                    width: if axes.vertical {
                        model.border_box.height
                    } else {
                        model.border_box.width
                    },
                    height: if axes.vertical {
                        model.border_box.width
                    } else {
                        model.border_box.height
                    },
                },
            )
        })
    });
    if fits && collision.is_none() {
        return Ok(BfcResume::Complete);
    }
    if let Some(context) = engine.float_context.as_ref() {
        let (y, after) = collision.map_or((current.y, current.segment_id), |bottom| {
            (bottom as f32, None)
        });
        let next = context.find_bfc_slot(
            y,
            insets,
            margins.map(|value| value as f32),
            direction,
            taffy::Clear::None,
            after,
        );
        engine.out_of_flow.truncate(deferred_start);
        Ok(BfcResume::Retry(next))
    } else {
        Err("BFC continuation lost active float context")
    }
}

impl InlineFlowFrame {
    // Explicit native continuation inputs preserve the existing formatter boundary.
    #[allow(clippy::too_many_arguments)]
    fn advance(
        mut self: Box<Self>,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &BTreeMap<u64, NodeGeometry>,
        paint: &BTreeMap<u64, Vec<PaintTextFragment>>,
        input: &LayoutInput,
        engine: &mut TextEngine,
        style: &BTreeMap<String, String>,
    ) -> Result<InlineAdvance, &'static str> {
        let _ = (geometry, paint);
        loop {
            engine.float_block_offset = self.float_origin + self.cursor;
            match self.remaining.next() {
                Some(inline_formatting::FlowItem::Block(id)) => {
                    return Ok(self.begin_block(id, nodes, styles, input, engine, style));
                }
                Some(inline_formatting::FlowItem::Paragraph(paragraph)) => {
                    self.prepare_boundaries(&paragraph, styles, input, engine);
                    if let Some(&id) = paragraph.atomic.iter().next() {
                        let request = self.atomic_request(id);
                        return Ok(InlineAdvance::Child(
                            request,
                            InlineChildResume::Atomic {
                                flow: self,
                                paragraph,
                                pending: id,
                                next: 1,
                            },
                        ));
                    }
                    let layout =
                        self.shape_ready_paragraph(&paragraph, style, styles, input, engine);
                    self.append_ready_paragraph(paragraph, layout);
                }
                None => {
                    let (paragraph, layout, blocks) = self.complete(engine);
                    return Ok(InlineAdvance::Complete(paragraph, layout, blocks));
                }
            }
        }
    }

    fn begin_block(
        mut self: Box<Self>,
        id: u64,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        input: &LayoutInput,
        engine: &mut TextEngine,
        style: &BTreeMap<String, String>,
    ) -> InlineAdvance {
        self.has_blocks = true;
        let child_style = &styles[&id];
        let compatible = style_value(child_style, "writing-mode", "horizontal-tb")
            == style_value(style, "writing-mode", "horizontal-tb");
        if !compatible {
            self.preceding_margin = None;
        }
        let spanning = engine
            .column_context
            .is_some_and(|(root, _)| columns::eligible_spanner(root, id, nodes, styles));
        if spanning
            && let Some(bottom) = engine
                .float_context
                .as_ref()
                .and_then(|c| c.cleared_threshold(taffy::Clear::Both))
        {
            self.cursor = self.cursor.max(f64::from(bottom) - self.float_origin);
            engine.float_block_offset = self.float_origin + self.cursor;
        }
        let incoming_margin = self.enter_block_margins(id, nodes, styles, input, engine);
        self.apply_block_clearance(id, styles, input, engine);
        let used_inline = if spanning {
            engine
                .column_context
                .map_or(self.full_inline, |(_, width)| width)
        } else {
            self.inline
        };
        let font = length(
            style_value(child_style, "font-size", "16px"),
            self.font_size,
            self.font_size,
            input.viewport,
        );
        let margins = edge(child_style, "margin", used_inline, font, input.viewport);
        let inline_margins = if self.axes.vertical {
            [margins.top, margins.bottom]
        } else {
            [margins.left, margins.right]
        };
        let block_margin = if self.axes.vertical {
            if self.axes.x_reverse {
                margins.right
            } else {
                margins.left
            }
        } else {
            margins.top
        };
        let direction = if style_value(style, "direction", "ltr") == "rtl" {
            taffy::Direction::Rtl
        } else {
            taffy::Direction::Ltr
        };
        let insets = engine.float_inline_insets.map(|v| v as f32);
        let avoids = !spanning
            && inline_formatting::establishes_bfc(child_style)
            && engine
                .float_context
                .as_ref()
                .is_some_and(|c| c.has_floats());
        let slot = if avoids {
            engine.float_context.as_ref().map(|c| {
                c.find_bfc_slot(
                    (self.float_origin + self.cursor + block_margin) as f32,
                    insets,
                    inline_margins.map(|v| v as f32),
                    direction,
                    taffy::Clear::None,
                    None,
                )
            })
        } else {
            None
        };
        let previous_available = engine.bfc_available_width;
        let mut block_x = 0.;
        if let Some(slot) = slot {
            self.cursor = self
                .cursor
                .max(f64::from(slot.y) - self.float_origin - block_margin);
            engine.float_block_offset = self.float_origin + self.cursor;
            engine.bfc_available_width = Some((id, f64::from(slot.stretch_width).max(0.)));
            block_x = f64::from(slot.x) - f64::from(insets[0]) - inline_margins[0];
        }
        let deferred_start = engine.out_of_flow.len();
        let request = LayoutRequest {
            id,
            containing: FlowBox {
                x: 0.,
                y: 0.,
                width: if self.axes.vertical {
                    self.block_available
                } else {
                    used_inline
                },
                height: if self.axes.vertical {
                    Some(used_inline)
                } else {
                    self.block_limit
                },
            },
            allocated: None,
            ancestor_hidden: false,
        };
        InlineAdvance::Child(
            request,
            InlineChildResume::Block {
                flow: self,
                node: id,
                incoming_margin,
                compatible,
                spanning,
                previous_available,
                block_x,
                deferred_start,
                slot,
                insets,
                inline_margins,
                direction,
                used_inline,
                block_margin,
            },
        )
    }

    fn apply_block_clearance(
        &mut self,
        id: u64,
        styles: &ComputedStyles,
        input: &LayoutInput,
        engine: &mut TextEngine,
    ) {
        engine.clearance_nodes.remove(&id);
        let style = &styles[&id];
        let clear = match style_value(style, "clear", "none") {
            "left" => taffy::Clear::Left,
            "right" => taffy::Clear::Right,
            "both" => taffy::Clear::Both,
            _ => taffy::Clear::None,
        };
        if let Some(bottom) = engine
            .float_context
            .as_ref()
            .and_then(|c| c.cleared_threshold(clear))
        {
            let font = length(
                style_value(style, "font-size", "16px"),
                self.font_size,
                self.font_size,
                input.viewport,
            );
            let margin = edge(style, "margin", self.inline, font, input.viewport);
            let before = if self.axes.vertical {
                if self.axes.x_reverse {
                    margin.right
                } else {
                    margin.left
                }
            } else {
                margin.top
            };
            let cleared = f64::from(bottom) - self.float_origin - before;
            if cleared > self.cursor + 0.0001 {
                engine.clearance_nodes.insert(id);
            }
            self.cursor = self.cursor.max(cleared);
            engine.float_block_offset = self.float_origin + self.cursor;
        }
    }

    fn enter_block_margins(
        &mut self,
        id: u64,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        input: &LayoutInput,
        engine: &mut TextEngine,
    ) -> inline_formatting::MarginStrut {
        let obstacles = margin_collapse::clear_obstacles(
            engine.float_context.as_ref(),
            engine.float_block_offset,
        );
        let top = if self.axes.vertical {
            margin_collapse::vertical_edge(margin_collapse::VerticalMarginEdgeInputs {
                id,
                after: false,
                reverse: self.axes.x_reverse,
                basis: self.inline,
                nodes,
                styles,
                viewport: input.viewport,
                obstacles,
            })
        } else {
            margin_collapse::outer_edge(
                id,
                false,
                self.inline,
                obstacles,
                margin_collapse::QueryContext {
                    nodes,
                    styles,
                    viewport: input.viewport,
                    cache: Some(&engine.margin_suffixes),
                },
            )
        };
        let previous = self.preceding_margin.take().unwrap_or_default();
        let incoming = previous.merge(top);
        self.cursor += incoming.value() - previous.value() - top.value();
        engine.float_block_offset = self.float_origin + self.cursor;
        incoming
    }

    fn prepare_boundaries(
        &self,
        paragraph: &inline_formatting::Paragraph,
        styles: &ComputedStyles,
        input: &LayoutInput,
        engine: &mut TextEngine,
    ) {
        for (boundary, (owner, start)) in &paragraph.boundaries {
            let style = &styles[owner];
            let size = length(
                style_value(style, "font-size", "16px"),
                self.font_size,
                self.font_size,
                input.viewport,
            );
            let margin = edge(style, "margin", self.inline, size, input.viewport);
            let padding = edge(style, "padding", self.inline, size, input.viewport);
            let border = border_edge(style, self.inline, size, input.viewport);
            let reversed = if self.axes.vertical {
                self.axes.y_reverse
            } else {
                self.axes.x_reverse
            };
            let start = *start != reversed;
            let advance = if style_value(style, "display", "inline") == "contents" {
                0.
            } else if self.axes.vertical {
                if start {
                    margin.top + padding.top + border.top
                } else {
                    margin.bottom + padding.bottom + border.bottom
                }
            } else if start {
                margin.left + padding.left + border.left
            } else {
                margin.right + padding.right + border.right
            };
            engine
                .inline_sizes
                .insert(*boundary, (advance, 0., parley::InlineBoxKind::InFlow));
        }
    }

    fn atomic_request(&self, id: u64) -> LayoutRequest {
        LayoutRequest {
            id,
            containing: FlowBox {
                x: 0.,
                y: 0.,
                width: if self.axes.vertical {
                    self.block_available
                } else {
                    self.inline
                },
                height: if self.axes.vertical {
                    Some(self.inline)
                } else {
                    self.block_limit
                },
            },
            allocated: None,
            ancestor_hidden: false,
        }
    }

    // Explicit native continuation inputs preserve the existing formatter boundary.
    #[allow(clippy::too_many_arguments)]
    fn begin(
        flow: Vec<inline_formatting::FlowItem>,
        style: &BTreeMap<String, String>,
        inline: f64,
        full_inline: f64,
        block_available: f64,
        block_limit: Option<f64>,
        font_size: f64,
        engine: &TextEngine,
    ) -> Self {
        Self {
            remaining: flow.into_iter(),
            context: inline_formatting::Paragraph::default(),
            combined: TextLayout::default(),
            cursor: 0.,
            byte_offset: 0,
            has_blocks: false,
            preceding_margin: None,
            float_origin: engine.float_block_offset,
            axes: writing_mode::Axes::new(
                style_value(style, "writing-mode", "horizontal-tb"),
                style_value(style, "direction", "ltr") == "rtl",
            ),
            inline,
            full_inline,
            block_available,
            block_limit,
            font_size,
        }
    }

    fn complete(
        mut self,
        engine: &mut TextEngine,
    ) -> (inline_formatting::Paragraph, TextLayout, bool) {
        engine.float_block_offset = self.float_origin;
        self.combined.height = self.cursor;
        (self.context, self.combined, self.has_blocks)
    }

    fn append_ready_paragraph(
        &mut self,
        paragraph: inline_formatting::Paragraph,
        mut layout: TextLayout,
    ) {
        let line_offset = self.combined.lines.len();
        let bytes = layout
            .line_ranges
            .last()
            .map(|range| range.end)
            .unwrap_or(0);
        for line in &mut layout.lines {
            line.3 += self.cursor;
        }
        for range in &mut layout.line_ranges {
            range.start += self.byte_offset;
            range.end += self.byte_offset;
        }
        for sources in &mut layout.provenance {
            for source in sources {
                source.bytes.start += self.byte_offset;
                source.bytes.end += self.byte_offset;
                for range in &mut source.glyph_ranges {
                    range.start += self.byte_offset;
                    range.end += self.byte_offset;
                }
            }
        }
        for cluster in &mut layout.clusters {
            cluster.line += line_offset;
            cluster.bytes.start += self.byte_offset;
            cluster.bytes.end += self.byte_offset;
            cluster.rect.y += self.cursor;
        }
        for cluster in &mut layout.source_clusters {
            cluster.line += line_offset;
            cluster.rect.y += self.cursor;
        }
        for item in &mut layout.inline_boxes {
            if item.line != usize::MAX {
                item.line += line_offset;
            }
            item.rect.y += self.cursor;
        }
        for line_break in &mut layout.breaks {
            line_break.line += line_offset;
            line_break.before.y += self.cursor;
            if let Some(after) = &mut line_break.after {
                after.y += self.cursor;
            }
        }
        if layout.height > 0. {
            self.preceding_margin = None;
        }
        self.cursor += layout.height;
        self.byte_offset += bytes + 1;
        self.combined.width = self.combined.width.max(layout.width);
        self.combined.baselines.extend(layout.baselines);
        self.combined.line_heights.extend(layout.line_heights);
        self.combined.lines.extend(layout.lines);
        self.combined.line_ranges.extend(layout.line_ranges);
        self.combined.glyph_runs.extend(layout.glyph_runs);
        self.combined.provenance.extend(layout.provenance);
        self.combined.clusters.extend(layout.clusters);
        self.combined.resolved_text.extend(layout.resolved_text);
        self.combined.source_clusters.extend(layout.source_clusters);
        self.combined.inline_boxes.extend(layout.inline_boxes);
        self.combined.breaks.extend(layout.breaks);
        self.context.inline_nodes.extend(paragraph.inline_nodes);
        self.context.atomic.extend(paragraph.atomic);
        self.context.boundaries.extend(paragraph.boundaries);
    }
    fn resume_atomic_measurement(
        &mut self,
        id: u64,
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &BTreeMap<u64, NodeGeometry>,
        paint: &BTreeMap<u64, Vec<PaintTextFragment>>,
        engine: &mut TextEngine,
    ) {
        let dimensions = geometry
            .get(&id)
            .and_then(|g| g.box_model.as_ref())
            .map(|model| {
                (
                    model.border_box.width + model.margin.left + model.margin.right,
                    model.border_box.height + model.margin.top + model.margin.bottom,
                )
            })
            .unwrap_or((0., 0.));
        let above = if self.axes.vertical {
            dimensions.0
        } else {
            engine.ruby_baselines.get(&id).copied().unwrap_or_else(|| {
                inline_formatting::atomic_baseline(id, dimensions.1, nodes, styles, geometry, paint)
            })
        };
        engine.inline_extents.insert(
            id,
            if self.axes.vertical {
                dimensions.0
            } else {
                dimensions.1
            },
        );
        let style = &styles[&id];
        let floating = style_value(style, "float", "none");
        let kind = if floating != "none" {
            let clear = match style_value(style, "clear", "none") {
                "left" => taffy::Clear::Left,
                "right" => taffy::Clear::Right,
                "both" => taffy::Clear::Both,
                _ => taffy::Clear::None,
            };
            engine.inline_floats.insert(
                id,
                float_lines::FloatSpec {
                    width: if self.axes.vertical {
                        dimensions.1
                    } else {
                        dimensions.0
                    },
                    height: if self.axes.vertical {
                        dimensions.0
                    } else {
                        dimensions.1
                    },
                    right: floating == "right",
                    clear,
                    monolithic: is_replaced_box(nodes[&id])
                        || matches!(
                            style_value(style, "display", "block"),
                            "inline-block" | "inline-flex" | "inline-grid" | "ruby"
                        ),
                },
            );
            parley::InlineBoxKind::CustomOutOfFlow
        } else if matches!(
            style_value(style, "position", "static"),
            "absolute" | "fixed"
        ) {
            parley::InlineBoxKind::OutOfFlow
        } else {
            parley::InlineBoxKind::InFlow
        };
        engine.inline_sizes.insert(
            id,
            (
                if self.axes.vertical {
                    dimensions.1
                } else {
                    dimensions.0
                },
                above,
                kind,
            ),
        );
    }

    fn shape_ready_paragraph(
        &mut self,
        paragraph: &inline_formatting::Paragraph,
        style: &BTreeMap<String, String>,
        styles: &ComputedStyles,
        input: &LayoutInput,
        engine: &mut TextEngine,
    ) -> TextLayout {
        let layout = if paragraph.atomic.is_empty()
            && !inline_formatting::paragraph_forms_line(
                paragraph,
                styles,
                style,
                self.inline,
                input.viewport,
            ) {
            self.context
                .empty_positions
                .extend(paragraph.inline_nodes.iter().map(|id| (*id, self.cursor)));
            TextLayout::default()
        } else {
            let mut paragraph_style = style.clone();
            if !self.combined.lines.is_empty() || self.has_blocks {
                paragraph_style.remove(text_pseudos::KEY);
            }
            engine.shape_source(
                &paragraph.source,
                self.inline.max(1.),
                self.font_size,
                &paragraph_style,
                input.viewport,
                styles,
            )
        };
        for node in paragraph.atomic.iter().chain(paragraph.boundaries.keys()) {
            engine.inline_sizes.remove(node);
            engine.inline_extents.remove(node);
            engine.inline_floats.remove(node);
        }
        layout
    }

    // Explicit native continuation inputs preserve the existing formatter boundary.
    #[allow(clippy::too_many_arguments)]
    fn append_block_placement(
        &mut self,
        node: u64,
        block_x: f64,
        incoming: inline_formatting::MarginStrut,
        compatible: bool,
        geometry: &BTreeMap<u64, NodeGeometry>,
        styles: &ComputedStyles,
        engine: &TextEngine,
    ) {
        if let Some(model) = geometry
            .get(&node)
            .and_then(|value| value.box_model.as_ref())
        {
            let absorbed = engine
                .absorbed_margins
                .get(&node)
                .copied()
                .unwrap_or([0.; 2]);
            let vertical = engine
                .absorbed_vertical_margins
                .get(&node)
                .copied()
                .unwrap_or([0.; 2]);
            let width = model.border_box.width + model.margin.left + model.margin.right
                - vertical[0]
                - vertical[1];
            let height = model.border_box.height + model.margin.top + model.margin.bottom
                - absorbed[0]
                - absorbed[1];
            let block = if self.axes.vertical { width } else { height };
            let advance = if self.axes.vertical { height } else { width };
            self.combined.inline_boxes.push(InlinePlacement {
                node,
                rect: CssRect {
                    x: block_x,
                    y: self.cursor,
                    width: advance,
                    height: block,
                },
                line: usize::MAX,
            });
            self.cursor += block;
            self.combined.width = self.combined.width.max(block_x + advance);
            let after = if self.axes.vertical {
                if self.axes.x_reverse {
                    model.margin.left
                } else {
                    model.margin.right
                }
            } else {
                model.margin.bottom
            };
            let empty = if self.axes.vertical {
                model.border_box.width == 0.
                    && model.border.left == 0.
                    && model.border.right == 0.
                    && model.padding.left == 0.
                    && model.padding.right == 0.
            } else {
                model.border_box.height == 0.
                    && model.border.top == 0.
                    && model.border.bottom == 0.
                    && model.padding.top == 0.
                    && model.padding.bottom == 0.
            };
            let through = compatible
                && absorbed == [0.; 2]
                && vertical == [0.; 2]
                && empty
                && !inline_formatting::establishes_bfc(&styles[&node])
                && !engine.clearance_nodes.contains(&node);
            self.preceding_margin = Some(if through {
                let joined = incoming.append(after);
                self.cursor += joined.value() - incoming.value() - after;
                joined
            } else if self.axes.vertical {
                if vertical[if self.axes.x_reverse { 0 } else { 1 }] != 0. {
                    inline_formatting::MarginStrut::default()
                } else {
                    inline_formatting::MarginStrut::default().append(after)
                }
            } else if absorbed[1] != 0. {
                inline_formatting::MarginStrut::default()
            } else {
                inline_formatting::MarginStrut::default().append(model.margin.bottom)
            });
        }
        if !compatible {
            self.preceding_margin = None;
        }
        self.context.atomic.insert(node);
    }
}

struct ColumnTrial {
    request: LayoutRequest,
    columns: columns::Columns,
    natural: f64,
    deferred_start: usize,
    pass: usize,
    low: Vec<f64>,
    high: Vec<f64>,
    targets: Vec<f64>,
    previous_trial: Option<f64>,
    previous_sections: Option<Vec<f64>>,
}

enum ColumnAdvance {
    Child(LayoutRequest, Box<ColumnTrial>),
    Complete(f64),
}

impl ColumnTrial {
    fn unwind(self, engine: &mut TextEngine) {
        match self.previous_trial {
            Some(value) => {
                engine.column_float_trials.insert(self.request.id, value);
            }
            None => {
                engine.column_float_trials.remove(&self.request.id);
            }
        }
        match self.previous_sections {
            Some(value) => {
                engine
                    .column_float_section_heights
                    .insert(self.request.id, value);
            }
            None => {
                engine.column_float_section_heights.remove(&self.request.id);
            }
        }
        engine.out_of_flow.truncate(self.deferred_start);
    }
    // Explicit native continuation inputs preserve the existing formatter boundary.
    #[allow(clippy::too_many_arguments)]
    fn begin(
        request: LayoutRequest,
        columns: columns::Columns,
        natural: f64,
        deferred_start: usize,
        engine: &mut TextEngine,
        nodes: &BTreeMap<u64, &LayoutNode>,
        geometry: &mut BTreeMap<u64, NodeGeometry>,
        diagnostics: &mut Vec<String>,
    ) -> ColumnAdvance {
        let high = vec![natural.max(0.01)];
        let trial = Self {
            request,
            columns,
            natural,
            deferred_start,
            pass: 0,
            low: high.clone(),
            high,
            targets: Vec::new(),
            previous_trial: engine.column_float_trials.get(&request.id).copied(),
            previous_sections: engine
                .column_float_section_heights
                .get(&request.id)
                .cloned(),
        };
        trial.dispatch(engine, nodes, geometry, diagnostics)
    }

    fn dispatch(
        mut self,
        engine: &mut TextEngine,
        nodes: &BTreeMap<u64, &LayoutNode>,
        geometry: &mut BTreeMap<u64, NodeGeometry>,
        diagnostics: &mut Vec<String>,
    ) -> ColumnAdvance {
        let id = self.request.id;
        if engine.column_balance_work >= 128 {
            diagnostics.push(format!(
                "node {id}: native float column balance exceeds 128 re-layout work budget"
            ));
            mark_unsupported_subtree(id, "float column balance work budget", nodes, geometry);
            self.unwind(engine);
            return ColumnAdvance::Complete(0.);
        }
        engine.column_balance_work += 1;
        self.targets = self
            .low
            .iter()
            .zip(&self.high)
            .map(|(low, high)| {
                if self.pass == 15 {
                    *high
                } else {
                    (low + high) / 2.
                }
            })
            .collect();
        engine.out_of_flow.truncate(self.deferred_start);
        engine.column_float_trials.insert(id, self.targets[0]);
        engine
            .column_float_section_heights
            .insert(id, self.targets.clone());
        ColumnAdvance::Child(self.request, Box::new(self))
    }

    fn resume(
        mut self,
        used: f64,
        engine: &mut TextEngine,
        nodes: &BTreeMap<u64, &LayoutNode>,
        geometry: &mut BTreeMap<u64, NodeGeometry>,
        diagnostics: &mut Vec<String>,
    ) -> ColumnAdvance {
        let id = self.request.id;
        engine.column_float_trials.remove(&id);
        if self.pass == 15 {
            return ColumnAdvance::Complete(used);
        }
        if self.pass == 0 {
            let sizes = engine
                .column_float_section_sizes
                .get(&id)
                .cloned()
                .unwrap_or_else(|| vec![self.natural]);
            self.high = sizes
                .iter()
                .map(|size| {
                    self.columns
                        .limit
                        .map(|limit| size.min(limit))
                        .unwrap_or(*size)
                        .max(0.01)
                })
                .collect();
            self.low = sizes
                .iter()
                .zip(&self.high)
                .map(|(size, high)| (size / self.columns.count as f64).max(0.01).min(*high))
                .collect();
        } else {
            let counts = engine.column_float_section_counts.get(&id);
            for (index, target) in self.targets.iter().copied().enumerate() {
                if counts
                    .and_then(|counts| counts.get(index))
                    .is_some_and(|count| *count <= self.columns.count)
                {
                    self.high[index] = target;
                } else {
                    self.low[index] = target;
                }
            }
        }
        self.pass += 1;
        self.dispatch(engine, nodes, geometry, diagnostics)
    }
}

impl PostInlineCapture {
    #[allow(clippy::too_many_arguments)]
    fn finish(
        self,
        inline_context: Option<inline_formatting::Paragraph>,
        mut text_layout: TextLayout,
        column_block_flow: bool,
        float_bfc_extent: Option<f64>,
        ruby_baseline: Option<f64>,
        by_id: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &mut BTreeMap<u64, NodeGeometry>,
        input: &LayoutInput,
        text_engine: &mut TextEngine,
    ) -> pre_child_production::PreChildResult {
        let Self {
            id,
            containing,
            allocated,
            ancestor_hidden: _,
            formatting_style,
            font_size,
            mut margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            available,
            intrinsic_widths,
            horizontal_edges,
            mut content_width,
            mut outer_width,
            specified_width,
            specified_height,
            height_basis,
            border_box_sizing,
            out_of_flow,
            invisible,
            mut x,
            y,
            axes,
            intrinsic_text,
            inline_size,
            text_inline,
            column_format,
            column_has_floats: _,
            column_float_height,
            column_deferred_start: _,
        } = self;
        let node = by_id[&id];
        let style = formatting_style.as_ref().unwrap_or(styles[&id].as_ref());
        let width_value = style_value(style, "width", "auto");
        let position = style_value(style, "position", "static");
        let bound = |name, default| {
            let value = style_value(style, name, default);
            let width = if is_intrinsic_width(value) {
                intrinsic_widths.and_then(|widths| {
                    resolve_intrinsic_width(
                        value,
                        (available - horizontal_edges).max(0.),
                        widths,
                        font_size,
                        input.viewport,
                    )
                })
            } else {
                optional_length(value, containing.width, font_size, input.viewport).map(|value| {
                    if border_box_sizing {
                        (value - horizontal_edges).max(0.)
                    } else {
                        value.max(0.)
                    }
                })
            };
            width.map(|value| (value - scrollbar_gutter.left - scrollbar_gutter.right).max(0.))
        };
        if ruby_baseline.is_some() && width_value == "auto" && allocated.is_none() {
            content_width = if axes.vertical {
                text_layout.height
            } else {
                text_layout.width
            };
            if let Some(max) = bound("max-width", "none") {
                content_width = content_width.min(max);
            }
            if let Some(min) = bound("min-width", "auto") {
                content_width = content_width.max(min);
            }
            outer_width =
                content_width + horizontal_edges + scrollbar_gutter.left + scrollbar_gutter.right;
        }
        if let Some(columns) = column_format.filter(|_| !column_block_flow) {
            columns.place(&mut text_layout);
            let step = (columns.inline + columns.gap).max(1.);
            let count = text_layout
                .lines
                .iter()
                .map(|line| {
                    ((if columns.reverse {
                        columns.available - columns.inline - line.4
                    } else {
                        line.4
                    }) / step)
                        .round()
                        .max(0.) as usize
                        + 1
                })
                .max()
                .unwrap_or(0);
            text_engine.column_runs.insert(
                id,
                vec![columns::ColumnRun {
                    columns,
                    count,
                    start: 0.,
                    block: text_layout.height,
                }],
            );
        }
        if axes.vertical {
            if allocated.is_none()
                && !out_of_flow
                && inline_context.is_some()
                && let Some((child, value)) = margin_collapse::used_child_vertical_end(
                    crate::margin_collapse::VerticalChildEndInputs {
                        id,
                        reverse: axes.x_reverse,
                        basis: inline_size,
                        nodes: by_id,
                        styles,
                        viewport: input.viewport,
                        geometry,
                        clearance: &text_engine.clearance_nodes,
                    },
                )
            {
                text_layout.height = (text_layout.height - value).max(0.);
                for item in &mut text_layout.inline_boxes {
                    if item.node == child {
                        item.rect.height = (item.rect.height - value).max(0.);
                    }
                }
                if axes.x_reverse {
                    let previous = margin.left;
                    margin.left = inline_formatting::MarginStrut::default()
                        .append(previous)
                        .append(value)
                        .value();
                    x += margin.left - previous;
                } else {
                    margin.right = inline_formatting::MarginStrut::default()
                        .append(margin.right)
                        .append(value)
                        .value();
                }
                text_engine
                    .absorbed_vertical_margins
                    .entry(child)
                    .or_insert([0.; 2])[if axes.x_reverse { 0 } else { 1 }] = value;
            }
            if width_value == "auto"
                && allocated.is_none()
                && (node.children.is_empty() || inline_context.is_some())
                && specified_width.is_none()
            {
                content_width = text_layout.height.max(float_bfc_extent.unwrap_or(0.));
                if let Some(max) = bound("max-width", "none") {
                    content_width = content_width.min(max);
                }
                if let Some(min) = bound("min-width", "auto") {
                    content_width = content_width.max(min);
                }
                outer_width = content_width
                    + horizontal_edges
                    + scrollbar_gutter.left
                    + scrollbar_gutter.right;
            }
            vertical_text::physicalize(
                &mut text_layout,
                content_width,
                inline_size,
                style_value(style, "writing-mode", "horizontal-tb"),
            );
        }
        let mut resolved = text_layout.resolved_text.clone();
        for run in &mut resolved {
            run.layout_node = id;
        }
        text_engine.resolved_text.insert(id, resolved);
        if text_engine.font_demand_error.is_none() {
            match text_pseudos::font_demands(&text_layout) {
                Ok(demands) => {
                    let old = text_engine
                        .font_demands
                        .get(&id)
                        .map(text_pseudos::demand_bytes)
                        .unwrap_or(0);
                    let bytes = text_engine
                        .font_demand_bytes
                        .saturating_sub(old)
                        .saturating_add(text_pseudos::demand_bytes(&demands));
                    if bytes > 64 * 1024 * 1024 {
                        text_engine.font_demand_error = Some(LayoutError::StyleTraversal(
                            "page font demand exceeds native 64-MiB bound".into(),
                        ));
                    } else {
                        text_engine.font_demand_bytes = bytes;
                        text_engine.font_demands.insert(id, demands);
                    }
                }
                Err(error) => text_engine.font_demand_error = Some(error),
            }
        }
        for line_break in &text_layout.breaks {
            let mut line_break = line_break.clone();
            line_break.layout_node = id;
            let shift = |rect: &mut CssRect| {
                rect.x += x + border.left + padding.left
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.x
                    };
                rect.y += y + border.top + padding.top
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.y
                    };
            };
            shift(&mut line_break.before);
            if let Some(after) = &mut line_break.after {
                shift(after);
            }
            text_engine.break_boxes.insert(line_break.node, line_break);
        }
        let mut source_geometry = BTreeMap::<u64, Vec<oryn_common::v4::TextClusterGeometry>>::new();
        for cluster in &text_layout.source_clusters {
            let rect = CssRect {
                x: x + border.left + padding.left + cluster.rect.x
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.x
                    },
                y: y + border.top + padding.top + cluster.rect.y
                    - if position == "fixed" {
                        0.
                    } else {
                        input.scroll.y
                    },
                ..cluster.rect
            };
            source_geometry.entry(cluster.text_node).or_default().push(
                oryn_common::v4::TextClusterGeometry {
                    layout_node: if inline_context
                        .as_ref()
                        .is_some_and(|paragraph| paragraph.inline_nodes.contains(&cluster.owner))
                    {
                        cluster.owner
                    } else {
                        id
                    },
                    start: cluster.utf16.start,
                    end: cluster.utf16.end,
                    rect,
                    matrix: transforms::Projection::IDENTITY.matrix(),
                    line: cluster.line,
                    rtl: cluster.rtl,
                },
            );
        }
        // Deeper direct-text layout supersedes ancestor aggregate measurements.
        text_engine.source_geometry.extend(source_geometry);
        let intrinsic_text_height = text_layout.height;
        let vertical_edges = padding.top + padding.bottom + border.top + border.bottom;
        let clamp_height = |value| {
            let gutter = scrollbar_gutter.top + scrollbar_gutter.bottom;
            (clamp_content_height(
                style,
                value + gutter,
                height_basis,
                font_size,
                input.viewport,
                vertical_edges - gutter,
            ) - gutter)
                .max(0.)
        };
        // An auto height remains indefinite even when min-height gives the box a
        // used height. Percentage descendants must not use that minimum as a basis.
        let definite_content_height = allocated
            .map(|size| (size.height - vertical_edges).max(0.))
            .or_else(|| {
                specified_height.map(|value| {
                    clamp_height(
                        (value
                            - if border_box_sizing {
                                vertical_edges
                            } else {
                                0.
                            })
                        .max(0.),
                    )
                })
            });
        pre_child_production::PreChildResult::Ready(pre_child_production::PreChildFrame {
            id,
            containing,
            allocated,
            formatting_style,
            margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            font_size,
            x,
            y,
            axes,
            content_width,
            outer_width,
            horizontal_edges,
            available,
            intrinsic_widths,
            vertical_edges,
            height_basis,
            specified_width,
            specified_height,
            border_box_sizing,
            out_of_flow,
            invisible,
            intrinsic_text,
            intrinsic_text_height,
            inline_context,
            text_layout,
            column_format,
            column_block_flow,
            column_float_height,
            float_bfc_extent,
            ruby_baseline,
            text_inline,
            inline_size,
            definite_content_height,
        })
    }
}

struct SuspendedBlock {
    frame: pre_child_production::PreChildFrame,
    ordered: Vec<u64>,
    next: usize,
    cursor: BlockFlowCursor,
    pending: BlockChildPlacement,
}

enum ChildrenPhase {
    Inline,
    Placed,
    Vertical,
    Block,
}

struct SuspendedPlaced {
    frame: pre_child_production::PreChildFrame,
    placements: Vec<(u64, ChildPlacement)>,
    next: usize,
    height: f64,
}

enum SuspendedNode {
    Block(SuspendedBlock),
    Placed(SuspendedPlaced),
    Vertical(SuspendedVertical),
    Inline(SuspendedInline),
    Column(Box<ColumnTrial>),
}
struct SuspendedVertical {
    frame: pre_child_production::PreChildFrame,
    ordered: Vec<u64>,
    next: usize,
    pending: u64,
    cursor: f64,
    extent: f64,
    height: f64,
}
enum BeginPlaced {
    Child(LayoutRequest, Box<SuspendedPlaced>),
    Complete(f64),
}

pub(super) struct BlockExecutor<'a> {
    pub(super) nodes: &'a BTreeMap<u64, &'a LayoutNode>,
    pub(super) styles: &'a ComputedStyles,
    pub(super) geometry: &'a mut BTreeMap<u64, NodeGeometry>,
    pub(super) offsets: &'a mut BTreeMap<u64, (f64, f64)>,
    pub(super) resolved: &'a mut BTreeMap<u64, BTreeMap<String, String>>,
    pub(super) text: &'a mut BTreeMap<u64, Vec<PaintTextFragment>>,
    pub(super) input: &'a LayoutInput,
    pub(super) diagnostics: &'a mut Vec<String>,
    pub(super) engine: &'a mut TextEngine,
}

impl BlockExecutor<'_> {
    fn reject_unsupported(&self, id: u64) -> Result<(), String> {
        if self
            .geometry
            .get(&id)
            .is_some_and(|geometry| geometry.availability == GeometryAvailability::Unsupported)
        {
            Err(format!("node {id}: native child layout incomplete"))
        } else {
            Ok(())
        }
    }
    fn finish_inline(&mut self, frame: pre_child_production::PreChildFrame) -> f64 {
        let style = frame
            .formatting_style
            .as_ref()
            .unwrap_or(self.styles[&frame.id].as_ref());
        let fixed = style_value(style, "position", "static") == "fixed";
        for item in &frame.text_layout.inline_boxes {
            let x = frame.x + frame.border.left + frame.padding.left + item.rect.x
                - if fixed { 0. } else { self.input.scroll.x };
            let y = frame.y + frame.border.top + frame.padding.top + item.rect.y
                - if fixed { 0. } else { self.input.scroll.y };
            if let Some(model) = self
                .geometry
                .get(&item.node)
                .and_then(|g| g.box_model.clone())
            {
                let dx = x
                    - (model.border_box.x - model.margin.left
                        + self
                            .engine
                            .absorbed_vertical_margins
                            .get(&item.node)
                            .map_or(0., |v| v[0]));
                let dy = y
                    - (model.border_box.y - model.margin.top
                        + self
                            .engine
                            .absorbed_margins
                            .get(&item.node)
                            .map_or(0., |v| v[0]));
                positioning::translate_subtree(
                    item.node,
                    dx,
                    dy,
                    self.nodes,
                    self.geometry,
                    self.text,
                    self.engine,
                );
            }
            for (child, containing) in &mut self.engine.out_of_flow {
                if *child == item.node {
                    containing.x = x + self.input.scroll.x;
                    containing.y = y + self.input.scroll.y;
                }
            }
        }
        if let Some(paragraph) = &frame.inline_context {
            for child in paragraph.atomic.iter().chain(paragraph.boundaries.keys()) {
                self.engine.inline_sizes.remove(child);
            }
        }
        self.finish(frame, 0.)
    }
    fn vertical_request(
        &self,
        frame: &pre_child_production::PreChildFrame,
        id: u64,
        cursor: f64,
    ) -> LayoutRequest {
        LayoutRequest {
            id,
            containing: FlowBox {
                x: cursor,
                y: frame.y + frame.border.top + frame.padding.top,
                width: frame.content_width,
                height: Some(frame.inline_size),
            },
            allocated: None,
            ancestor_hidden: false,
        }
    }

    fn resume_vertical(
        &mut self,
        mut suspended: SuspendedVertical,
    ) -> Result<Result<(LayoutRequest, SuspendedVertical), f64>, String> {
        self.reject_unsupported(suspended.pending)?;
        if !self.styles.get(&suspended.pending).is_some_and(|style| {
            matches!(
                style_value(style, "position", "static"),
                "absolute" | "fixed"
            )
        }) && let Some(model) = self
            .geometry
            .get(&suspended.pending)
            .and_then(|g| g.box_model.clone())
        {
            let advance = model.margin.left + model.border_box.width + model.margin.right;
            if suspended.frame.axes.x_reverse {
                positioning::translate_subtree(
                    suspended.pending,
                    -advance,
                    0.,
                    self.nodes,
                    self.geometry,
                    self.text,
                    self.engine,
                );
                suspended.cursor -= advance;
            } else {
                suspended.cursor += advance;
            }
            suspended.extent += advance;
            suspended.height = suspended
                .height
                .max(model.border_box.height + model.margin.top + model.margin.bottom);
        }
        if let Some(&id) = suspended.ordered.get(suspended.next) {
            let request = self.vertical_request(&suspended.frame, id, suspended.cursor);
            suspended.pending = id;
            suspended.next += 1;
            Ok(Ok((request, suspended)))
        } else {
            Ok(Err(self.finish_vertical(suspended)))
        }
    }

    fn finish_vertical(&mut self, mut suspended: SuspendedVertical) -> f64 {
        suspended.extent += self
            .engine
            .table_end_spacing
            .get(&suspended.frame.id)
            .copied()
            .unwrap_or(0.);
        let frame = &mut suspended.frame;
        let style = frame
            .formatting_style
            .as_ref()
            .unwrap_or(self.styles[&frame.id].as_ref());
        if style_value(style, "width", "auto") == "auto"
            && frame.allocated.is_none()
            && frame.specified_width.is_none()
        {
            let previous = frame.content_width;
            let bound = |name, default| {
                let value = style_value(style, name, default);
                let width = if is_intrinsic_width(value) {
                    frame.intrinsic_widths.and_then(|widths| {
                        resolve_intrinsic_width(
                            value,
                            (frame.available - frame.horizontal_edges).max(0.),
                            widths,
                            frame.font_size,
                            self.input.viewport,
                        )
                    })
                } else {
                    optional_length(
                        value,
                        frame.containing.width,
                        frame.font_size,
                        self.input.viewport,
                    )
                    .map(|value| {
                        if frame.border_box_sizing {
                            (value - frame.horizontal_edges).max(0.)
                        } else {
                            value.max(0.)
                        }
                    })
                };
                width.map(|value| {
                    (value - frame.scrollbar_gutter.left - frame.scrollbar_gutter.right).max(0.)
                })
            };
            let maximum = bound("max-width", "none");
            let minimum = bound("min-width", "auto");
            frame.content_width = suspended.extent.max(0.);
            if let Some(max) = maximum {
                frame.content_width = frame.content_width.min(max);
            }
            if let Some(min) = minimum {
                frame.content_width = frame.content_width.max(min);
            }
            frame.outer_width = frame.content_width
                + frame.horizontal_edges
                + frame.scrollbar_gutter.left
                + frame.scrollbar_gutter.right;
            if frame.axes.x_reverse {
                let dx = frame.content_width - previous;
                for child in &self.nodes[&frame.id].children {
                    positioning::translate_subtree(
                        *child,
                        dx,
                        0.,
                        self.nodes,
                        self.geometry,
                        self.text,
                        self.engine,
                    );
                }
                for line in &mut frame.text_layout.lines {
                    line.4 += dx;
                }
                for clusters in self.engine.source_geometry.values_mut() {
                    for cluster in clusters {
                        if cluster.layout_node == frame.id {
                            cluster.rect.x += dx;
                        }
                    }
                }
            }
        }
        self.finish(suspended.frame, suspended.height)
    }

    fn begin_placed(
        &mut self,
        frame: pre_child_production::PreChildFrame,
    ) -> Result<BeginPlaced, String> {
        let style = frame
            .formatting_style
            .as_ref()
            .unwrap_or(self.styles[&frame.id].as_ref());
        let placements = taffy_child_placements(ChildPlacementInputs {
            node: self.nodes[&frame.id],
            parent_style: style,
            styles: self.styles,
            width: frame.content_width,
            height: frame.definite_content_height,
            viewport: self.input.viewport,
            nodes: self.nodes,
            text_engine: self.engine,
        })
        .ok_or("native placed-child continuation layout failed")?;
        let placements = self.nodes[&frame.id]
            .children
            .iter()
            .copied()
            .zip(placements)
            .collect::<Vec<_>>();
        let Some(&(id, placement)) = placements.first() else {
            return Ok(BeginPlaced::Complete(self.finish(frame, 0.)));
        };
        let request = self.placed_request(&frame, id, placement);
        Ok(BeginPlaced::Child(
            request,
            Box::new(SuspendedPlaced {
                frame,
                placements,
                next: 1,
                height: 0.,
            }),
        ))
    }

    fn placed_request(
        &self,
        frame: &pre_child_production::PreChildFrame,
        id: u64,
        placement: ChildPlacement,
    ) -> LayoutRequest {
        LayoutRequest {
            id,
            containing: FlowBox {
                x: frame.x + frame.border.left + frame.padding.left + placement.x,
                y: frame.y
                    + frame.border.top
                    + frame.padding.top
                    + if frame.axes.vertical {
                        0.
                    } else {
                        frame.intrinsic_text_height
                    }
                    + placement.y,
                width: frame.content_width,
                height: frame.definite_content_height,
            },
            allocated: Some(TaffySize {
                width: placement.width,
                height: placement.height,
            }),
            ancestor_hidden: false,
        }
    }

    fn resume_placed(
        &mut self,
        mut suspended: SuspendedPlaced,
        used: f64,
    ) -> Result<Result<(LayoutRequest, SuspendedPlaced), f64>, String> {
        let (id, placement) = suspended.placements[suspended.next - 1];
        self.reject_unsupported(id)?;
        if !self.styles.get(&id).is_some_and(|style| {
            matches!(
                style_value(style, "position", "static"),
                "absolute" | "fixed"
            )
        }) {
            suspended.height = suspended.height.max(placement.y + used);
        }
        if let Some(&(id, placement)) = suspended.placements.get(suspended.next) {
            let request = self.placed_request(&suspended.frame, id, placement);
            suspended.next += 1;
            Ok(Ok((request, suspended)))
        } else {
            Ok(Err(self.finish(suspended.frame, suspended.height)))
        }
    }

    pub(super) fn run_block(&mut self, request: LayoutRequest) -> Result<f64, String> {
        // Nested synthetic-tree invocations must not pollute an outer scope.
        let previous_margin_suffixes =
            std::mem::take(&mut *self.engine.margin_suffixes.borrow_mut());
        #[cfg(test)]
        margin_collapse::reset_chain_profile();
        let mut stack: Vec<SuspendedNode> = Vec::new();
        let result = self.run_inner(request, &mut stack);
        *self.engine.margin_suffixes.borrow_mut() = previous_margin_suffixes;
        #[cfg(test)]
        margin_collapse::report_chain_profile();
        if result.is_err() {
            while let Some(frame) = stack.pop() {
                match frame {
                    SuspendedNode::Inline(frame) => frame.unwind(self.engine),
                    SuspendedNode::Column(frame) => frame.unwind(self.engine),
                    _ => {}
                }
            }
        }
        result
    }

    fn run_inner(
        &mut self,
        mut request: LayoutRequest,
        stack: &mut Vec<SuspendedNode>,
    ) -> Result<f64, String> {
        let mut prepared = None;
        'enter: loop {
            if self.engine.cancelled.as_ref().is_some_and(|check| check()) {
                self.engine.font_demand_error = Some(LayoutError::Cancelled);
                return Err("layout owner retired".into());
            }
            let mut completed = match prepared.take().unwrap_or_else(|| self.prepare(request)) {
                pre_child_production::PreChildResult::Complete(height) => height,
                pre_child_production::PreChildResult::Failed(error) => return Err(error.into()),
                pre_child_production::PreChildResult::Inline(child, suspended) => {
                    if stack.try_reserve(1).is_err() {
                        suspended.unwind(self.engine);
                        return Err("native inline continuation allocation failed".into());
                    }
                    stack.push(SuspendedNode::Inline(suspended));
                    request = child;
                    continue;
                }
                pre_child_production::PreChildResult::Replay(child, trial) => {
                    if stack.try_reserve(1).is_err() {
                        trial.unwind(self.engine);
                        return Err("native column continuation allocation failed".into());
                    }
                    stack.push(SuspendedNode::Column(trial));
                    request = child;
                    continue;
                }
                pre_child_production::PreChildResult::Ready(frame) => 'ready: {
                    match self.children_phase(&frame) {
                        ChildrenPhase::Placed => {
                            let (child, suspended) = match self.begin_placed(frame)? {
                                BeginPlaced::Child(child, suspended) => (child, *suspended),
                                BeginPlaced::Complete(height) => break 'ready height,
                            };
                            stack
                                .try_reserve(1)
                                .map_err(|_| "native placed continuation allocation failed")?;
                            stack.push(SuspendedNode::Placed(suspended));
                            request = child;
                            continue 'enter;
                        }
                        ChildrenPhase::Vertical => {
                            let ordered =
                                tables::ordered_children(self.nodes[&frame.id], self.styles);
                            let cursor = frame.x
                                + frame.border.left
                                + frame.padding.left
                                + if frame.axes.x_reverse {
                                    frame.content_width - frame.text_layout.width
                                } else {
                                    frame.text_layout.width
                                };
                            let Some(&id) = ordered.first() else {
                                let extent = frame.text_layout.width;
                                break 'ready self.finish_vertical(SuspendedVertical {
                                    frame,
                                    ordered,
                                    next: 0,
                                    pending: 0,
                                    cursor,
                                    extent,
                                    height: 0.,
                                });
                            };
                            request = self.vertical_request(&frame, id, cursor);
                            let extent = frame.text_layout.width;
                            stack
                                .try_reserve(1)
                                .map_err(|_| "native vertical continuation allocation failed")?;
                            stack.push(SuspendedNode::Vertical(SuspendedVertical {
                                frame,
                                ordered,
                                next: 1,
                                pending: id,
                                cursor,
                                extent,
                                height: 0.,
                            }));
                            continue 'enter;
                        }
                        ChildrenPhase::Inline => {
                            break 'ready self.finish_inline(frame);
                        }
                        ChildrenPhase::Block => {}
                    }
                    let ordered = tables::ordered_children(self.nodes[&frame.id], self.styles);
                    let y = frame.y
                        + frame.border.top
                        + frame.padding.top
                        + frame.intrinsic_text_height;
                    let mut cursor = BlockFlowCursor::new(y);
                    if let Some(&id) = ordered.first() {
                        let pending = self.child(&frame, &mut cursor, id);
                        request = LayoutRequest {
                            id,
                            containing: pending.containing,
                            allocated: None,
                            ancestor_hidden: false,
                        };
                        stack
                            .try_reserve(1)
                            .map_err(|_| "native block continuation allocation failed")?;
                        stack.push(SuspendedNode::Block(SuspendedBlock {
                            frame,
                            ordered,
                            next: 1,
                            cursor,
                            pending,
                        }));
                        continue 'enter;
                    }
                    self.finish(frame, 0.)
                }
            };
            loop {
                let Some(suspended) = stack.pop() else {
                    return Ok(completed);
                };
                if self.engine.cancelled.as_ref().is_some_and(|check| check()) {
                    stack.push(suspended);
                    self.engine.font_demand_error = Some(LayoutError::Cancelled);
                    return Err("layout owner retired".into());
                }
                let mut suspended = match suspended {
                    SuspendedNode::Column(trial) => match trial.resume(
                        completed,
                        self.engine,
                        self.nodes,
                        self.geometry,
                        self.diagnostics,
                    ) {
                        ColumnAdvance::Child(child, trial) => {
                            stack.push(SuspendedNode::Column(trial));
                            request = child;
                            break;
                        }
                        ColumnAdvance::Complete(height) => {
                            completed = height;
                            continue;
                        }
                    },
                    SuspendedNode::Inline(SuspendedInline { capture, flow }) => {
                        let style = capture
                            .formatting_style
                            .as_ref()
                            .unwrap_or(self.styles[&capture.id].as_ref());
                        let advance = match flow {
                            flow @ InlineChildResume::Block { .. } => flow.resume_block(
                                self.nodes,
                                self.styles,
                                self.geometry,
                                self.text,
                                self.input,
                                self.engine,
                                style,
                            ),
                            flow @ InlineChildResume::Atomic { .. } => flow.resume_atomic(
                                self.nodes,
                                self.styles,
                                self.geometry,
                                self.text,
                                self.input,
                                self.engine,
                                style,
                            ),
                        };
                        let advance = match advance {
                            Ok(advance) => advance,
                            Err(error) => {
                                self.engine.column_context = capture.previous_columns;
                                self.engine
                                    .out_of_flow
                                    .truncate(capture.column_deferred_start);
                                inline_formatting::leave_float_scope(
                                    self.engine,
                                    capture.float_scope,
                                );
                                return Err(error.into());
                            }
                        };
                        match advance {
                            InlineAdvance::Child(child, flow) => {
                                stack
                                    .push(SuspendedNode::Inline(SuspendedInline { capture, flow }));
                                request = child;
                                break;
                            }
                            InlineAdvance::Complete(context, layout, has_blocks) => {
                                let (capture, context, layout, blocks, extent) = capture
                                    .receive_inline(context, layout, has_blocks, self.engine);
                                match capture.continue_layout(
                                    context,
                                    layout,
                                    blocks,
                                    extent,
                                    self.nodes,
                                    self.styles,
                                    self.geometry,
                                    self.input,
                                    self.engine,
                                    self.diagnostics,
                                ) {
                                    PostInlineAdvance::Ready(frame) => {
                                        prepared = Some(
                                            pre_child_production::PreChildResult::Ready(*frame),
                                        )
                                    }
                                    PostInlineAdvance::Replay(child, trial) => {
                                        if stack.try_reserve(1).is_err() {
                                            trial.unwind(self.engine);
                                            return Err(
                                                "native replay continuation allocation failed"
                                                    .into(),
                                            );
                                        }
                                        stack.push(SuspendedNode::Column(trial));
                                        request = child;
                                    }
                                    PostInlineAdvance::Complete(height) => {
                                        completed = height;
                                        continue;
                                    }
                                }
                                break;
                            }
                        }
                    }
                    SuspendedNode::Placed(suspended) => {
                        match self.resume_placed(suspended, completed)? {
                            Ok((child, suspended)) => {
                                stack.push(SuspendedNode::Placed(suspended));
                                request = child;
                                break;
                            }
                            Err(height) => {
                                completed = height;
                                continue;
                            }
                        }
                    }
                    SuspendedNode::Vertical(suspended) => match self.resume_vertical(suspended)? {
                        Ok((child, suspended)) => {
                            stack.push(SuspendedNode::Vertical(suspended));
                            request = child;
                            break;
                        }
                        Err(height) => {
                            completed = height;
                            continue;
                        }
                    },
                    SuspendedNode::Block(suspended) => suspended,
                };
                self.reject_unsupported(suspended.pending.id)?;
                suspended.cursor.resume_child(
                    suspended.pending,
                    completed,
                    self.styles,
                    self.geometry,
                    self.engine,
                );
                if let Some(&id) = suspended.ordered.get(suspended.next) {
                    let pending = self.child(&suspended.frame, &mut suspended.cursor, id);
                    request = LayoutRequest {
                        id,
                        containing: pending.containing,
                        allocated: None,
                        ancestor_hidden: false,
                    };
                    suspended.pending = pending;
                    suspended.next += 1;
                    stack.push(SuspendedNode::Block(suspended));
                    break;
                }
                completed = self.finish(suspended.frame, suspended.cursor.height);
            }
        }
    }

    fn children_phase(&self, frame: &pre_child_production::PreChildFrame) -> ChildrenPhase {
        if frame.inline_context.is_some() {
            return ChildrenPhase::Inline;
        }
        let style = frame
            .formatting_style
            .as_ref()
            .unwrap_or(self.styles[&frame.id].as_ref());
        let floated = self.nodes[&frame.id].children.iter().any(|id| {
            self.styles.get(id).is_some_and(|style| {
                style_value(style, "float", "none") != "none"
                    || style_value(style, "clear", "none") != "none"
            })
        });
        if matches!(
            style_value(style, "display", "block"),
            "flex" | "inline-flex" | "grid" | "inline-grid" | "table-row"
        ) || floated
        {
            ChildrenPhase::Placed
        } else if frame.axes.vertical {
            ChildrenPhase::Vertical
        } else {
            ChildrenPhase::Block
        }
    }

    fn prepare(&mut self, request: LayoutRequest) -> pre_child_production::PreChildResult {
        pre_child_production::prepare_node(
            request.id,
            request.containing,
            self.nodes,
            self.styles,
            self.geometry,
            self.offsets,
            self.text,
            self.input,
            request.ancestor_hidden,
            self.diagnostics,
            self.engine,
            request.allocated,
        )
    }

    fn child(
        &mut self,
        frame: &pre_child_production::PreChildFrame,
        cursor: &mut BlockFlowCursor,
        id: u64,
    ) -> BlockChildPlacement {
        let style = frame
            .formatting_style
            .as_ref()
            .unwrap_or(self.styles[&frame.id].as_ref());
        cursor.prepare_child(
            id,
            FlowBox {
                x: frame.x + frame.border.left + frame.padding.left,
                y: cursor.cursor,
                width: frame.content_width,
                height: frame.definite_content_height,
            },
            style_value(style, "display", "block"),
            frame.axes.vertical,
            self.nodes,
            self.styles,
            self.input,
            self.engine,
        )
    }

    fn finish(&mut self, frame: pre_child_production::PreChildFrame, children_height: f64) -> f64 {
        let pre_child_production::PreChildFrame {
            id,
            containing,
            allocated,
            formatting_style,
            margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            font_size,
            x,
            y,
            axes,
            content_width,
            outer_width,
            horizontal_edges,
            available,
            intrinsic_widths,
            vertical_edges,
            height_basis,
            specified_width,
            specified_height,
            border_box_sizing,
            out_of_flow,
            invisible,
            intrinsic_text,
            intrinsic_text_height,
            inline_context,
            text_layout,
            column_format,
            column_block_flow,
            column_float_height,
            float_bfc_extent,
            ruby_baseline,
            text_inline,
            inline_size: _,
            definite_content_height,
        } = frame;
        post_child_production::finish_node(post_child_production::PostChildInputs {
            id,
            containing,
            allocated,
            formatting_style,
            margin,
            padding,
            authored_padding,
            border,
            scrollbar_gutter,
            absorbed,
            font_size,
            x,
            y,
            axes,
            content_width,
            outer_width,
            horizontal_edges,
            available,
            intrinsic_widths,
            vertical_edges,
            height_basis,
            specified_width,
            specified_height,
            border_box_sizing,
            out_of_flow,
            invisible,
            intrinsic_text,
            intrinsic_text_height,
            inline_context,
            text_layout,
            column_format,
            column_block_flow,
            column_float_height,
            float_bfc_extent,
            ruby_baseline,
            text_inline,
            definite_content_height,
            children_height,
            nodes: self.nodes,
            styles: self.styles,
            geometry: self.geometry,
            offsets: self.offsets,
            resolved: self.resolved,
            text: self.text,
            input: self.input,
            diagnostics: self.diagnostics,
            engine: self.engine,
        })
    }
}
