//! Native visibility proof for permission controls. Paint order and complete
//! projected regions are shared with layout; pointer-events never hides paint.
use super::*;
use oryn_common::{
    v4::{BoxModel, CssRect, PaintProjection},
    visibility as region,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Visible,
    Clipped,
    Occluded,
    StyleInvalid,
}
const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
fn css(rect: BindingRect) -> CssRect {
    CssRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
fn model(rect: CssRect) -> BoxModel {
    BoxModel {
        border_box: rect,
        padding_box: rect,
        content_box: rect,
        margin: Default::default(),
        padding: Default::default(),
        border: Default::default(),
    }
}
fn projection(layout: &BindingLayout, key: u64) -> Option<PaintProjection> {
    if let Some(projection) = layout.hit_projections.get(&key) {
        return Some(projection.clone());
    }
    if let Some(projection) = layout
        .generated_nodes
        .iter()
        .find(|node| node.node_id == key)
        .and_then(|node| node.projection.clone())
    {
        return Some(projection);
    }
    let BindingGeometry::Available(rect) = layout.geometry.get(&key)? else {
        return None;
    };
    Some(PaintProjection {
        scroll: None,
        fragment_boxes: layout
            .client_rects
            .get(&key)
            .map(|rects| rects.iter().map(|rect| model(css(*rect))).collect())
            .unwrap_or_default(),
        border_segments: Vec::new(),
        border_edges: None,
        text_matrix: None,
        text_clips: Vec::new(),
        matrix: IDENTITY,
        box_model: model(css(*rect)),
        text: Vec::new(),
        clips: Vec::new(),
    })
}
fn transparent(style: &BTreeMap<String, String>) -> bool {
    style
        .get("-oryn-anchor-hidden")
        .is_some_and(|value| value == "true")
        || style.get("display").is_some_and(|value| value == "none")
        || style
            .get("visibility")
            .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
        || style
            .get("opacity")
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value <= 0.)
}
fn masks(style: &BTreeMap<String, String>) -> bool {
    ["mask-image", "mask-border-source"].iter().any(|property| {
        style
            .get(*property)
            .is_some_and(|value| !value.split(',').all(|layer| layer.trim() == "none"))
    })
}
fn distorted(style: &BTreeMap<String, String>) -> bool {
    ["clip-path", "filter", "backdrop-filter", "perspective"]
        .iter()
        .any(|property| style.get(*property).is_some_and(|value| value != "none"))
        || style
            .get("opacity")
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value < 1.)
}
fn paints(style: &BTreeMap<String, String>, replaced: bool, text: bool) -> bool {
    replaced
        || text
        || style
            .get("background-color")
            .and_then(|color| oryn_common::color::parse_color(color))
            .is_some_and(|color| color[3] > 0)
        || [
            "background-image",
            "box-shadow",
            "filter",
            "backdrop-filter",
        ]
        .iter()
        .any(|property| style.get(*property).is_some_and(|value| value != "none"))
        || ["top", "right", "bottom", "left"].iter().any(|side| {
            style
                .get(&format!("border-{side}-style"))
                .is_some_and(|value| !matches!(value.as_str(), "none" | "hidden"))
                && style
                    .get(&format!("border-{side}-width"))
                    .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
                    .is_some_and(|value| value > 0.)
        })
}
fn beneath(mut key: u64, owner: u64, parents: &BTreeMap<u64, Option<u64>>) -> bool {
    for _ in 0..1024 {
        if key == owner {
            return true;
        }
        let Some(parent) = parents.get(&key).copied().flatten() else {
            return false;
        };
        key = parent;
    }
    true
}
fn painted_regions(
    projection: &PaintProjection,
    style: &BTreeMap<String, String>,
    viewport: CssRect,
) -> Result<Vec<region::Polygon>, String> {
    let mut regions = region::regions(projection, viewport)?;
    if let Some(shadows) = style
        .get("box-shadow")
        .filter(|value| value.as_str() != "none")
    {
        let current = style.get("color").map(String::as_str).unwrap_or("black");
        let font = style
            .get("font-size")
            .and_then(|value| value.trim_end_matches("px").parse().ok())
            .unwrap_or(16.);
        let shadows = oryn_common::box_shadow::parse(shadows, current, font)
            .ok_or("unresolved native shadow footprint")?;
        let rects = if projection.fragment_boxes.is_empty() {
            vec![projection.box_model.border_box]
        } else {
            projection
                .fragment_boxes
                .iter()
                .map(|model| model.border_box)
                .collect()
        };
        for shadow in shadows {
            if shadow.inset || shadow.color[3] == 0 {
                continue;
            }
            for rect in &rects {
                regions.push(region::projected(
                    oryn_common::box_shadow::bounds(*rect, &shadow),
                    projection.matrix,
                    Some(projection),
                    viewport,
                )?);
            }
        }
    }
    // Filter composition can extend descendants beyond their border planes. If
    // native layout has not supplied a finite effect footprint, visibility cannot
    // certify an unobstructed target; never quietly discard that painted effect.
    if ["filter", "backdrop-filter"]
        .iter()
        .any(|name| style.get(*name).is_some_and(|value| value != "none"))
    {
        return Err("unresolved native filter footprint".into());
    }
    Ok(regions)
}
fn native_coverage(
    state: &V8BindingState,
    keys: &BTreeSet<u64>,
    regions: &[region::Polygon],
) -> Result<Option<bool>, V8BindingError> {
    if keys.is_empty() {
        return Ok(Some(false));
    }
    let Some(provider) = state.layout_provider.borrow().clone() else {
        return Ok(None);
    };
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let source = {
        let dom = state.dom.borrow();
        dom.flat_tree_children(identity.0)?
            .into_iter()
            .find(|node| {
                dom.arena()
                    .get(*node)
                    .is_ok_and(|node| matches!(node.kind, NodeKind::Element { .. }))
            })
            .ok_or(V8BindingError::Allocation(
                "permission paint document element",
            ))?
    };
    let mut polygons = Vec::new();
    for polygon in regions {
        if polygon.len() < 3 {
            continue;
        }
        for index in 1..polygon.len() - 1 {
            let [a, b, c] = [polygon[0], polygon[index], polygon[index + 1]];
            polygons.push([a[0], a[1], b[0], b[1], c[0], c[1], c[0], c[1]]);
        }
    }
    let request = crate::drag_image::Capture {
        css_resources: state.captured_css_images(),
        document: identity.0,
        source,
        generation: identity.1,
        epoch: identity.2,
        hotspot: (0, 0),
        polygons,
        resources: snapshot_native_paint_resources(state, source)?,
    };
    let result = provider.permission_paint_coverage(
        &request,
        keys,
        &state.custom_states.borrow(),
        state.document_scroll_offset(),
        &state.element_scroll_offsets(),
    );
    if state.execution_retired()
        || identity
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            )
    {
        return Ok(Some(true));
    }
    // Missing/failed raster evidence cannot certify visibility.
    Ok(result.unwrap_or(Some(true)))
}
pub(super) fn assess(state: &V8BindingState, node: NodeId) -> Result<Status, V8BindingError> {
    assess_native(state, node, true, None)
}
pub(super) fn intersection_region_visible(
    state: &V8BindingState,
    node: NodeId,
    regions: &[region::Polygon],
) -> Result<bool, V8BindingError> {
    Ok(assess_native(state, node, false, Some(regions))? == Status::Visible)
}
fn assess_native(
    state: &V8BindingState,
    node: NodeId,
    permission: bool,
    observed_region: Option<&[region::Polygon]>,
) -> Result<Status, V8BindingError> {
    let dom = state.dom.borrow();
    if permission && dom.is_inert(node)? {
        return Ok(Status::Occluded);
    }
    let layout = state.layout.borrow();
    let Some((width, height)) = layout.viewport else {
        return Ok(Status::Clipped);
    };
    let viewport = CssRect {
        x: 0.,
        y: 0.,
        width,
        height,
    };
    let target = node_key(node);
    let mut stack = Vec::new();
    let mut parents = BTreeMap::new();
    let mut painted = BTreeSet::new();
    let empty = BTreeMap::new();
    let mut pending = vec![(state.document_root(), None)];
    let mut seen = BTreeSet::new();
    while let Some((node, parent)) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        let record = dom.arena().get(node)?;
        let mut next = parent;
        if let NodeKind::Element { name } = &record.kind {
            let key = node_key(node);
            let parent = layout
                .paint_parent_overrides
                .get(&key)
                .copied()
                .unwrap_or(parent);
            let style = layout
                .styles
                .get(&key)
                .map(|value| value.as_ref())
                .unwrap_or(&empty);
            parents.insert(key, parent);
            stack.push(oryn_common::paint_order::StackingNode {
                top_layer: dom.top_layer().position(|entry| {
                    entry.node == node
                        && !matches!(
                            entry.kind,
                            oryn_dom::TopLayerKind::SelectPicker
                                | oryn_dom::TopLayerKind::TemporalPicker
                                | oryn_dom::TopLayerKind::PermissionPrompt
                        )
                }),
                node_id: key,
                parent_id: parent,
                style,
            });
            if paints(
                style,
                matches!(
                    name.local.as_ref(),
                    "img" | "canvas" | "video" | "iframe" | "svg"
                ),
                false,
            ) {
                painted.insert(key);
            }
            next = Some(key);
        }
        pending.extend(
            dom.flat_tree_children(node)?
                .into_iter()
                .rev()
                .map(|child| (child, next)),
        );
    }
    for generated in &layout.generated_nodes {
        parents.insert(generated.node_id, generated.parent_id);
        let entry = oryn_common::paint_order::StackingNode {
            top_layer: generated.top_layer,
            node_id: generated.node_id,
            parent_id: generated.parent_id,
            style: &generated.style,
        };
        if generated.tag == "::before" {
            if let Some(index) = stack
                .iter()
                .position(|entry| Some(entry.node_id) == generated.parent_id)
            {
                stack.insert(index + 1, entry)
            } else {
                stack.push(entry)
            }
        } else {
            stack.push(entry)
        }
        if paints(
            &generated.style,
            generated.builtin_image.is_some() || generated.image_resource.is_some(),
            !generated.text.is_empty(),
        ) {
            painted.insert(generated.node_id);
        }
    }
    // Text clusters are keyed by DOM text nodes, while native paint ordering is
    // keyed by their layout owner. Keep individual projected clusters rather
    // than turning a text-only owner into an opaque border rectangle.
    let mut text_regions: BTreeMap<u64, Vec<region::Polygon>> = BTreeMap::new();
    for clusters in layout.text_clusters.values() {
        for cluster in clusters {
            let Some(owner_projection) = projection(&layout, cluster.layout_node) else {
                continue;
            };
            let owner_style = layout
                .styles
                .get(&cluster.layout_node)
                .map(|style| style.as_ref())
                .unwrap_or(&empty);
            if owner_style
                .get("color")
                .and_then(|value| oryn_common::color::parse_color(value))
                .is_some_and(|color| color[3] == 0)
            {
                continue;
            }
            let polygon = region::projected(
                cluster.rect,
                cluster.matrix,
                Some(&owner_projection),
                viewport,
            )
            .map_err(|_| {
                V8BindingError::Allocation("native permission text visibility geometry")
            })?;
            text_regions
                .entry(cluster.layout_node)
                .or_default()
                .push(polygon);
        }
    }
    let styles = stack
        .iter()
        .map(|entry| (entry.node_id, entry.style))
        .collect::<BTreeMap<_, _>>();
    let mut current = Some(target);
    while let Some(key) = current {
        if let Some(style) = styles.get(&key) {
            if masks(style) {
                return Ok(Status::StyleInvalid);
            }
            if transparent(style) {
                return Ok(Status::Clipped);
            }
            if distorted(style) {
                return Ok(Status::Occluded);
            }
        }
        current = parents.get(&key).copied().flatten();
    }
    let Some(target_projection) = projection(&layout, target) else {
        return Ok(Status::Clipped);
    };
    let matrix = target_projection.matrix;
    if !matrix.iter().all(|value| value.is_finite())
        || matrix[0] < 1.
        || matrix[0] != matrix[5]
        || [1, 2, 3, 4, 6, 7, 8, 9, 11, 14]
            .iter()
            .any(|index| matrix[*index] != 0.)
        // Native flat-tree projection flattens child depth to zero. This
        // leaves the control plane (z = 0) unchanged in screen space.
        || (matrix[10] != 0. && matrix[10] != 1.)
        || matrix[15] != 1.
    {
        return Ok(Status::Occluded);
    }
    if permission && !region::fully_unclipped(&target_projection, viewport).unwrap_or(false) {
        return Ok(Status::Clipped);
    }
    let target_regions = match observed_region {
        Some(regions) => regions.to_vec(),
        None => region::regions(&target_projection, viewport)
            .map_err(|_| V8BindingError::Allocation("native permission visibility geometry"))?,
    };
    if !target_regions
        .iter()
        .any(|polygon| region::area(polygon) > 0.)
    {
        return Ok(Status::Clipped);
    };
    let order = oryn_common::paint_order::stacking_order(&stack);
    let Some(index) = order.iter().position(|key| *key == target) else {
        return Ok(Status::Clipped);
    };
    let paint_keys = order
        .iter()
        .skip(index + 1)
        .copied()
        .filter(|key| !beneath(*key, target, &parents))
        .collect::<BTreeSet<_>>();
    let mut conservative_occluded = false;
    for key in order.into_iter().skip(index + 1) {
        if beneath(key, target, &parents)
            || (!painted.contains(&key) && !text_regions.contains_key(&key))
        {
            continue;
        }
        let mut hidden = false;
        let mut parent = Some(key);
        while let Some(key) = parent {
            if styles.get(&key).is_some_and(|style| transparent(style)) {
                hidden = true;
                break;
            }
            parent = parents.get(&key).copied().flatten();
        }
        if hidden {
            continue;
        }
        let Some(projection) = projection(&layout, key) else {
            continue;
        };
        let mut regions = if painted.contains(&key) {
            match painted_regions(
                &projection,
                styles.get(&key).copied().unwrap_or(&empty),
                viewport,
            ) {
                Ok(regions) => regions,
                Err(_) => {
                    conservative_occluded = true;
                    break;
                }
            }
        } else {
            Vec::new()
        };
        if let Some(text) = text_regions.get(&key) {
            regions.extend(text.iter().cloned());
        }
        if region::overlaps(&target_regions, &regions).unwrap_or(true) {
            conservative_occluded = true;
            break;
        }
    }
    drop(styles);
    drop(stack);
    drop(layout);
    drop(dom);
    match native_coverage(state, &paint_keys, &target_regions)? {
        Some(true) => Ok(Status::Occluded),
        Some(false) => Ok(Status::Visible),
        None => Ok(if conservative_occluded {
            Status::Occluded
        } else {
            Status::Visible
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_permission_visibility_catches_thin_pointer_transparent_overlay_and_ancestor_mask() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=parent><usermedia id=control></usermedia></div><div id=overlay></div></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let control = state.dom.borrow().find_by_id("control").unwrap();
        let parent = state.dom.borrow().find_by_id("parent").unwrap();
        let overlay = state.dom.borrow().find_by_id("overlay").unwrap();
        let style = |pairs: &[(&str, &str)]| {
            std::sync::Arc::new(
                pairs
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect::<BTreeMap<_, _>>(),
            )
        };
        {
            let mut layout = state.layout.borrow_mut();
            layout.viewport = Some((100., 100.));
            layout.geometry.insert(
                node_key(control),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.,
                }),
            );
            layout.geometry.insert(
                node_key(overlay),
                BindingGeometry::Available(BindingRect {
                    x: 24.,
                    y: 20.,
                    width: 0.01,
                    height: 10.,
                }),
            );
            layout
                .styles
                .insert(node_key(control), style(&[("display", "inline-block")]));
            layout.styles.insert(
                node_key(overlay),
                style(&[
                    ("position", "absolute"),
                    ("z-index", "20"),
                    ("background-color", "red"),
                    ("pointer-events", "none"),
                ]),
            );
        }
        assert_eq!(assess(&state, control).unwrap(), Status::Occluded);
        state.layout.borrow_mut().styles.insert(
            node_key(overlay),
            style(&[
                ("position", "absolute"),
                ("z-index", "20"),
                ("background-color", "transparent"),
            ]),
        );
        assert_eq!(assess(&state, control).unwrap(), Status::Visible);
        state.layout.borrow_mut().geometry.insert(
            node_key(overlay),
            BindingGeometry::Available(BindingRect {
                x: 110.,
                y: 20.,
                width: 10.,
                height: 10.,
            }),
        );
        state.layout.borrow_mut().styles.insert(
            node_key(overlay),
            style(&[
                ("position", "absolute"),
                ("z-index", "20"),
                ("box-shadow", "-30px 0px 0px red"),
            ]),
        );
        assert_eq!(assess(&state, control).unwrap(), Status::Occluded);
        state.layout.borrow_mut().styles.insert(
            node_key(overlay),
            style(&[
                ("position", "absolute"),
                ("z-index", "20"),
                ("box-shadow", "30px 0px 0px red"),
            ]),
        );
        assert_eq!(assess(&state, control).unwrap(), Status::Visible);
        // The native producer indexes clusters by text node, not element. A
        // transparent text-only owner must occlude only where its clusters paint.
        state.layout.borrow_mut().styles.insert(
            node_key(overlay),
            style(&[
                ("position", "absolute"),
                ("z-index", "20"),
                ("color", "black"),
            ]),
        );
        let text = state.dom.borrow_mut().create_text("X");
        state.dom.borrow_mut().append_child(overlay, text).unwrap();
        state.layout.borrow_mut().text_clusters.insert(
            node_key(text),
            vec![oryn_common::v4::TextClusterGeometry {
                layout_node: node_key(overlay),
                start: 0,
                end: 1,
                rect: CssRect {
                    x: 24.,
                    y: 20.,
                    width: 8.,
                    height: 10.,
                },
                matrix: IDENTITY,
                line: 0,
                rtl: false,
            }],
        );
        assert_eq!(assess(&state, control).unwrap(), Status::Occluded);
        state
            .layout
            .borrow_mut()
            .text_clusters
            .get_mut(&node_key(text))
            .unwrap()[0]
            .rect
            .x = 110.;
        assert_eq!(assess(&state, control).unwrap(), Status::Visible);
        state
            .layout
            .borrow_mut()
            .text_clusters
            .get_mut(&node_key(text))
            .unwrap()[0]
            .rect
            .x = 24.;
        state.layout.borrow_mut().styles.insert(
            node_key(overlay),
            style(&[
                ("position", "absolute"),
                ("z-index", "20"),
                ("color", "transparent"),
            ]),
        );
        assert_eq!(assess(&state, control).unwrap(), Status::Visible);
        state.layout.borrow_mut().text_clusters.clear();
        state.layout.borrow_mut().styles.insert(
            node_key(parent),
            style(&[("mask-image", "linear-gradient(black, transparent)")]),
        );
        assert_eq!(assess(&state, control).unwrap(), Status::StyleInvalid);
        state.layout.borrow_mut().styles.remove(&node_key(parent));
        let mut projected = projection(&state.layout.borrow(), node_key(control)).unwrap();
        projected.clips.push(oryn_common::v4::PaintClip {
            shape: None,
            owner: None,
            scroll_x: false,
            scroll_y: false,
            matrix: IDENTITY,
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 99.99,
                height: 0.,
            },
            clip_x: true,
            clip_y: false,
        });
        state
            .layout
            .borrow_mut()
            .hit_projections
            .insert(node_key(control), projected);
        assert_eq!(assess(&state, control).unwrap(), Status::Clipped);
    }
}
