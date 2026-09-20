//! SVG text clips use the same pinned SVG text-to-outline implementation as paint.
//! The input is native text, styles and captured font bytes; no script or URL IO.
use super::*;
use oryn_common::basic_shape::{self, Shape};
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}
fn serialize(
    input: &LayoutInput,
    node: &LayoutNode,
    styles: &ComputedStyles,
    root: bool,
    out: &mut String,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 || out.len() > 4 * 1024 * 1024 {
        return Err("SVG clip text serialization budget".into());
    }
    if node.tag == "#text" {
        out.push_str(&escape(&node.text));
        return Ok(());
    }
    if !matches!(node.tag.as_str(), "text" | "tspan" | "textPath" | "a") {
        return Ok(());
    }
    out.push('<');
    out.push_str(&node.tag);
    for (name, value) in &node.attributes {
        if !matches!(
            name.as_str(),
            "id" | "x"
                | "y"
                | "dx"
                | "dy"
                | "rotate"
                | "textLength"
                | "lengthAdjust"
                | "href"
                | "startOffset"
                | "method"
                | "spacing"
                | "side"
                | "transform"
        ) || root && name == "transform"
        {
            continue;
        }
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&escape(value));
        out.push('"');
    }
    for attr in &node.namespaced_attributes {
        if attr.namespace == "http://www.w3.org/1999/xlink" && attr.local_name == "href" {
            out.push_str(" xlink:href=\"");
            out.push_str(&escape(&attr.value));
            out.push('"');
        }
    }
    out.push_str(" style=\"");
    if let Some(style) = styles.get(&node.id) {
        for (name, value) in style.iter() {
            if name.starts_with("font-")
                || matches!(
                    name.as_str(),
                    "direction"
                        | "unicode-bidi"
                        | "writing-mode"
                        | "text-orientation"
                        | "text-anchor"
                        | "dominant-baseline"
                        | "alignment-baseline"
                        | "baseline-shift"
                        | "letter-spacing"
                        | "word-spacing"
                        | "white-space"
                        | "visibility"
                )
            {
                out.push_str(name);
                out.push(':');
                out.push_str(&escape(value));
                out.push(';');
            }
        }
    }
    out.push_str("fill:black;stroke:none;opacity:1\">");
    if let Some(content) = &node.content {
        for item in content {
            match item {
                LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                    out.push_str(&escape(text))
                }
                LayoutContent::Element(id) => {
                    if let Some(child) = input.nodes.iter().find(|child| child.id == *id) {
                        serialize(input, child, styles, false, out, depth + 1)?
                    }
                }
            }
        }
    } else {
        out.push_str(&escape(&node.text));
        for id in &node.children {
            if let Some(child) = input.nodes.iter().find(|child| child.id == *id) {
                serialize(input, child, styles, false, out, depth + 1)?
            }
        }
    }
    out.push_str("</");
    out.push_str(&node.tag);
    out.push('>');
    Ok(())
}
pub(super) fn paths(
    group: &usvg::Group,
    output: &mut Vec<Shape>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 || output.len() > 4096 {
        return Err("SVG clip glyph budget".into());
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => paths(group, output, depth + 1)?,
            usvg::Node::Text(text) => paths(text.flattened(), output, depth + 1)?,
            usvg::Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }
                let mut data = String::new();
                for segment in path.data().segments() {
                    use usvg::tiny_skia_path::PathSegment::*;
                    match segment {
                        MoveTo(p) => data.push_str(&format!("M{} {}", p.x, p.y)),
                        LineTo(p) => data.push_str(&format!("L{} {}", p.x, p.y)),
                        QuadTo(a, b) => data.push_str(&format!("Q{} {} {} {}", a.x, a.y, b.x, b.y)),
                        CubicTo(a, b, c) => data
                            .push_str(&format!("C{} {} {} {} {} {}", a.x, a.y, b.x, b.y, c.x, c.y)),
                        Close => data.push('Z'),
                    }
                    if data.len() > 4 * 1024 * 1024 {
                        return Err("SVG clip glyph outline budget".into());
                    }
                }
                let even = path
                    .fill()
                    .is_some_and(|fill| fill.rule() == usvg::FillRule::EvenOdd);
                let source = format!(
                    "path({}, {})",
                    if even { "evenodd" } else { "nonzero" },
                    serde_json::to_string(&data).map_err(|e| e.to_string())?
                );
                let shape = basic_shape::parse(
                    &source,
                    CssRect {
                        x: 0.,
                        y: 0.,
                        width: 1.,
                        height: 1.,
                    },
                    16.,
                    (1., 1.),
                )?
                .ok_or("SVG text outline absent")?;
                let m = path.abs_transform();
                output.push(Shape::Affine {
                    matrix: [
                        m.sx as f64,
                        m.ky as f64,
                        m.kx as f64,
                        m.sy as f64,
                        m.tx as f64,
                        m.ty as f64,
                    ],
                    shape: Box::new(shape),
                });
            }
            usvg::Node::Image(_) => return Err("image is not a text clip outline".into()),
        }
    }
    Ok(())
}
pub(super) fn resolve(
    input: &LayoutInput,
    node: &LayoutNode,
    reference: CssRect,
    styles: &ComputedStyles,
) -> Result<Shape, String> {
    let mut xml = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{}\"><defs>",
        reference.width.max(1.),
        reference.height.max(1.)
    );
    // textPath references remain confined to the original native tree scope.
    for path in input.nodes.iter().filter(|other| {
        other.tree_scope == node.tree_scope
            && other.namespace == "http://www.w3.org/2000/svg"
            && other.tag == "path"
    }) {
        if let (Some(id), Some(d)) = (path.attributes.get("id"), path.attributes.get("d")) {
            xml.push_str(&format!("<path id=\"{}\" d=\"{}\"", escape(id), escape(d)));
            if let Some(transform) = path.attributes.get("transform") {
                xml.push_str(&format!(" transform=\"{}\"", escape(transform)));
            }
            xml.push_str("/>");
        }
        if xml.len() > 4 * 1024 * 1024 {
            return Err("SVG text path reference budget".into());
        }
    }
    xml.push_str("</defs>");
    serialize(input, node, styles, true, &mut xml, 0)?;
    xml.push_str("</svg>");
    let mut options = usvg::Options {
        font_family: DEFAULT_FONT_FAMILY.into(),
        ..usvg::Options::default()
    };
    options
        .fontdb_mut()
        .load_font_data(DEFAULT_FONT_BYTES.to_vec());
    if let Some(family) = options
        .fontdb
        .faces()
        .next()
        .and_then(|face| face.families.first())
        .map(|(family, _)| family.clone())
    {
        options.font_family = family;
    }
    svg_fonts::install(&mut options, &input.web_fonts)?;
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
    let tree = usvg::Tree::from_str(&xml, &options).map_err(|e| e.to_string())?;
    let mut shapes = Vec::new();
    paths(tree.root(), &mut shapes, 0)?;
    Ok(Shape::Union(shapes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_clipping_uses_real_glyph_counters_and_svg_positioning() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":ViewportState::default()})).unwrap();
        let mut node:LayoutNode=serde_json::from_value(serde_json::json!({"id":1,"parent":null,"tree_scope":1,"tag":"text","namespace":"http://www.w3.org/2000/svg","attributes":{"x":"10","y":"50"},"text":"O"})).unwrap();
        input.nodes = vec![node.clone()];
        let styles = BTreeMap::from([(
            1,
            Arc::new(BTreeMap::from([("font-size".into(), "40px".into())])),
        )]);
        let reference = CssRect {
            x: 0.,
            y: 0.,
            width: 200.,
            height: 100.,
        };
        let outline = resolve(&input, &node, reference, &styles).unwrap();
        let bounds = outline.bounds();
        assert!(bounds.width > 10. && bounds.height > 10.);
        assert!(!outline.contains(bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.));
        node.attributes.insert("x".into(), "30".into());
        let moved = resolve(&input, &node, reference, &styles).unwrap().bounds();
        assert!((moved.x - bounds.x - 20.).abs() < 0.01);
        assert!((moved.width - bounds.width).abs() < 0.01);
    }
    #[test]
    fn interleaved_native_text_and_tspan_preserve_order_and_position() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":ViewportState::default()})).unwrap();
        let root:LayoutNode=serde_json::from_value(serde_json::json!({"id":1,"parent":null,"tag":"text","text":"ignored compatibility aggregate"})).unwrap();
        let span:LayoutNode=serde_json::from_value(serde_json::json!({"id":2,"parent":1,"tag":"tspan","attributes":{"dx":"20"},"text":"B"})).unwrap();
        let mut root = root;
        root.content = Some(vec![
            LayoutContent::Text {
                node_id: 3,
                text: "A".into(),
            },
            LayoutContent::Element(2),
            LayoutContent::Text {
                node_id: 4,
                text: "C".into(),
            },
        ]);
        input.nodes = vec![root.clone(), span];
        let mut xml = String::new();
        serialize(&input, &root, &BTreeMap::new(), true, &mut xml, 0).unwrap();
        assert!(xml.contains(">A<tspan"));
        assert!(xml.contains("</tspan>C</text>"));
        assert!(!xml.contains("ignored compatibility"));
    }
}
