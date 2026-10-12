//! Production post-child extraction draft; caller wiring must commit together.
use super::*;

pub(super) struct PostChildInputs<'a> {
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
    pub(super) definite_content_height: Option<f64>,
    pub(super) children_height: f64,
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

pub(super) fn finish_node(params: PostChildInputs<'_>) -> f64 {
    let PostChildInputs {
        id,
        containing,
        allocated,
        formatting_style,
        mut margin,
        padding,
        authored_padding,
        border,
        scrollbar_gutter,
        absorbed,
        font_size,
        x,
        y,
        axes,
        mut content_width,
        mut outer_width,
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
        mut intrinsic_text_height,
        inline_context,
        mut text_layout,
        column_format,
        column_block_flow,
        column_float_height,
        float_bfc_extent,
        ruby_baseline,
        text_inline,
        definite_content_height,
        mut children_height,
        nodes: by_id,
        styles,
        geometry,
        offsets: offset_dimensions,
        resolved: resolved_values,
        text: text_fragments,
        input,
        diagnostics,
        engine: text_engine,
    } = params;
    let node = by_id[&id];
    let style = formatting_style.as_ref().unwrap_or(styles[&id].as_ref());
    let position = style_value(style, "position", "static");
    let display = style_value(style, "display", "block");
    let width_value = style_value(style, "width", "auto");
    let bound = |property, default| {
        let value = style_value(style, property, default);
        let dimension = if is_intrinsic_width(value) {
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
        dimension.map(|value| (value - scrollbar_gutter.left - scrollbar_gutter.right).max(0.))
    };
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
    if let Some(columns) = column_format.filter(|_| column_block_flow) {
        let origin = (
            x + border.left + padding.left
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.x
                },
            y + border.top + padding.top
                - if position == "fixed" {
                    0.
                } else {
                    input.scroll.y
                },
        );
        match columns::fragment_flow(crate::columns::ColumnFragmentFlow {
            columns,
            root: id,
            origin,
            scroll: (input.scroll.x, input.scroll.y),
            block_extent: content_width,
            mode: style_value(style, "writing-mode", "horizontal-tb"),
            nodes: by_id,
            styles,
            geometry,
            offsets: offset_dimensions,
            text: text_fragments,
            layout: &mut text_layout,
            engine: text_engine,
        }) {
            Ok(frame) => {
                intrinsic_text_height = text_layout.height;
                if axes.vertical
                    && width_value == "auto"
                    && allocated.is_none()
                    && specified_width.is_none()
                {
                    content_width = frame;
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
            }
            Err(error) => diagnostics.push(format!("node {id}: {error}")),
        }
    }
    if let Some(row) = text_engine.table_rows.get(&id) {
        children_height = if row.vertical {
            row.inline_size
        } else {
            row.height
        };
    }
    if !axes.vertical {
        children_height += text_engine
            .table_end_spacing
            .get(&id)
            .copied()
            .unwrap_or(0.);
    }
    let mut content_height = ContentHeightInputs {
        specified: specified_height,
        text: intrinsic_text_height,
        children: children_height,
        width: content_width,
        horizontal_edges,
        vertical_edges,
        vertical: axes.vertical,
    }
    .resolve(node, style);
    if border_box_sizing && specified_height.is_some() {
        content_height =
            (content_height - padding.top - padding.bottom - border.top - border.bottom).max(0.0);
    }
    if column_float_height.is_none()
        && !axes.vertical
        && specified_height.is_none()
        && let Some(extent) = float_bfc_extent
    {
        content_height = content_height.max(extent);
    }
    if specified_height.is_none()
        && let Some(context) = inline_context.as_ref()
        && !axes.vertical
        && (node.parent.is_none()
            || matches!(
                display,
                "flow-root" | "inline-block" | "inline-flex" | "inline-grid"
            )
            || out_of_flow
            || style_value(style, "float", "none") != "none"
            || ["overflow-x", "overflow-y"]
                .iter()
                .any(|key| !matches!(style_value(style, key, "visible"), "visible" | "clip")))
    {
        for child in &context.atomic {
            if style_value(&styles[child], "float", "none") != "none"
                && let Some(model) = geometry
                    .get(child)
                    .and_then(|value| value.box_model.as_ref())
            {
                content_height = content_height.max(
                    model.border_box.bottom() + model.margin.bottom
                        - (y + border.top + padding.top
                            - if position == "fixed" {
                                0.
                            } else {
                                input.scroll.y
                            }),
                );
            }
        }
    }
    if allocated.is_none()
        && !out_of_flow
        && !axes.vertical
        && let Some((child, value)) = margin_collapse::used_child_bottom(
            id,
            content_width,
            by_id,
            styles,
            input.viewport,
            geometry,
            &text_engine.clearance_nodes,
        )
    {
        content_height = (content_height - value).max(0.);
        margin.bottom = inline_formatting::MarginStrut::default()
            .append(margin.bottom)
            .append(value)
            .value();
        text_engine.absorbed_margins.entry(child).or_insert([0.; 2])[1] = value;
    }
    content_height = clamp_height(content_height);
    if let Some(size) = allocated {
        content_height = (size.height - vertical_edges).max(0.);
    }
    let border_width = outer_width.max(0.0);
    let border_height = content_height + padding.top + padding.bottom + border.top + border.bottom;
    offset_dimensions.insert(id, (border_width, border_height));
    if let Some(baseline) = ruby_baseline {
        text_engine
            .ruby_baselines
            .insert(id, margin.top + border.top + padding.top + baseline);
    }
    // CSSOM resolves dimensions before transforms. Visibility suppresses paint,
    // not layout boxes. Non-replaced inline boxes have no used width/height.
    let replaced = is_replaced_box(node);
    if !matches!(display, "none" | "contents") && (display != "inline" || replaced) {
        let mut values = BTreeMap::new();
        if !matches!(
            display,
            "table-row" | "table-row-group" | "table-header-group" | "table-footer-group"
        ) {
            let width = if border_box_sizing {
                border_width
            } else {
                content_width + scrollbar_gutter.left + scrollbar_gutter.right
            };
            values.insert("width".into(), format!("{width}px"));
        }
        if !matches!(display, "table-column" | "table-column-group") {
            let height = if border_box_sizing {
                border_height
            } else {
                content_height + scrollbar_gutter.top + scrollbar_gutter.bottom
            };
            values.insert("height".into(), format!("{height}px"));
        }
        resolved_values.insert(id, values);
    }

    let reference_rect = CssRect {
        x,
        y,
        width: border_width,
        height: border_height,
    };
    let to_viewport = |fragment: CssRect| {
        let document_rect = fragment;
        CssRect {
            x: document_rect.x
                - if position == "fixed" {
                    0.0
                } else {
                    input.scroll.x
                },
            y: document_rect.y
                - if position == "fixed" {
                    0.0
                } else {
                    input.scroll.y
                },
            ..document_rect
        }
    };
    if let (Some(baseline), Some(line)) = (text_layout.baselines.first(), text_layout.lines.first())
    {
        text_engine.first_line_baselines.insert(
            id,
            if axes.vertical {
                line.4 + baseline
            } else {
                line.3 + baseline
            },
        );
    } else {
        text_engine.first_line_baselines.remove(&id);
    }
    let rect = to_viewport(reference_rect);
    let viewport_rect = CssRect {
        x: 0.0,
        y: 0.0,
        width: input.viewport.width,
        height: input.viewport.height,
    };
    let opacity = style_value(style, "opacity", "1")
        .parse::<f64>()
        .unwrap_or(1.0);
    let pointer_events = style_value(style, "pointer-events", "auto");
    let in_viewport = rect.intersects(viewport_rect);
    let client_rects =
        if intrinsic_text.is_empty() || style_value(style, "display", "block") != "inline" {
            // Atomic/block boxes contribute their border box, including zero-sized
            // boxes. Text paint lines inside them are not element client fragments.
            vec![rect]
        } else {
            text_layout
                .lines
                .iter()
                .map(|(_, width, height, top, left)| {
                    to_viewport(CssRect {
                        x: x + border.left + padding.left + left,
                        y: y + border.top + padding.top + top,
                        width: (*width).min(content_width),
                        height: *height,
                    })
                })
                .collect()
        };
    if let Some(paragraph) = &inline_context {
        let text_x = rect.x + border.left + padding.left;
        let text_y = rect.y + border.top + padding.top;
        inline_formatting::inline_geometry(crate::inline_formatting::InlineGeometryInputs {
            content_width,
            content_height,
            paragraph,
            layout: &text_layout,
            root: id,
            x: text_x,
            y: text_y,
            nodes: by_id,
            styles,
            viewport: input.viewport,
            inline_basis: text_inline,
            geometry,
            offsets: offset_dimensions,
        });
        for (owner, fragments) in
            inline_formatting::paint_fragments(&text_layout, id, text_x, text_y, styles)
        {
            text_fragments.entry(owner).or_default().extend(fragments);
        }
        for child in &paragraph.inline_nodes {
            if let Some(style) = styles.get(child)
                && style_value(style, "position", "static") == "relative"
            {
                let (dx, dy) = relative_position_offset(
                    style,
                    content_width,
                    definite_content_height,
                    style_value(style, "direction", "ltr") == "rtl",
                    style_value(style, "writing-mode", "horizontal-tb"),
                    font_size,
                    input.viewport,
                );
                positioning::translate_subtree(
                    *child,
                    dx,
                    dy,
                    by_id,
                    geometry,
                    text_fragments,
                    text_engine,
                );
            }
        }
    } else if !intrinsic_text.is_empty() && !invisible {
        let color = style_value(style, "color", "#000000").to_string();
        let font_family = style_value(style, "font-family", DEFAULT_FONT_FAMILY).to_string();
        let fragments = text_layout
            .lines
            .iter()
            .enumerate()
            .map(
                |(line, (text, width, height, top, left))| PaintTextFragment {
                    flow_line: Some(line),
                    flow_rect: Some({
                        let mut flow = text_layout
                            .flow_rect(line, style_value(style, "writing-mode", "horizontal-tb"));
                        flow.x += rect.x + border.left + padding.left;
                        flow.y += rect.y + border.top + padding.top;
                        flow
                    }),
                    clusters: text_layout.paint_clusters(
                        line,
                        rect.x + border.left + padding.left,
                        rect.y + border.top + padding.top,
                    ),
                    text: text.clone(),
                    glyph_runs: text_layout.paint_runs(line),
                    rect: CssRect {
                        x: rect.x + border.left + padding.left + left,
                        y: rect.y + border.top + padding.top + top,
                        width: *width,
                        height: *height,
                    },
                    font_family: font_family.clone(),
                    font_size,
                    color: color.clone(),
                },
            )
            .collect::<Vec<_>>();
        text_fragments.insert(id, fragments);
    }
    geometry.insert(
        id,
        NodeGeometry {
            availability: GeometryAvailability::Available,
            box_model: Some(BoxModel {
                border_box: rect,
                padding_box: to_viewport(inset(reference_rect, border)),
                content_box: to_viewport(inset(inset(reference_rect, border), padding)),
                margin,
                border,
                padding: authored_padding,
            }),
            client_rects,
            in_viewport,
            interactable: in_viewport
                && rect.width > 0.0
                && rect.height > 0.0
                && opacity > 0.0
                && pointer_events != "none",
            diagnostic: None,
        },
    );
    if invisible {
        // Hidden visibility retains layout boxes for CSSOM. Keep the semantic
        // and painting availability hidden, unlike display:none's absent box.
        if let Some(value) = geometry.get_mut(&id) {
            value.availability = GeometryAvailability::Hidden;
            value.in_viewport = false;
            value.interactable = false;
        }
    }
    if matches!(position, "absolute" | "fixed") {
        0.0
    } else {
        (if allocated.is_none() {
            margin.top - absorbed[0]
        } else {
            0.
        }) + border_height
            + margin.bottom
            - absorbed[1]
    }
}
