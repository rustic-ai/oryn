//! CSS page margin variable-axis sizing. Intrinsic measures come from the same
//! native paragraph shaper used to paint the eventual margin content.
use super::*;
#[derive(Clone, Copy, Default)]
struct Measure {
    min: f64,
    max: f64,
    fixed: Option<f64>,
}
impl Measure {
    fn twice(self) -> Self {
        Self {
            min: self.min * 2.,
            max: self.max * 2.,
            fixed: self.fixed.map(|v| v * 2.),
        }
    }
}
fn pair(available: f64, a: Measure, b: Measure) -> (f64, f64) {
    if let (Some(a), Some(b)) = (a.fixed, b.fixed) {
        return (a, b);
    }
    if let Some(a) = a.fixed {
        return (a, (available - a).max(0.));
    }
    if let Some(b) = b.fixed {
        return ((available - b).max(0.), b);
    }
    let min = a.min + b.min;
    let max = a.max + b.max;
    let (base_a, base_b, flex_a, flex_b) = if available < min {
        (a.min, b.min, a.min, b.min)
    } else if available < max {
        (a.min, b.min, a.max - a.min, b.max - b.min)
    } else {
        (a.max, b.max, a.max, b.max)
    };
    let free = available - base_a - base_b;
    let flex = flex_a + flex_b;
    let share = if flex > 0. { flex_a / flex } else { 0.5 };
    (
        (base_a + free * share).max(0.),
        (base_b + free * (1. - share)).max(0.),
    )
}
fn variable_sizes(available: f64, measures: [Option<Measure>; 3]) -> [f64; 3] {
    match measures {
        [left, None, right] => {
            let (l, r) = pair(
                available,
                left.unwrap_or(Measure {
                    fixed: Some(0.),
                    ..Default::default()
                }),
                right.unwrap_or(Measure {
                    fixed: Some(0.),
                    ..Default::default()
                }),
            );
            [l, 0., r]
        }
        [left, Some(center), right] => {
            let left = left.unwrap_or(Measure {
                fixed: Some(0.),
                ..Default::default()
            });
            let right = right.unwrap_or(Measure {
                fixed: Some(0.),
                ..Default::default()
            });
            let double = pair(available, left.twice(), center)
                .0
                .max(pair(available, right.twice(), center).0);
            let c = center.fixed.unwrap_or((available - double).max(0.));
            let side = (available - c).max(0.) / 2.;
            [left.fixed.unwrap_or(side), c, right.fixed.unwrap_or(side)]
        }
    }
}
pub(super) fn layout(
    styles: &BTreeMap<String, BTreeMap<String, String>>,
    dimensions: PrintDimensions,
    engine: &mut TextEngine,
    page: usize,
    total: usize,
) -> BTreeMap<String, oryn_common::v4::BoxModel> {
    let c = dimensions.content;
    let viewport = ViewportState {
        width: dimensions.width,
        height: dimensions.height,
        ..Default::default()
    };
    let mut result = BTreeMap::new();
    for (edge, names, rect, horizontal) in [
        (
            "top",
            ["top-left", "top-center", "top-right"],
            CssRect {
                x: c.x,
                y: 0.,
                width: c.width,
                height: c.y,
            },
            true,
        ),
        (
            "bottom",
            ["bottom-left", "bottom-center", "bottom-right"],
            CssRect {
                x: c.x,
                y: c.bottom(),
                width: c.width,
                height: dimensions.height - c.bottom(),
            },
            true,
        ),
        (
            "left",
            ["left-top", "left-middle", "left-bottom"],
            CssRect {
                x: 0.,
                y: c.y,
                width: c.x,
                height: c.height,
            },
            false,
        ),
        (
            "right",
            ["right-top", "right-middle", "right-bottom"],
            CssRect {
                x: c.right(),
                y: c.y,
                width: dimensions.width - c.right(),
                height: c.height,
            },
            false,
        ),
    ] {
        let _ = edge;
        let mut measures = [None; 3];
        for (index, name) in names.iter().enumerate() {
            let Some(style) = styles.get(*name) else {
                continue;
            };
            let Some(content) = style
                .get("content")
                .and_then(|v| page_rules::margin_content(v, page + 1, total))
            else {
                continue;
            };
            let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
            let margin = edge_values(style, "margin", rect, font, viewport);
            let padding = edge_values(style, "padding", rect, font, viewport);
            let border = borders(style, font, viewport);
            let extras = if horizontal {
                margin.left
                    + margin.right
                    + padding.left
                    + padding.right
                    + border.left
                    + border.right
            } else {
                margin.top
                    + margin.bottom
                    + padding.top
                    + padding.bottom
                    + border.top
                    + border.bottom
            };
            let (min, max) = if horizontal {
                engine.intrinsic_source(
                    &text_source::TextSource::synthetic(&content),
                    font,
                    style,
                    viewport,
                    &BTreeMap::new(),
                )
            } else {
                let shaped = engine.shape_styled(
                    &content,
                    (rect.width
                        - margin.left
                        - margin.right
                        - padding.left
                        - padding.right
                        - border.left
                        - border.right)
                        .max(0.),
                    font,
                    style,
                    viewport,
                );
                (shaped.height, shaped.height)
            };
            let property = if horizontal { "width" } else { "height" };
            let basis = if horizontal { rect.width } else { rect.height };
            let fixed_extras = if style_value(style, "box-sizing", "content-box") == "border-box" {
                if horizontal {
                    margin.left + margin.right
                } else {
                    margin.top + margin.bottom
                }
            } else {
                extras
            };
            let fixed =
                optional_length(style_value(style, property, "auto"), basis, font, viewport)
                    .map(|v| v + fixed_extras);
            measures[index] = Some(Measure {
                min: min + extras,
                max: max + extras,
                fixed,
            });
        }
        let available = if horizontal { rect.width } else { rect.height };
        let sizes = variable_sizes(available, measures);
        for (index, name) in names.iter().enumerate() {
            let Some(style) = styles.get(*name) else {
                continue;
            };
            if measures[index].is_none() {
                continue;
            }
            let offset = match index {
                0 => 0.,
                1 => (available - sizes[index]) / 2.,
                _ => (available - sizes[index]),
            };
            let slot = if horizontal {
                CssRect {
                    x: rect.x + offset,
                    width: sizes[index],
                    ..rect
                }
            } else {
                CssRect {
                    y: rect.y + offset,
                    height: sizes[index],
                    ..rect
                }
            };
            result.insert(
                (*name).into(),
                box_model(style, slot, rect, horizontal, viewport),
            );
        }
    }
    for (name, rect) in [
        (
            "top-left-corner",
            CssRect {
                x: 0.,
                y: 0.,
                width: c.x,
                height: c.y,
            },
        ),
        (
            "top-right-corner",
            CssRect {
                x: c.right(),
                y: 0.,
                width: dimensions.width - c.right(),
                height: c.y,
            },
        ),
        (
            "bottom-left-corner",
            CssRect {
                x: 0.,
                y: c.bottom(),
                width: c.x,
                height: dimensions.height - c.bottom(),
            },
        ),
        (
            "bottom-right-corner",
            CssRect {
                x: c.right(),
                y: c.bottom(),
                width: dimensions.width - c.right(),
                height: dimensions.height - c.bottom(),
            },
        ),
    ] {
        if let Some(style) = styles.get(name) {
            result.insert(name.into(), box_model(style, rect, rect, true, viewport));
        }
    }
    result
}
fn edge_values(
    style: &BTreeMap<String, String>,
    prefix: &str,
    basis: CssRect,
    font: f64,
    viewport: ViewportState,
) -> EdgeSizes {
    EdgeSizes {
        top: length(
            style_value(style, &format!("{prefix}-top"), "0px"),
            basis.height,
            font,
            viewport,
        ),
        bottom: length(
            style_value(style, &format!("{prefix}-bottom"), "0px"),
            basis.height,
            font,
            viewport,
        ),
        left: length(
            style_value(style, &format!("{prefix}-left"), "0px"),
            basis.width,
            font,
            viewport,
        ),
        right: length(
            style_value(style, &format!("{prefix}-right"), "0px"),
            basis.width,
            font,
            viewport,
        ),
    }
}
fn borders(style: &BTreeMap<String, String>, font: f64, viewport: ViewportState) -> EdgeSizes {
    let side = |name: &str| {
        if matches!(
            style_value(style, &format!("border-{name}-style"), "none"),
            "none" | "hidden"
        ) {
            return 0.;
        }
        match style_value(style, &format!("border-{name}-width"), "medium") {
            "thin" => 1.,
            "medium" => 3.,
            "thick" => 5.,
            value => length(value, 0., font, viewport),
        }
    };
    EdgeSizes {
        top: side("top"),
        right: side("right"),
        bottom: side("bottom"),
        left: side("left"),
    }
}
fn box_model(
    style: &BTreeMap<String, String>,
    slot: CssRect,
    basis: CssRect,
    horizontal: bool,
    viewport: ViewportState,
) -> oryn_common::v4::BoxModel {
    let font = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let mut margin = edge_values(style, "margin", basis, font, viewport);
    let padding = edge_values(style, "padding", basis, font, viewport);
    let border = borders(style, font, viewport);
    let mut width = (slot.width - margin.left - margin.right).max(0.);
    let mut height = (slot.height - margin.top - margin.bottom).max(0.);
    let border_box = style_value(style, "box-sizing", "content-box") == "border-box";
    if let Some(value) = optional_length(
        style_value(style, "width", "auto"),
        basis.width,
        font,
        viewport,
    ) {
        width = value
            + if border_box {
                0.
            } else {
                padding.left + padding.right + border.left + border.right
            };
    }
    if let Some(value) = optional_length(
        style_value(style, "height", "auto"),
        basis.height,
        font,
        viewport,
    ) {
        height = value
            + if border_box {
                0.
            } else {
                padding.top + padding.bottom + border.top + border.bottom
            };
    }
    // Auto margins along the fixed axis absorb free space; variable-axis auto
    // margins remain zero as required by CSS page margin-box sizing.
    let (before, after, available, used) = if horizontal {
        ("top", "bottom", slot.height, height)
    } else {
        ("left", "right", slot.width, width)
    };
    let auto_before = style_value(style, &format!("margin-{before}"), "0px") == "auto";
    let auto_after = style_value(style, &format!("margin-{after}"), "0px") == "auto";
    let (a, b) = if horizontal {
        (&mut margin.top, &mut margin.bottom)
    } else {
        (&mut margin.left, &mut margin.right)
    };
    let free = (available - used - *a - *b).max(0.);
    if auto_before && auto_after {
        *a = free / 2.;
        *b = free / 2.;
    } else if auto_before {
        *a = free;
    } else if auto_after {
        *b = free;
    }
    let border_box = CssRect {
        x: slot.x + margin.left,
        y: slot.y + margin.top,
        width,
        height,
    };
    let padding_box = inset(border_box, border);
    let content_box = inset(padding_box, padding);
    oryn_common::v4::BoxModel {
        border_box,
        padding_box,
        content_box,
        margin,
        border,
        padding,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_margin_intrinsic_flex_examples() {
        let m = |min, max| {
            Some(Measure {
                min,
                max,
                fixed: None,
            })
        };
        assert_eq!(
            variable_sizes(20., [m(1., 1.), None, m(3., 3.)]),
            [5., 0., 15.]
        );
        assert_eq!(
            variable_sizes(20., [m(18., 25.), m(12., 12.), m(6., 6.)]),
            [7.5, 5., 7.5]
        );
        assert_eq!(
            variable_sizes(20., [m(1., 9.), m(1., 17.), m(1., 3.)]),
            [5.25, 9.5, 5.25]
        );
    }
}
