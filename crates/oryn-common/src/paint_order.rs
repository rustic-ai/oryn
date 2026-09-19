//! CSS order-modified sibling order shared by layout, painting and hit testing.

pub fn sort_siblings<T>(children: &mut [T], parent_display: &str, order: impl FnMut(&T) -> i32) {
    if matches!(
        parent_display,
        "flex" | "inline-flex" | "grid" | "inline-grid"
    ) {
        children.sort_by_key(order);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_top_layer_escapes_ancestor_context_and_obeys_insertion_order() {
        let ordinary = BTreeMap::new();
        let high = [
            ("position".into(), "relative".into()),
            ("z-index".into(), "2147483647".into()),
        ]
        .into();
        let nodes = [
            StackingNode {
                top_layer: None,
                node_id: 1,
                parent_id: None,
                style: &ordinary,
            },
            StackingNode {
                top_layer: None,
                node_id: 2,
                parent_id: Some(1),
                style: &high,
            },
            StackingNode {
                top_layer: Some(1),
                node_id: 3,
                parent_id: Some(1),
                style: &ordinary,
            },
            StackingNode {
                top_layer: Some(0),
                node_id: 4,
                parent_id: Some(2),
                style: &ordinary,
            },
            StackingNode {
                top_layer: None,
                node_id: 5,
                parent_id: Some(4),
                style: &high,
            },
        ];
        assert_eq!(stacking_order(&nodes), vec![1, 2, 4, 5, 3]);
    }

    #[test]
    fn sorting_is_stable_and_only_applies_to_flex_and_grid() {
        for display in [
            "flex",
            "inline-flex",
            "grid",
            "inline-grid",
            "block",
            "table-row",
        ] {
            let mut children = [(1, 2), (2, -1), (3, -1), (4, 0)];
            sort_siblings(&mut children, display, |item| item.1);
            let ids = children.map(|item| item.0);
            assert_eq!(
                ids,
                if matches!(display, "block" | "table-row") {
                    [1, 2, 3, 4]
                } else {
                    [2, 3, 4, 1]
                }
            );
        }
    }
}

// Stacking contexts are atomic; ordinary boxes promote descendant contexts.
use std::collections::{BTreeMap, BTreeSet};
/// A borrowed element record in flat-tree order. No renderer dependency is needed.
pub struct StackingNode<'a> {
    pub top_layer: Option<usize>,
    pub node_id: u64,
    pub parent_id: Option<u64>,
    pub style: &'a BTreeMap<String, String>,
}

fn opacity(style: &BTreeMap<String, String>) -> f64 {
    style
        .get("opacity")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(1.)
        .clamp(0., 1.)
}

fn value<'a>(node: &'a StackingNode<'_>, name: &str, default: &'a str) -> &'a str {
    node.style.get(name).map(String::as_str).unwrap_or(default)
}
fn positioned(node: &StackingNode<'_>) -> bool {
    value(node, "position", "static") != "static"
}
fn level(node: &StackingNode<'_>, parent: Option<&StackingNode<'_>>) -> Option<i64> {
    let item = parent.is_some_and(|p| {
        matches!(
            value(p, "display", "block"),
            "flex" | "inline-flex" | "grid" | "inline-grid"
        )
    });
    (positioned(node) || item)
        .then(|| value(node, "z-index", "auto").parse().ok())
        .flatten()
}
fn context(node: &StackingNode<'_>, parent: Option<&StackingNode<'_>>) -> bool {
    if value(node, "display", "block") == "contents" {
        return false;
    }
    level(node, parent).is_some()
        || matches!(value(node, "position", "static"), "fixed" | "sticky")
        || opacity(node.style) < 1.
        || [
            "transform",
            "translate",
            "rotate",
            "scale",
            "perspective",
            "filter",
            "backdrop-filter",
            "clip-path",
            "mask-image",
            "mask-border-source",
        ]
        .iter()
        .any(|key| value(node, key, "none") != "none")
        || value(node, "isolation", "auto") == "isolate"
        || value(node, "mix-blend-mode", "normal") != "normal"
        || value(node, "contain", "none")
            .split_whitespace()
            .any(|v| matches!(v, "layout" | "paint" | "strict" | "content"))
        || value(node, "will-change", "auto").split(',').any(|v| {
            matches!(
                v.trim(),
                "opacity"
                    | "transform"
                    | "filter"
                    | "perspective"
                    | "clip-path"
                    | "mix-blend-mode"
                    | "isolation"
            )
        })
}

#[derive(Clone, Copy)]
pub struct StackingItem {
    pub id: u64,
    phase: u8,
    level: i64,
    pub nested: bool,
}
pub struct StackingContext {
    pub id: u64,
    pub items: Vec<StackingItem>,
}

/// Return the element painting order. Reversing this list gives the front-to-back
/// order for element hit testing; visibility and pointer-events remain caller rules.
pub fn stacking_order(input: &[StackingNode<'_>]) -> Vec<u64> {
    let (contexts, roots) = stacking_contexts(input);
    let mut order = Vec::new();
    let mut pending = roots
        .into_iter()
        .rev()
        .map(|id| (id, true))
        .collect::<Vec<_>>();
    while let Some((id, nested)) = pending.pop() {
        order.push(id);
        if nested && let Some(ctx) = contexts.get(&id) {
            pending.extend(ctx.items.iter().rev().map(|item| (item.id, item.nested)));
        }
    }
    order
}

pub fn stacking_contexts(input: &[StackingNode<'_>]) -> (BTreeMap<u64, StackingContext>, Vec<u64>) {
    let nodes = input
        .iter()
        .map(|n| (n.node_id, n))
        .collect::<BTreeMap<_, _>>();
    let mut children = BTreeMap::<u64, Vec<u64>>::new();
    let mut roots = Vec::new();
    for node in input {
        if let Some(parent) = node
            .parent_id
            .filter(|id| node.top_layer.is_none() && nodes.contains_key(id) && *id != node.node_id)
        {
            children.entry(parent).or_default().push(node.node_id);
        } else {
            roots.push(node.node_id);
        }
    }
    // The top layer is a sequence of independent stacking contexts painted
    // above all document contexts. CSS z-index does not reorder this sequence.
    roots.sort_by_key(|id| {
        nodes[id]
            .top_layer
            .map(|order| (1, order))
            .unwrap_or((0, 0))
    });
    for (parent, siblings) in &mut children {
        sort_siblings(siblings, value(nodes[parent], "display", "block"), |id| {
            value(nodes[id], "order", "0").parse::<i32>().unwrap_or(0)
        });
    }
    let mut contexts = BTreeMap::<u64, StackingContext>::new();
    let mut visited = BTreeSet::new();
    let mut pending = roots
        .iter()
        .rev()
        .map(|id| (*id, *id, 0_u8, true))
        .collect::<Vec<_>>();
    while let Some((id, owner, inherited_phase, root)) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let node = nodes[&id];
        let parent = node.parent_id.and_then(|id| nodes.get(&id).copied());
        let is_context = root || context(node, parent);
        let own_level = level(node, parent).unwrap_or(0);
        let phase = if is_context {
            if own_level < 0 {
                0
            } else if own_level > 0 {
                5
            } else {
                4
            }
        } else if positioned(node) {
            4
        } else if inherited_phase >= 2 {
            inherited_phase
        } else if value(node, "float", "none") != "none" {
            2
        } else if matches!(
            value(node, "display", "block"),
            "inline" | "inline-block" | "inline-table"
        ) {
            3
        } else {
            1
        };
        if !root {
            contexts
                .get_mut(&owner)
                .expect("ancestor context")
                .items
                .push(StackingItem {
                    id,
                    phase,
                    level: own_level,
                    nested: is_context,
                });
        }
        if is_context {
            contexts.insert(
                id,
                StackingContext {
                    id,
                    items: Vec::new(),
                },
            );
        }
        let next_owner = if is_context { id } else { owner };
        let next_phase = if is_context { 0 } else { phase };
        if let Some(children) = children.get(&id) {
            pending.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (*child, next_owner, next_phase, false)),
            );
        }
    }
    for ctx in contexts.values_mut() {
        // Stable sort preserves order-modified tree order at equal stack levels.
        ctx.items.sort_by_key(|item| {
            (
                item.phase,
                if matches!(item.phase, 0 | 5) {
                    item.level
                } else {
                    0
                },
            )
        });
    }
    (contexts, roots)
}
