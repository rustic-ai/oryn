//! Renderer adapter for the shared stacking-context planner.
use crate::{CssRect, PaintCommand, SvgEnvironment};
use oryn_common::paint_order::{StackingNode, stacking_contexts};
use oryn_common::v4::{PaintNodeSnapshot, PaintSnapshot};
use std::collections::BTreeMap;

fn value<'a>(node: &'a PaintNodeSnapshot, name: &str, default: &'a str) -> &'a str {
    node.style.get(name).map(String::as_str).unwrap_or(default)
}
fn record(node: &PaintNodeSnapshot) -> StackingNode<'_> {
    StackingNode {
        top_layer: node.top_layer,
        node_id: node.node_id,
        parent_id: node.parent_id,
        style: &node.style,
    }
}
fn nodes(snapshot: &PaintSnapshot) -> Vec<StackingNode<'_>> {
    let mut nodes = snapshot
        .nodes
        .iter()
        .filter(|node| !matches!(node.tag.as_str(), "::before" | "::after"))
        .map(record)
        .collect::<Vec<_>>();
    for node in snapshot
        .nodes
        .iter()
        .filter(|node| matches!(node.tag.as_str(), "::before" | "::after"))
    {
        if node.tag == "::before" {
            if let Some(index) = nodes
                .iter()
                .position(|candidate| Some(candidate.node_id) == node.parent_id)
            {
                nodes.insert(index + 1, record(node));
            } else {
                nodes.push(record(node));
            }
        } else {
            nodes.push(record(node));
        }
    }
    nodes
}
pub fn paint_order(snapshot: &PaintSnapshot) -> Vec<u64> {
    oryn_common::paint_order::stacking_order(&nodes(snapshot))
}
pub(crate) fn append(
    snapshot: &PaintSnapshot,
    mut own: BTreeMap<u64, Vec<PaintCommand>>,
    mut backgrounds: BTreeMap<u64, Vec<PaintCommand>>,
    mut outlines: BTreeMap<u64, Vec<PaintCommand>>,
    mut backdrops: BTreeMap<u64, Vec<PaintCommand>>,
    out: &mut Vec<PaintCommand>,
) -> Result<(), crate::RenderError> {
    let (contexts, roots) = stacking_contexts(&nodes(snapshot));
    let nodes = snapshot
        .nodes
        .iter()
        .map(|n| (n.node_id, n))
        .collect::<BTreeMap<_, _>>();
    // CSS table backgrounds form layers across the whole grid. A spanning
    // cell background must remain above every row background it crosses.
    // Only static table parts sharing the table's context participate; opacity,
    // transforms and positioned parts keep their own atomic paint context.
    let mut owners = BTreeMap::new();
    for context in contexts.values() {
        owners.insert(context.id, context.id);
        for item in &context.items {
            if !item.nested {
                owners.insert(item.id, context.id);
            }
        }
    }
    let mut layers = BTreeMap::<u64, Vec<(u8, usize, u64)>>::new();
    for (order, node) in snapshot.nodes.iter().enumerate() {
        if value(node, "position", "static") != "static" || !backgrounds.contains_key(&node.node_id)
        {
            continue;
        }
        let rank = match value(node, "display", "block") {
            "table-column-group" => 0,
            "table-column" => 1,
            "table-header-group" | "table-row-group" | "table-footer-group" => 2,
            "table-row" => 3,
            "table-cell" => 4,
            _ => continue,
        };
        let mut current = node.parent_id;
        let mut visited = std::collections::BTreeSet::new();
        while let Some(id) = current {
            if !visited.insert(id) {
                break;
            }
            let Some(parent) = nodes.get(&id) else {
                break;
            };
            if matches!(value(parent, "display", "block"), "table" | "inline-table") {
                if owners.get(&node.node_id) == owners.get(&id)
                    && owners.get(&node.node_id) != Some(&node.node_id)
                {
                    layers
                        .entry(id)
                        .or_default()
                        .push((rank, order, node.node_id));
                }
                break;
            }
            current = parent.parent_id;
        }
    }
    for (table, mut parts) in layers {
        parts.sort_by_key(|&(rank, order, _)| (rank, order));
        let mut prefix = backgrounds.remove(&table).unwrap_or_default();
        for (_, _, id) in parts {
            prefix.extend(backgrounds.remove(&id).unwrap_or_default());
        }
        prefix.extend(own.remove(&table).unwrap_or_default());
        own.insert(table, prefix);
    }
    for (id, mut background) in backgrounds {
        background.extend(own.remove(&id).unwrap_or_default());
        own.insert(id, background);
    }
    let mut complete = BTreeMap::<u64, Vec<PaintCommand>>::new();
    let mut pending = roots
        .iter()
        .rev()
        .map(|id| (*id, false))
        .collect::<Vec<_>>();
    while let Some((id, exit)) = pending.pop() {
        let Some(ctx) = contexts.get(&id) else {
            continue;
        };
        if !exit {
            pending.push((id, true));
            pending.extend(
                ctx.items
                    .iter()
                    .rev()
                    .filter(|item| item.nested)
                    .map(|item| (item.id, false)),
            );
            continue;
        }
        let mut commands = own.remove(&ctx.id).unwrap_or_default();
        if let Some(source) = nodes[&id]
            .style
            .get("backdrop-filter")
            .filter(|source| source.as_str() != "none")
        {
            let node = nodes[&id];
            let model = node
                .projection
                .as_ref()
                .map(|p| &p.box_model)
                .or(node.geometry.box_model.as_ref())
                .ok_or_else(|| crate::RenderError::Encode("backdrop geometry missing".into()))?;
            let plane = node.projection.as_ref().map(|p| p.matrix).unwrap_or([
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ]);
            commands.insert(
                0,
                PaintCommand::BackdropFilter {
                    environment: SvgEnvironment::for_style(
                        &node.style,
                        model.border_box,
                        snapshot.viewport,
                    ),
                    source: source.clone(),
                    current_color: value(node, "color", "black").into(),
                    plane,
                    rect: model.border_box,
                    radii: crate::borders::radii(&node.style, model.border_box),
                },
            );
        }

        for item in &ctx.items {
            commands.extend(
                if item.nested {
                    complete.remove(&item.id)
                } else {
                    own.remove(&item.id)
                }
                .unwrap_or_default(),
            );
        }
        // Appendix E permits outlines at the final phase of their stacking
        // context. Nested contexts keep their outlines inside their own group.
        commands.extend(outlines.remove(&ctx.id).unwrap_or_default());
        for item in ctx.items.iter().filter(|item| !item.nested) {
            commands.extend(outlines.remove(&item.id).unwrap_or_default());
        }
        if let Some(source) = nodes[&id]
            .style
            .get("filter")
            .filter(|source| source.as_str() != "none")
        {
            let plane = nodes[&id]
                .projection
                .as_ref()
                .map(|projection| projection.matrix)
                .unwrap_or([
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                ]);
            commands = vec![PaintCommand::Filter {
                environment: SvgEnvironment::for_style(
                    &nodes[&id].style,
                    nodes[&id]
                        .projection
                        .as_ref()
                        .map(|p| p.box_model.border_box)
                        .or_else(|| {
                            nodes[&id]
                                .geometry
                                .box_model
                                .as_ref()
                                .map(|model| model.border_box)
                        })
                        .unwrap_or(CssRect {
                            x: 0.,
                            y: 0.,
                            width: snapshot.viewport.width,
                            height: snapshot.viewport.height,
                        }),
                    snapshot.viewport,
                ),
                reference_box: nodes[&id]
                    .projection
                    .as_ref()
                    .map(|p| p.box_model.border_box)
                    .or_else(|| {
                        nodes[&id]
                            .geometry
                            .box_model
                            .as_ref()
                            .map(|model| model.border_box)
                    }),
                source: source.clone(),
                current_color: value(nodes[&id], "color", "black").into(),
                plane,
                commands,
            }];
        }
        let masks = crate::masks::layers(snapshot, nodes[&id])?;
        if !masks.is_empty() {
            commands = vec![PaintCommand::Mask {
                layers: masks,
                commands,
            }];
        }
        let alpha = if value(nodes[&id], "display", "block") == "contents" {
            1.
        } else {
            crate::opacity(&nodes[&id].style)
        };
        if alpha == 0. {
            commands.clear();
        } else if alpha < 1. && !commands.is_empty() {
            commands = vec![PaintCommand::Opacity {
                opacity: alpha,
                commands,
            }];
        }
        complete.insert(id, commands);
    }
    for id in roots {
        out.extend(backdrops.remove(&id).unwrap_or_default());
        out.extend(complete.remove(&id).unwrap_or_default());
    }
    Ok(())
}
