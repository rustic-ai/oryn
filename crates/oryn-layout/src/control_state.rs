//! Native control values normalized independently of theme/style decisions.
use super::LayoutNode;
use oryn_common::v4::{PaintControl, PaintMeterRegion};
pub fn control_paint_state(node: &LayoutNode) -> Option<PaintControl> {
    if node.namespace != "http://www.w3.org/1999/xhtml" {
        return None;
    }
    let number = |name: &str| {
        node.attributes
            .get(name)
            .and_then(|value| oryn_dom::parse_html_floating_point(value))
    };
    let disabled = node
        .disabled
        .unwrap_or_else(|| node.attributes.contains_key("disabled"));
    match node.tag.as_str() {
        "meter" => {
            let min = number("min").unwrap_or(0.);
            let max = number("max").unwrap_or(1.).max(min);
            let value = number("value").unwrap_or(0.).clamp(min, max);
            let low = number("low").unwrap_or(min).clamp(min, max);
            let high = number("high").unwrap_or(max).clamp(low, max);
            let optimum = number("optimum")
                .unwrap_or(min / 2. + max / 2.)
                .clamp(min, max);
            let region = if optimum < low {
                if value <= low {
                    PaintMeterRegion::Optimum
                } else if value <= high {
                    PaintMeterRegion::Suboptimal
                } else {
                    PaintMeterRegion::EvenLessGood
                }
            } else if optimum > high {
                if value >= high {
                    PaintMeterRegion::Optimum
                } else if value >= low {
                    PaintMeterRegion::Suboptimal
                } else {
                    PaintMeterRegion::EvenLessGood
                }
            } else if value < low || value > high {
                PaintMeterRegion::Suboptimal
            } else {
                PaintMeterRegion::Optimum
            };
            Some(PaintControl::Meter {
                fraction: fraction(value, min, max),
                region,
            })
        }
        "progress" => {
            let max = number("max").filter(|max| *max > 0.).unwrap_or(1.);
            Some(PaintControl::Progress {
                fraction: number("value").map(|value| (value.clamp(0., max) / max).clamp(0., 1.)),
            })
        }
        "select" => {
            let multiple = node.attributes.contains_key("multiple");
            Some(PaintControl::Select {
                active_option: None,
                open: false,
                multiple,
                disabled,
                selected: Vec::new(),
                label: String::new(),
                display_size: node
                    .attributes
                    .get("size")
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|size| *size > 0)
                    .unwrap_or(if multiple { 4 } else { 1 }),
            })
        }
        "input" => match node
            .attributes
            .get("type")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_default()
            .as_str()
        {
            "checkbox" => Some(PaintControl::Checkbox {
                checked: node
                    .checked
                    .unwrap_or_else(|| node.attributes.contains_key("checked")),
                indeterminate: node.indeterminate,
                disabled,
            }),
            "radio" => Some(PaintControl::Radio {
                checked: node
                    .checked
                    .unwrap_or_else(|| node.attributes.contains_key("checked")),
                disabled,
            }),
            "color" => {
                let value = node
                    .attributes
                    .get("value")
                    .map(String::as_str)
                    .unwrap_or("#000000");
                Some(PaintControl::Color {
                    rgba: oryn_dom::input_color_rgba8(value),
                    disabled,
                })
            }
            "range" => {
                let min = number("min").unwrap_or(0.);
                let max = number("max").unwrap_or(100.).max(min);
                let value = number("value")
                    .unwrap_or(min / 2. + max / 2.)
                    .clamp(min, max);
                Some(PaintControl::Range {
                    fraction: fraction(value, min, max),
                    disabled,
                    ticks: Vec::new(),
                })
            }
            _ => None,
        },
        _ => None,
    }
}
pub(super) fn fraction(value: f64, min: f64, max: f64) -> f64 {
    if min == max {
        return 0.;
    }
    let span = max - min;
    let result = if span.is_finite() {
        (value - min) / span
    } else {
        (value / 2. - min / 2.) / (max / 2. - min / 2.)
    };
    if result.is_finite() {
        result.clamp(0., 1.)
    } else {
        0.
    }
}

pub(super) fn options<'a>(
    node: &LayoutNode,
    sources: &'a std::collections::BTreeMap<u64, LayoutNode>,
) -> Vec<&'a LayoutNode> {
    let mut result = Vec::new();
    let mut pending = node.children.iter().rev().copied().collect::<Vec<_>>();
    let mut visited = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(child) = sources.get(&id) else {
            continue;
        };
        if child.tag == "select" {
            continue;
        }
        if child.tag == "option" {
            result.push(child);
        } else {
            pending.extend(child.children.iter().rev().copied());
        }
    }
    result
}

fn option_label(
    node: &LayoutNode,
    sources: &std::collections::BTreeMap<u64, LayoutNode>,
) -> String {
    if let Some(label) = node.attributes.get("label") {
        return label.clone();
    }
    enum Part<'a> {
        Node(&'a LayoutNode),
        Text(&'a str),
    }
    let mut pending = vec![Part::Node(node)];
    let mut text = String::new();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(part) = pending.pop() {
        match part {
            Part::Text(value) => text.push_str(value),
            Part::Node(node) => {
                if !seen.insert(node.id) || node.tag.starts_with("::") || node.tag == "script" {
                    continue;
                }
                if let Some(content) = &node.content {
                    pending.extend(content.iter().rev().filter_map(|item| match item {
                        super::LayoutContent::Text { text, .. }
                        | super::LayoutContent::StyledText { text, .. } => Some(Part::Text(text)),
                        super::LayoutContent::Element(id) => sources.get(id).map(Part::Node),
                    }));
                } else {
                    pending.extend(
                        node.children
                            .iter()
                            .rev()
                            .filter_map(|id| sources.get(id).map(Part::Node)),
                    );
                    pending.push(Part::Text(&node.text));
                }
            }
        }
    }
    text.split(['\t', '\n', '\u{c}', '\r', ' '])
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn measure_selects(
    input: &super::LayoutInput,
    styles: &super::ComputedStyles,
    engine: &mut super::TextEngine,
) {
    use super::*;
    let sources = input
        .nodes
        .iter()
        .map(|node| (node.id, node.clone()))
        .collect::<BTreeMap<_, _>>();
    for node in &input.nodes {
        if node.namespace != "http://www.w3.org/1999/xhtml" || node.tag != "select" {
            continue;
        }
        let Some(style) = styles.get(&node.id) else {
            continue;
        };
        if style_value(style, "appearance", "auto") == "base-select"
            || style_value(style, "field-sizing", "fixed") == "content"
        {
            continue;
        }
        let Some(PaintControl::Select {
            multiple: false,
            display_size: 1,
            ..
        }) = input
            .control_states
            .get(&node.id)
            .cloned()
            .or_else(|| control_paint_state(node))
        else {
            continue;
        };
        let font = length(
            style_value(style, "font-size", "16px"),
            16.,
            16.,
            input.viewport,
        );
        let mut label_style = (**style).clone();
        label_style.insert("white-space".into(), "nowrap".into());
        label_style.insert("text-wrap-mode".into(), "nowrap".into());
        let mut inline = 0f64;
        let mut block = 0f64;
        for option in options(node, &sources) {
            let text = option_label(option, &sources);
            let shaped = engine.shape_styled(&text, 1_000_000., font, &label_style, input.viewport);
            inline = inline.max(shaped.width);
            block = block.max(shaped.height);
        }
        let vertical =
            writing_mode::Axes::new(style_value(style, "writing-mode", "horizontal-tb"), false)
                .vertical;
        engine.select_sizes.insert(
            node.id,
            if vertical {
                (block, inline)
            } else {
                (inline, block)
            },
        );
    }
}

/// The select button and its option list are different formatting trees. The
/// private projection preserves source selectors/DOM while shaping the native
/// selected label through the same text engine as other inline content.
pub(super) fn project_select_buttons(
    input: &mut super::LayoutInput,
    styles: &super::ComputedStyles,
) {
    use super::{LayoutContent, style_value};
    use std::collections::{BTreeMap, BTreeSet};
    let sources = input
        .nodes
        .iter()
        .map(|node| (node.id, node.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut suppressed = BTreeSet::new();
    for node in &mut input.nodes {
        if node.namespace != "http://www.w3.org/1999/xhtml" || node.tag != "select" {
            continue;
        }
        let state = input
            .control_states
            .get(&node.id)
            .cloned()
            .or_else(|| control_paint_state(node));
        let Some(PaintControl::Select {
            multiple,
            display_size,
            label,
            ..
        }) = state
        else {
            continue;
        };
        if multiple || display_size > 1 {
            continue;
        }
        let customizable = styles
            .get(&node.id)
            .is_some_and(|style| style_value(style, "appearance", "auto") == "base-select");
        let button = customizable
            .then(|| {
                node.children.iter().copied().find(|id| {
                    sources
                        .get(id)
                        .is_some_and(|node| !node.tag.starts_with("::"))
                })
            })
            .flatten()
            .filter(|id| sources.get(id).is_some_and(|node| node.tag == "button"));
        let kept = node
            .children
            .iter()
            .copied()
            .filter(|child| {
                Some(*child) == button
                    || (customizable
                        && sources
                            .get(child)
                            .is_some_and(|node| node.tag.starts_with("::")))
            })
            .collect::<Vec<_>>();
        let mut pending = node
            .children
            .iter()
            .copied()
            .filter(|child| !kept.contains(child))
            .collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            if suppressed.insert(id)
                && let Some(child) = sources.get(&id)
            {
                pending.extend(child.children.iter().copied());
            }
        }
        node.children = kept;
        if let Some(button) = button {
            let _ = button;
            node.text.clear();
            node.content = Some(
                node.children
                    .iter()
                    .copied()
                    .map(LayoutContent::Element)
                    .collect(),
            );
        } else {
            let label = if input.control_states.contains_key(&node.id) {
                label
            } else {
                let options = options(&sources[&node.id], &sources);
                options
                    .iter()
                    .find(|option| option.attributes.contains_key("selected"))
                    .or(options.first())
                    .map(|option| option_label(option, &sources))
                    .unwrap_or_default()
            };
            node.text = label.clone();
            let mut content = node
                .children
                .iter()
                .copied()
                .filter(|id| sources.get(id).is_some_and(|node| node.tag == "::before"))
                .map(LayoutContent::Element)
                .collect::<Vec<_>>();
            content.push(LayoutContent::Text {
                node_id: node.id,
                text: label,
            });
            content.extend(
                node.children
                    .iter()
                    .copied()
                    .filter(|id| sources.get(id).is_some_and(|node| node.tag != "::before"))
                    .map(LayoutContent::Element),
            );
            node.content = Some(content);
        }
    }
    input.nodes.retain(|node| !suppressed.contains(&node.id));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_extreme_and_degenerate_ranges_are_finite() {
        assert_eq!(fraction(1.5, 1., 2.), 0.5);
        assert_eq!(fraction(0., -1e308, 1e308), 0.5);
        assert_eq!(fraction(2., 2., 2.), 0.);
    }
}
