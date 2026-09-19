//! Scrollable overflow is computed before scroll translations and world transforms.
//! Descendant overflow stops at clipping containers; transforms can enlarge but
//! never shrink the overflow contributed by a CSS box.
use super::*;
use oryn_common::v4::{BoxModel, PaintScrollGeometry};
fn finite(rect: CssRect) -> bool {
    [
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        rect.right(),
        rect.bottom(),
    ]
    .iter()
    .all(|v| v.is_finite())
        && rect.width >= 0.
        && rect.height >= 0.
}
fn union(a: CssRect, b: CssRect) -> CssRect {
    if !finite(b) {
        return a;
    }
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    CssRect {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}
fn clipped(mut rect: CssRect, port: CssRect, x: bool, y: bool) -> CssRect {
    if x {
        let end = rect.right().min(port.right());
        rect.x = rect.x.max(port.x);
        rect.width = (end - rect.x).max(0.);
    }
    if y {
        let end = rect.bottom().min(port.bottom());
        rect.y = rect.y.max(port.y);
        rect.height = (end - rect.y).max(0.);
    }
    rect
}
type Point = [f64; 4];
fn corners(rect: CssRect) -> [Point; 5] {
    [
        [rect.x, rect.y, 0., 1.],
        [rect.right(), rect.y, 0., 1.],
        [rect.right(), rect.bottom(), 0., 1.],
        [rect.x, rect.bottom(), 0., 1.],
        [rect.x, rect.y, 0., 1.],
    ]
}
fn point_bounds(points: &[Point]) -> Option<CssRect> {
    let mut rect = None;
    for point in points {
        if point[3] <= 0. {
            continue;
        }
        let (x, y) = (point[0] / point[3], point[1] / point[3]);
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let point = CssRect {
            x,
            y,
            width: 0.,
            height: 0.,
        };
        rect = Some(rect.map(|rect| union(rect, point)).unwrap_or(point));
    }
    rect
}
/// Keep the convex hull of a flattened plane: subsequent ancestor transforms
/// must operate on native vertices, not a repeatedly enlarged axis-aligned box.
fn flatten_hull(points: Vec<Point>) -> Vec<Point> {
    let mut points = points
        .into_iter()
        .filter(|p| p[3] > 0.)
        .map(|p| [p[0] / p[3], p[1] / p[3], 0., 1.])
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .collect::<Vec<_>>();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |a: Point, b: Point, c: Point| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut hull = Vec::new();
    for point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0. {
            hull.pop();
        }
        hull.push(*point);
    }
    let lower = hull.len();
    for point in points.iter().rev().skip(1) {
        while hull.len() > lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.
        {
            hull.pop();
        }
        hull.push(*point);
    }
    hull.pop();
    hull
}
fn clip_polygon(points: Vec<Point>, port: CssRect, clip_x: bool, clip_y: bool) -> Vec<Point> {
    // Clip native homogeneous edges before perspective division. Dropping a
    // behind-camera vertex first would lose its visible edge intersection.
    let mut polygon = points;
    for (plane, enabled) in [
        ([0., 0., 0., 1.], true),
        ([1., 0., 0., -port.x], clip_x),
        ([-1., 0., 0., port.right()], clip_x),
        ([0., 1., 0., -port.y], clip_y),
        ([0., -1., 0., port.bottom()], clip_y),
    ] {
        if !enabled || polygon.is_empty() {
            continue;
        }
        let distance = |point: Point| (0..4).map(|i| point[i] * plane[i]).sum::<f64>();
        let mut output = Vec::new();
        let mut previous = *polygon.last().unwrap();
        let mut before = distance(previous);
        for current in polygon {
            let after = distance(current);
            if (before >= 0.) != (after >= 0.) {
                let magnitude = before.abs().max(after.abs());
                let (a, b) = (before / magnitude, after / magnitude);
                let t = a / (a - b);
                output.push(std::array::from_fn(|i| {
                    previous[i] + t * (current[i] - previous[i])
                }));
            }
            if after >= 0. {
                output.push(current);
            }
            previous = current;
            before = after;
        }
        polygon = output;
    }
    polygon
}
fn clip_hull(points: Vec<Point>, port: CssRect, clip_x: bool, clip_y: bool) -> Vec<Point> {
    flatten_hull(clip_polygon(points, port, clip_x, clip_y))
}

pub(super) fn collect(
    input: &LayoutInput,
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    boxes: &BTreeMap<u64, BoxModel>,
    text: &BTreeMap<u64, Vec<PaintTextFragment>>,
    gutters: &BTreeMap<u64, EdgeSizes>,
    diagnostics: &mut Vec<String>,
) -> BTreeMap<u64, PaintScrollGeometry> {
    // Positioned descendants contribute to the box which establishes their
    // containing block, bypassing unrelated clipping ancestors in the DOM tree.
    let mut routed = BTreeMap::<u64, Vec<u64>>::new();
    let mut positioned = BTreeSet::new();
    for node in &input.nodes {
        let position = styles
            .get(&node.id)
            .map(|style| style_value(style, "position", "static"))
            .unwrap_or("static");
        if !matches!(position, "absolute" | "fixed") {
            continue;
        }
        positioned.insert(node.id);
        let fixed = position == "fixed";
        let mut ancestor = node.parent;
        let mut top = None;
        let mut target = None;
        while let Some(id) = ancestor {
            top = Some(id);
            if boxes.contains_key(&id)
                && styles
                    .get(&id)
                    .is_some_and(|style| positioning::establishes(style, fixed))
            {
                target = Some(id);
                break;
            }
            ancestor = nodes.get(&id).and_then(|node| node.parent);
        }
        if let Some(target) = target.or(if fixed { None } else { top }) {
            routed.entry(target).or_default().push(node.id);
        }
    }
    let mut result = BTreeMap::new();
    let mut overflow = BTreeMap::<u64, Vec<Point>>::new();
    let mut retained_points = 0usize;
    let mut pending = input
        .nodes
        .iter()
        .filter(|node| node.parent.is_none())
        .map(|node| (node.id, false))
        .collect::<Vec<_>>();
    while let Some((id, visited)) = pending.pop() {
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        if !visited {
            pending.push((id, true));
            pending.extend(node.children.iter().rev().map(|child| (*child, false)));
            continue;
        }
        let Some(model) = boxes.get(&id) else {
            // display:contents and document wrappers do not interrupt box ancestry.
            let mut inherited = Vec::new();
            for child in node
                .children
                .iter()
                .filter(|child| !positioned.contains(*child))
                .chain(routed.get(&id).into_iter().flatten())
            {
                if let Some(points) = overflow.remove(child) {
                    retained_points = retained_points.saturating_sub(points.len());
                    inherited.extend(points);
                }
            }
            retained_points += inherited.len();
            overflow.insert(id, inherited);
            continue;
        };
        let Some(style) = styles.get(&id) else {
            continue;
        };
        let port = inset(
            model.padding_box,
            gutters.get(&id).copied().unwrap_or_default(),
        );
        let mut bounds = port;
        let mode = style_value(style, "writing-mode", "horizontal-tb");
        let rtl = style_value(style, "direction", "ltr") == "rtl";
        let vertical = mode != "horizontal-tb";
        let negative_x = if vertical {
            matches!(mode, "vertical-rl" | "sideways-rl")
        } else {
            rtl
        };
        let negative_y = vertical && (rtl ^ (mode == "sideways-lr"));
        let padded = |mut rect: CssRect| {
            if negative_x {
                rect.x -= model.padding.left;
                rect.width += model.padding.left;
            } else {
                rect.width += model.padding.right;
            }
            if negative_y {
                rect.y -= model.padding.top;
                rect.height += model.padding.top;
            } else {
                rect.height += model.padding.bottom;
            }
            rect
        };
        let mut vertices = corners(model.border_box).to_vec();
        if let Some(fragments) = text.get(&id) {
            for fragment in fragments {
                bounds = union(bounds, padded(fragment.rect));
                vertices.extend(corners(fragment.rect));
            }
        }
        let font = length(
            style_value(style, "font-size", "16px"),
            16.,
            16.,
            input.viewport,
        );
        let perspective = if transformable_box(node, style) {
            transforms::prepare(
                model.border_box,
                &format!("perspective({})", style_value(style, "perspective", "none")),
                style_value(style, "perspective-origin", "50% 50%"),
                font,
                input.viewport,
            )
        } else {
            None
        };
        for child in node
            .children
            .iter()
            .filter(|child| !positioned.contains(*child))
            .chain(routed.get(&id).into_iter().flatten())
        {
            if let Some(mut points) = overflow.remove(child) {
                retained_points = retained_points.saturating_sub(points.len());
                if let Some(perspective) = perspective {
                    for point in &mut points {
                        *point = perspective.homogeneous(*point);
                    }
                }
                let horizon = points.iter().any(|p| p[3] <= 0.) && points.iter().any(|p| p[3] > 0.);
                let outside = points.iter().any(|point| {
                    point[3] > 0.
                        && (point[0].abs()
                            > oryn_common::layout_domain::MAX_SCROLL_COORDINATE * point[3]
                            || point[1].abs()
                                > oryn_common::layout_domain::MAX_SCROLL_COORDINATE * point[3])
                });
                if horizon || outside {
                    diagnostics.push(format!(
                        "node {id}: {} saturated to native signed scroll domain ±{} CSS px",
                        if horizon {
                            "mathematically unbounded perspective overflow"
                        } else {
                            "overflow beyond backend coordinate domain"
                        },
                        oryn_common::layout_domain::MAX_SCROLL_COORDINATE
                    ));
                    points = clip_polygon(
                        points,
                        oryn_common::layout_domain::SCROLL_DOMAIN,
                        true,
                        true,
                    );
                }
                if let Some(rect) = point_bounds(&points) {
                    bounds = union(bounds, padded(rect));
                }
                vertices.extend(points);
            }
        }
        if let Some(clipped) = oryn_common::layout_domain::clip_scroll_rect(bounds) {
            if clipped != bounds {
                diagnostics.push(format!(
                    "node {id}: padded scroll overflow saturated to native coordinate domain"
                ));
                bounds = union(port, clipped);
            }
        }
        // The unreachable negative overflow region does not enlarge the scroll area.
        // RTL and vertical-rl block progression instead have a negative horizontal origin.
        let scrollable = CssRect {
            x: if negative_x { bounds.x } else { port.x },
            y: if negative_y { bounds.y } else { port.y },
            width: if negative_x {
                port.right() - bounds.x
            } else {
                bounds.right() - port.x
            },
            height: if negative_y {
                port.bottom() - bounds.y
            } else {
                (bounds.bottom() - port.y).max(port.height)
            },
        };
        let eligible = |axis| {
            matches!(
                style_value(style, axis, "visible"),
                "auto" | "scroll" | "hidden"
            )
        };
        let (offset_x, offset_y) = input.element_scrolls.get(&id).copied().unwrap_or_default();
        let offset_x = if eligible("overflow-x") && offset_x.is_finite() {
            offset_x.clamp(
                (scrollable.x - port.x).min(0.),
                (scrollable.right() - port.right()).max(0.),
            )
        } else {
            0.
        };
        let offset_y = if eligible("overflow-y") && offset_y.is_finite() {
            offset_y.clamp(
                (scrollable.y - port.y).min(0.),
                (scrollable.bottom() - port.bottom()).max(0.),
            )
        } else {
            0.
        };
        result.insert(
            id,
            PaintScrollGeometry {
                offset_x,
                offset_y,
                scrollport: port,
                scrollable,
            },
        );
        let clip_x = style_value(style, "overflow-x", "visible") != "visible";
        let clip_y = style_value(style, "overflow-y", "visible") != "visible";
        if clip_x || clip_y {
            vertices = clip_hull(vertices, port, clip_x, clip_y);
            vertices.extend(corners(model.border_box));
            vertices = flatten_hull(vertices);
        } else if !preserves_3d(style) {
            vertices = flatten_hull(vertices);
        }
        if transformable_box(node, style) {
            let source = transforms::compose_source(
                style_value(style, "translate", "none"),
                style_value(style, "rotate", "none"),
                style_value(style, "scale", "none"),
                style_value(style, "transform", "none"),
            );
            let reference = if matches!(
                style_value(style, "transform-box", "view-box"),
                "content-box" | "fill-box"
            ) {
                model.content_box
            } else {
                model.border_box
            };
            let font = length(
                style_value(style, "font-size", "16px"),
                16.,
                16.,
                input.viewport,
            );
            if let Some(projection) = source.and_then(|source| {
                transforms::prepare(
                    reference,
                    &source,
                    style_value(style, "transform-origin", "50% 50%"),
                    font,
                    input.viewport,
                )
            }) {
                let transformed = vertices
                    .iter()
                    .map(|point| projection.homogeneous(*point))
                    .collect::<Vec<_>>();
                vertices.extend(transformed);
            }
        }
        if vertices.len() > 262_144 || retained_points.saturating_add(vertices.len()) > 1_000_000 {
            diagnostics.push(format!("node {id}: native overflow vertex budget exceeded"));
            vertices = point_bounds(&vertices)
                .map(corners)
                .map(Vec::from)
                .unwrap_or_default();
        }
        retained_points += vertices.len();
        overflow.insert(id, vertices);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_axis_does_not_leak_descendant_overflow() {
        let port = CssRect {
            x: 10.,
            y: 20.,
            width: 40.,
            height: 50.,
        };
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 200.,
            height: 200.,
        };
        assert_eq!(
            clipped(rect, port, true, false),
            CssRect {
                x: 10.,
                y: 0.,
                width: 40.,
                height: 200.
            }
        );
    }
    #[test]
    fn overflow_union_ignores_nonfinite_projection() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 30.,
        };
        assert_eq!(
            union(
                rect,
                CssRect {
                    x: f64::INFINITY,
                    ..rect
                }
            ),
            rect
        );
    }
    #[test]
    fn homogeneous_depth_survives_until_parent_perspective() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 50.,
            height: 30.,
        };
        let viewport = ViewportState::default();
        let local = transforms::prepare(rect, "translateZ(100px)", "0 0", 16., viewport).unwrap();
        let perspective =
            transforms::prepare(rect, "perspective(200px)", "0 0", 16., viewport).unwrap();
        let vertices = corners(rect).map(|p| perspective.homogeneous(local.homogeneous(p)));
        let bounds = point_bounds(&vertices).unwrap();
        assert!((bounds.width - 100.).abs() < 0.001);
        assert!((bounds.height - 60.).abs() < 0.001);
    }
    #[test]
    fn clipping_camera_crossing_preserves_visible_edges() {
        let points = vec![
            [0., 10., 0., 1.],
            [100., 10., 0., -1.],
            [100., 20., 0., -1.],
            [0., 20., 0., 1.],
        ];
        let clipped = clip_hull(
            points,
            CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
            },
            true,
            true,
        );
        let bounds = point_bounds(&clipped).unwrap();
        assert!((bounds.width - 100.).abs() < 0.001);
        assert!((bounds.y - 10.).abs() < 0.001);
        assert!((bounds.bottom() - 60.).abs() < 0.001);
    }
    #[test]
    fn clipping_uses_polygon_edges_instead_of_intermediate_bounds() {
        let points = vec![
            [0., 50., 0., 1.],
            [50., 0., 0., 1.],
            [100., 50., 0., 1.],
            [50., 100., 0., 1.],
        ];
        let clipped = clip_hull(
            points,
            CssRect {
                x: 0.,
                y: 0.,
                width: 25.,
                height: 100.,
            },
            true,
            false,
        );
        let bounds = point_bounds(&clipped).unwrap();
        assert!((bounds.y - 25.).abs() < 0.001);
        assert!((bounds.height - 50.).abs() < 0.001);
    }
}
