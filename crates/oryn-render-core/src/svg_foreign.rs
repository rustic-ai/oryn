//! Compose native HTML formatting inside SVG without giving SVG access to the
//! DOM, filesystem, network or a second browser engine.
use super::*;
thread_local! {static STATE:std::cell::Cell<(usize,usize,usize)>=const {std::cell::Cell::new((0,0,0))};}
pub(super) struct ParseGuard;
impl ParseGuard {
    pub(super) fn enter() -> Result<Self, RenderError> {
        STATE.with(|state| {
            let (depth, pixels, nodes) = state.get();
            if depth >= 8 {
                return Err(RenderError::InvalidPrint(
                    "nested SVG composition exceeds depth 8".into(),
                ));
            }
            state.set((
                depth + 1,
                if depth == 0 { 0 } else { pixels },
                if depth == 0 { 0 } else { nodes },
            ));
            Ok(Self)
        })
    }
}
impl Drop for ParseGuard {
    fn drop(&mut self) {
        STATE.with(|state| {
            let (d, p, n) = state.get();
            state.set((d.saturating_sub(1), p, n));
        });
    }
}
fn reserve(pixels: usize, nodes: usize) -> Result<(), RenderError> {
    STATE.with(|state| {
        let (d, p, n) = state.get();
        let p = p.checked_add(pixels).filter(|p| *p <= 16 * 1024 * 1024);
        let n = n.checked_add(nodes).filter(|n| *n <= 100_000);
        match (p, n) {
            (Some(p), Some(n)) => {
                state.set((d, p, n));
                Ok(())
            }
            _ => Err(RenderError::InvalidPrint(
                "SVG HTML composition exceeds aggregate resource budget".into(),
            )),
        }
    })
}
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
// This is conservative name reservation, not URL interpretation. Decode CSS
// escapes so an unresolved authored URL cannot acquire a generated target.
fn reservation_text(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        let Some(next) = chars.next() else { break };
        if next.is_ascii_hexdigit() {
            let mut scalar = next.to_digit(16).unwrap();
            for _ in 1..6 {
                let Some(c) = chars.peek().copied().filter(char::is_ascii_hexdigit) else {
                    break;
                };
                chars.next();
                scalar = scalar * 16 + c.to_digit(16).unwrap();
            }
            if chars
                .peek()
                .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
                && chars.next() == Some('\r')
                && chars.peek() == Some(&'\n')
            {
                chars.next();
            }
            result.push(
                char::from_u32(scalar)
                    .filter(|c| *c != '\0')
                    .unwrap_or('\u{fffd}'),
            );
        } else if next == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else if !matches!(next, '\n' | '\u{c}') {
            result.push(next);
        }
    }
    result
}
fn unique_xml_prefix(xml: &roxmltree::Document<'_>, base: String) -> String {
    let mut reserved = Vec::new();
    for node in xml.descendants() {
        for attribute in node.attributes() {
            reserved.push(attribute.value().to_owned());
            reserved.push(reservation_text(attribute.value()));
        }
        if node.is_text()
            && let Some(text) = node.text()
        {
            reserved.push(text.to_owned());
            reserved.push(reservation_text(text));
        }
    }
    let mut value = base;
    while reserved.iter().any(|text| text.contains(&value)) {
        value.push('_');
    }
    value
}
pub(super) fn viewport_id(
    xml: &roxmltree::Document<'_>,
    resource: usize,
    ordinal: usize,
) -> String {
    unique_xml_prefix(xml, format!("oryn-foreign-viewport-{resource}-{ordinal}_"))
}
#[derive(Debug)]
pub(super) struct ComposedSnapshot {
    pub data: Vec<u8>,
    pub style_roots: std::collections::BTreeSet<String>,
}
pub(super) fn compose_snapshot(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    data: Vec<u8>,
) -> Result<ComposedSnapshot, RenderError> {
    if !source.svg_document_snapshot {
        return Err(RenderError::InvalidPrint(
            "snapshot composition requires native SVG capture; fetched SVG must retain instance styles and content".into(),
        ));
    }
    let text = std::str::from_utf8(&data).map_err(|e| RenderError::Encode(e.to_string()))?;
    let xml = roxmltree::Document::parse(text).map_err(|e| RenderError::Encode(e.to_string()))?;
    let objects = xml
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "foreignObject"
                && !node.ancestors().skip(1).any(|parent| {
                    parent.is_element() && parent.tag_name().name() == "foreignObject"
                })
        })
        .collect::<Vec<_>>();
    if objects.is_empty() {
        return Ok(ComposedSnapshot {
            data,
            style_roots: Default::default(),
        });
    }
    if objects.len() > 256 {
        return Err(RenderError::InvalidPrint(
            "SVG exceeds 256 foreign objects".into(),
        ));
    }
    let resource = snapshot
        .images
        .iter()
        .position(|candidate| std::ptr::eq(candidate, source))
        .or_else(|| {
            snapshot.images.iter().position(|candidate| {
                candidate.url == source.url && candidate.bytes == source.bytes
            })
        })
        .ok_or_else(|| {
            RenderError::InvalidPrint(
                "SVG HTML composition resource is absent from snapshot".into(),
            )
        })?;
    let mut result = String::new();
    let mut style_roots = std::collections::BTreeSet::new();
    let mut cursor = 0;
    for (ordinal, object) in objects.into_iter().enumerate() {
        if ["width", "height"].iter().any(|name| {
            object
                .attribute(*name)
                .and_then(|value| {
                    value
                        .trim()
                        .trim_end_matches("px")
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .ok()
                })
                .is_some_and(|value| value <= 0.)
        }) {
            result.push_str(&text[cursor..object.range().start]);
            // An empty original can still be referenced by an instance whose
            // inherited dimensions are positive. Keep its reference identity.
            result.push_str("<g");
            for name in ["id", "data-oryn-native-style-id"] {
                if let Some(value) = object.attribute(name) {
                    result.push_str(&format!(" {name}=\"{}\"", escaped(value)));
                }
            }
            result.push_str("/>");
            cursor = object.range().end;
            continue;
        }
        let id = object
            .attribute("data-oryn-foreign-object")
            .ok_or_else(|| {
                RenderError::InvalidPrint("foreignObject lacks native capture identity".into())
            })?;
        let mut matches = snapshot
            .svg_foreign_objects
            .iter()
            .filter(|capture| capture.image_resource == resource && capture.element_id == id);
        let capture = matches.next().ok_or_else(|| {
            RenderError::InvalidPrint(format!("foreignObject {id} lacks native HTML layout"))
        })?;
        if matches.next().is_some() {
            return Err(RenderError::InvalidPrint(
                "ambiguous foreignObject capture identity".into(),
            ));
        }
        let viewport = capture.viewport;
        if !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || viewport.width <= 0.
            || viewport.height <= 0.
            || !viewport.device_pixel_ratio.is_finite()
            || !(0.1..=4.).contains(&viewport.device_pixel_ratio)
        {
            return Err(RenderError::InvalidPrint(
                "invalid foreignObject local viewport".into(),
            ));
        }
        let width = (viewport.width * viewport.device_pixel_ratio).ceil();
        let height = (viewport.height * viewport.device_pixel_ratio).ceil();
        let pixels = width * height;
        if !pixels.is_finite() || pixels > 16. * 1024. * 1024. {
            return Err(RenderError::InvalidPrint(
                "foreignObject surface exceeds 16 million pixels".into(),
            ));
        }

        let mut local = snapshot.clone();
        local.visual_viewport = None;
        local.viewport_scroll = None;
        local.frames.retain(|frame| {
            capture
                .nodes
                .iter()
                .chain(
                    snapshot
                        .svg_foreign_objects
                        .iter()
                        .flat_map(|object| object.nodes.iter()),
                )
                .any(|node| node.node_id == frame.owner_node)
        });
        local.nodes = capture.nodes.clone();
        local.viewport = viewport;
        local.scroll = oryn_common::v4::ScrollState::default();
        local.diagnostics.clear();
        for node in &mut local.nodes {
            node.geometry.in_viewport = true;
        }
        let options = RenderOptions {
            scale: viewport.device_pixel_ratio,
            max_pixels: 16 * 1024 * 1024,
            background: [0; 4],
        };
        let mut scene = build_display_list(&local, options)?;
        if crate::svg_coverage::text_capture_active() {
            scene.commands = crate::svg_coverage::text_only(&scene.commands);
        }
        if !scene.diagnostics.is_empty() {
            return Err(RenderError::InvalidPrint(format!(
                "incomplete foreignObject paint: {}",
                scene.diagnostics.join("; ")
            )));
        }
        let ink = crate::projected::ink_bounds(&local, &scene.commands, options.scale)?.unwrap_or(
            CssRect {
                x: 0.,
                y: 0.,
                width: viewport.width,
                height: viewport.height,
            },
        );
        let left = (ink.x.min(0.) * options.scale).floor() / options.scale;
        let top = (ink.y.min(0.) * options.scale).floor() / options.scale;
        let right = (ink.right().max(viewport.width) * options.scale).ceil() / options.scale;
        let bottom = (ink.bottom().max(viewport.height) * options.scale).ceil() / options.scale;
        let width = ((right - left) * options.scale).round();
        let height = ((bottom - top) * options.scale).round();
        if !width.is_finite() || !height.is_finite() || width * height > 16. * 1024. * 1024. {
            return Err(RenderError::InvalidPrint(
                "foreignObject overflow surface exceeds pixel bounds".into(),
            ));
        }
        reserve((width * height) as usize, capture.nodes.len())?;
        scene.width = width as u32;
        scene.height = height as u32;
        let mut matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        matrix[12] = -left;
        matrix[13] = -top;
        scene.commands = vec![PaintCommand::Projected {
            matrix,
            clips: Vec::new(),
            commands: scene.commands,
        }];
        local.viewport.width = right - left;
        local.viewport.height = bottom - top;
        let native = crate::print::vector::tree(&local, &scene, options)?;
        let content = native.to_string(&resvg::usvg::WriteOptions {
            id_prefix: Some(unique_xml_prefix(
                &xml,
                format!("foreign{resource}_{ordinal}_"),
            )),
            preserve_text: false,
            ..Default::default()
        });
        result.push_str(&text[cursor..object.range().start]);
        result.push_str("<g");
        for attribute in object.attributes() {
            if attribute.namespace().is_none()
                && !matches!(
                    attribute.name(),
                    "x" | "y" | "width" | "height" | "data-oryn-foreign-object"
                )
            {
                result.push_str(&format!(
                    " {}=\"{}\"",
                    attribute.name(),
                    escaped(attribute.value())
                ));
            }
        }
        // Root presentation/transform belongs to SVG. The captured HTML root
        // must not repeat those outer foreignObject effects in local paint.
        let position = |name: &str| -> Result<f64, RenderError> {
            object
                .attribute(name)
                .unwrap_or("0")
                .trim()
                .trim_end_matches("px")
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    RenderError::InvalidPrint(format!(
                        "foreignObject {name} is not a resolved native coordinate"
                    ))
                })
        };
        let viewport_id = viewport_id(&xml, resource, ordinal);
        style_roots.insert(viewport_id.clone());
        result.push_str(&format!("><g id=\"{viewport_id}\" transform=\"translate({} {})\"><g transform=\"translate({left} {top})\">{content}</g></g></g>",position("x")?,position("y")?));
        cursor = object.range().end;
        if result.len() > 16 * 1024 * 1024 {
            return Err(RenderError::InvalidPrint(
                "composed SVG exceeds 16 MiB".into(),
            ));
        }
    }
    result.push_str(&text[cursor..]);
    if result.len() > 16 * 1024 * 1024 {
        return Err(RenderError::InvalidPrint(
            "composed SVG exceeds 16 MiB".into(),
        ));
    }
    Ok(ComposedSnapshot {
        data: result.into_bytes(),
        style_roots,
    })
}

/// Locate native HTML line geometry through its actual parsed SVG viewport
/// group. Usvg retains named g elements; no marker paint is inserted and its
/// viewBox/ancestor transform implementation is reused at every nesting level.
#[cfg(test)]
pub(crate) fn semantic_lines(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    tree: &resvg::usvg::Tree,
) -> Result<Vec<crate::print::text_layer::PdfText>, RenderError> {
    semantic_lines_at_scale(source, snapshot, tree, snapshot.viewport.device_pixel_ratio)
}
pub(crate) fn semantic_lines_at_scale(
    source: &oryn_common::v4::PaintImageResource,
    snapshot: &PaintSnapshot,
    tree: &resvg::usvg::Tree,
    output_scale: f64,
) -> Result<Vec<crate::print::text_layer::PdfText>, RenderError> {
    let _guard = ParseGuard::enter()?;
    let data = crate::svg::xml_bytes_with_type(&source.bytes, source.content_type.as_deref())
        .map_err(RenderError::Encode)?;
    let text =
        std::str::from_utf8(&data).map_err(|error| RenderError::Encode(error.to_string()))?;
    let xml =
        roxmltree::Document::parse(text).map_err(|error| RenderError::Encode(error.to_string()))?;
    if !source.svg_document_snapshot
        && xml.descendants().any(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://www.w3.org/2000/svg")
                && node.tag_name().name() == "foreignObject"
        })
    {
        let owned = crate::svg_secure_foreign::prepare(source, snapshot, data)?;
        return semantic_lines_at_scale(&owned.source, &owned.snapshot, tree, output_scale);
    }
    let Some(resource) = snapshot
        .images
        .iter()
        .position(|candidate| candidate.url == source.url && candidate.bytes == source.bytes)
    else {
        return Ok(Vec::new());
    };
    let mut output = crate::svg_text::collect(snapshot, tree, output_scale, resource)?;
    let mut coverage_budget = crate::svg_coverage::Budget::default();
    for (ordinal, object) in xml
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "foreignObject"
                && !node
                    .ancestors()
                    .skip(1)
                    .any(|p| p.is_element() && p.tag_name().name() == "foreignObject")
        })
        .enumerate()
    {
        // Display/visibility suppress semantic content independently of masks.
        if object.ancestors().any(|node| {
            let value = |name: &str| -> Option<String> {
                node.attribute("style")
                    .and_then(|style| {
                        style
                            .split(';')
                            .filter_map(|declaration| declaration.split_once(':'))
                            .rfind(|(property, _)| property.trim() == name)
                            .map(|(_, value)| {
                                value
                                    .trim()
                                    .trim_end_matches("!important")
                                    .trim()
                                    .to_owned()
                            })
                    })
                    .or_else(|| node.attribute(name).map(str::to_owned))
            };
            value("display").is_some_and(|v| v == "none")
                || value("visibility").is_some_and(|v| v != "visible")
                || value("opacity")
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_some_and(|v| v <= 0.)
        }) {
            continue;
        }
        let Some(token) = object.attribute("data-oryn-foreign-object") else {
            continue;
        };
        let Some(capture) = snapshot
            .svg_foreign_objects
            .iter()
            .find(|capture| capture.image_resource == resource && capture.element_id == token)
        else {
            continue;
        };
        let mut local = snapshot.clone();
        local.visual_viewport = None;
        local.viewport_scroll = None;
        local.frames.retain(|frame| {
            capture
                .nodes
                .iter()
                .chain(
                    snapshot
                        .svg_foreign_objects
                        .iter()
                        .flat_map(|object| object.nodes.iter()),
                )
                .any(|node| node.node_id == frame.owner_node)
        });
        local.nodes = capture.nodes.clone();
        local.viewport = capture.viewport;
        local.scroll = oryn_common::v4::ScrollState::default();
        local.diagnostics.clear();
        for node in &mut local.nodes {
            node.geometry.in_viewport = true;
        }
        let scene = build_display_list(
            &local,
            RenderOptions {
                background: [0; 4],
                scale: capture.viewport.device_pixel_ratio,
                ..Default::default()
            },
        )?;
        let id = viewport_id(&xml, resource, ordinal);
        let Some(resvg::usvg::Node::Group(group)) = tree.node_by_id(&id) else {
            continue;
        };
        let transform = group.abs_transform();
        let lines = if crate::svg_coverage::required(tree, &id)? {
            crate::svg_coverage::retain(&local, tree, &id, output_scale, &mut coverage_budget)?
        } else {
            crate::print::text_layer::collect_local(&local, &scene)?
        };
        for line in lines {
            let mut corners = Vec::new();
            for (x, y) in line.quad.unwrap_or([
                (line.rect.x, line.rect.y),
                (line.rect.right(), line.rect.y),
                (line.rect.right(), line.rect.bottom()),
                (line.rect.x, line.rect.bottom()),
            ]) {
                corners.push((
                    f64::from(transform.sx) * x
                        + f64::from(transform.kx) * y
                        + f64::from(transform.tx),
                    f64::from(transform.ky) * x
                        + f64::from(transform.sy) * y
                        + f64::from(transform.ty),
                ));
            }
            if corners.iter().any(|(x, y)| {
                !x.is_finite()
                    || !y.is_finite()
                    || *x < 0.
                    || *y < 0.
                    || *x > f64::from(tree.size().width())
                    || *y > f64::from(tree.size().height())
            }) {
                continue;
            }
            let x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let rect = CssRect {
                x,
                y,
                width: corners
                    .iter()
                    .map(|p| p.0)
                    .fold(f64::NEG_INFINITY, f64::max)
                    - x,
                height: corners
                    .iter()
                    .map(|p| p.1)
                    .fold(f64::NEG_INFINITY, f64::max)
                    - y,
            };
            let clusters = line
                .clusters
                .into_iter()
                .map(|mut cluster| {
                    cluster.quad = cluster.quad.map(|(x, y)| {
                        (
                            f64::from(transform.sx) * x
                                + f64::from(transform.kx) * y
                                + f64::from(transform.tx),
                            f64::from(transform.ky) * x
                                + f64::from(transform.sy) * y
                                + f64::from(transform.ty),
                        )
                    });
                    cluster
                })
                .collect();
            output.push(crate::print::text_layer::PdfText {
                clusters,
                text: line.text,
                rect,
                quad: Some([corners[0], corners[1], corners[2], corners[3]]),
            });
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::v4::{
        BoxModel, EdgeSizes, NodeGeometry, PaintImageResource, PaintNodeSnapshot,
        SvgForeignObjectSnapshot,
    };
    #[test]
    fn generated_prefix_reserves_ids_unresolved_urls_and_css_escapes() {
        let xml = roxmltree::Document::parse(r##"<svg xmlns="http://www.w3.org/2000/svg"><defs id="native_0"/><use href="#native__missing"/><rect style="fill:url(#\6e ative___missing)"/><style>rect { clip-path: url("#native____missing") }</style></svg>"##).unwrap();
        assert_eq!(unique_xml_prefix(&xml, "native_".into()), "native_____");
        assert_eq!(unique_xml_prefix(&xml, "unrelated_".into()), "unrelated_");
        // Keep these separate: a longer literal reservation must not mask a
        // failure to decode an escaped reference in the allocator itself.
        for source in [
            r##"<svg xmlns="http://www.w3.org/2000/svg"><rect style="fill:url(#\6e ative_missing)"/></svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg"><style>rect { fill:url("#\00006e ative_missing") }</style></svg>"##,
        ] {
            let escaped = roxmltree::Document::parse(source).unwrap();
            assert_eq!(unique_xml_prefix(&escaped, "native_".into()), "native__");
        }
        assert_eq!(reservation_text(r"\00006e ative_"), "native_");
        assert_eq!(reservation_text(r"\n ative_"), "n ative_");
    }

    #[test]
    fn snapshot_compositor_rejects_fetched_svg_before_empty_object_fast_path() {
        let mut snapshot = fixture();
        snapshot.images[0].svg_document_snapshot = false;
        for data in [
            br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject width="1" height="1"/></svg>"#.as_slice(),
        ] {
            let error = compose_snapshot(&snapshot.images[0], &snapshot, data.to_vec()).unwrap_err();
            assert!(matches!(error, RenderError::InvalidPrint(ref detail) if detail.contains("must retain instance styles and content")));
        }
    }

    fn fixture() -> PaintSnapshot {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(PaintImageResource{svg_document_snapshot:true, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/foreign.svg".into(),media_type:"image/svg+xml".into(),bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><foreignObject id="author-id" data-oryn-foreign-object="native-42" x="10" y="20" width="30" height="15" opacity="0.5"><div xmlns="http://www.w3.org/1999/xhtml">Native content</div></foreignObject></svg>"##.to_vec()});
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 30.,
            height: 15.,
        };
        let model = BoxModel {
            border_box: rect,
            padding_box: rect,
            content_box: rect,
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
        };
        let node = PaintNodeSnapshot {
            builtin_image: None,
            backdrop: None,
            control: None,
            backdrop_style: None,
            top_layer: None,
            parent_id: None,
            node_id: 42,
            tag: "div".into(),
            geometry: NodeGeometry {
                availability: GeometryAvailability::Available,
                box_model: Some(model),
                client_rects: vec![rect],
                in_viewport: true,
                interactable: true,
                diagnostic: None,
            },
            projection: None,
            style: std::collections::BTreeMap::from([(
                "background-color".into(),
                "rgb(0, 255, 0)".into(),
            )]),
            text: Vec::new(),
            image_resource: None,
        };
        snapshot.svg_foreign_objects.push(SvgForeignObjectSnapshot {
            image_resource: 0,
            element_id: "native-42".into(),
            viewport: oryn_common::v4::ViewportState {
                width: 30.,
                height: 15.,
                device_pixel_ratio: 1.,
            },
            nodes: vec![node],
        });
        snapshot
    }
    #[test]
    fn captured_use_selects_independent_html_vectors_when_original_is_empty() {
        use oryn_common::v4::{SvgInstanceContent, SvgInstanceSnapshot};
        let mut snapshot = fixture();
        let mut first = snapshot.svg_foreign_objects[0].clone();
        first.element_id = "native-red".into();
        first.nodes[0]
            .style
            .insert("background-color".into(), "red".into());
        let mut second = first.clone();
        second.element_id = "native-blue".into();
        second.nodes[0]
            .style
            .insert("background-color".into(), "blue".into());
        snapshot.svg_foreign_objects = vec![first, second];
        snapshot.images[0].bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="90" height="15"><defs><foreignObject id="shape" data-oryn-native-style-id="3" width="0" height="15"/><foreignObject id="native-red" data-oryn-foreign-object="native-red" width="30" height="15"/><foreignObject id="native-blue" data-oryn-foreign-object="native-blue" width="30" height="15"/></defs><use href="#shape" data-oryn-native-style-id="4"/><use href="#shape" x="30" data-oryn-native-style-id="5"/><use href="#shape" x="60" data-oryn-native-style-id="6"/></svg>"##.to_vec();
        snapshot.images[0].svg_instances = vec![
            SvgInstanceSnapshot {
                ancestry: vec![4],
                source: 3,
                presentation: String::new(),
                content: SvgInstanceContent::Reference("native-red".into()),
            },
            SvgInstanceSnapshot {
                ancestry: vec![5],
                source: 3,
                presentation: String::new(),
                content: SvgInstanceContent::Reference("native-blue".into()),
            },
            SvgInstanceSnapshot {
                ancestry: vec![6],
                source: 3,
                presentation: String::new(),
                content: SvgInstanceContent::Empty,
            },
        ];
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let pixels = crate::svg::raster(&tree, 90, 15).unwrap();
        assert_eq!(pixels.get_pixel(10, 5).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(40, 5).0, [0, 0, 255, 255]);
        assert_eq!(pixels.get_pixel(70, 5).0, [0, 0, 0, 0]);
    }

    #[test]
    fn arbitrary_svg_clip_and_mask_semantics_follow_native_glyph_coverage() {
        let _coverage_failure = crate::svg_coverage::CoverageDiagnosticsOnFailure;
        use oryn_common::v4::{
            PaintFontResource, PaintGlyph, PaintGlyphOrientation, PaintGlyphRun, PaintTextCluster,
        };
        for (definition, effect, expected) in [
            (
                "<clipPath id='coverage'><circle cx='20' cy='30' r='30'/></clipPath>",
                "clip-path='url(#coverage)'",
                "AB",
            ),
            (
                "<clipPath id='coverage'><path d='M0 0H24V60H0Z'/></clipPath>",
                "clip-path='url(#coverage)'",
                "A",
            ),
            (
                "<mask id='coverage' maskUnits='userSpaceOnUse' x='0' y='0' width='80' height='60'><rect width='80' height='60' fill='white'/><rect x='24' width='56' height='60' fill='black'/></mask>",
                "mask='url(#coverage)'",
                "A",
            ),
            (
                "<mask id='coverage' maskContentUnits='objectBoundingBox'><rect width='.5' height='1' fill='white'/></mask>",
                "mask='url(#coverage)'",
                "A",
            ),
        ] {
            let _ = crate::svg_coverage::take_coverage_diagnostics();
            let mut snapshot = fixture();
            snapshot.fonts.insert(
                "native".into(),
                PaintFontResource {
                    bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                    collection_index: 0,
                },
            );
            let font =
                Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
            let rect = CssRect {
                x: 2.,
                y: 2.,
                width: 26.,
                height: 12.,
            };
            snapshot.svg_foreign_objects[0].nodes[0]
                .text
                .push(PaintTextFragment {
                    flow_line: None,
                    flow_rect: None,
                    clusters: vec![
                        PaintTextCluster {
                            start: 0,
                            end: 1,
                            rect: CssRect { width: 10., ..rect },
                            rtl: false,
                        },
                        PaintTextCluster {
                            start: 1,
                            end: 2,
                            rect: CssRect {
                                x: 17.,
                                width: 10.,
                                ..rect
                            },
                            rtl: false,
                        },
                    ],
                    text: "AB".into(),
                    rect,
                    font_family: "native".into(),
                    font_size: 10.,
                    color: "black".into(),
                    glyph_runs: vec![PaintGlyphRun {
                        line_box: None,
                        text_box: None,
                        background: None,
                        decoration_rect: Some(rect),
                        scale_x: 1.,
                        vertical: false,
                        decorations: Vec::new(),
                        orientation: PaintGlyphOrientation::Upright,
                        embolden: false,
                        skew_degrees: 0.,
                        normalized_coords: Vec::new(),
                        color: None,
                        font_key: "native".into(),
                        font_size: 10.,
                        glyphs: vec![
                            PaintGlyph {
                                source: Some(0..1),
                                id: u32::from(font.lookup_glyph_index('A')),
                                x: 0.,
                                y: 10.,
                                advance: 10.,
                            },
                            PaintGlyph {
                                source: Some(1..2),
                                id: u32::from(font.lookup_glyph_index('B')),
                                x: 15.,
                                y: 10.,
                                advance: 10.,
                            },
                        ],
                    }],
                });
            let source = String::from_utf8(snapshot.images[0].bytes.clone())
                .unwrap()
                .replace(
                    "<foreignObject",
                    &format!("<defs>{definition}</defs><g {effect}><foreignObject"),
                )
                .replace("</foreignObject>", "</foreignObject></g>");
            snapshot.images[0].bytes = source.into_bytes();
            let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
            let lines = semantic_lines(&snapshot.images[0], &snapshot, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                expected,
                "{effect}: {definition}; native coverage={:?}",
                crate::svg_coverage::take_coverage_diagnostics()
            );
            // The inner mask uses the complete native HTML box, including its
            // background. Isolating A must not shrink that objectBoundingBox.
            let mut nested_snapshot = snapshot.clone();
            let mut inner = nested_snapshot.svg_foreign_objects[0].clone();
            inner.image_resource = 1;
            inner.element_id = "nested-mask".into();
            inner.nodes[0].node_id = 43;
            nested_snapshot.images.push(PaintImageResource{svg_document_snapshot:true, svg_instances: Vec::new(),content_type:None,url_aliases:Vec::new(),url:"https://example.test/nested-mask.svg".into(),media_type:"image/svg+xml".into(),bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="15"><defs><mask id="inner" maskContentUnits="objectBoundingBox"><rect width=".5" height="1" fill="white"/></mask></defs><g mask="url(#inner)"><foreignObject data-oryn-foreign-object="nested-mask" width="30" height="15"><div xmlns="http://www.w3.org/1999/xhtml">AB</div></foreignObject></g></svg>"##.to_vec()});
            nested_snapshot.svg_foreign_objects.push(inner);
            let mut nested_image = crate::tests::paint_node(
                44,
                None,
                "svg",
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 30.,
                    height: 15.,
                },
            );
            nested_image.image_resource = Some(1);
            nested_snapshot.svg_foreign_objects[0].nodes = vec![nested_image];
            let tree = crate::svg::parse(&nested_snapshot.images[0], &nested_snapshot).unwrap();
            let lines =
                semantic_lines(&nested_snapshot.images[0], &nested_snapshot, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "A",
                "nested original effect boxes: {effect}: {definition}"
            );
            let mut repeated = nested_snapshot.clone();
            let mut second = repeated.svg_foreign_objects[0].nodes[0].clone();
            second.node_id = 49;
            second.projection.as_mut().unwrap().matrix[12] = 15.;
            repeated.svg_foreign_objects[0].nodes.push(second);
            let tree = crate::svg::parse(&repeated.images[0], &repeated).unwrap();
            let lines = semantic_lines(&repeated.images[0], &repeated, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                if expected == "AB" { "AA" } else { "A" },
                "independent repeated captured views: {effect}: {definition}"
            );
            let mut pure = nested_snapshot.clone();
            pure.svg_foreign_objects
                .retain(|object| object.image_resource != 1);
            pure.images[1].bytes=br##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="15"><defs><mask id="inner" maskUnits="userSpaceOnUse" x="0" y="0" width="30" height="15"><rect width="14" height="15" fill="white"/></mask></defs><g mask="url(#inner)"><text x="2 17" y="12" font-size="10">AB</text></g></svg>"##.to_vec();
            let tree = crate::svg::parse(&pure.images[0], &pure).unwrap();
            let lines = semantic_lines(&pure.images[0], &pure, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "A",
                "pure SVG native clusters inside masked HTML: {effect}: {definition}"
            );
            let mut second = pure.svg_foreign_objects[0].nodes[0].clone();
            second.node_id = 49;
            second.projection.as_mut().unwrap().matrix[12] = 15.;
            pure.svg_foreign_objects[0].nodes.push(second);
            let tree = crate::svg::parse(&pure.images[0], &pure).unwrap();
            let lines = semantic_lines(&pure.images[0], &pure, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                if expected == "AB" { "AA" } else { "A" },
                "repeated pure SVG native clusters: {effect}: {definition}"
            );
            // A projective HTML image owner takes the localized raster path;
            // it must retain the same original inner mask reference boxes.
            nested_snapshot.svg_foreign_objects[0].nodes[0]
                .projection
                .as_mut()
                .unwrap()
                .matrix[3] = 0.001;
            let tree = crate::svg::parse(&nested_snapshot.images[0], &nested_snapshot).unwrap();
            let lines =
                semantic_lines(&nested_snapshot.images[0], &nested_snapshot, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                "A",
                "projective nested original effect boxes: {effect}: {definition}"
            );
            // The child document deliberately owns its font and resources.
            // Parent coverage must project the same native glyphs through frame
            // placement without resolving them against the parent resource set.
            let mut child = snapshot.clone();
            child.nodes = snapshot.svg_foreign_objects[0].nodes.clone();
            child.viewport = snapshot.svg_foreign_objects[0].viewport;
            child.images.clear();
            child.svg_foreign_objects.clear();
            child.frames.clear();
            child.visual_viewport = None;
            child.viewport_scroll = None;
            let owner = crate::tests::paint_node(
                44,
                None,
                "iframe",
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 30.,
                    height: 15.,
                },
            );
            snapshot.svg_foreign_objects[0].nodes = vec![owner];
            snapshot.frames.push(oryn_common::v4::PaintFrameSnapshot {
                owner_node: 44,
                snapshot: Box::new(child),
            });
            snapshot.fonts.clear();
            let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
            let lines = semantic_lines(&snapshot.images[0], &snapshot, &tree).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<String>(),
                expected,
                "framed {effect}: {definition}"
            );
        }
    }

    #[test]
    fn nested_svg_inside_native_html_keeps_one_semantic_line() {
        use oryn_common::v4::{
            PaintFontResource, PaintGlyph, PaintGlyphOrientation, PaintGlyphRun,
        };
        let mut snapshot = fixture();
        snapshot.fonts.insert(
            "native".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let mut nested = snapshot.svg_foreign_objects[0].clone();
        nested.image_resource = 1;
        nested.element_id = "nested".into();
        nested.nodes[0].node_id = 43;
        nested.nodes[0].text.push(PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: "A".into(),
            rect: CssRect {
                x: 2.,
                y: 2.,
                width: 12.,
                height: 12.,
            },
            font_family: "native".into(),
            font_size: 10.,
            color: "black".into(),
            glyph_runs: vec![PaintGlyphRun {
                line_box: None,
                text_box: None,
                background: None,
                decoration_rect: None,
                scale_x: 1.,
                vertical: false,
                decorations: Vec::new(),
                orientation: PaintGlyphOrientation::Upright,
                embolden: false,
                skew_degrees: 0.,
                normalized_coords: Vec::new(),
                color: None,
                font_key: "native".into(),
                font_size: 10.,
                glyphs: vec![PaintGlyph {
                    source: None,
                    id: u32::from(font.lookup_glyph_index('A')),
                    x: 0.,
                    y: 10.,
                    advance: 8.,
                }],
            }],
        });
        let mut source = snapshot.images[0].clone();
        source.url = "https://example.test/nested.svg".into();
        source.bytes = String::from_utf8(source.bytes)
            .unwrap()
            .replace("native-42", "nested")
            .into_bytes();
        snapshot.images.push(source);
        snapshot.svg_foreign_objects.push(nested);
        let mut image = snapshot.svg_foreign_objects[0].nodes[0].clone();
        image.node_id = 44;
        image.parent_id = Some(42);
        image.tag = "svg".into();
        image.image_resource = Some(1);
        image.style.clear();
        snapshot.svg_foreign_objects[0].nodes.push(image);
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let lines = semantic_lines(&snapshot.images[0], &snapshot, &tree).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "A");
        assert!(lines[0].rect.x > 10. && lines[0].rect.y > 20.);
    }
    #[test]
    fn html_semantic_line_uses_actual_svg_viewbox_and_glyph_transform() {
        use oryn_common::v4::{
            PaintFontResource, PaintGlyph, PaintGlyphOrientation, PaintGlyphRun,
        };
        let mut snapshot = fixture();
        snapshot.fonts.insert(
            "native".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        snapshot.svg_foreign_objects[0].nodes[0]
            .text
            .push(PaintTextFragment {
                flow_line: None,
                flow_rect: None,
                clusters: Vec::new(),
                text: "A".into(),
                rect: CssRect {
                    x: 2.,
                    y: 2.,
                    width: 12.,
                    height: 12.,
                },
                font_family: "native".into(),
                font_size: 10.,
                color: "black".into(),
                glyph_runs: vec![PaintGlyphRun {
                    line_box: None,
                    text_box: None,
                    background: None,
                    decoration_rect: None,
                    scale_x: 1.,
                    vertical: false,
                    decorations: Vec::new(),
                    orientation: PaintGlyphOrientation::Upright,
                    embolden: false,
                    skew_degrees: 0.,
                    normalized_coords: Vec::new(),
                    color: None,
                    font_key: "native".into(),
                    font_size: 10.,
                    glyphs: vec![PaintGlyph {
                        source: None,
                        id: u32::from(font.lookup_glyph_index('A')),
                        x: 0.,
                        y: 10.,
                        advance: 8.,
                    }],
                }],
            });
        snapshot.images[0].bytes = String::from_utf8(snapshot.images[0].bytes.clone())
            .unwrap()
            .replace(
                "width=\"80\" height=\"60\"",
                "width=\"160\" height=\"120\" viewBox=\"0 0 80 60\"",
            )
            .into_bytes();
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let lines = semantic_lines(&snapshot.images[0], &snapshot, &tree).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "A");
        assert!((lines[0].rect.x - 24.).abs() < 0.01);
        assert!((lines[0].rect.y - 44.).abs() < 0.01);
        assert!((lines[0].rect.width - 24.).abs() < 0.01);
    }
    #[test]
    fn native_html_shapes_stay_vector_in_pdf_and_at_higher_density() {
        let snapshot = fixture();
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let (chunk, _) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default()).unwrap();
        assert!(!String::from_utf8_lossy(chunk.as_bytes()).contains("/Subtype /Image"));
        let image = crate::svg::raster(&tree, 320, 240).unwrap();
        assert_eq!(image.get_pixel(80, 100)[1], 255);
        assert_eq!(image.get_pixel(39, 100)[3], 0);
    }
    #[test]
    fn visible_html_ink_outside_foreign_viewport_is_retained() {
        let mut snapshot = fixture();
        let node = &mut snapshot.svg_foreign_objects[0].nodes[0];
        let rect = CssRect {
            x: -5.,
            y: -5.,
            width: 50.,
            height: 30.,
        };
        let model = node.geometry.box_model.as_mut().unwrap();
        model.border_box = rect;
        model.padding_box = rect;
        model.content_box = rect;
        node.geometry.client_rects = vec![rect];
        node.geometry.in_viewport = false;
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let image = crate::svg::raster(&tree, 80, 60).unwrap();
        assert!(image.get_pixel(7, 17)[3] > 0);
        assert!(image.get_pixel(50, 40)[3] > 0);
        assert_eq!(image.get_pixel(56, 46)[3], 0);
    }
    #[test]
    fn zero_extent_foreign_object_needs_no_capture_and_paints_nothing() {
        let mut snapshot = fixture();
        snapshot.svg_foreign_objects.clear();
        snapshot.images[0].bytes=br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><foreignObject width="0" height="15"><div xmlns="http://www.w3.org/1999/xhtml">Hidden</div></foreignObject></svg>"##.to_vec();
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let image = crate::svg::raster(&tree, 80, 60).unwrap();
        assert!(image.pixels().all(|pixel| pixel[3] == 0));
    }
    #[test]
    fn ordinary_foreign_object_native_vectors_are_isolated_from_author_css() {
        let mut snapshot = fixture();
        let source = String::from_utf8(snapshot.images[0].bytes.clone()).unwrap();
        snapshot.images[0].bytes = source.replace("</svg>", "<style>path { fill:red !important }</style><path d=\"M0 0H5V5H0Z\" fill=\"blue\"/></svg>").into_bytes();
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let image = crate::svg::raster(&tree, 80, 60).unwrap();
        assert_eq!(image.get_pixel(20, 25)[1], 255);
        assert!((127..=128).contains(&image.get_pixel(20, 25)[3]));
        assert_eq!(image.get_pixel(2, 2).0, [255, 0, 0, 255]);
    }

    #[test]
    fn fetched_foreign_object_native_vectors_are_isolated_from_author_css() {
        let source = PaintImageResource {
            svg_document_snapshot: false, svg_instances: Vec::new(),
            content_type: None, url_aliases: Vec::new(),
            url: "https://example.test/ordinary-foreign.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><style>path { fill:red !important }</style><foreignObject width="2" height="2"><div xmlns="http://www.w3.org/1999/xhtml" style="width:2px;height:2px;background:lime"/></foreignObject><path d="M2 0H4V2H2Z" fill="blue"/></svg>"##.to_vec(),
        };
        let tree = crate::svg::parse(&source, &crate::tests::empty_snapshot()).unwrap();
        let image = crate::svg::raster(&tree, 4, 2).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(3, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn native_html_composes_at_svg_position_with_outer_opacity_once() {
        let snapshot = fixture();
        let tree = crate::svg::parse(&snapshot.images[0], &snapshot).unwrap();
        let image = crate::svg::raster(&tree, 80, 60).unwrap();
        assert_eq!(image.get_pixel(20, 25)[1], 255);
        assert!((127..=128).contains(&image.get_pixel(20, 25)[3]));
        assert_eq!(image.get_pixel(5, 25)[3], 0);
        assert_eq!(image.get_pixel(45, 25)[3], 0);
        let composed = compose_snapshot(
            &snapshot.images[0],
            &snapshot,
            snapshot.images[0].bytes.clone(),
        )
        .unwrap();
        assert!(
            String::from_utf8(composed.data)
                .unwrap()
                .contains("id=\"author-id\"")
        );
    }
    #[test]
    fn missing_native_layout_and_recursive_composition_are_explicit_errors() {
        let mut snapshot = fixture();
        snapshot.svg_foreign_objects.clear();
        assert!(crate::svg::parse(&snapshot.images[0], &snapshot).is_err());
        let mut snapshot = fixture();
        snapshot.svg_foreign_objects[0].nodes[0].image_resource = Some(0);
        assert!(crate::svg::parse(&snapshot.images[0], &snapshot).is_err());
    }
}
