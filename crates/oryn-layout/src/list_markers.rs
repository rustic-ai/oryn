//! Native marker numbering and physical placement. Marker IDs are generated
//! paint identities, never selectable DOM text nodes.
use super::*;
pub(super) fn ordinals(
    nodes: &BTreeMap<u64, LayoutNode>,
    styles: &ComputedStyles,
) -> BTreeMap<u64, i64> {
    let mut result = BTreeMap::new();
    let eligible = |id: &u64| {
        styles.get(id).is_some_and(|style| {
            style_value(style, "display", "inline")
                .split_ascii_whitespace()
                .any(|part| part == "list-item")
        })
    };
    // HTML list ownership crosses ordinary wrapper boxes, but stops at nested
    // lists. Source child order, not allocation order, determines ordinals.
    for parent in nodes
        .values()
        .filter(|node| matches!(node.tag.as_str(), "ol" | "ul" | "menu"))
    {
        let mut pending = parent.children.iter().rev().copied().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = nodes.get(&id) else { continue };
            if matches!(node.tag.as_str(), "ol" | "ul" | "menu") {
                continue;
            }
            if eligible(&id) {
                items.push(id);
            }
            pending.extend(node.children.iter().rev().copied());
        }
        let reversed = parent.tag == "ol" && parent.attributes.contains_key("reversed");
        let step = if reversed { -1i64 } else { 1 };
        let mut next = parent
            .attributes
            .get("start")
            .filter(|_| parent.tag == "ol")
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(if reversed { items.len() as i64 } else { 1 });
        for id in items {
            if let Some(value) = nodes[&id]
                .attributes
                .get("value")
                .filter(|_| nodes[&id].tag == "li")
                .and_then(|value| value.trim().parse::<i64>().ok())
            {
                next = value;
            }
            result.insert(id, next);
            next = next.saturating_add(step);
        }
    }
    // CSS-created lists outside HTML list containers start sibling numbering.
    for parent in nodes.values() {
        let mut next = 1i64;
        for id in parent.children.iter().filter(|id| eligible(id)) {
            result.entry(*id).or_insert(next);
            next = next.saturating_add(1);
        }
    }
    result
}
pub(super) fn place_outside(
    nodes: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    text: &mut BTreeMap<u64, Vec<PaintTextFragment>>,
    engine: &mut TextEngine,
) {
    for marker in nodes.values().filter(|node| {
        styles
            .get(&node.id)
            .is_some_and(|style| style_value(style, "oryn-marker-position", "") == "outside")
    }) {
        let Some(owner) = marker.parent else { continue };
        let Some(parent) = geometry
            .get(&owner)
            .and_then(|geometry| geometry.box_model.clone())
        else {
            continue;
        };
        let Some(model) = geometry
            .get(&marker.id)
            .and_then(|geometry| geometry.box_model.clone())
        else {
            continue;
        };
        let Some(style) = styles.get(&owner) else {
            continue;
        };
        let axes = writing_mode::Axes::new(
            style_value(style, "writing-mode", "horizontal-tb"),
            style_value(style, "direction", "ltr") == "rtl",
        );
        let vertical = axes.vertical;
        let rtl = if vertical {
            axes.y_reverse
        } else {
            axes.x_reverse
        };
        let gap = 0.; // The marker suffix already supplies the typographic gap.
        let (mut x, mut y) = if vertical {
            (
                parent.content_box.x,
                if rtl {
                    parent.content_box.bottom() + gap
                } else {
                    parent.content_box.y - model.border_box.height - gap
                },
            )
        } else {
            (
                if rtl {
                    parent.content_box.right() + gap
                } else {
                    parent.content_box.x - model.border_box.width - gap
                },
                parent.content_box.y,
            )
        };
        {
            let mut pending = vec![owner];
            let mut seen = BTreeSet::new();
            let mut first = None;
            while let Some(id) = pending.pop() {
                if id == marker.id || !seen.insert(id) {
                    continue;
                }
                if let Some(node) = nodes.get(&id) {
                    if id != owner
                        && styles.get(&id).is_some_and(|style| {
                            matches!(
                                style_value(style, "position", "static"),
                                "absolute" | "fixed"
                            )
                        })
                    {
                        continue;
                    }
                    if let (Some(offset), Some(model)) = (
                        engine.first_line_baselines.get(&id),
                        geometry.get(&id).and_then(|value| value.box_model.as_ref()),
                    ) {
                        if styles.get(&id).is_some_and(|style| {
                            (style_value(style, "writing-mode", "horizontal-tb") != "horizontal-tb")
                                == vertical
                        }) {
                            first = Some(if vertical {
                                model.content_box.x + offset
                            } else {
                                model.content_box.y + offset
                            });
                            break;
                        }
                    }
                    pending.extend(node.children.iter().rev().copied());
                }
            }
            if let Some(first) = first {
                if vertical {
                    x = if let Some(marker_offset) = engine.first_line_baselines.get(&marker.id) {
                        first - marker_offset - (model.content_box.x - model.border_box.x)
                    } else {
                        first - model.border_box.width / 2.
                    };
                } else {
                    y = if let Some(marker_offset) = engine.first_line_baselines.get(&marker.id) {
                        first - marker_offset - (model.content_box.y - model.border_box.y)
                    } else {
                        first - model.border_box.height
                    };
                }
            }
        }
        positioning::translate_subtree(
            marker.id,
            x - model.border_box.x,
            y - model.border_box.y,
            nodes,
            geometry,
            text,
            engine,
        );
    }
}
