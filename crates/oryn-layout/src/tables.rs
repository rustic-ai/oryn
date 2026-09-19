//! Native table grid construction and shared column constraints. Rows consume
//! one table-wide plan instead of independently dividing their width in grids.
use super::*;
const MAX_COLUMNS: usize = 1000;
#[derive(Clone, Default)]
pub(super) struct RowPlan {
    pub cells: BTreeMap<u64, ChildPlacement>,
    pub height: f64,
    pub vertical: bool,
    pub inline_size: f64,
}
#[derive(Clone, Copy)]
struct Cell {
    id: u64,
    row: usize,
    column: usize,
    columns: usize,
    rows: usize,
}
fn span(node: &LayoutNode, name: &str, maximum: usize) -> usize {
    node.attributes
        .get(name)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, maximum)
}
pub(super) fn ordered_children(node: &LayoutNode, styles: &ComputedStyles) -> Vec<u64> {
    let mut children = node.children.clone();
    if styles
        .get(&node.id)
        .is_some_and(|s| matches!(style_value(s, "display", "block"), "table" | "inline-table"))
    {
        children.retain(|id| {
            !styles.get(id).is_some_and(|s| {
                matches!(
                    style_value(s, "display", "block"),
                    "table-column" | "table-column-group"
                )
            })
        });
        children.sort_by_key(|id| {
            match styles.get(id).map(|s| style_value(s, "display", "block")) {
                Some("table-caption") => {
                    if styles
                        .get(id)
                        .is_some_and(|s| style_value(s, "caption-side", "top") == "bottom")
                    {
                        4
                    } else {
                        0
                    }
                }
                Some("table-header-group") => 1,
                Some("table-footer-group") => 3,
                _ => 2,
            }
        });
    }
    children
}
fn rows(
    node: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> Vec<u64> {
    let mut result = Vec::new();
    for id in ordered_children(node, styles) {
        let Some(style) = styles.get(&id) else {
            continue;
        };
        match style_value(style, "display", "block") {
            "table-row" => result.push(id),
            "table-header-group" | "table-row-group" | "table-footer-group" => {
                if let Some(group) = nodes.get(&id) {
                    result.extend(
                        group
                            .children
                            .iter()
                            .filter(|id| {
                                styles.get(*id).is_some_and(|s| {
                                    style_value(s, "display", "block") == "table-row"
                                })
                            })
                            .copied(),
                    );
                }
            }
            _ => {}
        }
    }
    result
}
fn columns(
    table: &LayoutNode,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> Vec<(Option<u64>, Option<u64>)> {
    let mut result = Vec::new();
    for id in &table.children {
        let Some(node) = nodes.get(id) else {
            continue;
        };
        let Some(style) = styles.get(id) else {
            continue;
        };
        if style_value(style, "display", "block") == "table-column" {
            result.extend(std::iter::repeat_n(
                (None, Some(*id)),
                span(node, "span", MAX_COLUMNS),
            ));
        } else if style_value(style, "display", "block") == "table-column-group" {
            let children = node
                .children
                .iter()
                .filter(|id| {
                    styles
                        .get(*id)
                        .is_some_and(|s| style_value(s, "display", "block") == "table-column")
                })
                .collect::<Vec<_>>();
            if children.is_empty() {
                result.extend(std::iter::repeat_n(
                    (Some(*id), None),
                    span(node, "span", MAX_COLUMNS),
                ));
            } else {
                for child in children {
                    result.extend(std::iter::repeat_n(
                        (Some(*id), Some(*child)),
                        span(nodes[child], "span", MAX_COLUMNS),
                    ));
                }
            }
        }
        result.truncate(MAX_COLUMNS);
    }
    result
}
pub(super) fn formatting_style(
    id: u64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> Option<BTreeMap<String, String>> {
    let style = styles.get(&id)?;
    if !matches!(
        style_value(style, "display", "block"),
        "table-row"
            | "table-row-group"
            | "table-header-group"
            | "table-footer-group"
            | "table-column"
            | "table-column-group"
    ) {
        return None;
    }
    let mut parent = nodes.get(&id)?.parent;
    let mut seen = BTreeSet::new();
    while let Some(id) = parent {
        if !seen.insert(id) {
            break;
        }
        if let Some(table) = styles.get(&id) {
            if matches!(
                style_value(table, "display", "block"),
                "table" | "inline-table"
            ) {
                let mut used = style.as_ref().clone();
                for property in ["writing-mode", "direction"] {
                    used.insert(
                        property.into(),
                        style_value(
                            table,
                            property,
                            if property == "direction" {
                                "ltr"
                            } else {
                                "horizontal-tb"
                            },
                        )
                        .into(),
                    );
                }
                return Some(used);
            }
        }
        parent = nodes.get(&id).and_then(|node| node.parent);
    }
    None
}

pub(super) fn containing_axes(
    id: u64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> writing_mode::Axes {
    let mut current = Some(id);
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            break;
        }
        if let Some(style) = styles.get(&id) {
            if matches!(
                style_value(style, "display", "block"),
                "table" | "inline-table"
            ) {
                return writing_mode::Axes::new(
                    style_value(style, "writing-mode", "horizontal-tb"),
                    style_value(style, "direction", "ltr") == "rtl",
                );
            }
        }
        current = nodes.get(&id).and_then(|node| node.parent);
    }
    writing_mode::Axes::new("horizontal-tb", false)
}

fn logical_edges(edge: EdgeSizes, axes: writing_mode::Axes) -> EdgeSizes {
    if !axes.vertical {
        return edge;
    }
    EdgeSizes {
        top: if axes.x_reverse {
            edge.right
        } else {
            edge.left
        },
        bottom: if axes.x_reverse {
            edge.left
        } else {
            edge.right
        },
        left: edge.top,
        right: edge.bottom,
    }
}
fn physical_side(side: usize, axes: writing_mode::Axes) -> usize {
    if !axes.vertical {
        side
    } else if axes.x_reverse {
        [1, 2, 3, 0][side]
    } else {
        [3, 2, 1, 0][side]
    }
}
fn grid_rect(rect: CssRect, axes: writing_mode::Axes) -> CssRect {
    if !axes.vertical {
        rect
    } else {
        CssRect {
            x: rect.y,
            y: if axes.x_reverse {
                -rect.right()
            } else {
                rect.x
            },
            width: rect.height,
            height: rect.width,
        }
    }
}
fn physical_rect(rect: CssRect, axes: writing_mode::Axes) -> CssRect {
    if !axes.vertical {
        rect
    } else {
        CssRect {
            x: if axes.x_reverse {
                -rect.bottom()
            } else {
                rect.y
            },
            y: rect.x,
            width: rect.height,
            height: rect.width,
        }
    }
}

// First in-flow native line baseline, measured with the same shaping engine
// and used inline size as cell layout. Empty cells fall back to their content
// bottom in the caller, per the table baseline rule.
fn first_baseline(
    node: &LayoutNode,
    width: f64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    viewport: ViewportState,
    engine: &mut TextEngine,
    seen: &mut BTreeSet<u64>,
) -> Option<f64> {
    if !seen.insert(node.id) {
        return None;
    }
    let style = styles.get(&node.id)?;
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let mut preceding = 0.;
    for item in inline_formatting::collect(node, nodes, styles) {
        let id = match item {
            inline_formatting::FlowItem::Paragraph(paragraph) => {
                let shaped = inline_formatting::measure_paragraph(
                    paragraph, style, width, nodes, styles, viewport, engine,
                );
                if let (Some(line), Some(baseline)) =
                    (shaped.lines.first(), shaped.baselines.first())
                {
                    return Some(preceding + line.3 + baseline);
                }
                preceding += shaped.height;
                continue;
            }
            inline_formatting::FlowItem::Block(id) => id,
        };
        let Some(child) = nodes.get(&id) else {
            continue;
        };
        let Some(style) = styles.get(&id) else {
            continue;
        };
        if style_value(style, "display", "block") == "none"
            || matches!(
                style_value(style, "position", "static"),
                "absolute" | "fixed"
            )
        {
            continue;
        }
        let font = length(
            style_value(style, "font-size", "16px"),
            font,
            font,
            viewport,
        );
        let padding = edge(style, "padding", width, font, viewport);
        let border = border_edge(style, width, font, viewport);
        let margin = edge(style, "margin", width, font, viewport);
        let extra = padding.left + padding.right + border.left + border.right;
        let child_width =
            optional_length(style_value(style, "width", "auto"), width, font, viewport)
                .map(|v| {
                    v - if style_value(style, "box-sizing", "content-box") == "border-box" {
                        extra
                    } else {
                        0.
                    }
                })
                .unwrap_or(width - extra - margin.left - margin.right)
                .max(0.);
        if let Some(baseline) =
            first_baseline(child, child_width, nodes, styles, viewport, engine, seen)
        {
            return Some(preceding + margin.top + padding.top + border.top + baseline);
        }
        preceding += engine.measure_content_height(child, child_width, nodes, styles, viewport)
            + margin.top
            + margin.bottom
            + padding.top
            + padding.bottom
            + border.top
            + border.bottom;
    }
    None
}

pub(super) fn prepare(
    table: &LayoutNode,
    width: f64,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    viewport: ViewportState,
    engine: &mut TextEngine,
) {
    let table_style = &styles[&table.id];
    let axes = writing_mode::Axes::new(
        style_value(table_style, "writing-mode", "horizontal-tb"),
        style_value(table_style, "direction", "ltr") == "rtl",
    );
    let inline_property = if axes.vertical { "height" } else { "width" };
    let block_property = if axes.vertical { "width" } else { "height" };
    let block_basis = if axes.vertical {
        viewport.width
    } else {
        viewport.height
    };
    let rows = rows(table, nodes, styles);
    if rows.is_empty() {
        return;
    }
    let mut column_owners = columns(table, nodes, styles);
    let mut cells = Vec::new();
    let mut occupied = Vec::<usize>::new();
    let mut columns = column_owners.len();
    for (row_index, row_id) in rows.iter().enumerate() {
        let row = nodes[row_id];
        let mut column = 0;
        let group = row.parent;
        let group_end = rows
            .iter()
            .enumerate()
            .skip(row_index)
            .take_while(|(_, id)| nodes[*id].parent == group)
            .last()
            .map(|(index, _)| index + 1)
            .unwrap_or(row_index + 1);
        for id in &row.children {
            let Some(node) = nodes.get(id) else {
                continue;
            };
            if !styles
                .get(id)
                .is_some_and(|s| style_value(s, "display", "block") == "table-cell")
            {
                continue;
            }
            while column < occupied.len() && occupied[column] > row_index {
                column += 1;
            }
            if column >= MAX_COLUMNS {
                break;
            }
            let requested = span(node, "colspan", MAX_COLUMNS);
            while column < MAX_COLUMNS
                && occupied
                    .iter()
                    .enumerate()
                    .skip(column)
                    .take(requested)
                    .any(|(_, end)| *end > row_index)
            {
                column += 1;
            }
            if column >= MAX_COLUMNS {
                break;
            }
            let colspan = requested.min(MAX_COLUMNS - column);
            let rowspan = if node.attributes.get("rowspan").is_some_and(|s| s == "0") {
                group_end - row_index
            } else {
                span(node, "rowspan", 65534).min(group_end - row_index)
            };
            occupied.resize(occupied.len().max(column + colspan), 0);
            for end in &mut occupied[column..column + colspan] {
                *end = row_index + rowspan;
            }
            cells.push(Cell {
                id: *id,
                row: row_index,
                column,
                columns: colspan,
                rows: rowspan,
            });
            column += colspan;
            columns = columns.max(column);
        }
    }
    if columns == 0 {
        return;
    }
    let table_style = &styles[&table.id];
    let font = length(
        style_value(table_style, "font-size", "16px"),
        16.,
        16.,
        viewport,
    );
    if if axes.vertical {
        axes.y_reverse
    } else {
        axes.x_reverse
    } {
        for cell in &mut cells {
            cell.column = columns - cell.column - cell.columns;
        }
        column_owners.resize(columns, (None, None));
        column_owners.reverse();
    }
    let collapsed = style_value(table_style, "border-collapse", "separate") == "collapse";
    let plan = CollapsedTable {
        table: table.id,
        rows: rows.clone(),
        cells: cells.clone(),
        columns: column_owners.clone(),
        column_offsets: Vec::new(),
        column_gap: 0.,
        axes,
        collapsed,
    };
    if collapsed {
        engine
            .table_borders
            .extend(used_borders(&plan, nodes, styles));
    }
    engine
        .collapsed_tables
        .retain(|existing| existing.table != table.id);
    engine.collapsed_tables.push(plan);
    let spacing = style_value(table_style, "border-spacing", "0px")
        .split_whitespace()
        .collect::<Vec<_>>();
    let gap_x = if collapsed {
        0.
    } else {
        length(
            if axes.vertical {
                spacing.get(1).or(spacing.first())
            } else {
                spacing.first()
            }
            .copied()
            .unwrap_or("0px"),
            width,
            font,
            viewport,
        )
    };
    let gap_y = if collapsed {
        0.
    } else {
        length(
            if axes.vertical {
                spacing.first()
            } else {
                spacing.get(1).or(spacing.first())
            }
            .copied()
            .unwrap_or("0px"),
            width,
            font,
            viewport,
        )
    };
    let mut minimum = vec![0_f64; columns];
    let mut maximum = minimum.clone();
    let mut fixed_columns = vec![false; columns];
    let fixed = style_value(table_style, "table-layout", "auto") == "fixed"
        && style_value(table_style, inline_property, "auto") != "auto";
    for (index, (group, column)) in column_owners.iter().enumerate() {
        let specified = column
            .and_then(|id| {
                optional_length(
                    style_value(&styles[&id], inline_property, "auto"),
                    width,
                    font,
                    viewport,
                )
            })
            .or_else(|| {
                group.and_then(|id| {
                    optional_length(
                        style_value(&styles[&id], inline_property, "auto"),
                        width,
                        font,
                        viewport,
                    )
                })
            });
        if let Some(value) = specified {
            minimum[index] = value;
            maximum[index] = value;
            fixed_columns[index] = true;
        }
    }
    let mut constraints = Vec::new();
    for cell in &cells {
        let node = nodes[&cell.id];
        let style = &styles[&cell.id];
        let font = length(
            style_value(style, "font-size", "16px"),
            font,
            font,
            viewport,
        );
        let padding = logical_edges(edge(style, "padding", width, font, viewport), axes);
        let border = logical_edges(
            engine
                .table_borders
                .get(&cell.id)
                .copied()
                .unwrap_or_else(|| border_edge(style, width, font, viewport)),
            axes,
        );
        let extra = padding.left + padding.right + border.left + border.right;
        let cell_vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let intrinsic = if axes.vertical {
            if let Some((_, height)) = node
                .intrinsic_image_size
                .or(node.document_replaced.then_some((300, 150)))
            {
                (f64::from(height), f64::from(height))
            } else if cell_vertical {
                let source =
                    text_source::TextSource::collect_visible(node, nodes, true, Some(styles));
                engine.intrinsic_source(&source, font, style, viewport, styles)
            } else {
                let preferred = optional_length(
                    style_value(style, "width", "auto"),
                    viewport.width,
                    font,
                    viewport,
                )
                .unwrap_or_else(|| {
                    engine
                        .intrinsic_node(node, nodes, styles, viewport, &mut BTreeSet::new())
                        .1
                        .min(viewport.width)
                });
                let height =
                    engine.measure_content_height(node, preferred.max(1.), nodes, styles, viewport);
                (height, height)
            }
        } else {
            engine.intrinsic_node(node, nodes, styles, viewport, &mut BTreeSet::new())
        };
        let specified = optional_length(
            style_value(style, inline_property, "auto"),
            width,
            font,
            viewport,
        )
        .map(|v| {
            v + if style_value(style, "box-sizing", "content-box") == "border-box" {
                0.
            } else {
                extra
            }
        });
        let (min, max) = if fixed {
            if cell.row == 0 {
                specified.map(|v| (v, v)).unwrap_or((0., 0.))
            } else {
                (0., 0.)
            }
        } else {
            (
                intrinsic.0 + extra,
                intrinsic.1.max(specified.unwrap_or(0.) - extra) + extra,
            )
        };
        constraints.push((
            *cell,
            if fixed {
                min
            } else {
                min.max(specified.unwrap_or(0.))
            },
            max.max(min),
            cell.row == 0 && specified.is_some(),
        ));
    }
    constraints.sort_by_key(|(cell, _, _, _)| cell.columns);
    for (cell, min, max, definite) in constraints {
        let slice = cell.column..cell.column + cell.columns;
        if fixed {
            if !definite {
                continue;
            }
            let undefined = slice
                .clone()
                .filter(|index| !fixed_columns[*index])
                .collect::<Vec<_>>();
            let extra = (min
                - gap_x * (cell.columns - 1) as f64
                - minimum[slice.clone()].iter().sum::<f64>())
            .max(0.);
            if !undefined.is_empty() {
                for index in &undefined {
                    minimum[*index] = extra / undefined.len() as f64;
                    maximum[*index] = minimum[*index];
                    fixed_columns[*index] = true;
                }
            }
            continue;
        }
        let min_deficit =
            (min - gap_x * (cell.columns - 1) as f64 - minimum[slice.clone()].iter().sum::<f64>())
                .max(0.);
        let max_deficit =
            (max - gap_x * (cell.columns - 1) as f64 - maximum[slice.clone()].iter().sum::<f64>())
                .max(0.);
        for index in slice {
            minimum[index] += min_deficit / cell.columns as f64;
            maximum[index] =
                (maximum[index] + max_deficit / cell.columns as f64).max(minimum[index]);
        }
    }
    let min = minimum.iter().sum::<f64>();
    let max = maximum.iter().sum::<f64>();
    let spacing = gap_x * (columns + 1) as f64;
    let mut caption_min = 0_f64;
    for id in &table.children {
        let Some(style) = styles
            .get(id)
            .filter(|s| style_value(s, "display", "block") == "table-caption")
        else {
            continue;
        };
        let font = length(
            style_value(style, "font-size", "16px"),
            font,
            font,
            viewport,
        );
        let padding = logical_edges(edge(style, "padding", width, font, viewport), axes);
        let border = logical_edges(border_edge(style, width, font, viewport), axes);
        let margin = logical_edges(edge(style, "margin", width, font, viewport), axes);
        let intrinsic = if axes.vertical {
            let source =
                text_source::TextSource::collect_visible(nodes[id], nodes, true, Some(styles));
            engine
                .intrinsic_source(&source, font, style, viewport, styles)
                .0
        } else {
            engine
                .intrinsic_node(nodes[id], nodes, styles, viewport, &mut BTreeSet::new())
                .0
        };
        let edges = padding.left + padding.right + border.left + border.right;
        let specified = optional_length(
            style_value(style, inline_property, "auto"),
            width,
            font,
            viewport,
        )
        .map(|value| {
            value
                + if style_value(style, "box-sizing", "content-box") == "border-box" {
                    0.
                } else {
                    edges
                }
        })
        .unwrap_or(0.);
        caption_min =
            caption_min.max((intrinsic + edges).max(specified) + margin.left + margin.right);
    }
    let requested_min = optional_length(
        style_value(table_style, &format!("min-{inline_property}"), "auto"),
        width,
        font,
        viewport,
    )
    .unwrap_or(0.);
    let outer = if style_value(table_style, inline_property, "auto") == "auto" {
        width
            .min(max + spacing)
            .max(min + spacing)
            .max(caption_min)
            .max(requested_min)
    } else {
        width.max(caption_min).max(requested_min)
    };
    let available = (outer - spacing).max(0.);
    let unknown = fixed_columns.iter().filter(|defined| !**defined).count();
    let widths = (0..columns)
        .map(|i| {
            if fixed && unknown > 0 {
                return if !fixed_columns[i] {
                    (available - min).max(0.) / unknown as f64
                } else {
                    minimum[i]
                };
            }
            if available <= min {
                minimum[i]
            } else if available < max {
                minimum[i] + (available - min) * (maximum[i] - minimum[i]) / (max - min)
            } else {
                maximum[i]
                    + (available - max)
                        * if max > 0. {
                            maximum[i] / max
                        } else {
                            1. / columns as f64
                        }
            }
        })
        .collect::<Vec<_>>();
    if let Some(plan) = engine
        .collapsed_tables
        .iter_mut()
        .find(|plan| plan.table == table.id)
    {
        let mut position = gap_x;
        plan.column_gap = gap_x;
        plan.column_offsets = vec![position];
        for width in &widths {
            position += *width + gap_x;
            plan.column_offsets.push(position);
        }
    }
    let mut heights = rows
        .iter()
        .map(|id| {
            if engine.consumed_table_rows.contains(id) {
                0.
            } else {
                optional_length(
                    style_value(&styles[id], block_property, "auto"),
                    block_basis,
                    font,
                    viewport,
                )
                .unwrap_or(0.)
            }
        })
        .collect::<Vec<_>>();
    let mut height_constraints = Vec::new();
    let mut baselines = Vec::new();
    for cell in &cells {
        let style = &styles[&cell.id];
        let font = length(
            style_value(style, "font-size", "16px"),
            font,
            font,
            viewport,
        );
        let width = widths[cell.column..cell.column + cell.columns]
            .iter()
            .sum::<f64>()
            + gap_x * (cell.columns - 1) as f64;
        let padding = logical_edges(edge(style, "padding", width, font, viewport), axes);
        let border = logical_edges(
            engine
                .table_borders
                .get(&cell.id)
                .copied()
                .unwrap_or_else(|| border_edge(style, width, font, viewport)),
            axes,
        );
        let vertical = padding.top + padding.bottom + border.top + border.bottom;
        let available = (width - padding.left - padding.right - border.left - border.right).max(0.);
        let cell_vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        let content = match (axes.vertical, cell_vertical) {
            (true, true) => engine.vertical_block_extent(
                nodes[&cell.id],
                available,
                nodes,
                styles,
                viewport,
                &mut BTreeSet::new(),
            ),
            (false, false) => {
                engine.measure_content_height(nodes[&cell.id], available, nodes, styles, viewport)
            }
            (true, false) => optional_length(
                style_value(style, "width", "auto"),
                viewport.width,
                font,
                viewport,
            )
            .unwrap_or_else(|| {
                engine
                    .intrinsic_node(
                        nodes[&cell.id],
                        nodes,
                        styles,
                        viewport,
                        &mut BTreeSet::new(),
                    )
                    .1
                    .min(viewport.width)
            }),
            (false, true) => optional_length(
                style_value(style, "height", "auto"),
                viewport.height,
                font,
                viewport,
            )
            .unwrap_or(viewport.height),
        };
        let specified = optional_length(
            style_value(style, block_property, "auto"),
            block_basis,
            font,
            viewport,
        )
        .map(|v| {
            v + if style_value(style, "box-sizing", "content-box") == "border-box" {
                0.
            } else {
                vertical
            }
        })
        .unwrap_or(0.);
        let natural_height = (content + vertical).max(specified);
        height_constraints.push((*cell, natural_height));
        if style_value(style, "vertical-align", "baseline") == "baseline" {
            let baseline = border.top
                + padding.top
                + if cell_vertical == axes.vertical {
                    first_baseline(
                        nodes[&cell.id],
                        available,
                        nodes,
                        styles,
                        viewport,
                        engine,
                        &mut BTreeSet::new(),
                    )
                    .unwrap_or(content)
                } else {
                    content
                };
            baselines.push((*cell, baseline, natural_height));
        }
    }
    let mut row_baselines = vec![0_f64; rows.len()];
    for (cell, baseline, _) in &baselines {
        row_baselines[cell.row] = row_baselines[cell.row].max(*baseline);
    }
    for (cell, baseline, height) in baselines {
        let shift = (row_baselines[cell.row] - baseline).max(0.);
        engine.table_baseline_offsets.insert(cell.id, shift);
        // Baseline displacement participates in row height, rather than
        // clipping a taller descender or changing only the paint position.
        if let Some((_, required)) = height_constraints.iter_mut().find(|(c, _)| c.id == cell.id) {
            *required = height + shift;
        }
    }
    height_constraints.sort_by_key(|(cell, _)| cell.rows);
    for (cell, height) in height_constraints {
        let slice = cell.row..cell.row + cell.rows;
        let active = slice
            .clone()
            .filter(|index| !engine.consumed_table_rows.contains(&rows[*index]))
            .collect::<Vec<_>>();
        let deficit = (height
            - gap_y * active.len().saturating_sub(1) as f64
            - heights[slice].iter().sum::<f64>())
        .max(0.);
        if !active.is_empty() {
            for index in &active {
                heights[*index] += deficit / active.len() as f64;
            }
        }
    }
    for (index, row) in rows.iter().enumerate() {
        engine.table_rows.insert(
            *row,
            RowPlan {
                cells: BTreeMap::new(),
                height: heights[index]
                    + if engine.consumed_table_rows.contains(row) {
                        0.
                    } else {
                        gap_y
                    },
                vertical: axes.vertical,
                inline_size: widths.iter().sum::<f64>() + gap_x * (columns + 1) as f64,
            },
        );
    }
    for cell in cells {
        let x = gap_x * (cell.column + 1) as f64 + widths[..cell.column].iter().sum::<f64>();
        let width = widths[cell.column..cell.column + cell.columns]
            .iter()
            .sum::<f64>()
            + gap_x * (cell.columns - 1) as f64;
        let active = rows[cell.row..cell.row + cell.rows]
            .iter()
            .filter(|id| !engine.consumed_table_rows.contains(*id))
            .count();
        let height = heights[cell.row..cell.row + cell.rows].iter().sum::<f64>()
            + gap_y * active.saturating_sub(1) as f64;
        let placement = if axes.vertical {
            ChildPlacement {
                x: if axes.x_reverse {
                    heights[cell.row] - height
                } else {
                    gap_y
                },
                y: x,
                width: height,
                height: width,
            }
        } else {
            ChildPlacement {
                x,
                y: gap_y,
                width,
                height,
            }
        };
        engine
            .table_rows
            .get_mut(&rows[cell.row])
            .unwrap()
            .cells
            .insert(cell.id, placement);
    }
    let active_rows = rows
        .iter()
        .filter(|row| !engine.consumed_table_rows.contains(row))
        .count();
    engine
        .table_end_spacing
        .insert(table.id, if active_rows == 0 { 0. } else { gap_y });
    engine.table_block_sizes.insert(
        table.id,
        heights.iter().sum::<f64>()
            + if active_rows == 0 {
                0.
            } else {
                gap_y * (active_rows + 1) as f64
            },
    );
    engine.table_widths.insert(
        table.id,
        widths.iter().sum::<f64>() + gap_x * (columns + 1) as f64,
    );
}

#[derive(Clone)]
pub(super) struct CollapsedTable {
    table: u64,
    rows: Vec<u64>,
    cells: Vec<Cell>,
    columns: Vec<(Option<u64>, Option<u64>)>,
    column_offsets: Vec<f64>,
    column_gap: f64,
    axes: writing_mode::Axes,
    collapsed: bool,
}
#[derive(Clone)]
struct Candidate {
    edge: oryn_common::v4::PaintBorderEdge,
    origin: u8,
    order: usize,
}
fn candidate(style: &BTreeMap<String, String>, side: usize, origin: u8, order: usize) -> Candidate {
    let side = ["top", "right", "bottom", "left"][side];
    let kind = style_value(style, &format!("border-{side}-style"), "none");
    let width = match style_value(style, &format!("border-{side}-width"), "medium") {
        "thin" => 1.,
        "medium" => 3.,
        "thick" => 5.,
        value => length(value, 0., 16., ViewportState::default()),
    };
    let color = style_value(style, &format!("border-{side}-color"), "currentcolor");
    Candidate {
        edge: oryn_common::v4::PaintBorderEdge {
            width: if kind == "none" { 0. } else { width },
            style: kind.into(),
            color: if color == "currentcolor" {
                style_value(style, "color", "black").into()
            } else {
                color.into()
            },
        },
        origin,
        order,
    }
}
fn style_rank(style: &str) -> u8 {
    match style {
        "hidden" => 10,
        "double" => 9,
        "solid" => 8,
        "dashed" => 7,
        "dotted" => 6,
        "ridge" => 5,
        "outset" => 4,
        "groove" => 3,
        "inset" => 2,
        _ => 0,
    }
}
fn winner(a: Candidate, b: Candidate) -> Candidate {
    let rank = |v: &Candidate| {
        (
            v.edge.style == "hidden",
            v.edge.style != "none",
            v.origin,
            usize::MAX - v.order,
        )
    };
    let compare = (a.edge.style == "hidden")
        .cmp(&(b.edge.style == "hidden"))
        .then((a.edge.style != "none").cmp(&(b.edge.style != "none")))
        .then(a.edge.width.total_cmp(&b.edge.width))
        .then(style_rank(&a.edge.style).cmp(&style_rank(&b.edge.style)))
        .then(rank(&a).cmp(&rank(&b)));
    if compare.is_ge() { a } else { b }
}
fn adjacent(a: Cell, b: Cell, side: usize) -> bool {
    match side {
        0 => {
            b.row + b.rows == a.row
                && b.column < a.column + a.columns
                && b.column + b.columns > a.column
        }
        1 => a.column + a.columns == b.column && b.row < a.row + a.rows && b.row + b.rows > a.row,
        2 => {
            a.row + a.rows == b.row
                && b.column < a.column + a.columns
                && b.column + b.columns > a.column
        }
        _ => b.column + b.columns == a.column && b.row < a.row + a.rows && b.row + b.rows > a.row,
    }
}
fn own_candidate(
    table: &CollapsedTable,
    cell: Cell,
    side: usize,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> Candidate {
    own_candidate_at(table, cell, side, nodes, styles, None)
}
fn own_candidate_at(
    table: &CollapsedTable,
    cell: Cell,
    side: usize,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    segment_column: Option<usize>,
) -> Candidate {
    let columns = table
        .cells
        .iter()
        .map(|c| c.column + c.columns)
        .max()
        .unwrap_or(0);
    let rtl = style_value(&styles[&table.table], "direction", "ltr") == "rtl";
    let tie = cell.row * MAX_COLUMNS
        + if rtl {
            MAX_COLUMNS - cell.column - cell.columns
        } else {
            cell.column
        };
    let outside = match side {
        0 => cell.row == 0,
        1 => cell.column + cell.columns == columns,
        2 => cell.row + cell.rows == table.rows.len(),
        _ => cell.column == 0,
    };
    let mut value = candidate(&styles[&cell.id], physical_side(side, table.axes), 5, tie);
    let row = table.rows[if side == 2 {
        cell.row + cell.rows - 1
    } else {
        cell.row
    }];
    // A row's left/right border belongs to the perimeter of the whole row,
    // never to every interior cell edge. Columns have the transposed rule.
    if side % 2 == 0 || outside {
        value = winner(
            value,
            candidate(&styles[&row], physical_side(side, table.axes), 4, tie),
        );
    }
    if let Some(group) = nodes[&row].parent.filter(|id| *id != table.table) {
        let group_rows = table
            .rows
            .iter()
            .enumerate()
            .filter(|(_, id)| nodes[*id].parent == Some(group))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if side % 2 == 1 && outside
            || side == 0 && group_rows.first() == Some(&cell.row)
            || side == 2 && group_rows.last() == Some(&(cell.row + cell.rows - 1))
        {
            value = winner(
                value,
                candidate(&styles[&group], physical_side(side, table.axes), 3, tie),
            );
        }
    }
    for index in cell.column..cell.column + cell.columns {
        if side % 2 == 0 && segment_column.is_some_and(|column| column != index) {
            continue;
        }
        if let Some((group, column)) = table.columns.get(index) {
            let touches = match side {
                0 | 2 => outside,
                1 => index + 1 == cell.column + cell.columns,
                _ => index == cell.column,
            };
            if touches {
                if let Some(column) = column {
                    value = winner(
                        value,
                        candidate(&styles[column], physical_side(side, table.axes), 2, tie),
                    );
                }
                if let Some(group) = group {
                    let group_start = table
                        .columns
                        .iter()
                        .position(|(owner, _)| owner == &Some(*group))
                        .unwrap_or(index);
                    let group_end = table
                        .columns
                        .iter()
                        .rposition(|(owner, _)| owner == &Some(*group))
                        .unwrap_or(index)
                        + 1;
                    if side % 2 == 0
                        || side == 1 && index + 1 == group_end
                        || side == 3 && index == group_start
                    {
                        value = winner(
                            value,
                            candidate(&styles[group], physical_side(side, table.axes), 1, tie),
                        );
                    }
                }
            }
        }
    }
    if outside {
        value = winner(
            value,
            candidate(
                &styles[&table.table],
                physical_side(side, table.axes),
                0,
                tie,
            ),
        );
    }
    value
}
fn used_borders(
    table: &CollapsedTable,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> BTreeMap<u64, EdgeSizes> {
    table
        .cells
        .iter()
        .map(|cell| {
            let values: [f64; 4] = std::array::from_fn(|side| {
                let mut value = own_candidate(table, *cell, side, nodes, styles);
                for other in &table.cells {
                    if adjacent(*cell, *other, side) {
                        value = winner(
                            value,
                            own_candidate(table, *other, (side + 2) % 4, nodes, styles),
                        );
                    }
                }
                if matches!(value.edge.style.as_str(), "none" | "hidden") {
                    0.
                } else {
                    value.edge.width / 2.
                }
            });
            let mut physical = [0.; 4];
            for side in 0..4 {
                physical[physical_side(side, table.axes)] = values[side];
            }
            (
                cell.id,
                EdgeSizes {
                    top: physical[0],
                    right: physical[1],
                    bottom: physical[2],
                    left: physical[3],
                },
            )
        })
        .collect()
}
/// Split a rowspan/colspan edge at every neighbor endpoint. Each shared edge
/// paints once (from its upper/left cell), using the winning declaration for
/// that segment rather than promoting a winner over the whole spanning side.
pub(super) fn project_borders(
    tables: &[CollapsedTable],
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    projections: &mut BTreeMap<u64, oryn_common::v4::PaintProjection>,
) {
    use oryn_common::v4::{PaintBorderEdge, PaintBorderSegment};
    for table in tables.iter().filter(|table| table.collapsed) {
        let boxes = table
            .cells
            .iter()
            .filter_map(|c| {
                projections
                    .get(&c.id)
                    .map(|p| (c.id, grid_rect(p.box_model.border_box, table.axes)))
            })
            .collect::<BTreeMap<_, _>>();
        for id in std::iter::once(table.table)
            .chain(table.rows.iter().copied())
            .chain(table.rows.iter().filter_map(|id| nodes[id].parent))
            .chain(table.cells.iter().map(|c| c.id))
        {
            if let Some(projection) = projections.get_mut(&id) {
                projection.border_edges = Some(std::array::from_fn(|_| PaintBorderEdge {
                    width: 0.,
                    color: "transparent".into(),
                    style: "hidden".into(),
                }));
            }
        }
        for cell in &table.cells {
            let Some(rect) = boxes.get(&cell.id).copied() else {
                continue;
            };
            let mut segments = Vec::new();
            for side in 0..4 {
                let horizontal = side % 2 == 0;
                let (start, end) = if horizontal {
                    (rect.x, rect.right())
                } else {
                    (rect.y, rect.bottom())
                };
                let neighbors = table
                    .cells
                    .iter()
                    .filter(|other| adjacent(*cell, **other, side))
                    .filter_map(|other| boxes.get(&other.id).map(|rect| (*other, *rect)))
                    .collect::<Vec<_>>();
                let mut breaks = vec![start, end];
                let origin = rect.x - table.column_offsets.get(cell.column).copied().unwrap_or(0.);
                if horizontal {
                    for offset in table
                        .column_offsets
                        .iter()
                        .skip(cell.column + 1)
                        .take(cell.columns.saturating_sub(1))
                    {
                        breaks.push((origin + offset).clamp(start, end));
                    }
                }
                for (_, rect) in &neighbors {
                    let (a, b) = if horizontal {
                        (rect.x, rect.right())
                    } else {
                        (rect.y, rect.bottom())
                    };
                    breaks.push(a.clamp(start, end));
                    breaks.push(b.clamp(start, end));
                }
                breaks.sort_by(f64::total_cmp);
                breaks.dedup_by(|a, b| (*a - *b).abs() < 0.001);
                for pair in breaks.windows(2) {
                    let middle = (pair[0] + pair[1]) / 2.;
                    let covering = neighbors
                        .iter()
                        .filter(|(_, r)| {
                            if horizontal {
                                middle >= r.x && middle < r.right()
                            } else {
                                middle >= r.y && middle < r.bottom()
                            }
                        })
                        .collect::<Vec<_>>();
                    if (side == 0 || side == 3) && !covering.is_empty() {
                        continue;
                    }
                    let column = horizontal
                        .then(|| {
                            table.column_offsets.windows(2).position(|pair| {
                                middle >= origin + pair[0] && middle < origin + pair[1]
                            })
                        })
                        .flatten();
                    let mut value = own_candidate_at(table, *cell, side, nodes, styles, column);
                    for (other, _) in covering {
                        value = winner(
                            value,
                            own_candidate_at(table, *other, (side + 2) % 4, nodes, styles, column),
                        );
                    }
                    if matches!(value.edge.style.as_str(), "none" | "hidden")
                        || value.edge.width <= 0.
                    {
                        continue;
                    }
                    let center = match side {
                        0 => rect.y,
                        1 => rect.right(),
                        2 => rect.bottom(),
                        _ => rect.x,
                    };
                    let segment = if horizontal {
                        CssRect {
                            x: pair[0],
                            y: center - value.edge.width / 2.,
                            width: pair[1] - pair[0],
                            height: value.edge.width,
                        }
                    } else {
                        CssRect {
                            x: center - value.edge.width / 2.,
                            y: pair[0],
                            width: value.edge.width,
                            height: pair[1] - pair[0],
                        }
                    };
                    segments.push(PaintBorderSegment {
                        rect: physical_rect(segment, table.axes),
                        edge: value.edge,
                        horizontal: horizontal != table.axes.vertical,
                    });
                }
            }
            if let Some(projection) = projections.get_mut(&cell.id) {
                projection.border_segments.extend(segments);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collapsed_border_conflicts_apply_hidden_width_style_and_origin_precedence() {
        let make = |width, kind: &str, origin: u8| Candidate {
            edge: oryn_common::v4::PaintBorderEdge {
                width,
                style: kind.into(),
                color: origin.to_string(),
            },
            origin,
            order: 0,
        };
        assert_eq!(
            winner(make(10., "solid", 5), make(1., "hidden", 0))
                .edge
                .style,
            "hidden"
        );
        assert_eq!(
            winner(make(4., "solid", 5), make(6., "dotted", 0))
                .edge
                .width,
            6.
        );
        assert_eq!(
            winner(make(4., "solid", 5), make(4., "double", 0))
                .edge
                .style,
            "double"
        );
        assert_eq!(winner(make(4., "solid", 5), make(4., "solid", 2)).origin, 5);
    }
}

pub(super) fn rowspan_coverage(
    input: &LayoutInput,
    styles: &ComputedStyles,
) -> BTreeMap<u64, Vec<u64>> {
    let nodes = input
        .nodes
        .iter()
        .map(|n| (n.id, n))
        .collect::<BTreeMap<_, _>>();
    let mut coverage = BTreeMap::new();
    for table in input.nodes.iter().filter(|n| {
        styles
            .get(&n.id)
            .is_some_and(|s| matches!(style_value(s, "display", "block"), "table" | "inline-table"))
    }) {
        let rows = rows(table, &nodes, styles);
        for (index, row) in rows.iter().enumerate() {
            let available = rows
                .iter()
                .skip(index)
                .take_while(|id| nodes[*id].parent == nodes[row].parent)
                .copied()
                .collect::<Vec<_>>();
            for id in &nodes[row].children {
                let node = nodes[id];
                let count = if node.attributes.get("rowspan").is_some_and(|v| v == "0") {
                    available.len()
                } else {
                    span(node, "rowspan", 65534).min(available.len())
                };
                if count > 1 {
                    coverage.insert(*id, available[..count].to_vec());
                }
            }
        }
    }
    coverage
}

/// Distribute cell block-axis free space after descendant layout, preserving
/// the allocated cell border box and moving only its content. Pagination and
/// hit-testing consume the same shifted text/child geometry.
pub(super) fn align_contents(
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    engine: &mut TextEngine,
) {
    let cells = engine
        .table_rows
        .values()
        .flat_map(|row| row.cells.keys().copied())
        .collect::<BTreeSet<_>>();
    for id in cells {
        let style = &styles[&id];
        let axes = writing_mode::Axes::new(
            style_value(style, "writing-mode", "horizontal-tb"),
            style_value(style, "direction", "ltr") == "rtl",
        );
        let align = style_value(style, "vertical-align", "baseline");
        if !matches!(align, "top" | "middle" | "bottom" | "baseline") {
            continue;
        }
        let Some(content) = geometry
            .get(&id)
            .and_then(|g| g.box_model.as_ref())
            .map(|b| grid_rect(b.content_box, axes))
        else {
            continue;
        };
        // Cell alignment allocates CSS line-box free space. Native selection
        // bounds may extend beyond a line with negative leading; retain them
        // and translate them with the line instead of using them as its height.
        let mut top = f64::INFINITY;
        let mut bottom = f64::NEG_INFINITY;
        // An inline owner may hold all of the cell's text. Its client box is
        // selection/decorative geometry too, so walk through inline wrappers
        // to their line flows. Independent child boxes contribute their real
        // outer extent and are not counted again through descendants.
        let mut pending = vec![id];
        let mut seen = BTreeSet::new();
        while let Some(owner) = pending.pop() {
            if !seen.insert(owner) {
                continue;
            }
            for fragment in text.get(&owner).into_iter().flatten() {
                let rect = grid_rect(fragment.flow_rect.unwrap_or(fragment.rect), axes);
                top = top.min(rect.y);
                bottom = bottom.max(rect.bottom());
            }
            for child in &nodes[&owner].children {
                let Some(style) = styles.get(child) else {
                    continue;
                };
                if style_value(style, "display", "block") == "none"
                    || matches!(
                        style_value(style, "position", "static"),
                        "absolute" | "fixed"
                    )
                {
                    continue;
                }
                if !is_replaced_box(nodes[child])
                    && matches!(
                        style_value(style, "display", "block"),
                        "inline"
                            | "contents"
                            | "ruby-base"
                            | "ruby-text"
                            | "ruby-base-container"
                            | "ruby-text-container"
                    )
                {
                    pending.push(*child);
                    continue;
                }
                if let Some(model) = geometry.get(child).and_then(|g| g.box_model.as_ref()) {
                    let rect = grid_rect(model.border_box, axes);
                    let margin = logical_edges(model.margin, axes);
                    top = top.min(rect.y - margin.top);
                    bottom = bottom.max(rect.bottom() + margin.bottom);
                }
            }
        }
        if !top.is_finite() {
            top = content.y;
            bottom = top;
        }
        let free = (content.height - (bottom - top)).max(0.);
        let dy = match align {
            "baseline" => engine
                .table_baseline_offsets
                .get(&id)
                .copied()
                .unwrap_or(0.),
            "middle" => content.y + free / 2. - top,
            "bottom" => content.y + free - top,
            _ => content.y - top,
        };
        if dy == 0. {
            continue;
        }
        let (dx, dy) = if axes.vertical {
            (if axes.x_reverse { -dy } else { dy }, 0.)
        } else {
            (0., dy)
        };
        for child in &nodes[&id].children {
            positioning::translate_subtree(*child, dx, dy, nodes, geometry, text, engine);
        }
        if let Some(fragments) = text.get_mut(&id) {
            for fragment in fragments {
                fragment.rect.x += dx;
                fragment.rect.y += dy;
                if let Some(flow) = &mut fragment.flow_rect {
                    flow.x += dx;
                    flow.y += dy;
                }
                for cluster in &mut fragment.clusters {
                    cluster.rect.x += dx;
                    cluster.rect.y += dy;
                }
            }
        }
        for clusters in engine.source_geometry.values_mut() {
            for cluster in clusters {
                if cluster.layout_node == id {
                    cluster.rect.x += dx;
                    cluster.rect.y += dy;
                }
            }
        }
        for line_break in engine.break_boxes.values_mut() {
            if line_break.layout_node == id {
                line_break.before.x += dx;
                line_break.before.y += dy;
                if let Some(after) = &mut line_break.after {
                    after.x += dx;
                    after.y += dy;
                }
            }
        }
    }
}

/// Columns are grid tracks, not flow children. Their backgrounds and CSSOM
/// rectangles cover the resolved track through all table rows, in the same
/// physical coordinate system as the cell grid.
pub(super) fn column_geometry(
    plans: &[CollapsedTable],
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    offsets: &mut BTreeMap<u64, (f64, f64)>,
) {
    for plan in plans {
        let cells = plan
            .cells
            .iter()
            .filter_map(|cell| {
                geometry
                    .get(&cell.id)
                    .and_then(|g| g.box_model.as_ref())
                    .map(|model| (*cell, grid_rect(model.border_box, plan.axes)))
            })
            .collect::<Vec<_>>();
        let Some((first, rect)) = cells.first() else {
            continue;
        };
        let origin = rect.x - plan.column_offsets.get(first.column).copied().unwrap_or(0.);
        let top = cells
            .iter()
            .map(|(_, rect)| rect.y)
            .fold(f64::INFINITY, f64::min);
        let bottom = cells
            .iter()
            .map(|(_, rect)| rect.bottom())
            .fold(f64::NEG_INFINITY, f64::max);
        let mut boxes = BTreeMap::<u64, CssRect>::new();
        for (index, (group, column)) in plan.columns.iter().enumerate() {
            let Some((&start, &end)) = plan
                .column_offsets
                .get(index)
                .zip(plan.column_offsets.get(index + 1))
            else {
                continue;
            };
            let rect = CssRect {
                x: origin + start,
                y: top,
                width: (end - start - plan.column_gap).max(0.),
                height: (bottom - top).max(0.),
            };
            for id in group.iter().chain(column.iter()) {
                boxes
                    .entry(*id)
                    .and_modify(|existing| {
                        let right = existing.right().max(rect.right());
                        existing.x = existing.x.min(rect.x);
                        existing.width = right - existing.x;
                    })
                    .or_insert(rect);
            }
        }
        for (id, rect) in boxes {
            let rect = physical_rect(rect, plan.axes);
            offsets.insert(id, (rect.width, rect.height));
            geometry.insert(
                id,
                NodeGeometry {
                    availability: GeometryAvailability::Available,
                    box_model: Some(BoxModel {
                        border_box: rect,
                        padding_box: rect,
                        content_box: rect,
                        margin: EdgeSizes::default(),
                        padding: EdgeSizes::default(),
                        border: EdgeSizes::default(),
                    }),
                    client_rects: vec![rect],
                    in_viewport: true,
                    interactable: false,
                    diagnostic: None,
                },
            );
        }
    }
}
