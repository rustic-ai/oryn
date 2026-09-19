//! Native SVG descendant geometry. The SVG parser owns SVG units/viewBox/text;
//! CSS layout owns the outer viewport and its transforms. Neither runs script.
use super::*;
use oryn_common::v4::{PaintClip, PaintProjection};
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}
fn native_id(id: u64) -> String {
    format!("oryn-svg-geometry-{id}")
}
fn local_reference(value: &str, owner: &LayoutNode, ids: &BTreeMap<String, u64>) -> String {
    let fragment = if let Some(id) = value.strip_prefix('#') {
        Some(id.to_owned())
    } else {
        owner.document_url.as_ref().and_then(|base| {
            let mut base = url::Url::parse(base).ok()?;
            let mut reference = base.join(value).ok()?;
            let fragment = reference.fragment()?.to_owned();
            base.set_fragment(None);
            reference.set_fragment(None);
            (base == reference).then_some(fragment)
        })
    };
    fragment
        .and_then(|id| {
            let bytes = id.as_bytes();
            let mut decoded = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' && i + 2 < bytes.len() {
                    if let (Some(a), Some(b)) = (
                        (bytes[i + 1] as char).to_digit(16),
                        (bytes[i + 2] as char).to_digit(16),
                    ) {
                        decoded.push((a * 16 + b) as u8);
                        i += 3;
                        continue;
                    }
                }
                decoded.push(bytes[i]);
                i += 1;
            }
            String::from_utf8(decoded).ok()
        })
        .and_then(|id| ids.get(&id).copied())
        .map(|id| format!("#{}", native_id(id)))
        .unwrap_or_else(|| value.into())
}
fn emit(
    input: &LayoutInput,
    node: &LayoutNode,
    root: u64,
    size: CssRect,
    styles: &ComputedStyles,
    by_id: &BTreeMap<u64, &LayoutNode>,
    ids: &BTreeMap<String, u64>,
    output: &mut String,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 || output.len() > 16 * 1024 * 1024 {
        return Err("SVG geometry serialization budget".into());
    }
    if node.tag == "#text" {
        output.push_str(&escaped(&node.text));
        return Ok(());
    }
    if node.namespace != "http://www.w3.org/2000/svg"
        || matches!(
            node.tag.as_str(),
            "script" | "style" | "animate" | "animateTransform" | "set"
        )
    {
        return Ok(());
    }
    if !node
        .tag
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err("invalid SVG geometry element name".into());
    }
    let emitted_tag = if matches!(node.tag.as_str(), "image" | "foreignObject") {
        "rect"
    } else {
        node.tag.as_str()
    };
    output.push('<');
    output.push_str(emitted_tag);
    output.push_str(&format!(
        " id=\"{}\"",
        if node.tag == "svg" && node.id != root {
            format!("oryn-svg-viewport-{}", node.id)
        } else {
            native_id(node.id)
        }
    ));
    if node.id == root {
        output.push_str(&format!(
            " width=\"{}\" height=\"{}\"",
            size.width, size.height
        ));
        output.push_str(
            " xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
        );
    }
    for (name, value) in &node.attributes {
        if !matches!(
            name.as_str(),
            "x" | "y"
                | "width"
                | "height"
                | "cx"
                | "cy"
                | "r"
                | "rx"
                | "ry"
                | "x1"
                | "x2"
                | "y1"
                | "y2"
                | "d"
                | "points"
                | "viewBox"
                | "preserveAspectRatio"
                | "transform"
                | "href"
                | "dx"
                | "dy"
                | "rotate"
                | "textLength"
                | "lengthAdjust"
                | "startOffset"
                | "method"
                | "spacing"
                | "side"
        ) || node.id == root
            && matches!(name.as_str(), "x" | "y" | "width" | "height" | "transform")
        {
            continue;
        }
        if node.tag == "image" && matches!(name.as_str(), "href" | "width" | "height") {
            continue;
        }
        let text_position = matches!(node.tag.as_str(), "text" | "tspan" | "textPath")
            && matches!(name.as_str(), "x" | "y");
        if !text_position
            && matches!(
                name.as_str(),
                "x" | "y"
                    | "width"
                    | "height"
                    | "cx"
                    | "cy"
                    | "r"
                    | "rx"
                    | "ry"
                    | "d"
                    | "transform"
            )
            && styles
                .get(&node.id)
                .is_some_and(|style| style.contains_key(name))
        {
            continue;
        }
        let value = if name == "href" {
            local_reference(value, node, ids)
        } else {
            value.clone()
        };
        output.push_str(&format!(" {name}=\"{}\"", escaped(&value)));
    }
    for attr in &node.namespaced_attributes {
        if node.tag != "image"
            && attr.namespace == "http://www.w3.org/1999/xlink"
            && attr.local_name == "href"
            && !node.attributes.contains_key("href")
        {
            output.push_str(&format!(
                " href=\"{}\"",
                escaped(&local_reference(&attr.value, node, ids))
            ));
        }
    }
    // usvg's CSS parser accepts presentation properties but not SVG2 geometry
    // declarations. The native CSS cascade has already resolved precedence; pass
    // its geometry to the SVG parser as attributes instead of losing those values.
    if let Some(style) = styles.get(&node.id) {
        for name in [
            "x", "y", "width", "height", "cx", "cy", "r", "rx", "ry", "d",
        ] {
            // Text x/y are SVG positioning lists, not the rectangle geometry CSS
            // properties. In particular an absent tspan y must retain the current text
            // position; serializing CSS's initial y:0 would reset its inherited baseline.
            if matches!(node.tag.as_str(), "text" | "tspan" | "textPath")
                && matches!(name, "x" | "y")
            {
                continue;
            }
            if node.id == root && matches!(name, "x" | "y" | "width" | "height")
                || node.tag == "image" && matches!(name, "width" | "height")
            {
                continue;
            }
            let Some(value) = style.get(name).filter(|value| value.as_str() != "auto") else {
                continue;
            };
            let value = if name == "d" {
                geometry_path(value)
            } else {
                Some(value.clone())
            };
            if let Some(value) = value {
                output.push_str(&format!(" {name}=\"{}\"", escaped(&value)));
            }
        }
    }
    let mut image_dimensions = None;
    output.push_str(" style=\"");
    if let Some(style) = styles.get(&node.id) {
        for (name, value) in style.iter() {
            if matches!(
                name.as_str(),
                "x" | "y"
                    | "width"
                    | "height"
                    | "cx"
                    | "cy"
                    | "r"
                    | "rx"
                    | "ry"
                    | "d"
                    | "display"
                    | "overflow"
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
                    | "white-space"
            ) && !(node.id == root
                && matches!(name.as_str(), "x" | "y" | "width" | "height" | "transform"))
                && !(value == "auto"
                    && matches!(
                        name.as_str(),
                        "x" | "y" | "width" | "height" | "cx" | "cy" | "r" | "rx" | "ry"
                    ))
            {
                output.push_str(name);
                output.push(':');
                output.push_str(&escaped(value));
                output.push_str(" !important;");
            }
        }
    }
    if node.tag == "image" {
        // Geometry never decodes or fetches the image. Use only native admitted
        // intrinsic metadata; replacing the image with a rect keeps missing pixels
        // from erasing its SVG bounding box.
        let style = styles.get(&node.id);
        let intrinsic = node.image_intrinsics;
        let natural_width = intrinsic
            .and_then(|value| value.width())
            .or_else(|| node.intrinsic_image_size.map(|size| size.0 as f64));
        let natural_height = intrinsic
            .and_then(|value| value.height())
            .or_else(|| node.intrinsic_image_size.map(|size| size.1 as f64));
        let ratio = intrinsic.and_then(|value| value.ratio()).or_else(|| {
            natural_width
                .zip(natural_height)
                .and_then(|(w, h)| (h > 0.).then_some(w / h))
        });
        let dimension = |name: &str, basis: f64| {
            style
                .and_then(|style| style.get(name))
                .or_else(|| node.attributes.get(name))
                .filter(|value| value.as_str() != "auto")
                .and_then(|value| {
                    oryn_common::css_math::resolve_length(
                        value,
                        oryn_common::css_math::LengthContext {
                            percentage_basis: basis,
                            font_size: 16.,
                            root_font_size: 16.,
                            viewport_width: input.viewport.width,
                            viewport_height: input.viewport.height,
                        },
                    )
                    .or_else(|| value.parse::<f64>().ok())
                })
                .filter(|value| value.is_finite() && *value >= 0.)
        };
        let viewport = local_viewport(node, by_id, styles, size);
        let width = dimension("width", viewport.width);
        let height = dimension("height", viewport.height);
        let width = width
            .or_else(|| height.zip(ratio).map(|(h, r)| h * r))
            .or(natural_width)
            .unwrap_or(0.);
        let height = height
            .or_else(|| ratio.filter(|ratio| *ratio > 0.).map(|ratio| width / ratio))
            .or(natural_height)
            .unwrap_or(0.);
        image_dimensions = Some((width, height));
    }
    // Geometry exists even for unpainted shapes. Clip/mask/filter are composed
    // separately; they must not erase the original bounding-client rectangle.
    output.push_str("fill:black!important;stroke:none!important;opacity:1!important;visibility:visible!important;");
    if node.id == root {
        output.push_str(&format!(
            "width:{}px!important;height:{}px!important;transform:none!important;",
            size.width, size.height
        ));
    }
    output.push('"');
    if let Some((width, height)) = image_dimensions {
        output.push_str(&format!(" width=\"{width}\" height=\"{height}\""));
    }
    output.push('>');
    if node.tag == "svg" && node.id != root {
        output.push_str(&format!("<g id=\"{}\">", native_id(node.id)));
    }
    if node.tag == "foreignObject" {
    } else if let Some(content) = &node.content {
        for item in content {
            match item {
                LayoutContent::Text { text, .. } | LayoutContent::StyledText { text, .. } => {
                    output.push_str(&escaped(text))
                }
                LayoutContent::Element(id) => {
                    if let Some(child) = by_id.get(id) {
                        emit(
                            input,
                            child,
                            root,
                            size,
                            styles,
                            by_id,
                            ids,
                            output,
                            depth + 1,
                        )?
                    }
                }
            }
        }
    } else {
        output.push_str(&escaped(&node.text));
        for id in &node.children {
            if let Some(child) = by_id.get(id) {
                emit(
                    input,
                    child,
                    root,
                    size,
                    styles,
                    by_id,
                    ids,
                    output,
                    depth + 1,
                )?
            }
        }
    }
    if node.tag == "svg" && node.id != root {
        output.push_str("</g>");
    }
    output.push_str("</");
    output.push_str(emitted_tag);
    output.push('>');
    if output.len() > 16 * 1024 * 1024 {
        return Err("SVG geometry serialization budget".into());
    }
    Ok(())
}
fn geometry_path(value: &str) -> Option<String> {
    if value == "none" {
        return Some(String::new());
    }
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    if parser.expect_function_matching("path").is_ok() {
        let path = parser
            .parse_nested_block(|nested| {
                let value = nested.expect_string_cloned()?.to_string();
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(value)
            })
            .ok()?;
        parser.expect_exhausted().ok()?;
        Some(path)
    } else {
        Some(value.into())
    }
}
fn matrix(transform: usvg::Transform) -> transforms::Projection {
    transforms::Projection::from_matrix([
        transform.sx as f64,
        transform.ky as f64,
        0.,
        0.,
        transform.kx as f64,
        transform.sy as f64,
        0.,
        0.,
        0.,
        0.,
        1.,
        0.,
        transform.tx as f64,
        transform.ty as f64,
        0.,
        1.,
    ])
}
fn rect(rect: usvg::Rect) -> CssRect {
    CssRect {
        x: rect.x() as f64,
        y: rect.y() as f64,
        width: rect.width() as f64,
        height: rect.height() as f64,
    }
}
fn local_viewport(
    node: &LayoutNode,
    by_id: &BTreeMap<u64, &LayoutNode>,
    styles: &ComputedStyles,
    fallback: CssRect,
) -> CssRect {
    let mut chain = Vec::new();
    let mut cursor = Some(node.id);
    let mut seen = BTreeSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) || chain.len() > 128 {
            break;
        }
        let Some(node) = by_id.get(&id) else { break };
        if node.tag == "svg" {
            chain.push(*node);
        }
        cursor = node.parent;
    }
    let mut space = CssRect {
        x: 0.,
        y: 0.,
        ..fallback
    };
    for (index, node) in chain.into_iter().rev().enumerate() {
        if let Some(values) = node
            .attributes
            .get("viewBox")
            .and_then(|value| {
                value
                    .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
                    .filter(|value| !value.is_empty())
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|values| {
                values.len() == 4
                    && values.iter().all(|v| v.is_finite())
                    && values[2] > 0.
                    && values[3] > 0.
            })
        {
            space = CssRect {
                x: values[0],
                y: values[1],
                width: values[2],
                height: values[3],
            };
            continue;
        }
        // The outer SVG's CSS content box is already the admitted viewport. Its
        // percentage width/height were resolved by CSS layout and must not be applied
        // a second time. A viewBox above still establishes a different user space.
        if index == 0 {
            continue;
        }
        let dimension = |name: &str, basis: f64| {
            styles
                .get(&node.id)
                .and_then(|style| style.get(name))
                .or_else(|| node.attributes.get(name))
                .filter(|value| value.as_str() != "auto")
                .and_then(|value| {
                    oryn_common::css_math::resolve_length(
                        value,
                        oryn_common::css_math::LengthContext {
                            percentage_basis: basis,
                            font_size: 16.,
                            root_font_size: 16.,
                            viewport_width: fallback.width,
                            viewport_height: fallback.height,
                        },
                    )
                    .or_else(|| value.parse::<f64>().ok())
                })
                .filter(|value| value.is_finite() && *value >= 0.)
                .unwrap_or(basis)
        };
        space = CssRect {
            x: 0.,
            y: 0.,
            width: dimension("width", space.width),
            height: dimension("height", space.height),
        };
    }
    space
}
// Nested SVG viewport clipping is generated by the same SVG normalization
// that resolved preserveAspectRatio. Carry those exact planes to observers.
fn normalized_clips(
    group: &usvg::Group,
    outer: transforms::Projection,
    inherited: &[PaintClip],
    output: &mut BTreeMap<String, Vec<PaintClip>>,
    budget: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 || output.len() > 65536 || inherited.len() > 128 {
        return Err("SVG viewport clip budget".into());
    }
    let mut clips = inherited.to_vec();
    let mut next = group.clip_path();
    let mut count = 0;
    while let Some(clip) = next {
        count += 1;
        if count > 64 {
            return Err("SVG viewport clip chain budget".into());
        }
        let mut paths = Vec::new();
        super::svg_clip_text::paths(clip.root(), &mut paths, 0)?;
        let shape = oryn_common::basic_shape::Shape::Union(paths);
        let plane = outer
            .then(matrix(group.abs_transform()))
            .then(matrix(clip.transform()));
        clips.push(PaintClip {
            shape: Some(shape.clone()),
            owner: None,
            scroll_x: false,
            scroll_y: false,
            matrix: plane.matrix(),
            rect: shape.bounds(),
            clip_x: true,
            clip_y: true,
        });
        next = clip.clip_path();
    }
    if !group.id().is_empty() {
        *budget = budget.saturating_add(1 + clips.len());
        if *budget > 262144 {
            return Err("SVG viewport clip storage budget".into());
        }
        output.insert(group.id().into(), clips.clone());
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => {
                normalized_clips(child, outer, &clips, output, budget, depth + 1)?
            }
            _ => {
                if !node.id().is_empty() {
                    *budget = budget.saturating_add(1 + clips.len());
                    if *budget > 262144 {
                        return Err("SVG viewport clip storage budget".into());
                    }
                    output.insert(node.id().into(), clips.clone());
                }
            }
        }
    }
    Ok(())
}
fn text_element_bounds(
    group: &usvg::Group,
    output: &mut BTreeMap<String, (CssRect, usvg::Transform)>,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 || output.len() > 65536 {
        return Err("SVG text geometry budget".into());
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => text_element_bounds(group, output, depth + 1)?,
            usvg::Node::Text(text) => {
                for span in text.layouted() {
                    for id in &span.source_ids {
                        let bounds = rect(span.metric_bounds);
                        output
                            .entry(id.clone())
                            .and_modify(|(old, _)| {
                                let right = old.right().max(bounds.right());
                                let bottom = old.bottom().max(bounds.bottom());
                                old.x = old.x.min(bounds.x);
                                old.y = old.y.min(bounds.y);
                                old.width = right - old.x;
                                old.height = bottom - old.y;
                            })
                            .or_insert((bounds, text.abs_transform()));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
thread_local! {static FOREIGN_DEPTH:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
struct ForeignDepth;
impl ForeignDepth {
    fn enter() -> Result<Self, String> {
        FOREIGN_DEPTH.with(|depth| {
            if depth.get() >= 8 {
                return Err("SVG foreignObject geometry depth".into());
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}
impl Drop for ForeignDepth {
    fn drop(&mut self) {
        FOREIGN_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}
pub(super) fn project(
    input: &LayoutInput,
    styles: &ComputedStyles,
    limits: &LayoutLimits,
    geometry: &mut BTreeMap<u64, NodeGeometry>,
    projections: &mut BTreeMap<u64, PaintProjection>,
) -> Result<(), String> {
    let by_id = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let roots = input
        .nodes
        .iter()
        .filter(|node| node.namespace == "http://www.w3.org/2000/svg" && node.tag == "svg")
        .filter(|node| {
            let mut parent = node.parent;
            while let Some(id) = parent {
                let Some(parent_node) = by_id.get(&id) else {
                    break;
                };
                if parent_node.tag == "foreignObject" {
                    break;
                }
                if parent_node.namespace == "http://www.w3.org/2000/svg" && parent_node.tag == "svg"
                {
                    return false;
                }
                parent = parent_node.parent;
            }
            true
        })
        .collect::<Vec<_>>();
    for root in roots {
        let Some(outer) = projections.get(&root.id).cloned() else {
            continue;
        };
        let viewport = outer.box_model.content_box;
        if viewport.width <= 0. || viewport.height <= 0. {
            continue;
        }
        let mut descendants = Vec::new();
        let mut stack = root.children.iter().rev().copied().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        let mut ids = BTreeMap::new();
        if let Some(id) = root.attributes.get("id") {
            ids.insert(id.clone(), root.id);
        }
        while let Some(id) = stack.pop() {
            if !seen.insert(id) || seen.len() > 65536 {
                return Err("SVG geometry tree budget".into());
            }
            let Some(node) = by_id.get(&id) else { continue };
            if let Some(id) = node.attributes.get("id") {
                ids.entry(id.clone()).or_insert(node.id);
            }
            descendants.push(*node);
            if node.tag != "foreignObject" {
                stack.extend(node.children.iter().rev());
            }
        }
        let mut source = String::new();
        emit(
            input,
            root,
            root.id,
            viewport,
            styles,
            &by_id,
            &ids,
            &mut source,
            0,
        )?;
        let mut options = usvg::Options::default();
        options.font_family = DEFAULT_FONT_FAMILY.into();
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
        options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        let tree = usvg::Tree::from_str(&source, &options).map_err(|error| error.to_string())?;
        let outer_plane = transforms::Projection::from_matrix(outer.matrix)
            .then(transforms::Projection::translation(viewport.x, viewport.y));
        let mut text_bounds = BTreeMap::new();
        text_element_bounds(tree.root(), &mut text_bounds, 0)?;
        let mut viewport_clips = BTreeMap::new();
        normalized_clips(
            tree.root(),
            outer_plane,
            &[],
            &mut viewport_clips,
            &mut 0,
            0,
        )?;
        for node in descendants {
            if node.namespace != "http://www.w3.org/2000/svg" {
                continue;
            }
            let parsed = tree.node_by_id(&native_id(node.id));
            let metrics = parsed
                .map(|parsed| (rect(parsed.bounding_box()), parsed.abs_transform()))
                .or_else(|| text_bounds.get(&native_id(node.id)).copied());
            let Some((local, native_transform)) = metrics else {
                geometry.insert(node.id, hidden_geometry());
                projections.remove(&node.id);
                if node.tag == "foreignObject" {
                    let mut pending = node.children.clone();
                    let mut hidden = BTreeSet::new();
                    while let Some(id) = pending.pop() {
                        if !hidden.insert(id) || hidden.len() > 65536 {
                            return Err("SVG hidden subtree budget".into());
                        }
                        if input.top_layer.iter().any(|entry| entry.node_id == id) {
                            continue;
                        }
                        geometry.insert(id, hidden_geometry());
                        projections.remove(&id);
                        if let Some(child) = by_id.get(&id) {
                            pending.extend(child.children.iter().copied());
                        }
                    }
                }
                continue;
            };
            let projection = outer_plane.then(matrix(native_transform));
            let bounds = projection.project(local);
            let model = BoxModel {
                border_box: local,
                padding_box: local,
                content_box: local,
                margin: EdgeSizes::default(),
                border: EdgeSizes::default(),
                padding: EdgeSizes::default(),
            };
            let mut paint = outer.clone();
            let mut clip_owner = Some(node.id);
            while let Some(id) = clip_owner {
                if let Some(clips) = viewport_clips.get(&native_id(id)) {
                    paint.clips.extend(clips.iter().cloned());
                    break;
                }
                clip_owner = by_id.get(&id).and_then(|owner| owner.parent);
            }
            paint.matrix = projection.matrix();
            paint.box_model = model.clone();
            paint.text.clear();
            paint.text_clips.clear();
            paint.text_matrix = None;
            paint.scroll = None;
            paint.fragment_boxes.clear();
            paint.border_segments.clear();
            paint.border_edges = None;
            let mut ancestor = Some(node.id);
            while let Some(id) = ancestor {
                if id == root.id {
                    break;
                }
                let Some(owner) = by_id.get(&id) else { break };
                if let Some(value) = styles
                    .get(&id)
                    .and_then(|style| style.get("clip-path"))
                    .filter(|value| value.as_str() != "none")
                {
                    if let Some(parsed_owner) = tree.node_by_id(&native_id(id)) {
                        let reference = rect(parsed_owner.bounding_box());
                        let viewport = local_viewport(owner, &by_id, styles, viewport);
                        let shape = super::svg_clip::resolve_svg(
                            input, owner, value, reference, viewport, styles,
                        )?;
                        let transform = outer_plane.then(matrix(parsed_owner.abs_transform()));
                        paint.clips.push(PaintClip {
                            shape: Some(shape.clone()),
                            owner: Some(id),
                            scroll_x: false,
                            scroll_y: false,
                            matrix: transform.matrix(),
                            rect: shape.bounds(),
                            clip_x: true,
                            clip_y: true,
                        });
                    }
                }
                ancestor = owner.parent;
            }
            if node.tag == "foreignObject" {
                let style = styles.get(&node.id);
                let x = style
                    .and_then(|style| style.get("overflow-x"))
                    .is_none_or(|value| value != "visible");
                let y = style
                    .and_then(|style| style.get("overflow-y"))
                    .is_none_or(|value| value != "visible");
                if x || y {
                    paint.clips.push(PaintClip {
                        shape: None,
                        owner: Some(node.id),
                        scroll_x: x,
                        scroll_y: y,
                        matrix: projection.matrix(),
                        rect: local,
                        clip_x: x,
                        clip_y: y,
                    });
                }
            }
            geometry.insert(
                node.id,
                NodeGeometry {
                    availability: GeometryAvailability::Available,
                    box_model: Some(BoxModel {
                        border_box: bounds,
                        padding_box: bounds,
                        content_box: bounds,
                        ..model
                    }),
                    client_rects: vec![bounds],
                    in_viewport: bounds.intersects(CssRect {
                        x: 0.,
                        y: 0.,
                        width: input.viewport.width,
                        height: input.viewport.height,
                    }),
                    interactable: styles.get(&node.id).is_none_or(|style| {
                        style
                            .get("visibility")
                            .is_none_or(|value| !matches!(value.as_str(), "hidden" | "collapse"))
                            && style
                                .get("pointer-events")
                                .is_none_or(|value| value != "none")
                    }),
                    diagnostic: None,
                },
            );
            projections.insert(node.id, paint.clone());
            if node.tag == "foreignObject" && local.width > 0. && local.height > 0. {
                let _depth = ForeignDepth::enter()?;
                let local_viewport = ViewportState {
                    width: local.width,
                    height: local.height,
                    device_pixel_ratio: input.viewport.device_pixel_ratio,
                };
                let formatted = super::compute_subtree_with_styles(
                    input,
                    node.id,
                    local_viewport,
                    styles,
                    limits,
                )
                .map_err(|error| error.to_string())?;
                let plane = projection.then(transforms::Projection::translation(local.x, local.y));
                for (id, mut child_geometry) in formatted.geometry {
                    if id == node.id {
                        continue;
                    }
                    // Promoted top-layer descendants retain their initial containing block.
                    let mut ancestor = Some(id);
                    let mut escaped = false;
                    while let Some(id) = ancestor {
                        if id == node.id {
                            break;
                        }
                        if input.top_layer.iter().any(|entry| entry.node_id == id) {
                            escaped = true;
                            break;
                        }
                        ancestor = by_id.get(&id).and_then(|node| node.parent);
                    }
                    if escaped {
                        continue;
                    }
                    if let Some(model) = &mut child_geometry.box_model {
                        model.border_box = plane.project(model.border_box);
                        model.padding_box = plane.project(model.padding_box);
                        model.content_box = plane.project(model.content_box);
                    }
                    for rect in &mut child_geometry.client_rects {
                        *rect = plane.project(*rect);
                    }
                    child_geometry.in_viewport =
                        child_geometry.box_model.as_ref().is_some_and(|model| {
                            model.border_box.intersects(CssRect {
                                x: 0.,
                                y: 0.,
                                width: input.viewport.width,
                                height: input.viewport.height,
                            })
                        });
                    geometry.insert(id, child_geometry);
                    if let Some(mut child) = formatted.paint_projections.get(&id).cloned() {
                        child.matrix = plane
                            .then(transforms::Projection::from_matrix(child.matrix))
                            .matrix();
                        if let Some(matrix) = &mut child.text_matrix {
                            *matrix = plane
                                .then(transforms::Projection::from_matrix(*matrix))
                                .matrix();
                        }
                        for clip in child.clips.iter_mut().chain(child.text_clips.iter_mut()) {
                            clip.matrix = plane
                                .then(transforms::Projection::from_matrix(clip.matrix))
                                .matrix();
                        }
                        child.clips.splice(0..0, paint.clips.iter().cloned());
                        child.text_clips.splice(0..0, paint.clips.iter().cloned());
                        projections.insert(id, child);
                    } else {
                        projections.remove(&id);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> LayoutInput {
        serde_json::from_value(serde_json::json!({"nodes":[
   {"id":1,"parent":null,"children":[2],"tag":"body","attributes":{"style":"margin:0"}},
   {"id":2,"parent":1,"children":[3,5],"tag":"svg","namespace":"http://www.w3.org/2000/svg","tree_scope":1,"document_url":"https://example.test/page","attributes":{"width":"400","height":"200","viewBox":"0 0 200 100","style":"display:block"}},
   {"id":3,"parent":2,"children":[4],"tag":"clipPath","namespace":"http://www.w3.org/2000/svg","tree_scope":1,"document_url":"https://example.test/page","attributes":{"id":"clip"}},
   {"id":4,"parent":3,"tag":"circle","namespace":"http://www.w3.org/2000/svg","tree_scope":1,"document_url":"https://example.test/page","attributes":{"cx":"75","cy":"50","r":"20"}},
   {"id":5,"parent":2,"children":[6],"tag":"g","namespace":"http://www.w3.org/2000/svg","tree_scope":1,"document_url":"https://example.test/page","attributes":{"clip-path":"url(#clip)"}},
   {"id":6,"parent":5,"tag":"rect","namespace":"http://www.w3.org/2000/svg","tree_scope":1,"document_url":"https://example.test/page","attributes":{"x":"50","y":"25","width":"100","height":"50"}}
  ],"stylesheets":[],"viewport":{"width":500,"height":300,"device_pixel_ratio":1},"scroll":ScrollState::default()})).unwrap()
    }
    #[test]
    fn cascaded_svg_geometry_reaches_native_parser_as_attributes() {
        let input = fixture();
        let nodes = input
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        let styles = BTreeMap::from([(
            6,
            Arc::new(BTreeMap::from([
                ("x".into(), "25px".into()),
                ("y".into(), "10px".into()),
                ("width".into(), "40px".into()),
                ("height".into(), "20px".into()),
            ])),
        )]);
        let mut source = String::new();
        emit(
            &input,
            nodes[&2],
            2,
            CssRect {
                x: 0.,
                y: 0.,
                width: 400.,
                height: 200.,
            },
            &styles,
            &nodes,
            &BTreeMap::new(),
            &mut source,
            0,
        )
        .unwrap();
        let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
        let bounds = tree.node_by_id(&native_id(6)).unwrap().bounding_box();
        assert_eq!(
            (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
            (25., 10., 40., 20.)
        );
        assert_eq!((tree.size().width(), tree.size().height()), (400., 200.));
        assert_eq!(
            geometry_path("path(\"M 0 0 L 10 0 L 10 10 Z\")").as_deref(),
            Some("M 0 0 L 10 0 L 10 10 Z")
        );
        assert_eq!(geometry_path("none").as_deref(), Some(""));
    }
    #[test]
    fn svg_viewbox_preserves_unclipped_target_bounds_and_user_space_clip_plane() {
        // Native producer for intersection-observer/svg-clipped-rect-target.html.
        let output = compute(&fixture(), &LayoutLimits::default()).unwrap();
        let bounds = output.geometry[&6].box_model.as_ref().unwrap().border_box;
        assert!((bounds.x - 100.).abs() < 0.01);
        assert!((bounds.y - 50.).abs() < 0.01);
        assert!((bounds.width - 200.).abs() < 0.01);
        assert!((bounds.height - 100.).abs() < 0.01);
        let clip = output.paint_projections[&6]
            .clips
            .iter()
            .find(|clip| clip.owner == Some(5))
            .unwrap();
        let shape = clip.shape.as_ref().unwrap();
        assert!(shape.contains(75., 50.));
        assert!(!shape.contains(50., 25.));
        let bounds = transforms::Projection::from_matrix(clip.matrix).project(shape.bounds());
        assert!((bounds.x - 110.).abs() < 0.01);
        assert!((bounds.y - 60.).abs() < 0.01);
        assert!((bounds.width - 80.).abs() < 0.01);
        assert!((bounds.height - 80.).abs() < 0.01);
    }
    #[test]
    fn svg_presentation_hints_yield_to_author_none_and_author_geometry() {
        let mut input = fixture();
        input.stylesheets = vec!["g {clip-path:none;transform:none} rect {width:40px}".into()];
        input.nodes[4]
            .attributes
            .insert("transform".into(), "translate(10 20)".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(output.computed_styles[&5]["clip-path"], "none");
        assert!(
            !output.paint_projections[&6]
                .clips
                .iter()
                .any(|clip| clip.owner == Some(5))
        );
        let bounds = output.geometry[&6].box_model.as_ref().unwrap().border_box;
        assert!((bounds.x - 100.).abs() < 0.01);
        assert!((bounds.width - 80.).abs() < 0.01);
    }
    #[test]
    fn svg_attribute_transform_uses_svg_grammar_and_display_none_removes_geometry() {
        let mut input = fixture();
        input.nodes[4]
            .attributes
            .insert("transform".into(), "translate(10 20)".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let bounds = output.geometry[&6].box_model.as_ref().unwrap().border_box;
        assert!((bounds.x - 120.).abs() < 0.01);
        assert!((bounds.y - 90.).abs() < 0.01);
        input.stylesheets = vec!["g {display:none}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(!output.paint_projections.contains_key(&6));
    }
    #[test]
    fn nested_svg_viewport_keeps_its_own_transform_and_clip() {
        let mut input = fixture();
        input.nodes[4].tag = "svg".into();
        input.nodes[4].attributes = BTreeMap::from([
            ("x".into(), "20".into()),
            ("y".into(), "10".into()),
            ("width".into(), "80".into()),
            ("height".into(), "40".into()),
            ("viewBox".into(), "0 0 40 20".into()),
            ("overflow".into(), "hidden".into()),
        ]);
        input.nodes[5].attributes = BTreeMap::from([
            ("x".into(), "0".into()),
            ("y".into(), "0".into()),
            ("width".into(), "60".into()),
            ("height".into(), "30".into()),
        ]);
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let bounds = output.geometry[&6].box_model.as_ref().unwrap().border_box;
        assert!((bounds.x - 40.).abs() < 0.01);
        assert!((bounds.y - 20.).abs() < 0.01);
        assert!((bounds.width - 240.).abs() < 0.01);
        assert!(output.paint_projections[&6].clips.iter().any(|clip| {
            let bounds = transforms::Projection::from_matrix(clip.matrix).project(clip.rect);
            (bounds.x - 40.).abs() < 0.01
                && (bounds.y - 20.).abs() < 0.01
                && (bounds.width - 160.).abs() < 0.01
                && (bounds.height - 80.).abs() < 0.01
        }));
    }
    #[test]
    fn svg_image_geometry_does_not_depend_on_image_decoder_or_external_io() {
        let mut input = fixture();
        let image = &mut input.nodes[5];
        image.tag = "image".into();
        image
            .attributes
            .insert("href".into(), "https://unavailable.test/image.png".into());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let bounds = output.geometry[&6].box_model.as_ref().unwrap().border_box;
        assert!((bounds.x - 100.).abs() < 0.01);
        assert!((bounds.width - 200.).abs() < 0.01);
        assert!((bounds.height - 100.).abs() < 0.01);
    }
    #[test]
    fn foreign_object_descendants_share_paint_local_viewport_and_svg_transform() {
        let mut input = fixture();
        input.nodes[5].tag = "foreignObject".into();
        input.nodes[5].children = vec![7];
        input.nodes.push(serde_json::from_value(serde_json::json!({"id":7,"parent":6,"tag":"div","attributes":{"style":"width:50%;height:10px"}})).unwrap());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let bounds = output.geometry[&7].box_model.as_ref().unwrap().border_box;
        assert!((bounds.x - 100.).abs() < 0.01);
        assert!((bounds.y - 50.).abs() < 0.01);
        assert!((bounds.width - 100.).abs() < 0.01);
        assert!((bounds.height - 20.).abs() < 0.01);
        assert!(
            output.paint_projections[&7]
                .clips
                .iter()
                .any(|clip| clip.owner == Some(5))
        );
        input.stylesheets = vec!["foreignObject{display:none}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert!(!output.paint_projections.contains_key(&7));
        assert!(output.geometry[&7].box_model.is_none());
    }
    #[test]
    fn tspan_geometry_preserves_its_own_glyph_metric_extent() {
        let mut input = fixture();
        input.nodes[5].tag = "text".into();
        input.nodes[5].attributes =
            BTreeMap::from([("x".into(), "10".into()), ("y".into(), "30".into())]);
        input.nodes[5].text = "A".into();
        input.nodes[5].children = vec![7];
        input.nodes.push(serde_json::from_value(serde_json::json!({"id":7,"parent":6,"tag":"tspan","namespace":"http://www.w3.org/2000/svg","tree_scope":1,"attributes":{"x":"100","dy":"20"},"text":"B"})).unwrap());
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let parent = output.geometry[&6].box_model.as_ref().unwrap().border_box;
        let span = output.geometry[&7].box_model.as_ref().unwrap().border_box;
        assert!((span.x - 200.).abs() < 0.01);
        assert!(span.width > 0. && span.height > 0.);
        assert!(parent.width > span.width);
        assert!(span.y > parent.y);
        assert!(output.paint_projections.contains_key(&7));
    }
    #[test]
    fn svg_native_zoom_metadata_comes_from_the_computed_ancestor_cascade() {
        let mut input = fixture();
        input.stylesheets = vec!["body {zoom:.1} g {zoom:2}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        let scale = output.computed_styles[&6]["oryn-effective-zoom"]
            .parse::<f64>()
            .unwrap();
        assert!((scale - 0.2).abs() < 0.00001);
        input.stylesheets = vec!["body {zoom:10} g {zoom:normal}".into()];
        let output = compute(&input, &LayoutLimits::default()).unwrap();
        assert_eq!(
            output.computed_styles[&6]["oryn-effective-zoom"]
                .parse::<f64>()
                .unwrap(),
            10.
        );
    }
    #[test]
    fn svg_viewport_percentages_use_the_resolved_outer_box_once() {
        let mut input = fixture();
        input.nodes[1].attributes.remove("viewBox");
        input.nodes[1]
            .attributes
            .insert("width".into(), "50%".into());
        input.nodes[1]
            .attributes
            .insert("height".into(), "50%".into());
        input.nodes[4].tag = "svg".into();
        input.nodes[4].attributes = BTreeMap::from([
            ("width".into(), "50%".into()),
            ("height".into(), "25%".into()),
        ]);
        let by_id = input.nodes.iter().map(|node| (node.id, node)).collect();
        let viewport = local_viewport(
            &input.nodes[5],
            &by_id,
            &BTreeMap::new(),
            CssRect {
                x: 0.,
                y: 0.,
                width: 400.,
                height: 200.,
            },
        );
        assert_eq!((viewport.width, viewport.height), (200., 50.));
        input.nodes[1]
            .attributes
            .insert("viewBox".into(), "10 20 100 80".into());
        let by_id = input.nodes.iter().map(|node| (node.id, node)).collect();
        let viewport = local_viewport(
            &input.nodes[5],
            &by_id,
            &BTreeMap::new(),
            CssRect {
                x: 0.,
                y: 0.,
                width: 400.,
                height: 200.,
            },
        );
        assert_eq!((viewport.width, viewport.height), (50., 20.));
    }
    #[test]
    fn native_svg_reference_rewrites_decoded_ids_without_changing_foreign_urls() {
        let owner = fixture().nodes.remove(5);
        let ids = BTreeMap::from([("a b".into(), 19)]);
        assert_eq!(
            local_reference("#a%20b", &owner, &ids),
            "#oryn-svg-geometry-19"
        );
        assert_eq!(
            local_reference("https://example.test/page#a%20b", &owner, &ids),
            "#oryn-svg-geometry-19"
        );
        assert_eq!(
            local_reference("https://other.test/page#a%20b", &owner, &ids),
            "https://other.test/page#a%20b"
        );
    }
}
