//! Static inline SVG capture from the authoritative DOM and computed styles.
use crate::RuntimeError;
use oryn_common::v4::PaintImageResource;
use oryn_dom::{NodeId, NodeKind, html::ParsedDocument};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct CapturedSvg {
    pub resource: PaintImageResource,
    pub descendants: BTreeSet<NodeId>,
    pub resources: BTreeSet<String>,
    pub foreign_objects: BTreeMap<NodeId, String>,
    pub foreign_rects: BTreeMap<NodeId, oryn_common::v4::CssRect>,
}

/// Membership for an SVG viewport that paints nothing. Even a zero-area root
/// owns its descendants; they must not escape into ordinary HTML painting.
pub(crate) fn descendants(
    document: &ParsedDocument,
    root: NodeId,
) -> Result<BTreeSet<NodeId>, RuntimeError> {
    let mut result = BTreeSet::new();
    let mut pending = vec![(root, 0usize)];
    while let Some((id, depth)) = pending.pop() {
        if depth > 256 || result.len() >= 100_000 {
            return Err(RuntimeError::PaintUnavailable("SVG tree limit".into()));
        }
        if id != root && !result.insert(id) {
            return Err(RuntimeError::PaintUnavailable(
                "SVG tree repeats a node".into(),
            ));
        }
        let node = document
            .dom
            .get(id)
            .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
        pending.extend(node.children.iter().map(|id| (*id, depth + 1)));
    }
    Ok(result)
}
pub(crate) fn capture(
    document: &ParsedDocument,
    root: NodeId,
    styles: &oryn_layout::ComputedStyles,
    base: &str,
    size: Option<(f64, f64)>,
    max_bytes: usize,
) -> Result<CapturedSvg, RuntimeError> {
    let failure = |message: &str| RuntimeError::PaintUnavailable(message.into());
    let mut dom = document
        .clone()
        .into_runtime()
        .map_err(|error| failure(&error.to_string()))?;
    let fallback = if matches!(base, "about:blank" | "about:srcdoc") {
        document
            .dom
            .get(document.document)
            .ok()
            .and_then(|node| node.document_metadata.as_ref())
            .and_then(|metadata| metadata.inherited_base_url.as_deref())
            .unwrap_or(base)
    } else {
        base
    };
    let resolution_base = document
        .dom
        .resolve_document_base(document.document, fallback, true)
        .map_err(|error| failure(&error.to_string()))?;

    if let Some((width, height)) = size {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(failure("inline SVG has no positive viewport"));
        }
        dom.set_attribute(root, "width", width.to_string())
            .map_err(|error| failure(&error.to_string()))?;
        dom.set_attribute(root, "height", height.to_string())
            .map_err(|error| failure(&error.to_string()))?;
    }
    let viewport = size;
    let mut stack = vec![(root, 0usize, false)];
    let mut descendants = BTreeSet::new();
    let mut resources = BTreeSet::new();
    let mut foreign_objects = BTreeMap::new();
    let mut foreign_rects = BTreeMap::new();
    let mut style_bytes = 0usize;
    while let Some((id, depth, in_html)) = stack.pop() {
        if depth > 256 || (id != root && descendants.len() >= 99_999) {
            return Err(failure("inline SVG tree limit"));
        }
        if id != root {
            descendants.insert(id);
        }
        let node = dom
            .arena()
            .get(id)
            .map_err(|error| failure(&error.to_string()))?;
        let children = node.children.clone();
        // HTML is formatted independently. Its nested SVG trees belong to
        // separate image resources and must not register with this owner.
        if in_html {
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1, true));
            }
            continue;
        }
        let mut style = None;
        let mut resolved_href = None;
        let foreign_object = matches!(&node.kind,NodeKind::Element{name}
            if name.ns.as_ref()=="http://www.w3.org/2000/svg" && name.local.as_ref()=="foreignObject");
        if let NodeKind::Element { name } = &node.kind {
            if id == root
                && (name.ns.as_ref() != "http://www.w3.org/2000/svg"
                    || name.local.as_ref() != "svg")
            {
                return Err(failure("inline SVG capture requires an SVG root"));
            }
            if matches!(name.local.as_ref(), "image" | "feImage" | "use") {
                if let Some(value) = node
                    .attributes
                    .get_null("href")
                    .or_else(|| {
                        node.attributes
                            .namespaced(Some("http://www.w3.org/1999/xlink"), "href")
                            .map(|attribute| &attribute.value)
                    })
                    .filter(|value| !value.starts_with('#'))
                {
                    let mut ancestor = Some(id);
                    let mut bases = Vec::new();
                    let mut depth = 0;
                    while let Some(parent) = ancestor {
                        depth += 1;
                        if depth > 256 {
                            return Err(failure("SVG XML base depth bound"));
                        }
                        let record = document
                            .dom
                            .get(parent)
                            .map_err(|error| failure(&error.to_string()))?;
                        if let Some(base) = record
                            .attributes
                            .namespaced(Some("http://www.w3.org/XML/1998/namespace"), "base")
                        {
                            bases.push(base.value.clone());
                        }
                        ancestor = record.parent;
                    }
                    let mut resolved = url::Url::parse(&resolution_base)
                        .map_err(|error| failure(&error.to_string()))?;
                    for base in bases.into_iter().rev() {
                        resolved = resolved
                            .join(&base)
                            .map_err(|error| failure(&error.to_string()))?;
                    }
                    resolved = resolved
                        .join(value)
                        .map_err(|error| failure(&error.to_string()))?;
                    resolved_href = Some(resolved.to_string());
                    resolved.set_fragment(None);
                    resources.insert(resolved.to_string());
                }
            }
            let packed = (u64::from(id.generation) << 32) | u64::from(id.slot);
            if let Some(computed) = styles.get(&packed) {
                let mut value = String::new();
                for (property, content) in computed.iter() {
                    if presentation_property(property)
                        && !(id == root
                            && matches!(
                                property.as_str(),
                                "opacity" | "transform" | "x" | "y" | "width" | "height"
                            ))
                    {
                        value.push_str(property);
                        value.push(':');
                        value.push_str(content);
                        value.push_str(" !important;");
                    }
                }
                if !value.is_empty() {
                    style = Some(value);
                }
            }
            // The outer paint projection already owns these effects. Override
            // both presentation attributes and author CSS in the captured copy.
            if id == root {
                let value = style.get_or_insert_with(|| {
                    node.attributes
                        .get_null("style")
                        .cloned()
                        .unwrap_or_default()
                });
                value.push_str(";opacity:1 !important;transform:none !important;x:0 !important;y:0 !important;");
                if let Some((width, height)) = viewport {
                    value.push_str(&format!(
                        "width:{width}px !important;height:{height}px !important;"
                    ));
                }
            }
        }
        if let Some(value) = resolved_href {
            dom.set_attribute(id, "href", value)
                .map_err(|error| failure(&error.to_string()))?;
        }
        if let Some(style) = style {
            style_bytes = style_bytes.saturating_add(style.len());
            if style_bytes > max_bytes {
                return Err(failure("inline SVG computed-style byte limit"));
            }
            dom.set_attribute(id, "style", style)
                .map_err(|error| failure(&error.to_string()))?;
        }
        if foreign_object {
            let identity = format!("oryn-fo-{}-{}", id.generation, id.slot);
            dom.set_attribute(id, "data-oryn-foreign-object", identity.clone())
                .map_err(|error| failure(&error.to_string()))?;
            let rect = foreign_object_rect(document, id, styles, viewport.unwrap_or((300., 150.)))?;
            for (property, value) in [
                ("x", rect.x),
                ("y", rect.y),
                ("width", rect.width),
                ("height", rect.height),
            ] {
                dom.set_attribute(id, property, value.to_string())
                    .map_err(|error| failure(&error.to_string()))?;
            }
            foreign_objects.insert(id, identity);
            foreign_rects.insert(id, rect);
        }
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1, foreign_object));
        }
    }
    let source = dom
        .serialize_xml(root)
        .map_err(|error| failure(&error.to_string()))?;
    if source.len() > max_bytes.min(16 * 1024 * 1024) {
        return Err(failure("inline SVG source byte limit"));
    }
    Ok(CapturedSvg {
        resource: PaintImageResource {
            svg_document_snapshot: true,
            content_type: None,
            url_aliases: Vec::new(),
            url: base.into(),
            media_type: "image/svg+xml".into(),
            bytes: source.into_bytes(),
        },
        descendants,
        resources,
        foreign_objects,
        foreign_rects,
    })
}

fn foreign_object_rect(
    document: &ParsedDocument,
    id: NodeId,
    styles: &oryn_layout::ComputedStyles,
    mut viewport: (f64, f64),
) -> Result<oryn_common::v4::CssRect, RuntimeError> {
    let failure = |message: String| RuntimeError::PaintUnavailable(message);
    let node = document
        .dom
        .get(id)
        .map_err(|error| failure(error.to_string()))?;
    let mut ancestor = node.parent;
    while let Some(parent) = ancestor {
        let parent = document
            .dom
            .get(parent)
            .map_err(|error| failure(error.to_string()))?;
        if matches!(&parent.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/2000/svg" && name.local.as_ref()=="svg")
        {
            if let Some(viewbox) = parent.attributes.get_null("viewBox") {
                let values = viewbox
                    .split(|c: char| c.is_ascii_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>();
                if let Ok(values) = values {
                    if values.len() == 4
                        && values.iter().all(|n| n.is_finite())
                        && values[2] > 0.
                        && values[3] > 0.
                    {
                        viewport = (values[2], values[3]);
                    }
                }
            }
            break;
        }
        ancestor = parent.parent;
    }
    let computed = styles.get(&((u64::from(id.generation) << 32) | u64::from(id.slot)));
    let length = |name: &str, basis: f64| -> Result<f64, RuntimeError> {
        let value = computed
            .and_then(|style| style.get(name))
            .filter(|value| value.as_str() != "auto")
            .or_else(|| node.attributes.get_null(name))
            .map(String::as_str)
            .unwrap_or("0")
            .trim();
        let (number, scale) = if let Some(number) = value.strip_suffix('%') {
            (number, basis / 100.)
        } else if let Some(number) = value.strip_suffix("px") {
            (number, 1.)
        } else {
            (value, 1.)
        };
        let value = number
            .parse::<f64>()
            .ok()
            .map(|n| n * scale)
            .filter(|n| n.is_finite())
            .ok_or_else(|| failure(format!("unresolved foreignObject {name}: {value}")))?;
        Ok(value)
    };
    Ok(oryn_common::v4::CssRect {
        x: length("x", viewport.0)?,
        y: length("y", viewport.1)?,
        width: length("width", viewport.0)?.max(0.),
        height: length("height", viewport.1)?.max(0.),
    })
}

fn presentation_property(name: &str) -> bool {
    matches!(
        name,
        "x" | "y"
            | "width"
            | "height"
            | "cx"
            | "cy"
            | "r"
            | "rx"
            | "ry"
            | "d"
            | "color"
            | "fill"
            | "fill-opacity"
            | "fill-rule"
            | "stroke"
            | "stroke-width"
            | "stroke-opacity"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-miterlimit"
            | "clip-path"
            | "clip-rule"
            | "mask"
            | "filter"
            | "opacity"
            | "visibility"
            | "display"
            | "transform"
            | "font-family"
            | "font-size"
            | "font-style"
            | "font-weight"
            | "font-stretch"
            | "letter-spacing"
            | "word-spacing"
            | "text-anchor"
            | "dominant-baseline"
            | "alignment-baseline"
            | "baseline-shift"
            | "writing-mode"
            | "direction"
            | "stop-color"
            | "stop-opacity"
            | "flood-color"
            | "flood-opacity"
            | "lighting-color"
            | "color-interpolation"
            | "color-interpolation-filters"
            | "paint-order"
            | "vector-effect"
            | "shape-rendering"
            | "text-rendering"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};

    #[test]
    fn capture_preserves_dom_and_projects_styles_without_outer_effects() {
        let document = oryn_dom::html::parse(
            r#"<svg id="s" width="90" height="50" opacity="0.4" style="transform:rotate(10deg)"><g id="g"><text>Hello &amp; world</text></g></svg><p id="outside">outside</p>"#,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let group = dom.find_by_id("g").unwrap();
        let before = dom.serialize_xml(root).unwrap();
        let packed = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let styles = BTreeMap::from([
            (
                packed(root),
                Arc::new(BTreeMap::from([
                    ("fill".into(), "rgb(12, 34, 56)".into()),
                    ("opacity".into(), "0.4".into()),
                    ("transform".into(), "rotate(10deg)".into()),
                ])),
            ),
            (
                packed(group),
                Arc::new(BTreeMap::from([("opacity".into(), "0.5".into())])),
            ),
        ]);
        let captured = capture(
            &document,
            root,
            &styles,
            "https://example.test/page",
            Some((180., 100.)),
            4096,
        )
        .unwrap();
        let xml = String::from_utf8(captured.resource.bytes).unwrap();
        assert!(xml.contains("http://www.w3.org/2000/svg"));
        assert!(xml.contains("fill:rgb(12, 34, 56) !important"));
        assert!(xml.contains("opacity:1 !important;transform:none !important"));
        assert!(xml.contains("opacity:0.5 !important"));
        assert!(xml.contains("width:180px !important;height:100px !important"));
        assert!(xml.contains("Hello &amp; world"));
        assert!(captured.descendants.contains(&group));
        assert!(!captured.descendants.contains(&root));
        assert!(
            !captured
                .descendants
                .contains(&dom.find_by_id("outside").unwrap())
        );
        assert_eq!(
            document
                .clone()
                .into_runtime()
                .unwrap()
                .serialize_xml(root)
                .unwrap(),
            before
        );
    }

    #[test]
    fn nested_html_svg_resources_belong_to_their_own_capture() {
        let document = oryn_dom::html::parse(
            r#"<svg id="outer"><foreignObject id="first" width="100" height="100"><div><svg id="inner"><foreignObject id="second" width="20" height="20"><span>Nested</span></foreignObject><image href="inner.png"/></svg></div></foreignObject><image href="outer.png"/></svg>"#,
        );
        let dom = document.clone().into_runtime().unwrap();
        let first = dom.find_by_id("first").unwrap();
        let second = dom.find_by_id("second").unwrap();
        let outer = capture(
            &document,
            dom.find_by_id("outer").unwrap(),
            &Default::default(),
            "https://example.test/",
            None,
            8192,
        )
        .unwrap();
        assert_eq!(
            outer.foreign_objects.keys().copied().collect::<Vec<_>>(),
            vec![first]
        );
        assert_eq!(
            outer.resources,
            BTreeSet::from(["https://example.test/outer.png".into()])
        );
        assert!(outer.descendants.contains(&second));
        let inner = capture(
            &document,
            dom.find_by_id("inner").unwrap(),
            &Default::default(),
            "https://example.test/",
            None,
            8192,
        )
        .unwrap();
        assert_eq!(
            inner.foreign_objects.keys().copied().collect::<Vec<_>>(),
            vec![second]
        );
        assert_eq!(
            inner.resources,
            BTreeSet::from(["https://example.test/inner.png".into()])
        );
    }

    #[test]
    fn foreign_object_identity_preserves_author_ids_and_replaces_authored_tokens() {
        let document = oryn_dom::html::parse(
            r#"<svg id="s"><foreignObject id="author" data-oryn-foreign-object="forged" width="100" height="50"><div>HTML</div></foreignObject><foreignObject width="20" height="20"><span>second</span></foreignObject></svg>"#,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let author = dom.find_by_id("author").unwrap();
        let captured = capture(
            &document,
            root,
            &Default::default(),
            "https://example.test/",
            None,
            4096,
        )
        .unwrap();
        assert_eq!(captured.foreign_objects.len(), 2);
        assert_eq!(
            captured
                .foreign_objects
                .values()
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        let xml = String::from_utf8(captured.resource.bytes).unwrap();
        assert!(xml.contains("id=\"author\""));
        assert!(!xml.contains("forged"));
        assert!(xml.contains(&format!(
            "data-oryn-foreign-object=\"{}\"",
            captured.foreign_objects[&author]
        )));
        assert_eq!(
            document
                .dom
                .get(author)
                .unwrap()
                .attributes
                .get_null("data-oryn-foreign-object")
                .map(String::as_str),
            Some("forged")
        );
    }

    #[test]
    fn capture_collects_external_resources_and_enforces_limits() {
        let document = oryn_dom::html::parse(
            r##"<svg id="s"><image href="images/a.png"/><use href="#local"/><image xlink:href="https://example.test/b.png"/></svg><div id="html"></div>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let styles = oryn_layout::ComputedStyles::new();
        let captured = capture(
            &document,
            root,
            &styles,
            "https://example.test/page",
            None,
            4096,
        )
        .unwrap();
        assert_eq!(
            captured.resources,
            BTreeSet::from([
                "https://example.test/images/a.png".into(),
                "https://example.test/b.png".into()
            ])
        );
        assert!(
            capture(
                &document,
                root,
                &styles,
                "https://example.test/page",
                None,
                8
            )
            .is_err()
        );
        for size in [(0., 50.), (50., f64::NAN), (f64::INFINITY, 50.)] {
            assert!(
                capture(
                    &document,
                    root,
                    &styles,
                    "https://example.test/page",
                    Some(size),
                    4096
                )
                .is_err()
            );
        }
        assert!(
            capture(
                &document,
                dom.find_by_id("html").unwrap(),
                &styles,
                "https://example.test/page",
                None,
                4096
            )
            .is_err()
        );
    }
}

/// Bind fragment identities using the current native Document tree, including
/// hidden SVG definitions. Resource URL equality never grants this binding.
pub(crate) fn local_effects(
    document: &ParsedDocument,
    styles: &oryn_layout::ComputedStyles,
    base: &str,
    images: &mut Vec<PaintImageResource>,
    limit: usize,
) -> Result<(BTreeMap<String, usize>, BTreeSet<String>), RuntimeError> {
    let mut identities = BTreeSet::new();
    let mut definitions = Vec::new();
    let mut pending = vec![document.document];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) || visited.len() > 100_000 {
            return Err(RuntimeError::PaintUnavailable(
                "local effect document tree bound".into(),
            ));
        }
        let node = document
            .dom
            .get(id)
            .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
        if let NodeKind::Element { name } = &node.kind {
            if let Some(identifier) = node.attributes.get_null("id") {
                if identities.insert(identifier.clone())
                    && name.ns.as_ref() == "http://www.w3.org/2000/svg"
                    && matches!(name.local.as_ref(), "filter" | "mask")
                {
                    let mut root = node.parent;
                    let mut owner = None;
                    while let Some(parent) = root {
                        let record = document
                            .dom
                            .get(parent)
                            .map_err(|error| RuntimeError::PaintUnavailable(error.to_string()))?;
                        if matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()=="svg")
                        {
                            owner = Some(parent);
                            break;
                        }
                        root = record.parent;
                    }
                    if let Some(owner) = owner {
                        if definitions.len() >= 256 || identifier.len() > 64 * 1024 {
                            return Err(RuntimeError::PaintUnavailable(
                                "local effect identity bound".into(),
                            ));
                        }
                        definitions.push((identifier.clone(), owner));
                    }
                }
            }
        }
        pending.extend(node.children.iter().rev().copied());
    }
    let mut roots = BTreeMap::new();
    let mut references = BTreeMap::new();
    let mut resources = BTreeSet::new();
    let mut bytes = images.iter().map(|image| image.bytes.len()).sum::<usize>();
    for (identifier, root) in definitions {
        let index = if let Some(index) = roots.get(&root) {
            *index
        } else {
            let captured = capture(
                document,
                root,
                styles,
                base,
                None,
                limit.saturating_sub(bytes),
            )?;
            bytes = bytes
                .checked_add(captured.resource.bytes.len())
                .filter(|bytes| *bytes <= limit)
                .ok_or_else(|| {
                    RuntimeError::PaintUnavailable("local effect resource budget".into())
                })?;
            resources.extend(captured.resources);
            let index = images.len();
            images.push(captured.resource);
            roots.insert(root, index);
            index
        };
        references.insert(identifier, index);
    }
    Ok((references, resources))
}

#[cfg(test)]
mod local_effect_tests {
    use super::*;
    #[test]
    fn hidden_definitions_bind_to_native_document_order_and_not_url_equality() {
        let document = oryn_dom::html::parse(
            r#"<div id="duplicate"></div><svg style="display:none"><filter id="duplicate"/><filter id="local"><feFlood flood-color="red"/></filter><mask id="mask"><rect width="10" height="10" fill="white"/></mask></svg>"#,
        );
        let mut images=vec![PaintImageResource{svg_document_snapshot:false,content_type:None,url:"https://same.test/".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="local"><feFlood flood-color="blue"/></filter></svg>"#.to_vec()}];
        let (map, _) = local_effects(
            &document,
            &Default::default(),
            "https://same.test/",
            &mut images,
            1024 * 1024,
        )
        .unwrap();
        assert!(!map.contains_key("duplicate"));
        assert_eq!(map["local"], 1);
        assert_eq!(map["mask"], 1);
        assert!(String::from_utf8_lossy(&images[map["local"]].bytes).contains("red"));
        let other = oryn_dom::html::parse("<p>no definitions</p>");
        let (other_map, _) = local_effects(
            &other,
            &Default::default(),
            "https://same.test/",
            &mut images,
            1024 * 1024,
        )
        .unwrap();
        assert!(other_map.is_empty());
    }
}

#[cfg(test)]
mod effect_base_tests {
    use super::*;
    #[test]
    fn filter_image_uses_document_and_xml_base_without_changing_capture_identity() {
        let mut document = oryn_dom::html::parse(
            r#"<base href="https://cdn.test/assets/"><svg id="s" xml:base="../images/"><filter id="f"><feImage href="tile.png"/></filter></svg>"#,
        );
        let mut runtime = document.clone().into_runtime().unwrap();
        let root = runtime.find_by_id("s").unwrap();
        let literal = capture(
            &document,
            root,
            &Default::default(),
            "https://document.test/page",
            None,
            8192,
        )
        .unwrap();
        assert_eq!(literal.resource.url, "https://document.test/page");
        assert_eq!(
            literal.resources,
            BTreeSet::from(["https://cdn.test/assets/tile.png".into()])
        );
        // HTML foreign-attribute adjustment does not put literal xml:base in the XML namespace.
        runtime
            .set_attribute_ns(
                root,
                Some("http://www.w3.org/XML/1998/namespace".into()),
                "xml:base",
                "../images/",
            )
            .unwrap();
        document.dom = runtime.snapshot().0;
        let captured = capture(
            &document,
            root,
            &Default::default(),
            "https://document.test/page",
            None,
            8192,
        )
        .unwrap();
        assert_eq!(captured.resource.url, "https://document.test/page");
        assert_eq!(
            captured.resources,
            BTreeSet::from(["https://cdn.test/images/tile.png".into()])
        );
        assert!(
            String::from_utf8_lossy(&captured.resource.bytes)
                .contains("https://cdn.test/images/tile.png")
        );
        assert!(
            runtime
                .serialize_xml(root)
                .unwrap()
                .contains("href=\"tile.png\"")
        );
    }
}
