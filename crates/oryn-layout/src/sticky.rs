//! Sticky offsets are paint/geometry translations; normal-flow sizes and sibling
//! positions remain unchanged. All inputs here precede CSS transforms.

/// Inputs for resolve sticky axis.
pub(crate) struct StickyAxis {
    pub(crate) normal: f64,
    pub(crate) size: f64,
    pub(crate) port_start: f64,
    pub(crate) port_size: f64,
    pub(crate) start: Option<f64>,
    pub(crate) end: Option<f64>,
    pub(crate) cb_start: f64,
    pub(crate) cb_size: f64,
    pub(crate) margin_start: f64,
    pub(crate) margin_end: f64,
}
use super::*;

fn resolve_sticky_axis(params: StickyAxis) -> f64 {
    let StickyAxis {
        normal,
        size,
        port_start,
        port_size,
        start,
        end,
        cb_start,
        cb_size,
        margin_start,
        margin_end,
    } = params;

    if start.is_none() && end.is_none() {
        return 0.;
    }
    let start_inset = start.unwrap_or(0.);
    let mut end_inset = end.unwrap_or(0.);
    // A sticky view rectangle must fit the border box; the effective end inset
    // may become negative when the box is taller/wider than its scrollport.
    if port_size - start_inset - end_inset < size {
        end_inset = port_size - start_inset - size;
    }
    let mut target = normal;
    if start.is_some() {
        target = target.max(port_start + start_inset);
    }
    if end.is_some() {
        target = target.min(port_start + port_size - end_inset - size);
    }
    let start_margin = margin_start.min((normal - cb_start).max(0.));
    let end_margin = margin_end.min((cb_start + cb_size - normal - size).max(0.));
    let lower = cb_start + start_margin;
    let upper = cb_start + cb_size - end_margin - size;
    // An oversized box cannot fit inside its containing block. Preserve the
    // original position on an axis where no legal translation interval exists.
    if lower > upper {
        return 0.;
    }
    target.clamp(lower, upper) - normal
}

pub(super) fn translation(
    id: u64,
    input: &LayoutInput,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    boxes: &BTreeMap<u64, BoxModel>,
    metrics: &BTreeMap<u64, oryn_common::v4::PaintScrollGeometry>,
    viewport_port: Option<CssRect>,
) -> (f64, f64) {
    let Some(model) = boxes.get(&id) else {
        return (0., 0.);
    };
    let style = &styles[&id];
    if style_value(style, "position", "static") != "sticky" {
        return (0., 0.);
    }
    let Some(parent) = by_id[&id].parent else {
        return (0., 0.);
    };
    let Some(cb) = boxes.get(&parent) else {
        return (0., 0.);
    };
    let mut ancestor = Some(parent);
    let mut scroll = (0., 0.);
    let mut port = viewport_port.unwrap_or(CssRect {
        x: 0.,
        y: 0.,
        width: input.viewport.width,
        height: input.viewport.height,
    });
    while let Some(id) = ancestor {
        let ancestor_style = &styles[&id];
        let scrollable = |axis| {
            matches!(
                style_value(ancestor_style, axis, "visible"),
                "auto" | "scroll" | "hidden"
            )
        };
        let offsets = metrics
            .get(&id)
            .map(|scroll| (scroll.offset_x, scroll.offset_y))
            .unwrap_or_default();
        if scrollable("overflow-x") {
            scroll.0 += offsets.0;
        }
        if scrollable("overflow-y") {
            scroll.1 += offsets.1;
        }
        if scrollable("overflow-x") || scrollable("overflow-y") {
            if let Some(scroll) = metrics.get(&id) {
                port = scroll.scrollport;
            } else if let Some(model) = boxes.get(&id) {
                port = model.padding_box;
            }
            break;
        }
        ancestor = by_id.get(&id).and_then(|node| node.parent);
    }
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
    let axes = writing_mode::Axes::new(
        style_value(style, "writing-mode", "horizontal-tb"),
        style_value(style, "direction", "ltr") == "rtl",
    );
    let directed = |reverse: bool,
                    normal: f64,
                    size: f64,
                    port_start: f64,
                    port_size: f64,
                    start: Option<f64>,
                    end: Option<f64>,
                    cb_start: f64,
                    cb_size: f64,
                    margin_start: f64,
                    margin_end: f64| {
        if reverse {
            -resolve_sticky_axis(StickyAxis {
                normal: -normal - size,
                size,
                port_start: -port_start - port_size,
                port_size,
                start: end,
                end: start,
                cb_start: -cb_start - cb_size,
                cb_size,
                margin_start: margin_end,
                margin_end: margin_start,
            })
        } else {
            resolve_sticky_axis(StickyAxis {
                normal,
                size,
                port_start,
                port_size,
                start,
                end,
                cb_start,
                cb_size,
                margin_start,
                margin_end,
            })
        }
    };
    (
        directed(
            axes.x_reverse,
            model.border_box.x - scroll.0,
            model.border_box.width,
            port.x,
            port.width,
            inset("left", port.width),
            inset("right", port.width),
            cb.content_box.x - scroll.0,
            cb.content_box.width,
            model.margin.left,
            model.margin.right,
        ),
        directed(
            axes.y_reverse,
            model.border_box.y - scroll.1,
            model.border_box.height,
            port.y,
            port.height,
            inset("top", port.height),
            inset("bottom", port.height),
            cb.content_box.y - scroll.1,
            cb.content_box.height,
            model.margin.top,
            model.margin.bottom,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sticky_axis_pins_then_releases_at_containing_block_end() {
        assert_eq!(
            resolve_sticky_axis(StickyAxis {
                normal: 30.,
                size: 20.,
                port_start: 0.,
                port_size: 100.,
                start: Some(10.),
                end: None,
                cb_start: 0.,
                cb_size: 300.,
                margin_start: 0.,
                margin_end: 0.
            }),
            0.
        );
        assert_eq!(
            resolve_sticky_axis(StickyAxis {
                normal: -30.,
                size: 20.,
                port_start: 0.,
                port_size: 100.,
                start: Some(10.),
                end: None,
                cb_start: -60.,
                cb_size: 300.,
                margin_start: 0.,
                margin_end: 0.
            }),
            40.
        );
        assert_eq!(
            resolve_sticky_axis(StickyAxis {
                normal: -250.,
                size: 20.,
                port_start: 0.,
                port_size: 100.,
                start: Some(10.),
                end: None,
                cb_start: -280.,
                cb_size: 300.,
                margin_start: 0.,
                margin_end: 0.
            }),
            250.
        );
    }
    #[test]
    fn end_inset_and_oversized_view_rectangle_preserve_start_constraint() {
        assert_eq!(
            resolve_sticky_axis(StickyAxis {
                normal: 95.,
                size: 20.,
                port_start: 0.,
                port_size: 100.,
                start: None,
                end: Some(5.),
                cb_start: 0.,
                cb_size: 300.,
                margin_start: 0.,
                margin_end: 0.
            }),
            -20.
        );
        assert_eq!(
            resolve_sticky_axis(StickyAxis {
                normal: -30.,
                size: 150.,
                port_start: 0.,
                port_size: 100.,
                start: Some(10.),
                end: Some(10.),
                cb_start: -60.,
                cb_size: 300.,
                margin_start: 0.,
                margin_end: 0.
            }),
            40.
        );
        assert_eq!(
            resolve_sticky_axis(StickyAxis {
                normal: -30.,
                size: 20.,
                port_start: 0.,
                port_size: 100.,
                start: None,
                end: None,
                cb_start: -60.,
                cb_size: 300.,
                margin_start: 0.,
                margin_end: 0.
            }),
            0.
        );
    }
}
