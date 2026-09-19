//! Same-document SVG clip reference resolution using native tree-scope identity.
use super::*;
use oryn_common::{
    basic_shape::{self, Shape},
    css_math::{self, LengthContext},
};
fn identifier(source: &str, owner: &LayoutNode) -> Result<String, String> {
    let inner = source
        .trim()
        .strip_prefix("url(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or("SVG clip URL expected")?
        .trim();
    let value = inner
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(inner);
    let fragment = if let Some(id) = value.strip_prefix('#') {
        id.to_owned()
    } else {
        let mut document = url::Url::parse(
            owner
                .document_url
                .as_deref()
                .ok_or("SVG clip Document URL missing")?,
        )
        .map_err(|_| "SVG clip Document URL invalid")?;
        let mut target = document.join(value).map_err(|_| "SVG clip URL invalid")?;
        let fragment = target
            .fragment()
            .ok_or("SVG clip URL has no fragment")?
            .to_owned();
        target.set_fragment(None);
        document.set_fragment(None);
        if target != document {
            return Err("external SVG clip resource has no admitted native source".into());
        }
        fragment
    };
    let mut bytes = Vec::new();
    let data = fragment.as_bytes();
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'%' && index + 2 < data.len() {
            let h = (data[index + 1] as char).to_digit(16);
            let l = (data[index + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                bytes.push((h * 16 + l) as u8);
                index += 3;
                continue;
            }
        }
        bytes.push(data[index]);
        index += 1;
    }
    String::from_utf8(bytes).map_err(|_| "SVG clip fragment encoding invalid".into())
}
fn target<'a>(input: &'a LayoutInput, owner: &LayoutNode, id: &str) -> Option<&'a LayoutNode> {
    input.nodes.iter().find(|node| {
        node.tree_scope == owner.tree_scope
            && node.namespace == "http://www.w3.org/2000/svg"
            && node.attributes.get("id").is_some_and(|value| value == id)
    })
}
fn transform(shape: Shape, matrix: [f64; 6]) -> Shape {
    Shape::Affine {
        matrix,
        shape: Box::new(shape),
    }
}
fn build(
    input: &LayoutInput,
    node: &LayoutNode,
    reference: CssRect,
    styles: &ComputedStyles,
    seen: &mut BTreeSet<u64>,
    depth: usize,
) -> Result<Shape, String> {
    if depth > 64 || !seen.insert(node.id) {
        return Err("SVG clip reference cycle/depth".into());
    }
    let result = (|| {
        let style = styles.get(&node.id);
        let attribute = |name: &str| {
            style
                .and_then(|style| style.get(name))
                .map(String::as_str)
                .filter(|value| !matches!(*value, "auto" | "normal"))
                .or_else(|| node.attributes.get(name).map(String::as_str))
        };
        if style
            .and_then(|style| style.get("display"))
            .is_some_and(|value| value == "none")
            || style
                .and_then(|style| style.get("visibility"))
                .is_some_and(|value| value == "hidden")
        {
            return Ok(Shape::Union(Vec::new()));
        }
        let font = style
            .and_then(|style| style.get("font-size"))
            .and_then(|value| value.trim_end_matches("px").parse().ok())
            .unwrap_or(16.);
        let x = LengthContext {
            percentage_basis: reference.width,
            font_size: font,
            root_font_size: 16.,
            viewport_width: input.viewport.width,
            viewport_height: input.viewport.height,
        };
        let y = LengthContext {
            percentage_basis: reference.height,
            ..x
        };
        let length = |name: &str, context: LengthContext, default: f64| {
            attribute(name)
                .map(|value| {
                    css_math::resolve_length(value, context)
                        .or_else(|| value.parse::<f64>().ok().filter(|n| n.is_finite()))
                        .ok_or_else(|| format!("unresolved SVG clip {name}"))
                })
                .unwrap_or(Ok(default))
        };
        let even_odd = attribute("clip-rule") == Some("evenodd");
        let shape = match node.tag.as_str() {
            "clipPath" | "g" => {
                let mut shapes = Vec::new();
                for id in &node.children {
                    if shapes.len() >= 4096 {
                        return Err("SVG clip child budget".into());
                    }
                    let Some(child) = input.nodes.iter().find(|node| node.id == *id) else {
                        continue;
                    };
                    if child.namespace == "http://www.w3.org/2000/svg" {
                        shapes.push(build(input, child, reference, styles, seen, depth + 1)?);
                    }
                }
                Shape::Union(shapes)
            }
            "rect" => {
                let rect = CssRect {
                    x: length("x", x, 0.)?,
                    y: length("y", y, 0.)?,
                    width: length("width", x, 0.)?.max(0.),
                    height: length("height", y, 0.)?.max(0.),
                };
                let rx = length("rx", x, length("ry", y, 0.)?)?
                    .max(0.)
                    .min(rect.width / 2.);
                let ry = length("ry", y, rx)?.max(0.).min(rect.height / 2.);
                Shape::RoundedRect {
                    rect,
                    radii: [[rx, ry]; 4],
                }
            }
            "circle" => {
                let radius = length(
                    "r",
                    LengthContext {
                        percentage_basis: (reference.width.powi(2) + reference.height.powi(2))
                            .sqrt()
                            / 2f64.sqrt(),
                        ..x
                    },
                    0.,
                )?
                .max(0.);
                Shape::Ellipse {
                    center: [length("cx", x, 0.)?, length("cy", y, 0.)?],
                    radius: [radius; 2],
                }
            }
            "ellipse" => Shape::Ellipse {
                center: [length("cx", x, 0.)?, length("cy", y, 0.)?],
                radius: [length("rx", x, 0.)?.max(0.), length("ry", y, 0.)?.max(0.)],
            },
            "polygon" | "polyline" => Shape::Polygon {
                points: basic_shape::svg_points(attribute("points").unwrap_or(""))?,
                even_odd,
            },
            "path" => {
                let source = attribute("d")
                    .filter(|value| *value != "none")
                    .unwrap_or("");
                let source = if source.starts_with("path(") {
                    source.to_owned()
                } else {
                    format!(
                        "path({}, {})",
                        if even_odd { "evenodd" } else { "nonzero" },
                        serde_json::to_string(source).map_err(|e| e.to_string())?
                    )
                };
                basic_shape::parse(
                    &source,
                    CssRect {
                        x: 0.,
                        y: 0.,
                        ..reference
                    },
                    font,
                    (input.viewport.width, input.viewport.height),
                )?
                .ok_or("empty SVG clip path")?
            }
            "use" => {
                let href = node
                    .attributes
                    .get("href")
                    .map(String::as_str)
                    .or_else(|| {
                        node.namespaced_attributes
                            .iter()
                            .find(|attr| {
                                attr.namespace == "http://www.w3.org/1999/xlink"
                                    && attr.local_name == "href"
                            })
                            .map(|attr| attr.value.as_str())
                    })
                    .ok_or("SVG clip use href missing")?;
                let id = href
                    .strip_prefix('#')
                    .ok_or("external SVG clip use not admitted")?;
                let referenced = target(input, node, id).ok_or("SVG clip use target absent")?;
                let shape = build(input, referenced, reference, styles, seen, depth + 1)?;
                transform(
                    shape,
                    [1., 0., 0., 1., length("x", x, 0.)?, length("y", y, 0.)?],
                )
            }
            "text" => super::svg_clip_text::resolve(input, node, reference, styles)?,
            "line" | "title" | "desc" | "metadata" => Shape::Union(Vec::new()),
            _ => {
                return Err(format!(
                    "SVG clip child requires native geometry: {}",
                    node.tag
                ));
            }
        };
        let shape = if let Some(source) = attribute("clip-path").filter(|source| *source != "none")
        {
            let clip = if source.starts_with("url(") {
                let id = identifier(source, node)?;
                let clip = target(input, node, &id).ok_or("nested SVG clip target absent")?;
                if clip.tag != "clipPath" {
                    return Err("nested SVG clip target is not clipPath".into());
                }
                let object = clip
                    .attributes
                    .get("clipPathUnits")
                    .is_some_and(|value| value == "objectBoundingBox");
                let bounds = shape.bounds();
                let space = if object {
                    CssRect {
                        x: 0.,
                        y: 0.,
                        width: 1.,
                        height: 1.,
                    }
                } else {
                    reference
                };
                let clip = build(input, clip, space, styles, seen, depth + 1)?;
                if object {
                    transform(
                        clip,
                        [bounds.width, 0., 0., bounds.height, bounds.x, bounds.y],
                    )
                } else {
                    clip
                }
            } else {
                basic_shape::parse(
                    source,
                    shape.bounds(),
                    font,
                    (input.viewport.width, input.viewport.height),
                )?
                .ok_or("nested SVG clip shape absent")?
            };
            Shape::Intersection(vec![shape, clip])
        } else {
            shape
        };
        let matrix = if let Some(css) = style
            .and_then(|style| style.get("transform"))
            .filter(|value| value.as_str() != "none")
        {
            let origin = style
                .and_then(|style| style.get("transform-origin"))
                .map(String::as_str)
                .unwrap_or("0px 0px");
            let projection = transforms::prepare(reference, css, origin, font, input.viewport)
                .ok_or("unresolved SVG clip CSS transform")?;
            let m = projection.matrix();
            if [2, 3, 6, 7, 8, 9, 11, 14]
                .iter()
                .any(|index| m[*index] != 0.)
                || m[10] != 1.
                || m[15] != 1.
            {
                return Err("SVG clip needs a planar affine transform".into());
            }
            [m[0], m[1], m[4], m[5], m[12], m[13]]
        } else if style.and_then(|style| style.get("transform")).is_some() {
            [1., 0., 0., 1., 0., 0.]
        } else {
            basic_shape::svg_transform(
                node.attributes
                    .get("transform")
                    .map(String::as_str)
                    .unwrap_or(""),
            )?
        };
        Ok(transform(shape, matrix))
    })();
    seen.remove(&node.id);
    result
}
/// SVG graphics use their current user coordinate system; unlike CSS boxes,
/// userSpaceOnUse must not be translated by the element's bounding-box origin.
pub(super) fn resolve_svg(
    input: &LayoutInput,
    owner: &LayoutNode,
    source: &str,
    bounds: CssRect,
    viewport: CssRect,
    styles: &ComputedStyles,
) -> Result<Shape, String> {
    if !source.starts_with("url(") {
        return basic_shape::parse(source, bounds, 16., (viewport.width, viewport.height))?
            .ok_or("SVG clip shape absent".into());
    }
    let id = match identifier(source, owner) {
        Ok(id) => id,
        Err(error) if error == "external SVG clip resource has no admitted native source" => {
            return super::svg_clip_external::resolve_styled(
                input,
                owner,
                source,
                bounds,
                Some(viewport),
                styles,
            );
        }
        Err(error) => return Err(error),
    };
    let root = target(input, owner, &id).ok_or("SVG clip target absent in original tree scope")?;
    if root.tag != "clipPath" {
        return Err("SVG clip target is not clipPath".into());
    }
    let object = root
        .attributes
        .get("clipPathUnits")
        .is_some_and(|value| value == "objectBoundingBox");
    let space = if object {
        CssRect {
            x: 0.,
            y: 0.,
            width: 1.,
            height: 1.,
        }
    } else {
        viewport
    };
    let shape = build(input, root, space, styles, &mut BTreeSet::new(), 0)?;
    Ok(if object {
        transform(
            shape,
            [bounds.width, 0., 0., bounds.height, bounds.x, bounds.y],
        )
    } else {
        shape
    })
}
pub(super) fn resolve(
    input: &LayoutInput,
    owner: &LayoutNode,
    source: &str,
    reference: CssRect,
    styles: &ComputedStyles,
) -> Result<Shape, String> {
    let id = match identifier(source, owner) {
        Ok(id) => id,
        Err(error) if error == "external SVG clip resource has no admitted native source" => {
            return super::svg_clip_external::resolve_styled(
                input, owner, source, reference, None, styles,
            );
        }
        Err(error) => return Err(error),
    };
    let root = target(input, owner, &id).ok_or("SVG clip target absent in original tree scope")?;
    if root.tag != "clipPath" {
        return Err("SVG clip target is not clipPath".into());
    }
    let object = root
        .attributes
        .get("clipPathUnits")
        .is_some_and(|value| value == "objectBoundingBox");
    let space = if object {
        CssRect {
            x: 0.,
            y: 0.,
            width: 1.,
            height: 1.,
        }
    } else {
        CssRect {
            x: 0.,
            y: 0.,
            ..reference
        }
    };
    let shape = build(input, root, space, styles, &mut BTreeSet::new(), 0)?;
    Ok(transform(
        shape,
        [
            if object { reference.width } else { 1. },
            0.,
            0.,
            if object { reference.height } else { 1. },
            reference.x,
            reference.y,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: u64, tag: &str, attributes: serde_json::Value, children: Vec<u64>) -> LayoutNode {
        serde_json::from_value(serde_json::json!({"id":id,"parent":null,"tree_scope":1,"document_url":"https://example.test/page","tag":tag,"namespace":"http://www.w3.org/2000/svg","children":children,"attributes":attributes,"text":""})).unwrap()
    }
    #[test]
    fn native_clip_id_scope_units_union_and_transform() {
        let mut input:LayoutInput=serde_json::from_value(serde_json::json!({"nodes":[],"stylesheets":[],"scroll":ScrollState::default(),"viewport":{"width":100,"height":100,"device_pixel_ratio":1}})).unwrap();
        let owner = node(1, "rect", serde_json::json!({}), vec![]);
        let clip = node(
            2,
            "clipPath",
            serde_json::json!({"id":"c","clipPathUnits":"objectBoundingBox"}),
            vec![3, 4],
        );
        let left = node(
            3,
            "rect",
            serde_json::json!({"x":"0","y":"0","width":".25","height":"1"}),
            vec![],
        );
        let right = node(
            4,
            "rect",
            serde_json::json!({"x":".75","y":"0","width":".25","height":"1"}),
            vec![],
        );
        input.nodes = vec![owner.clone(), clip, left, right];
        let shape = resolve(
            &input,
            &owner,
            "url(\"https://example.test/page#c\")",
            CssRect {
                x: 10.,
                y: 20.,
                width: 100.,
                height: 100.,
            },
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(shape.contains(20., 50.));
        assert!(shape.contains(100., 50.));
        assert!(!shape.contains(60., 50.));
        input.nodes[1].tree_scope = Some(2);
        assert!(
            resolve(
                &input,
                &owner,
                "url(#c)",
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.
                },
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            resolve(
                &input,
                &owner,
                "url(https://other.test/page#c)",
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.
                },
                &BTreeMap::new()
            )
            .is_err()
        );
    }
}
