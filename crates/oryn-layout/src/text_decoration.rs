//! Decoration propagation is independent of CSS inheritance. Descendant `none`
//! does not remove a decorating ancestor; atomic and out-of-flow boxes stop it.
use super::*;
use oryn_common::v4::PaintTextDecoration;
const PRIVATE: &str = "-oryn-paint-decorations";
fn own(
    style: &BTreeMap<String, String>,
    font_size: f64,
    viewport: ViewportState,
) -> Vec<PaintTextDecoration> {
    let line = style_value(style, "text-decoration-line", "none");
    if !line
        .split_whitespace()
        .any(|line| matches!(line, "underline" | "overline" | "line-through"))
    {
        return Vec::new();
    }
    let size = length(
        style_value(style, "font-size", "16px"),
        font_size,
        font_size,
        viewport,
    );
    let value = |key: &str| {
        let value = style_value(style, key, "auto");
        if matches!(value, "auto" | "from-font") {
            None
        } else {
            let value = length(value, size, size, viewport);
            value.is_finite().then_some(value)
        }
    };
    let color = style_value(style, "text-decoration-color", "currentcolor");
    vec![PaintTextDecoration {
        line: line.into(),
        style: style_value(style, "text-decoration-style", "solid").into(),
        color: oryn_common::color::parse_color(if color.eq_ignore_ascii_case("currentcolor") {
            style_value(style, "color", "black")
        } else {
            color
        })
        .unwrap_or([0, 0, 0, 255]),
        thickness: value("text-decoration-thickness").map(|v| v.max(0.)),
        underline_offset: value("text-underline-offset"),
        position: style_value(style, "text-underline-position", "auto").into(),
        skip_ink: style_value(style, "text-decoration-skip-ink", "auto") != "none",
        font_size: size,
    }]
}
pub(super) fn read(
    style: &BTreeMap<String, String>,
    font_size: f64,
    viewport: ViewportState,
) -> Vec<PaintTextDecoration> {
    style
        .get(PRIVATE)
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_else(|| own(style, font_size, viewport))
}
/// Pseudo decorations propagate in addition to ordinary decorating ancestors.
pub(super) fn add_pseudo(
    target: &mut BTreeMap<String, String>,
    pseudo: &BTreeMap<String, String>,
    viewport: ViewportState,
) {
    let mut decorations = read(target, 16., viewport);
    for decoration in own(pseudo, 16., viewport) {
        if !decorations.contains(&decoration) {
            decorations.push(decoration);
        }
    }
    target.insert(
        PRIVATE.into(),
        serde_json::to_string(&decorations).expect("finite pseudo decorations"),
    );
}
pub(super) fn propagate(input: &LayoutInput, styles: &ComputedStyles) -> ComputedStyles {
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut result = ComputedStyles::new();
    // Explicit ancestor stack handles snapshots whose records are not in tree order.
    for node in &input.nodes {
        let mut chain = Vec::new();
        let mut current = Some(node.id);
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if result.contains_key(&id) || !seen.insert(id) {
                break;
            }
            chain.push(id);
            current = nodes.get(&id).and_then(|node| node.parent);
        }
        for id in chain.into_iter().rev() {
            let Some(style) = styles.get(&id) else {
                continue;
            };
            let barrier = nodes.get(&id).is_some_and(|node| {
                node.namespace == "http://www.w3.org/1999/xhtml"
                    && matches!(
                        node.tag.as_str(),
                        "geolocation" | "usermedia" | "camera" | "install"
                    )
            }) || matches!(
                style_value(style, "position", "static"),
                "absolute" | "fixed"
            ) || style_value(style, "float", "none") != "none"
                || matches!(
                    style_value(style, "display", "block"),
                    "inline-block"
                        | "inline-table"
                        | "inline-flex"
                        | "inline-grid"
                        | "flow-root"
                        | "ruby-text"
                        | "ruby-text-container"
                );
            let mut decorations = if barrier {
                Vec::new()
            } else {
                nodes
                    .get(&id)
                    .and_then(|node| node.parent)
                    .and_then(|id| result.get(&id))
                    .map(|style: &ComputedStyle| read(style, 16., input.viewport))
                    .unwrap_or_default()
            };
            decorations.extend(own(style, 16., input.viewport));
            let mut private = style.as_ref().clone();
            private.insert(
                PRIVATE.into(),
                serde_json::to_string(&decorations).expect("finite decoration metadata"),
            );
            result.insert(id, Arc::new(private));
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn none_does_not_erase_ancestor_and_owner_color_is_preserved() {
        let mut outer = BTreeMap::from([
            ("text-decoration-line".into(), "underline".into()),
            ("color".into(), "red".into()),
            ("font-size".into(), "20px".into()),
        ]);
        let viewport = ViewportState::default();
        let parent = own(&outer, 16., viewport);
        outer.insert("text-decoration-line".into(), "none".into());
        outer.insert("color".into(), "blue".into());
        outer.insert(PRIVATE.into(), serde_json::to_string(&parent).unwrap());
        let child = read(&outer, 16., viewport);
        assert_eq!(child[0].color, [255, 0, 0, 255]);
        assert_eq!(child[0].font_size, 20.);
    }
    #[test]
    fn explicit_overline_thickness_uses_decoration_owner_em() {
        // css/css-text-decor/text-decoration-thickness-overline-001.html
        let style = BTreeMap::from([
            ("text-decoration-line".into(), "overline".into()),
            ("font-size".into(), "20px".into()),
            ("text-decoration-thickness".into(), "4em".into()),
            ("text-decoration-skip-ink".into(), "none".into()),
        ]);
        let decoration = own(&style, 16., ViewportState::default());
        assert_eq!(decoration[0].thickness, Some(80.));
        assert!(!decoration[0].skip_ink);
    }
}
