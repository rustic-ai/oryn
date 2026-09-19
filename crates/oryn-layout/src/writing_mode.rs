//! Physical directions of the logical inline/block axes. Geometry consumers use
//! these directions instead of treating RTL as synonymous with right-to-left x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Axes {
    pub vertical: bool,
    pub x_reverse: bool,
    pub y_reverse: bool,
}
impl Axes {
    pub fn new(mode: &str, rtl: bool) -> Self {
        match mode {
            "vertical-rl" | "sideways-rl" => Self {
                vertical: true,
                x_reverse: true,
                y_reverse: rtl,
            },
            "vertical-lr" => Self {
                vertical: true,
                x_reverse: false,
                y_reverse: rtl,
            },
            "sideways-lr" => Self {
                vertical: true,
                x_reverse: false,
                y_reverse: !rtl,
            },
            _ => Self {
                vertical: false,
                x_reverse: rtl,
                y_reverse: false,
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direction_reverses_inline_axis_without_reversing_block_progression() {
        assert_eq!(
            Axes::new("horizontal-tb", true),
            Axes {
                vertical: false,
                x_reverse: true,
                y_reverse: false
            }
        );
        assert_eq!(
            Axes::new("vertical-rl", false),
            Axes {
                vertical: true,
                x_reverse: true,
                y_reverse: false
            }
        );
        assert_eq!(
            Axes::new("vertical-rl", true),
            Axes {
                vertical: true,
                x_reverse: true,
                y_reverse: true
            }
        );
        assert_eq!(
            Axes::new("vertical-lr", true),
            Axes {
                vertical: true,
                x_reverse: false,
                y_reverse: true
            }
        );
        assert_eq!(
            Axes::new("sideways-lr", false),
            Axes {
                vertical: true,
                x_reverse: false,
                y_reverse: true
            }
        );
        assert_eq!(
            Axes::new("sideways-lr", true),
            Axes {
                vertical: true,
                x_reverse: false,
                y_reverse: false
            }
        );
    }
}

use std::collections::BTreeMap;
fn swap(style: &mut BTreeMap<String, String>, a: &str, b: &str) {
    let left = style.remove(a);
    let right = style.remove(b);
    if let Some(value) = right {
        style.insert(a.into(), value);
    }
    if let Some(value) = left {
        style.insert(b.into(), value);
    }
}

/// Taffy receives physical axes while CSS flex/grid declarations use logical
/// axes. Transform the algorithm inputs, not the completed geometry dimensions.
pub(super) fn container(style: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut physical = style.clone();
    let axes = Axes::new(
        crate::style_value(style, "writing-mode", "horizontal-tb"),
        crate::style_value(style, "direction", "ltr") == "rtl",
    );
    if !axes.vertical {
        return physical;
    }
    let display = crate::style_value(style, "display", "block");
    physical.insert("direction".into(), "ltr".into());
    swap(&mut physical, "column-gap", "row-gap");
    if matches!(display, "flex" | "inline-flex") {
        let direction = crate::style_value(style, "flex-direction", "row");
        let row = matches!(direction, "row" | "row-reverse");
        let reverse =
            direction.ends_with("reverse") ^ if row { axes.y_reverse } else { axes.x_reverse };
        physical.insert(
            "flex-direction".into(),
            match (row, reverse) {
                (true, false) => "column",
                (true, true) => "column-reverse",
                (false, false) => "row",
                (false, true) => "row-reverse",
            }
            .into(),
        );
        let cross_reverse = if row { axes.x_reverse } else { axes.y_reverse };
        let wrap = crate::style_value(style, "flex-wrap", "nowrap");
        if cross_reverse && wrap != "nowrap" {
            physical.insert(
                "flex-wrap".into(),
                if wrap == "wrap" {
                    "wrap-reverse"
                } else {
                    "wrap"
                }
                .into(),
            );
        }
    } else if matches!(display, "grid" | "inline-grid") {
        for (a, b) in [
            ("grid-template-columns", "grid-template-rows"),
            ("grid-auto-columns", "grid-auto-rows"),
            ("align-items", "justify-items"),
            ("align-content", "justify-content"),
        ] {
            swap(&mut physical, a, b);
        }
        let flow = crate::style_value(style, "grid-auto-flow", "row");
        physical.insert(
            "grid-auto-flow".into(),
            if flow.contains("column") {
                if flow.contains("dense") {
                    "row dense"
                } else {
                    "row"
                }
            } else if flow.contains("dense") {
                "column dense"
            } else {
                "column"
            }
            .into(),
        );
    }
    physical
}
pub(super) fn item(
    style: &BTreeMap<String, String>,
    parent: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut physical = style.clone();
    let axes = Axes::new(
        crate::style_value(parent, "writing-mode", "horizontal-tb"),
        crate::style_value(parent, "direction", "ltr") == "rtl",
    );
    if !axes.vertical {
        return physical;
    }
    physical.insert("direction".into(), "ltr".into());
    if matches!(
        crate::style_value(parent, "display", "block"),
        "grid" | "inline-grid"
    ) {
        for (a, b) in [
            ("grid-column-start", "grid-row-start"),
            ("grid-column-end", "grid-row-end"),
            ("align-self", "justify-self"),
        ] {
            swap(&mut physical, a, b);
        }
    }
    physical
}
