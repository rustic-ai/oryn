//! Native classic scrollbar gutters. Overlay mode has zero layout cost.
use super::*;
pub(super) fn gutter(
    style: &BTreeMap<String, String>,
    width: f64,
    overflow: (bool, bool),
) -> EdgeSizes {
    let width = match style_value(style, "scrollbar-width", "auto") {
        "none" => 0.,
        "thin" => width / 2.,
        _ => width,
    };
    let stable = style_value(style, "scrollbar-gutter", "auto")
        .split_whitespace()
        .any(|part| part == "stable");
    let both = style_value(style, "scrollbar-gutter", "auto")
        .split_whitespace()
        .any(|part| part == "both-edges");
    let vertical = style_value(style, "writing-mode", "horizontal-tb") != "horizontal-tb";
    let axis = |name, over, inline| match style_value(style, name, "visible") {
        "scroll" => true,
        "auto" => over || (stable && inline),
        "hidden" => stable && inline,
        _ => false,
    };
    let mut gutter = EdgeSizes::default();
    let x = axis("overflow-x", overflow.0, vertical);
    let y = axis("overflow-y", overflow.1, !vertical);
    if y {
        if style_value(style, "direction", "ltr") == "rtl" && !vertical {
            gutter.left = width;
        } else {
            gutter.right = width;
        }
    }
    if x {
        gutter.bottom = width;
    }
    // both-edges mirrors the inline-edge gutter, not the block-edge scrollbar.
    if stable && both {
        if vertical && x {
            gutter.top = width;
        } else if !vertical && y {
            gutter.left = width;
            gutter.right = width;
        }
    }
    gutter
}
pub(super) fn merge(a: EdgeSizes, b: EdgeSizes) -> EdgeSizes {
    EdgeSizes {
        left: a.left.max(b.left),
        right: a.right.max(b.right),
        top: a.top.max(b.top),
        bottom: a.bottom.max(b.bottom),
    }
}
pub(super) fn viewport_style(
    input: &LayoutInput,
    styles: &ComputedStyles,
) -> Option<(u64, u64, BTreeMap<String, String>)> {
    let root = input
        .nodes
        .iter()
        .find(|node| node.parent.is_none() && node.tag == "html")?;
    let mut style = styles.get(&root.id)?.as_ref().clone();
    let mut source = root.id;
    if ["overflow-x", "overflow-y"]
        .iter()
        .all(|axis| style_value(&style, axis, "visible") == "visible")
    {
        if let Some(body) = input
            .nodes
            .iter()
            .find(|node| node.parent == Some(root.id) && node.tag == "body")
        {
            if let Some(body_style) = styles.get(&body.id) {
                if !matches!(
                    style_value(body_style, "display", "block"),
                    "none" | "contents"
                ) {
                    for axis in ["overflow-x", "overflow-y"] {
                        style.insert(axis.into(), style_value(body_style, axis, "visible").into());
                    }
                    source = body.id;
                }
            }
        }
    }
    for axis in ["overflow-x", "overflow-y"] {
        let value = style_value(&style, axis, "visible");
        if matches!(value, "visible" | "clip") {
            style.insert(
                axis.into(),
                if value == "visible" { "auto" } else { "hidden" }.into(),
            );
        }
    }
    Some((root.id, source, style))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_none_thin_and_stable_have_distinct_native_gutters() {
        let mut style = BTreeMap::from([
            ("overflow-y".into(), "auto".into()),
            ("scrollbar-gutter".into(), "stable both-edges".into()),
        ]);
        assert_eq!(gutter(&style, 0., (false, false)), EdgeSizes::default());
        assert_eq!(gutter(&style, 16., (false, false)).left, 16.);
        style.insert("scrollbar-width".into(), "thin".into());
        assert_eq!(gutter(&style, 16., (false, false)).right, 8.);
        style.insert("scrollbar-width".into(), "none".into());
        assert_eq!(gutter(&style, 16., (true, true)), EdgeSizes::default());
    }
}
