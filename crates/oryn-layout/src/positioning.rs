//! Resolve out-of-flow insets after ancestor used box heights are available.
//! This pass changes placement only; intrinsic/percentage sizing belongs to the
//! containing-block-aware formatting pass, not a post-layout rescale.
use super::*;

pub(super) fn establishes(style: &BTreeMap<String, String>, fixed: bool) -> bool {
    (!fixed && style_value(style, "position", "static") != "static")
        || [
            "transform",
            "translate",
            "rotate",
            "scale",
            "perspective",
            "filter",
            "backdrop-filter",
        ]
        .iter()
        .any(|key| style_value(style, key, "none") != "none")
        || style_value(style, "contain", "none")
            .split_whitespace()
            .any(|v| matches!(v, "layout" | "paint" | "strict" | "content"))
        || style_value(style, "will-change", "auto")
            .split(',')
            .any(|v| matches!(v.trim(), "transform" | "perspective" | "filter"))
}

pub(super) fn viewport_fixed(
    id: u64,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
) -> bool {
    if !styles
        .get(&id)
        .is_some_and(|s| style_value(s, "position", "static") == "fixed")
    {
        return false;
    }
    let mut ancestor = by_id.get(&id).and_then(|n| n.parent);
    while let Some(id) = ancestor {
        if styles.get(&id).is_some_and(|s| establishes(s, true)) {
            return false;
        }
        ancestor = by_id.get(&id).and_then(|n| n.parent);
    }
    true
}

pub(super) fn containing_box(
    id: u64,
    fixed: bool,
    input: &LayoutInput,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &BTreeMap<u64, NodeGeometry>,
    viewport_gutter: EdgeSizes,
) -> CssRect {
    let mut ancestor = by_id.get(&id).and_then(|n| n.parent);
    while let Some(id) = ancestor {
        if establishes(&styles[&id], fixed) {
            if let Some(model) = geometry.get(&id).and_then(|g| g.box_model.as_ref()) {
                return model.padding_box;
            }
        }
        ancestor = by_id.get(&id).and_then(|n| n.parent);
    }
    CssRect {
        x: viewport_gutter.left + if fixed { 0. } else { -input.scroll.x },
        y: viewport_gutter.top + if fixed { 0. } else { -input.scroll.y },
        width: (input.viewport.width - viewport_gutter.left - viewport_gutter.right).max(0.),
        height: (input.viewport.height - viewport_gutter.top - viewport_gutter.bottom).max(0.),
    }
}

fn axis(
    start: Option<f64>,
    end: Option<f64>,
    origin: f64,
    extent: f64,
    size: f64,
    margin_start: f64,
    margin_end: f64,
    auto_start: bool,
    auto_end: bool,
    reverse: bool,
    static_position: f64,
) -> f64 {
    match (start, end) {
        (Some(start), Some(end)) => {
            let free = extent
                - start
                - end
                - size
                - if auto_start { 0. } else { margin_start }
                - if auto_end { 0. } else { margin_end };
            let used_start = if auto_start && auto_end {
                if free >= 0. {
                    free / 2.
                } else if reverse {
                    free
                } else {
                    0.
                }
            } else if auto_start {
                free
            } else {
                margin_start
            };
            if reverse && !auto_start && !auto_end {
                origin + extent - end - size - margin_end
            } else {
                origin + start + used_start
            }
        }
        (Some(start), None) => origin + start + margin_start,
        (None, Some(end)) => origin + extent - end - size - margin_end,
        (None, None) => static_position,
    }
}

pub(super) fn resolve(
    input: &LayoutInput,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    clusters: &mut BTreeMap<u64, Vec<oryn_common::v4::TextClusterGeometry>>,
    viewport_gutter: EdgeSizes,
) {
    normalize_fixed(input, by_id, styles, geometry, text, clusters);
    let mut pending = input
        .nodes
        .iter()
        .filter(|n| n.parent.is_none())
        .map(|n| n.id)
        .collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        pending.extend(by_id[&id].children.iter().rev());
        resolve_one(
            id,
            input,
            by_id,
            styles,
            geometry,
            text,
            clusters,
            viewport_gutter,
        );
    }
}

pub(super) fn normalize_fixed(
    input: &LayoutInput,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    clusters: &mut BTreeMap<u64, Vec<oryn_common::v4::TextClusterGeometry>>,
) {
    // The flow pass expresses fixed boxes in viewport coordinates, while its
    // ordinary descendants still receive the document scroll subtraction.
    // Restore that shared plane once, stopping at nested fixed roots.
    for root in input
        .nodes
        .iter()
        .filter(|n| style_value(&styles[&n.id], "position", "static") == "fixed")
    {
        let mut descendants = root.children.clone();
        let mut moved = BTreeSet::new();
        while let Some(id) = descendants.pop() {
            if style_value(&styles[&id], "position", "static") == "fixed" || !moved.insert(id) {
                continue;
            }
            descendants.extend(by_id[&id].children.iter());
            let shift = |rect: &mut CssRect| {
                rect.x += input.scroll.x;
                rect.y += input.scroll.y;
            };
            if let Some(value) = geometry.get_mut(&id) {
                if let Some(model) = &mut value.box_model {
                    shift(&mut model.border_box);
                    shift(&mut model.padding_box);
                    shift(&mut model.content_box);
                }
                for rect in &mut value.client_rects {
                    shift(rect);
                }
            }
            if let Some(fragments) = text.get_mut(&id) {
                for fragment in fragments {
                    shift(&mut fragment.rect);
                    if let Some(flow) = &mut fragment.flow_rect {
                        shift(flow);
                    }
                    for cluster in &mut fragment.clusters {
                        shift(&mut cluster.rect);
                    }
                }
            }
        }
        for values in clusters.values_mut() {
            for cluster in values {
                if moved.contains(&cluster.layout_node) {
                    cluster.rect.x += input.scroll.x;
                    cluster.rect.y += input.scroll.y;
                }
            }
        }
    }
}

pub(super) fn resolve_one(
    id: u64,
    input: &LayoutInput,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    clusters: &mut BTreeMap<u64, Vec<oryn_common::v4::TextClusterGeometry>>,
    viewport_gutter: EdgeSizes,
) {
    let node = by_id[&id];
    let style = &styles[&id];
    let position = style_value(style, "position", "static");
    if !matches!(position, "absolute" | "fixed") {
        return;
    }
    let fixed = position == "fixed";
    let Some(model) = geometry.get(&id).and_then(|g| g.box_model.clone()) else {
        return;
    };
    let mut ancestor = node.parent;
    let mut containing = None;
    while let Some(parent) = ancestor {
        if establishes(&styles[&parent], fixed) {
            containing = geometry
                .get(&parent)
                .and_then(|g| g.box_model.as_ref())
                .map(|m| (parent, m.padding_box));
            if containing.is_some() {
                break;
            }
        }
        ancestor = by_id[&parent].parent;
    }
    let cb = containing.map(|(_, rect)| rect).unwrap_or(CssRect {
        x: viewport_gutter.left + if fixed { 0. } else { -input.scroll.x },
        y: viewport_gutter.top + if fixed { 0. } else { -input.scroll.y },
        width: (input.viewport.width - viewport_gutter.left - viewport_gutter.right).max(0.),
        height: (input.viewport.height - viewport_gutter.top - viewport_gutter.bottom).max(0.),
    });
    let font = length(
        style_value(style, "font-size", "16px"),
        16.,
        16.,
        input.viewport,
    );
    let inset = |name, basis| {
        optional_length(
            style_value(style, name, "auto"),
            basis,
            font,
            input.viewport,
        )
    };
    let auto = |name| style_value(style, name, "0px") == "auto";
    let cb_style = containing.and_then(|(id, _)| styles.get(&id));
    let axes = writing_mode::Axes::new(
        cb_style
            .map(|s| style_value(s, "writing-mode", "horizontal-tb"))
            .unwrap_or("horizontal-tb"),
        cb_style.is_some_and(|s| style_value(s, "direction", "ltr") == "rtl"),
    );
    let x = axis(
        inset("left", cb.width),
        inset("right", cb.width),
        cb.x,
        cb.width,
        model.border_box.width,
        model.margin.left,
        model.margin.right,
        auto("margin-left"),
        auto("margin-right"),
        axes.x_reverse,
        model.border_box.x,
    );
    let y = axis(
        inset("top", cb.height),
        inset("bottom", cb.height),
        cb.y,
        cb.height,
        model.border_box.height,
        model.margin.top,
        model.margin.bottom,
        auto("margin-top"),
        auto("margin-bottom"),
        axes.y_reverse,
        model.border_box.y,
    );
    let aligned = |key: &str, size: f64, start: f64, end: f64, ordinary: f64| {
        let Some(value) = style.get(key) else {
            return ordinary;
        };
        let mut values = value
            .split_whitespace()
            .filter_map(|value| value.parse::<f64>().ok());
        let (Some(coordinate), Some(fraction)) = (values.next(), values.next()) else {
            return ordinary;
        };
        coordinate - size * fraction + start * (1. - fraction) - end * fraction
    };
    let x = aligned(
        "-oryn-anchor-x",
        model.border_box.width,
        model.margin.left,
        model.margin.right,
        x,
    );
    let y = aligned(
        "-oryn-anchor-y",
        model.border_box.height,
        model.margin.top,
        model.margin.bottom,
        y,
    );
    let dx = x - model.border_box.x;
    let dy = y - model.border_box.y;
    if dx == 0. && dy == 0. {
        return;
    }
    let mut subtree = vec![id];
    let mut moved = BTreeSet::new();
    while let Some(child) = subtree.pop() {
        if !moved.insert(child) {
            continue;
        }
        subtree.extend(by_id[&child].children.iter());
        let shift = |rect: &mut CssRect| {
            rect.x += dx;
            rect.y += dy;
        };
        if let Some(value) = geometry.get_mut(&child) {
            if let Some(model) = &mut value.box_model {
                shift(&mut model.border_box);
                shift(&mut model.padding_box);
                shift(&mut model.content_box);
            }
            for rect in &mut value.client_rects {
                shift(rect);
            }
        }
        if let Some(fragments) = text.get_mut(&child) {
            for fragment in fragments {
                shift(&mut fragment.rect);
                if let Some(flow) = &mut fragment.flow_rect {
                    shift(flow);
                }
                for cluster in &mut fragment.clusters {
                    shift(&mut cluster.rect);
                }
            }
        }
    }
    for values in clusters.values_mut() {
        for cluster in values {
            if moved.contains(&cluster.layout_node) {
                cluster.rect.x += dx;
                cluster.rect.y += dy;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opposing_insets_distribute_auto_margins_and_obey_direction() {
        assert_eq!(
            axis(
                Some(10.),
                Some(20.),
                100.,
                300.,
                50.,
                0.,
                0.,
                true,
                true,
                false,
                0.
            ),
            220.
        );
        assert_eq!(
            axis(
                Some(10.),
                Some(20.),
                100.,
                300.,
                50.,
                4.,
                6.,
                false,
                false,
                false,
                0.
            ),
            114.
        );
        assert_eq!(
            axis(
                Some(10.),
                Some(20.),
                100.,
                300.,
                50.,
                4.,
                6.,
                false,
                false,
                true,
                0.
            ),
            324.
        );
        assert_eq!(
            axis(
                None,
                Some(20.),
                100.,
                300.,
                50.,
                4.,
                6.,
                false,
                false,
                false,
                0.
            ),
            324.
        );
        assert_eq!(
            axis(
                None, None, 100., 300., 50., 4., 6., false, false, false, 42.
            ),
            42.
        );
    }
}

/// Move a formatted subtree and its deferred static-position anchors together.
/// Used by logical block flow after the child's used block size is available.
pub(super) fn translate_subtree(
    id: u64,
    dx: f64,
    dy: f64,
    by_id: &BTreeMap<u64, &LayoutNode>,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    engine: &mut TextEngine,
) {
    let mut pending = vec![id];
    let mut moved = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !moved.insert(id) {
            continue;
        }
        pending.extend(by_id[&id].children.iter());
        let shift = |rect: &mut CssRect| {
            rect.x += dx;
            rect.y += dy;
        };
        if let Some(value) = geometry.get_mut(&id) {
            if let Some(model) = &mut value.box_model {
                shift(&mut model.border_box);
                shift(&mut model.padding_box);
                shift(&mut model.content_box);
            }
            for rect in &mut value.client_rects {
                shift(rect);
            }
        }
        if let Some(fragments) = text.get_mut(&id) {
            for fragment in fragments {
                shift(&mut fragment.rect);
                if let Some(flow) = &mut fragment.flow_rect {
                    shift(flow);
                }
                for cluster in &mut fragment.clusters {
                    shift(&mut cluster.rect);
                }
            }
        }
    }
    for clusters in engine.source_geometry.values_mut() {
        for cluster in clusters {
            if moved.contains(&cluster.layout_node) {
                cluster.rect.x += dx;
                cluster.rect.y += dy;
            }
        }
    }
    for line_break in engine.break_boxes.values_mut() {
        if moved.contains(&line_break.layout_node) {
            line_break.before.x += dx;
            line_break.before.y += dy;
            if let Some(after) = &mut line_break.after {
                after.x += dx;
                after.y += dy;
            }
        }
    }
    for (id, anchor) in &mut engine.out_of_flow {
        if moved.contains(id) {
            anchor.x += dx;
            anchor.y += dy;
        }
    }
}
