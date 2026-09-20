//! Static inline SVG capture from the authoritative DOM and computed styles.

/// Inputs retained together for local effects with style sample.
pub(crate) struct LocalSvgEffectsInput<'a> {
    pub(crate) document: &'a ParsedDocument,
    pub(crate) styles: &'a oryn_layout::ComputedStyles,
    pub(crate) base: &'a str,
    pub(crate) images: &'a mut Vec<PaintImageResource>,
    pub(crate) limit: usize,
    pub(crate) native: Option<(&'a oryn_layout::LayoutInput, &'a oryn_layout::LayoutLimits)>,
    pub(crate) print_media: bool,
    pub(crate) sample: Option<&'a oryn_layout::svg_instances::StyleSample>,
}
/// Inputs retained together for capture impl.
pub(crate) struct SvgCaptureInput<'a> {
    pub(crate) document: &'a ParsedDocument,
    pub(crate) root: NodeId,
    pub(crate) styles: &'a oryn_layout::ComputedStyles,
    pub(crate) base: &'a str,
    pub(crate) size: Option<(f64, f64)>,
    pub(crate) max_bytes: usize,
    pub(crate) native: Option<(&'a oryn_layout::LayoutInput, &'a oryn_layout::LayoutLimits)>,
    pub(crate) inherited: Option<(&'a oryn_render_core::CapturedSvgInstances, &'a [u64])>,
    pub(crate) print_media: bool,
    pub(crate) sample: Option<&'a oryn_layout::svg_instances::StyleSample>,
    pub(crate) paint_keys: Option<&'a BTreeSet<u64>>,
}
/// Inputs retained together for capture with paint selection.
pub(crate) struct SelectedSvgCaptureInput<'a> {
    pub(crate) document: &'a ParsedDocument,
    pub(crate) root: NodeId,
    pub(crate) styles: &'a oryn_layout::ComputedStyles,
    pub(crate) base: &'a str,
    pub(crate) size: Option<(f64, f64)>,
    pub(crate) max_bytes: usize,
    pub(crate) native: (&'a oryn_layout::LayoutInput, &'a oryn_layout::LayoutLimits),
    pub(crate) inherited: Option<(&'a oryn_render_core::CapturedSvgInstances, &'a [u64])>,
    pub(crate) print_media: bool,
    pub(crate) sample: Option<&'a oryn_layout::svg_instances::StyleSample>,
    pub(crate) paint_keys: Option<&'a BTreeSet<u64>>,
}
/// Inputs retained together for capture with style sample.
pub(crate) struct StyledSvgCaptureInput<'a> {
    pub(crate) document: &'a ParsedDocument,
    pub(crate) root: NodeId,
    pub(crate) styles: &'a oryn_layout::ComputedStyles,
    pub(crate) base: &'a str,
    pub(crate) size: Option<(f64, f64)>,
    pub(crate) max_bytes: usize,
    pub(crate) native: (&'a oryn_layout::LayoutInput, &'a oryn_layout::LayoutLimits),
    pub(crate) inherited: Option<(&'a oryn_render_core::CapturedSvgInstances, &'a [u64])>,
    pub(crate) print_media: bool,
    pub(crate) sample: Option<&'a oryn_layout::svg_instances::StyleSample>,
}
/// Inputs retained together for capture with native input for media.
#[cfg(test)]
pub(crate) struct MediaSvgCaptureInput<'a> {
    pub(crate) document: &'a ParsedDocument,
    pub(crate) root: NodeId,
    pub(crate) styles: &'a oryn_layout::ComputedStyles,
    pub(crate) base: &'a str,
    pub(crate) size: Option<(f64, f64)>,
    pub(crate) max_bytes: usize,
    pub(crate) native: (&'a oryn_layout::LayoutInput, &'a oryn_layout::LayoutLimits),
    pub(crate) inherited: Option<(&'a oryn_render_core::CapturedSvgInstances, &'a [u64])>,
    pub(crate) print_media: bool,
}
/// Inputs retained together for capture with native input.
#[cfg(test)]
pub(crate) struct NativeSvgCaptureInput<'a> {
    pub(crate) document: &'a ParsedDocument,
    pub(crate) root: NodeId,
    pub(crate) styles: &'a oryn_layout::ComputedStyles,
    pub(crate) base: &'a str,
    pub(crate) size: Option<(f64, f64)>,
    pub(crate) max_bytes: usize,
    pub(crate) native: (&'a oryn_layout::LayoutInput, &'a oryn_layout::LayoutLimits),
    pub(crate) inherited: Option<(&'a oryn_render_core::CapturedSvgInstances, &'a [u64])>,
}
use crate::RuntimeError;
use oryn_common::v4::PaintImageResource;
use oryn_dom::{NodeId, NodeKind, html::ParsedDocument};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ForeignInstanceCapture {
    pub source: NodeId,
    pub identity: String,
    pub rect: oryn_common::v4::CssRect,
    pub ancestry: Vec<u64>,
}

pub(crate) struct CapturedSvg {
    pub resource: PaintImageResource,
    pub instances: Option<oryn_render_core::CapturedSvgInstances>,
    pub foreign_instances: Vec<ForeignInstanceCapture>,
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
/// Paint serialization follows the document tree, excluding inert template
/// contents. Public XMLSerializer continues to serialize template contents.
fn serialize_paint_xml(
    dom: &oryn_dom::DomRuntime,
    root: NodeId,
) -> Result<String, oryn_dom::DomError> {
    struct PaintResolver<'a>(&'a oryn_dom::DomRuntime);
    impl oryn_dom::SerializationResolver for PaintResolver<'_> {
        fn node(&self, node: oryn_dom::NodeRef) -> Result<oryn_dom::Node, oryn_dom::DomError> {
            self.0.arena().resolve(node).cloned()
        }
        fn template_content(
            &self,
            node: oryn_dom::NodeRef,
        ) -> Result<Option<oryn_dom::NodeRef>, oryn_dom::DomError> {
            self.0.arena().resolve(node)?;
            Ok(None)
        }
    }
    oryn_dom::serialize_xml_with_resolver(
        oryn_dom::NodeRef {
            arena: dom.arena().id(),
            node: root,
        },
        &PaintResolver(dom),
    )
}

fn native_source(
    dom: &oryn_dom::DomRuntime,
    root: NodeId,
    max_nodes: usize,
    max_bytes: usize,
) -> Result<(String, Vec<NodeId>), RuntimeError> {
    let fail = |detail: String| RuntimeError::PaintUnavailable(detail);
    let original = serialize_paint_xml(dom, root).map_err(|error| fail(error.to_string()))?;
    if original.len() > max_bytes.min(16 * 1024 * 1024) {
        return Err(fail("inline SVG original source byte limit".into()));
    }
    let mut elements = Vec::new();
    let mut pending = vec![(root, 0usize)];
    let mut visited = BTreeSet::new();
    while let Some((id, depth)) = pending.pop() {
        if depth > 256 || !visited.insert(id) || visited.len() > max_nodes {
            return Err(fail("inline SVG native identity traversal limit".into()));
        }
        let node = dom
            .arena()
            .get(id)
            .map_err(|error| fail(error.to_string()))?;
        if matches!(&node.kind, NodeKind::ShadowRoot { .. }) {
            continue;
        }
        if matches!(&node.kind, NodeKind::Element { .. }) {
            elements.push(id);
        }
        if matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" && name.local.as_ref() == "template")
        {
            continue;
        }
        pending.extend(node.children.iter().rev().map(|id| (*id, depth + 1)));
    }
    Ok((original, elements))
}

/// Publish instance-only demand before BindingLayout is handed to the CSS font
/// loader. Recomputed layout replaces prior demand, preserving invalidation.
#[cfg(test)]
pub(crate) fn publish_instance_fonts(
    dom: &oryn_dom::DomRuntime,
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
    output: &mut oryn_layout::LayoutOutput,
) -> Result<(), oryn_layout::LayoutError> {
    publish_instance_fonts_with_sample(dom, input, limits, output, None)
}

pub(crate) fn publish_instance_fonts_with_sample(
    dom: &oryn_dom::DomRuntime,
    input: &oryn_layout::LayoutInput,
    limits: &oryn_layout::LayoutLimits,
    output: &mut oryn_layout::LayoutOutput,
    sample: Option<&oryn_layout::svg_instances::StyleSample>,
) -> Result<(), oryn_layout::LayoutError> {
    let fail = |detail: String| oryn_layout::LayoutError::StyleTraversal(detail);
    let nodes = input
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut roots = BTreeSet::new();
    for node in &input.nodes {
        if node.namespace != "http://www.w3.org/2000/svg" || node.tag != "use" {
            continue;
        }
        let mut current = node.parent;
        let mut outer = None;
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err(fail("SVG font demand ancestry cycle".into()));
            }
            let ancestor = nodes
                .get(&id)
                .ok_or_else(|| fail("SVG font demand ancestor missing".into()))?;
            if ancestor.namespace == "http://www.w3.org/2000/svg" && ancestor.tag == "svg" {
                outer = Some(id);
            }
            current = ancestor.parent;
        }
        if let Some(root) = outer {
            roots.insert(root);
        }
    }
    for root in roots {
        let root = NodeId {
            slot: root as u32,
            generation: (root >> 32) as u32,
        };
        let (source, elements) = native_source(dom, root, limits.max_nodes, 16 * 1024 * 1024)
            .map_err(|error| fail(error.to_string()))?;
        let ids = elements
            .iter()
            .map(|id| (u64::from(id.generation) << 32) | u64::from(id.slot))
            .collect::<Vec<_>>();
        let projection = oryn_render_core::capture_svg_instances_with_style_sample(
            &source, &ids, input, limits, false, sample,
        )
        .map_err(|error| fail(error.to_string()))?;
        for (family, characters) in projection.text_fonts.demands {
            output
                .font_demands
                .entry(family)
                .or_default()
                .extend(characters);
        }
        output.paint_fonts.extend(projection.text_fonts.resources);
        output.diagnostics.extend(projection.text_fonts.diagnostics);
    }
    let demand_bytes = output
        .font_demands
        .iter()
        .try_fold(0usize, |total, (family, characters)| {
            total
                .checked_add(family.len() + 128)
                .and_then(|total| total.checked_add(characters.len() * 32))
        })
        .ok_or_else(|| fail("SVG font demand byte overflow".into()))?;
    if demand_bytes > 64 * 1024 * 1024 {
        return Err(fail("SVG font demand exceeds native bound".into()));
    }
    let font_bytes = output
        .paint_fonts
        .values()
        .try_fold(0usize, |total, face| total.checked_add(face.bytes.len()))
        .ok_or_else(|| fail("SVG font capture byte overflow".into()))?;
    if font_bytes > limits.max_page_font_bytes {
        return Err(fail("SVG font capture exceeds page budget".into()));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn capture(
    document: &ParsedDocument,
    root: NodeId,
    styles: &oryn_layout::ComputedStyles,
    base: &str,
    size: Option<(f64, f64)>,
    max_bytes: usize,
) -> Result<CapturedSvg, RuntimeError> {
    capture_impl(SvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native: None,
        inherited: None,
        print_media: false,
        sample: None,
        paint_keys: None,
    })
}

#[cfg(test)]
pub(crate) fn capture_with_native_input(
    params: NativeSvgCaptureInput<'_>,
) -> Result<CapturedSvg, RuntimeError> {
    let NativeSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
    } = params;

    capture_with_native_input_for_media(MediaSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media: false,
    })
}

#[cfg(test)]
pub(crate) fn capture_with_native_input_for_media(
    params: MediaSvgCaptureInput<'_>,
) -> Result<CapturedSvg, RuntimeError> {
    let MediaSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media,
    } = params;

    capture_with_style_sample(StyledSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media,
        sample: None,
    })
}

pub(crate) fn capture_with_style_sample(
    params: StyledSvgCaptureInput<'_>,
) -> Result<CapturedSvg, RuntimeError> {
    let StyledSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media,
        sample,
    } = params;

    capture_with_paint_selection(SelectedSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media,
        sample,
        paint_keys: None,
    })
}

/// Filter native direct paint after resolving the unchanged style/layout input.
/// Reference definitions and container effects remain available to selected use owners.
pub(crate) fn capture_with_paint_selection(
    params: SelectedSvgCaptureInput<'_>,
) -> Result<CapturedSvg, RuntimeError> {
    let SelectedSvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media,
        sample,
        paint_keys,
    } = params;

    capture_impl(SvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native: Some(native),
        inherited,
        print_media,
        sample,
        paint_keys,
    })
}

fn selection_override(
    document: &ParsedDocument,
    id: NodeId,
    keys: &BTreeSet<u64>,
) -> Result<Option<&'static str>, RuntimeError> {
    let failure = |error: &dyn std::fmt::Display| RuntimeError::PaintUnavailable(error.to_string());
    let packed = (u64::from(id.generation) << 32) | u64::from(id.slot);
    if keys.contains(&packed) {
        return Ok(None);
    }
    let node = document.dom.get(id).map_err(|error| failure(&error))?;
    let NodeKind::Element { name } = &node.kind else {
        return Ok(None);
    };
    if name.ns.as_ref() != "http://www.w3.org/2000/svg" {
        return Ok(None);
    }
    let mut ancestor = node.parent;
    let mut seen = BTreeSet::new();
    while let Some(id) = ancestor {
        if !seen.insert(id) {
            return Err(RuntimeError::PaintUnavailable(
                "SVG selection ancestor cycle".into(),
            ));
        }
        let record = document.dom.get(id).map_err(|error| failure(&error))?;
        if matches!(&record.kind, NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/2000/svg"
            && matches!(name.local.as_ref(), "defs" | "clipPath" | "mask" | "pattern" | "marker" | "filter" | "symbol"))
        {
            return Ok(None);
        }
        ancestor = record.parent;
    }
    Ok(match name.local.as_ref() {
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "image"
        | "use" => Some(";opacity:0 !important;"),
        // Text containers may have independently selected tspan descendants.
        "text" | "tspan" | "textPath" => Some(";fill:none !important;stroke:none !important;"),
        _ => None,
    })
}

fn capture_impl(params: SvgCaptureInput<'_>) -> Result<CapturedSvg, RuntimeError> {
    let SvgCaptureInput {
        document,
        root,
        styles,
        base,
        size,
        max_bytes,
        native,
        inherited,
        print_media,
        sample,
        paint_keys,
    } = params;

    let failure = |message: &str| RuntimeError::PaintUnavailable(message.into());
    let mut dom = document
        .clone()
        .into_runtime()
        .map_err(|error| failure(&error.to_string()))?;
    let instances = if let Some((input, limits)) = native {
        // Capture topology before computed declarations overwrite author styles.
        // The paired IDs come from the native DOM traversal, not author markers.
        let (original, elements) = native_source(&dom, root, limits.max_nodes, max_bytes)?;
        let ids = elements
            .iter()
            .map(|id| (u64::from(id.generation) << 32) | u64::from(id.slot))
            .collect::<Vec<_>>();
        let instances = if let Some((projection, ancestry)) = inherited {
            projection.for_subtree(&ids, ancestry)
        } else {
            oryn_render_core::capture_svg_instances_with_style_sample(
                &original,
                &ids,
                input,
                limits,
                print_media,
                sample,
            )
        }
        .map_err(|error| failure(&error.to_string()))?;
        for (id, packed) in elements.into_iter().zip(ids) {
            dom.set_attribute(id, "data-oryn-native-style-id", packed.to_string())
                .map_err(|error| failure(&error.to_string()))?;
        }
        Some(instances)
    } else {
        None
    };
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
            if matches!(name.local.as_ref(), "image" | "feImage" | "use")
                && let Some(value) = node
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
            let packed = (u64::from(id.generation) << 32) | u64::from(id.slot);
            if let Some(computed) = styles.get(&packed) {
                collect_presentation_resources(computed, &resolution_base, &mut resources)?;
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
                        value.push_str(&oryn_render_core::native_svg_style_urls(
                            content,
                            &resolution_base,
                        ));
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
        if let Some(keys) = paint_keys
            && let Some(suppression) = selection_override(document, id, keys)?
        {
            style.get_or_insert_with(String::new).push_str(suppression);
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
    if let Some(projection) = &instances {
        for (key, style) in &projection.computed {
            let id = NodeId {
                slot: key.source as u32,
                generation: (key.source >> 32) as u32,
            };
            let node = document
                .dom
                .get(id)
                .map_err(|error| failure(&error.to_string()))?;
            if matches!(&node.kind, NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/2000/svg")
            {
                collect_presentation_resources(style, &resolution_base, &mut resources)?;
            }
        }
    }
    if resources.len() > 256 {
        return Err(failure("inline SVG exceeds 256 resource references"));
    }
    let mut source =
        serialize_paint_xml(&dom, root).map_err(|error| failure(&error.to_string()))?;
    if source.len() > max_bytes.min(16 * 1024 * 1024) {
        return Err(failure("inline SVG source byte limit"));
    }
    let mut svg_instances = instances
        .as_ref()
        .map(|instances| {
            instances
                .presentation
                .iter()
                .map(|(key, presentation)| oryn_common::v4::SvgInstanceSnapshot {
                    ancestry: key.ancestry.clone(),
                    source: key.source,
                    presentation: presentation.clone(),
                    content: oryn_common::v4::SvgInstanceContent::Original,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut foreign_instances = Vec::new();
    if let Some(projection) = &instances {
        let mut prefix = "oryn-native-instance-".to_owned();
        // Avoid aliasing any authored identifier or previously unresolved URL,
        // including references held only in native computed declarations.
        while source.contains(&prefix)
            || styles
                .values()
                .any(|style| style.values().any(|value| value.contains(&prefix)))
            || projection
                .computed
                .values()
                .any(|style| style.values().any(|value| value.contains(&prefix)))
            || projection.pseudos.values().any(|pseudos| {
                pseudos
                    .values()
                    .any(|style| style.values().any(|value| value.contains(&prefix)))
            })
        {
            prefix.push('_');
        }
        let mut definitions = String::new();
        for (ordinal, instance) in svg_instances.iter_mut().enumerate() {
            let id = NodeId {
                slot: instance.source as u32,
                generation: (instance.source >> 32) as u32,
            };
            if !foreign_objects.contains_key(&id) {
                continue;
            }
            let style = projection
                .computed
                .iter()
                .find(|(key, _)| key.source == instance.source && key.ancestry == instance.ancestry)
                .map(|(_, style)| style.clone())
                .ok_or_else(|| failure("missing native foreign instance style"))?;
            let selected = BTreeMap::from([(instance.source, style)]);
            let rect =
                foreign_object_rect(document, id, &selected, viewport.unwrap_or((300., 150.)))?;
            if rect.width <= 0. || rect.height <= 0. {
                instance.content = oryn_common::v4::SvgInstanceContent::Empty;
                continue;
            }
            if foreign_objects.len() + foreign_instances.len() >= 256 {
                return Err(failure(
                    "inline SVG original and instance foreignObject count limit",
                ));
            }
            let identity = format!("{prefix}{ordinal}");
            definitions.push_str(&format!("<foreignObject xmlns=\"http://www.w3.org/2000/svg\" id=\"{identity}\" data-oryn-foreign-object=\"{identity}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>", rect.x, rect.y, rect.width, rect.height));
            instance.content = oryn_common::v4::SvgInstanceContent::Reference(identity.clone());
            foreign_instances.push(ForeignInstanceCapture {
                source: id,
                identity,
                rect,
                ancestry: instance.ancestry.clone(),
            });
        }
        if !definitions.is_empty() {
            let close = source
                .rfind("</")
                .ok_or_else(|| failure("captured SVG closing root missing"))?;
            source.insert_str(
                close,
                &format!("<defs xmlns=\"http://www.w3.org/2000/svg\">{definitions}</defs>"),
            );
        }
    }
    let metadata_bytes = svg_instances
        .iter()
        .try_fold(0usize, |size, instance| {
            size.checked_add(instance.presentation.len())
                .and_then(|size| {
                    size.checked_add(match &instance.content {
                        oryn_common::v4::SvgInstanceContent::Reference(value) => value.len(),
                        _ => 0,
                    })
                })
                .and_then(|size| size.checked_add(instance.ancestry.len() * 8 + 8))
        })
        .ok_or_else(|| failure("inline SVG instance metadata size overflow"))?;
    if source.len() > 16 * 1024 * 1024
        || source
            .len()
            .checked_add(metadata_bytes)
            .is_none_or(|total| total > max_bytes)
    {
        return Err(failure(
            "inline SVG source and instance metadata byte limit",
        ));
    }
    Ok(CapturedSvg {
        foreign_instances,
        instances,
        resource: PaintImageResource {
            svg_document_snapshot: true,
            svg_instances,
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

/// Resource discovery shares the native CSS tokenizer with ordinary painting.
/// Keep document-local fragments local; admitted external documents are queued
/// by the caller under the same document CSP and broker authority as hrefs.
fn collect_presentation_resources(
    style: &BTreeMap<String, String>,
    base: &str,
    output: &mut BTreeSet<String>,
) -> Result<(), RuntimeError> {
    let fail = |detail: String| RuntimeError::PaintUnavailable(detail);
    let mut document = url::Url::parse(base).map_err(|error| fail(error.to_string()))?;
    document.set_fragment(None);
    for property in [
        "fill",
        "stroke",
        "filter",
        "clip-path",
        "mask",
        "mask-image",
        "marker",
        "marker-start",
        "marker-mid",
        "marker-end",
    ] {
        let Some(value) = style.get(property) else {
            continue;
        };
        if value.len() > 16 * 1024 * 1024 {
            return Err(fail("inline SVG resource declaration byte limit".into()));
        }
        for reference in oryn_layout::cssom::resource_urls(value) {
            let mut target = document
                .join(&reference)
                .map_err(|error| fail(error.to_string()))?;
            let fragment = target.fragment().is_some();
            target.set_fragment(None);
            if fragment && target == document {
                continue;
            }
            output.insert(target.to_string());
            if output.len() > 256
                || output.iter().map(String::len).sum::<usize>() > 16 * 1024 * 1024
            {
                return Err(fail("inline SVG resource reference budget".into()));
            }
        }
    }
    Ok(())
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
                if let Ok(values) = values
                    && values.len() == 4
                    && values.iter().all(|n| n.is_finite())
                    && values[2] > 0.
                    && values[3] > 0.
                {
                    viewport = (values[2], values[3]);
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
    fn selection_preserves_definitions_and_container_effects_but_suppresses_own_paint() {
        let document = oryn_dom::html::parse(
            r##"<svg id="s"><defs><rect id="definition"/></defs><g id="group" opacity=".5"><rect id="selected"/><rect id="excluded"/><text id="text">first<tspan id="span">second</tspan></text><use id="use" href="#definition"/></g></svg>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let key = |name| {
            let id = dom.find_by_id(name).unwrap();
            (u64::from(id.generation) << 32) | u64::from(id.slot)
        };
        let keys = BTreeSet::from([key("selected"), key("span"), key("use")]);
        for name in ["s", "group", "definition", "selected", "span", "use"] {
            assert_eq!(
                selection_override(&document, dom.find_by_id(name).unwrap(), &keys).unwrap(),
                None,
                "{name}"
            );
        }
        assert_eq!(
            selection_override(&document, dom.find_by_id("excluded").unwrap(), &keys).unwrap(),
            Some(";opacity:0 !important;")
        );
        assert_eq!(
            selection_override(&document, dom.find_by_id("text").unwrap(), &keys).unwrap(),
            Some(";fill:none !important;stroke:none !important;")
        );
        assert_eq!(
            selection_override(&document, dom.find_by_id("use").unwrap(), &BTreeSet::new())
                .unwrap(),
            Some(";opacity:0 !important;")
        );
    }

    #[test]
    fn native_capture_preserves_instance_cascade_and_replaces_forged_markers() {
        let document = oryn_dom::html::parse(
            r##"<svg id="s" xmlns="http://www.w3.org/2000/svg"><defs id="d"><rect id="shape" data-oryn-native-style-id="900"/></defs><use id="u" href="#shape" style="--tone:red"/><use id="v" href="#shape" style="--tone:blue"/></svg>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let packed = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let before = dom.serialize_xml(root).unwrap();
        let mut nodes = Vec::new();
        for name in ["s", "d", "shape", "u", "v"] {
            let id = dom.find_by_id(name).unwrap();
            let node = dom.arena().get(id).unwrap();
            let NodeKind::Element { name: tag } = &node.kind else {
                panic!("expected element")
            };
            let mut attributes = BTreeMap::new();
            for attribute in ["id", "href", "style", "data-oryn-native-style-id"] {
                if let Some(value) = node.attributes.get_null(attribute) {
                    attributes.insert(attribute, value.clone());
                }
            }
            nodes.push(serde_json::json!({"id":packed(id), "parent":if id == root {None} else {node.parent.map(packed)}, "tag":tag.local.as_ref(), "namespace":tag.ns.as_ref(), "attributes":attributes, "children":node.children.iter().copied().map(packed).collect::<Vec<_>>()}));
        }
        let input: oryn_layout::LayoutInput = serde_json::from_value(serde_json::json!({"nodes":nodes,"stylesheets":["#shape{fill:var(--tone)}"],"document_base_url":"https://example.test/page","viewport":{"width":4,"height":2,"device_pixel_ratio":1},"scroll":{"x":0,"y":0,"content_width":4,"content_height":2}})).unwrap();
        let styles = BTreeMap::from([(
            packed(dom.find_by_id("shape").unwrap()),
            Arc::new(BTreeMap::from([("fill".into(), "black".into())])),
        )]);
        let captured = capture_with_native_input(NativeSvgCaptureInput {
            document: &document,
            root,
            styles: &styles,
            base: "https://example.test/page",
            size: Some((4., 2.)),
            max_bytes: 1024 * 1024,
            native: (&input, &oryn_layout::LayoutLimits::default()),
            inherited: None,
        })
        .unwrap();
        assert_eq!(captured.resource.svg_instances.len(), 2);
        for (owner, color) in [("u", "rgb(255, 0, 0)"), ("v", "rgb(0, 0, 255)")] {
            let instance = captured
                .resource
                .svg_instances
                .iter()
                .find(|value| value.ancestry == [packed(dom.find_by_id(owner).unwrap())])
                .unwrap();
            assert!(instance.presentation.contains(color));
        }
        let xml = String::from_utf8(captured.resource.bytes).unwrap();
        assert!(!xml.contains("data-oryn-native-style-id=\"900\""));
        assert!(xml.contains("fill:black !important"));
        assert_eq!(dom.serialize_xml(root).unwrap(), before);
    }

    #[test]
    fn native_foreign_instances_keep_independent_captures_and_empty_content() {
        let document = oryn_dom::html::parse(
            r##"<svg id="s"><defs id="d"><foreignObject id="shape"><div id="child">HTML</div></foreignObject></defs><use id="u" href="#shape" style="--width:2px;color:red"/><use id="v" href="#shape" style="--width:2px;color:blue"/><use id="empty" href="#shape" style="--width:0px"/></svg>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let packed = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let mut nodes = Vec::new();
        for name in ["s", "d", "shape", "child", "u", "v", "empty"] {
            let id = dom.find_by_id(name).unwrap();
            let node = dom.arena().get(id).unwrap();
            let NodeKind::Element { name: tag } = &node.kind else {
                panic!("expected element")
            };
            let mut attributes = BTreeMap::new();
            for attribute in ["id", "href", "style"] {
                if let Some(value) = node.attributes.get_null(attribute) {
                    attributes.insert(attribute, value.clone());
                }
            }
            let children = node
                .children
                .iter()
                .filter(|id| {
                    matches!(
                        &dom.arena().get(**id).unwrap().kind,
                        NodeKind::Element { .. }
                    )
                })
                .copied()
                .map(packed)
                .collect::<Vec<_>>();
            nodes.push(serde_json::json!({"id":packed(id),"parent":if id == root {None} else {node.parent.map(packed)},"tag":tag.local.as_ref(),"namespace":tag.ns.as_ref(),"attributes":attributes,"children":children}));
        }
        let input: oryn_layout::LayoutInput = serde_json::from_value(serde_json::json!({"nodes":nodes,"stylesheets":["#shape{width:var(--width,0px);height:2px} #child{display:block;background:currentColor}"],"document_base_url":"https://example.test/page","viewport":{"width":6,"height":2,"device_pixel_ratio":1},"scroll":{"x":0,"y":0,"content_width":6,"content_height":2}})).unwrap();
        let shape = dom.find_by_id("shape").unwrap();
        let styles = BTreeMap::from([(
            packed(shape),
            Arc::new(BTreeMap::from([
                ("width".into(), "0px".into()),
                ("height".into(), "2px".into()),
            ])),
        )]);
        let captured = capture_with_native_input(NativeSvgCaptureInput {
            document: &document,
            root,
            styles: &styles,
            base: "https://example.test/page",
            size: Some((6., 2.)),
            max_bytes: 1024 * 1024,
            native: (&input, &oryn_layout::LayoutLimits::default()),
            inherited: None,
        })
        .unwrap();
        assert_eq!(captured.foreign_rects[&shape].width, 0.);
        assert_eq!(captured.foreign_instances.len(), 2);
        assert_ne!(
            captured.foreign_instances[0].identity,
            captured.foreign_instances[1].identity
        );
        for instance in &captured.foreign_instances {
            assert_eq!(instance.rect.width, 2.);
            assert_eq!(instance.rect.height, 2.);
            assert_eq!(instance.source, shape);
        }
        for (owner, empty) in [("u", false), ("v", false), ("empty", true)] {
            let value = captured
                .resource
                .svg_instances
                .iter()
                .find(|value| {
                    value.source == packed(shape)
                        && value.ancestry == [packed(dom.find_by_id(owner).unwrap())]
                })
                .unwrap();
            if empty {
                assert!(matches!(
                    value.content,
                    oryn_common::v4::SvgInstanceContent::Empty
                ));
            } else {
                assert!(matches!(
                    value.content,
                    oryn_common::v4::SvgInstanceContent::Reference(_)
                ));
            }
        }
        let xml = String::from_utf8(captured.resource.bytes).unwrap();
        for instance in &captured.foreign_instances {
            assert!(xml.contains(&format!("id=\"{}\"", instance.identity)));
        }
    }

    #[test]
    fn presentation_resource_discovery_preserves_document_identity_and_css_escaping() {
        let style = BTreeMap::from([
            (
                "fill".into(),
                "url(https://example.test/page?mode=a#paint) red".into(),
            ),
            (
                "stroke".into(),
                r#"url("assets/pa\69 nt.svg#stroke")"#.into(),
            ),
            ("filter".into(), "url(https://other.test/f.svg#fx)".into()),
            (
                "mask-image".into(),
                "url(https://example.test/page?mode=b#mask)".into(),
            ),
            (
                "--unused".into(),
                "url(https://unused.test/ignored.svg)".into(),
            ),
        ]);
        let mut resources = BTreeSet::new();
        collect_presentation_resources(
            &style,
            "https://example.test/page?mode=a#old",
            &mut resources,
        )
        .unwrap();
        assert_eq!(
            resources,
            BTreeSet::from([
                "https://example.test/assets/paint.svg".into(),
                "https://other.test/f.svg".into(),
                "https://example.test/page?mode=b".into(),
            ])
        );
        collect_presentation_resources(&style, "https://example.test/page?mode=a", &mut resources)
            .unwrap();
        assert_eq!(resources.len(), 3);
    }

    #[test]
    fn presentation_resource_discovery_enforces_shared_reference_bound() {
        let mut resources = (0..256)
            .map(|id| format!("https://example.test/{id}.svg"))
            .collect();
        let style = BTreeMap::from([("fill".into(), "url(extra.svg#paint)".into())]);
        assert!(
            collect_presentation_resources(&style, "https://example.test/page", &mut resources)
                .is_err()
        );
    }

    #[test]
    fn early_instance_demand_precedes_admitted_face_capture() {
        use sha2::{Digest, Sha256};
        let document = oryn_dom::html::parse(
            r##"<svg id="s"><defs id="d"><text id="shape">P</text></defs><use id="u" href="#shape" style="font-family:InstanceOnly,sans-serif"/></svg>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let packed = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let mut nodes = Vec::new();
        for name in ["s", "d", "shape", "u"] {
            let id = dom.find_by_id(name).unwrap();
            let node = dom.arena().get(id).unwrap();
            let NodeKind::Element { name: tag } = &node.kind else {
                panic!("expected element")
            };
            let mut attributes = BTreeMap::new();
            for attribute in ["id", "href", "style"] {
                if let Some(value) = node.attributes.get_null(attribute) {
                    attributes.insert(attribute, value.clone());
                }
            }
            let children = node
                .children
                .iter()
                .filter(|id| {
                    matches!(
                        &dom.arena().get(**id).unwrap().kind,
                        NodeKind::Element { .. }
                    )
                })
                .copied()
                .map(packed)
                .collect::<Vec<_>>();
            nodes.push(serde_json::json!({"id":packed(id),"parent":if id == root {None} else {node.parent.map(packed)},"tag":tag.local.as_ref(),"namespace":tag.ns.as_ref(),"attributes":attributes,"children":children,"text":if name == "shape" {"P"} else {""}}));
        }
        let mut input: oryn_layout::LayoutInput = serde_json::from_value(serde_json::json!({"nodes":nodes,"stylesheets":["defs{display:none}"],"document_base_url":"https://example.test/page","viewport":{"width":100,"height":40,"device_pixel_ratio":1},"scroll":{"x":0,"y":0,"content_width":100,"content_height":40}})).unwrap();
        input.web_fonts.push(oryn_layout::WebFont {
            pending: true,
            blocked: false,
            order: 0,
            descriptors: "font-family:InstanceOnly;font-weight:400;font-style:normal".into(),
            family: "InstanceOnly".into(),
            source_url: "https://example.test/instance.ttf".into(),
            sha256: String::new(),
            bytes: Vec::new(),
        });
        let limits = oryn_layout::LayoutLimits::default();
        let admitted =
            include_bytes!("../../oryn-render-core/tests/assets/variabletest_matching.ttf")
                .to_vec();
        let key = format!("{:x}:0", Sha256::digest(&admitted));
        let mut pending = oryn_layout::compute(&input, &limits).unwrap();
        publish_instance_fonts(&dom, &input, &limits, &mut pending).unwrap();
        assert!(pending.font_demands["instanceonly"].contains(&('P' as u32)));
        assert!(!pending.paint_fonts.contains_key(&key));
        input.web_fonts[0].pending = false;
        input.web_fonts[0].sha256 = format!("{:x}", Sha256::digest(&admitted));
        input.web_fonts[0].bytes = admitted.clone();
        let mut loaded = oryn_layout::compute(&input, &limits).unwrap();
        publish_instance_fonts(&dom, &input, &limits, &mut loaded).unwrap();
        assert!(loaded.font_demands["instanceonly"].contains(&('P' as u32)));
        assert_eq!(loaded.paint_fonts[&key].bytes, admitted);
        assert_eq!(loaded.paint_fonts[&key].collection_index, 0);
    }

    #[test]
    fn paint_serialization_omits_template_contents_without_changing_dom_serializer() {
        let document = oryn_dom::html::parse(
            r##"<svg id="s"><foreignObject id="foreign"><div id="html"><template id="inert"><svg><rect id="hidden-shape"/></svg></template><span id="visible">visible</span></div></foreignObject><use id="use" href="#hidden-shape"/></svg>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let original = dom.serialize_xml(root).unwrap();
        assert!(original.contains("id=\"hidden-shape\""));
        let (paint, identities) = native_source(&dom, root, 100, 64 * 1024).unwrap();
        assert!(!paint.contains("id=\"hidden-shape\""));
        assert!(
            paint.contains("href=\"#hidden-shape\""),
            "unresolved authored references remain unresolved"
        );
        assert!(paint.contains("id=\"visible\""));
        let expected = ["s", "foreign", "html", "inert", "visible", "use"]
            .map(|name| dom.find_by_id(name).unwrap());
        assert_eq!(identities, expected);
        assert_eq!(dom.serialize_xml(root).unwrap(), original);
    }

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

/// Payload bytes charged to native image budgets, including instance styles,
/// ancestry, source identity and generated content references.
pub(crate) fn resource_bytes(image: &PaintImageResource) -> Option<usize> {
    image
        .svg_instances
        .iter()
        .try_fold(image.bytes.len(), |total, instance| {
            let ancestry_bytes = instance.ancestry.len().checked_mul(8)?.checked_add(8)?;
            let reference_bytes = match &instance.content {
                oryn_common::v4::SvgInstanceContent::Reference(value) => value.len(),
                _ => 0,
            };
            total
                .checked_add(instance.presentation.len())?
                .checked_add(ancestry_bytes)?
                .checked_add(reference_bytes)
        })
}

#[cfg(test)]
pub(crate) fn local_effects(
    document: &ParsedDocument,
    styles: &oryn_layout::ComputedStyles,
    base: &str,
    images: &mut Vec<PaintImageResource>,
    limit: usize,
    native: Option<(&oryn_layout::LayoutInput, &oryn_layout::LayoutLimits)>,
    print_media: bool,
) -> Result<(BTreeMap<String, usize>, BTreeSet<String>), RuntimeError> {
    local_effects_with_style_sample(LocalSvgEffectsInput {
        document,
        styles,
        base,
        images,
        limit,
        native,
        print_media,
        sample: None,
    })
}

pub(crate) fn local_effects_with_style_sample(
    params: LocalSvgEffectsInput<'_>,
) -> Result<(BTreeMap<String, usize>, BTreeSet<String>), RuntimeError> {
    let LocalSvgEffectsInput {
        document,
        styles,
        base,
        images,
        limit,
        native,
        print_media,
        sample,
    } = params;

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
        if let NodeKind::Element { name } = &node.kind
            && let Some(identifier) = node.attributes.get_null("id")
            && identities.insert(identifier.clone())
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
        pending.extend(node.children.iter().rev().copied());
    }
    let mut roots = BTreeMap::new();
    let mut references = BTreeMap::new();
    let mut resources = BTreeSet::new();
    let mut bytes = images
        .iter()
        .try_fold(0usize, |total, image| {
            total.checked_add(resource_bytes(image)?)
        })
        .filter(|bytes| *bytes <= limit)
        .ok_or_else(|| RuntimeError::PaintUnavailable("local effect resource budget".into()))?;
    for (identifier, root) in definitions {
        let index = if let Some(index) = roots.get(&root) {
            *index
        } else {
            let captured = capture_with_style_sample(StyledSvgCaptureInput {
                document,
                root,
                styles,
                base,
                size: None,
                max_bytes: limit.saturating_sub(bytes),
                native: native.ok_or_else(|| {
                    RuntimeError::PaintUnavailable(
                        "local SVG effects require current native layout input".into(),
                    )
                })?,
                inherited: None,
                print_media,
                sample,
            })?;
            bytes = bytes
                .checked_add(resource_bytes(&captured.resource).ok_or_else(|| {
                    RuntimeError::PaintUnavailable("local effect resource budget overflow".into())
                })?)
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
    fn input(document: &ParsedDocument) -> oryn_layout::LayoutInput {
        crate::runtime::layout_input(
            document,
            Default::default(),
            Default::default(),
            oryn_common::v4::PreferredColorScheme::Light,
            oryn_common::v4::ReducedMotion::NoPreference,
            &[],
        )
    }
    #[test]
    fn existing_instance_metadata_counts_toward_local_effect_budget() {
        let document = oryn_dom::html::parse("<p>no definitions</p>");
        assert!(
            local_effects(
                &document,
                &Default::default(),
                "about:blank",
                &mut Vec::new(),
                1024,
                None,
                false
            )
            .unwrap()
            .0
            .is_empty()
        );
        let resource = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: vec![oryn_common::v4::SvgInstanceSnapshot {
                ancestry: vec![1, 2],
                source: 3,
                presentation: "fill:red".into(),
                content: oryn_common::v4::SvgInstanceContent::Reference("target".into()),
            }],
            content_type: None,
            url: "about:blank".into(),
            url_aliases: Vec::new(),
            media_type: "image/svg+xml".into(),
            bytes: b"<svg/>".to_vec(),
        };
        // Six XML bytes, eight style bytes, two ancestors plus source, six reference bytes.
        let expected = 6 + 8 + 24 + 6;
        assert_eq!(resource_bytes(&resource), Some(expected));
        let mut images = vec![resource];
        assert!(
            local_effects(
                &document,
                &Default::default(),
                "about:blank",
                &mut images,
                expected - 1,
                Some((&input(&document), &Default::default())),
                false
            )
            .is_err()
        );
        assert_eq!(images.len(), 1);
        assert!(
            local_effects(
                &document,
                &Default::default(),
                "about:blank",
                &mut images,
                expected,
                Some((&input(&document), &Default::default())),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn hidden_definitions_bind_to_native_document_order_and_not_url_equality() {
        let document = oryn_dom::html::parse(
            r#"<div id="duplicate"></div><svg style="display:none"><filter id="duplicate"/><filter id="local"><feFlood flood-color="red"/></filter><mask id="mask"><rect width="10" height="10" fill="white"/></mask></svg>"#,
        );
        let mut images=vec![PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://same.test/".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="local"><feFlood flood-color="blue"/></filter></svg>"#.to_vec()}];
        let (map, _) = local_effects(
            &document,
            &Default::default(),
            "https://same.test/",
            &mut images,
            1024 * 1024,
            Some((&input(&document), &Default::default())),
            false,
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
            Some((&input(&other), &Default::default())),
            false,
        )
        .unwrap();
        assert!(other_map.is_empty());
    }
    #[test]
    fn local_mask_capture_uses_its_owning_animation_sample() {
        let document = oryn_dom::html::parse(
            r##"<style>use{animation:tint 2s linear both}@keyframes tint{from{fill:red}to{fill:blue}}</style><svg><defs><rect id="shape" width="10" height="10"/></defs><mask id="mask"><use id="animated" href="#shape"/></mask></svg>"##,
        );
        let native = input(&document);
        let dom = document.clone().into_runtime().unwrap();
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let source = pack(dom.find_by_id("shape").unwrap());
        let owner = pack(dom.find_by_id("animated").unwrap());
        let limits = oryn_layout::LayoutLimits::default();
        let mut engine = oryn_layout::StyleEngine::default();
        engine.compute_at(&native, &limits, 0.).unwrap();
        let mut fills = Vec::new();
        for time in [0.5, 1.5] {
            let layout = engine.compute_at(&native, &limits, time).unwrap();
            let sample = engine.instance_style_sample().unwrap();
            let mut images = Vec::new();
            let (effects, _) = local_effects_with_style_sample(LocalSvgEffectsInput {
                document: &document,
                styles: &layout.computed_styles,
                base: "about:blank",
                images: &mut images,
                limit: 1024 * 1024,
                native: Some((&native, &limits)),
                print_media: false,
                sample: Some(&sample),
            })
            .unwrap();
            let instance = images[effects["mask"]]
                .svg_instances
                .iter()
                .find(|instance| instance.source == source && instance.ancestry == [owner])
                .unwrap();
            let fill = layout.computed_styles[&owner]["fill"].clone();
            assert!(
                instance.presentation.contains(&format!("fill:{fill};")),
                "{}",
                instance.presentation
            );
            fills.push(fill);
        }
        assert_ne!(fills[0], fills[1]);
    }

    #[test]
    fn local_mask_capture_preserves_distinct_native_use_inheritance() {
        let document = oryn_dom::html::parse(
            r##"<style>#shape{fill:var(--tone)}@media print{#shape{fill:lime}}</style><svg><defs><rect id="shape" width="10" height="10"/></defs><mask id="mask"><use id="red" href="#shape" style="--tone:red"/><use id="blue" href="#shape" style="--tone:blue"/></mask></svg>"##,
        );
        let native = input(&document);
        let dom = document.clone().into_runtime().unwrap();
        let pack = |id: NodeId| (u64::from(id.generation) << 32) | u64::from(id.slot);
        let source = pack(dom.find_by_id("shape").unwrap());
        let mut images = Vec::new();
        let (effects, _) = local_effects(
            &document,
            &Default::default(),
            "about:blank",
            &mut images,
            1024 * 1024,
            Some((&native, &Default::default())),
            false,
        )
        .unwrap();
        let mut missing_context_images = Vec::new();
        assert!(
            local_effects(
                &document,
                &Default::default(),
                "about:blank",
                &mut missing_context_images,
                1024 * 1024,
                None,
                false
            )
            .is_err()
        );
        assert!(missing_context_images.is_empty());
        let resource = &images[effects["mask"]];
        for (name, color) in [("red", "rgb(255, 0, 0)"), ("blue", "rgb(0, 0, 255)")] {
            let owner = pack(dom.find_by_id(name).unwrap());
            let style = resource
                .svg_instances
                .iter()
                .find(|instance| instance.source == source && instance.ancestry == [owner])
                .unwrap();
            assert!(
                style.presentation.contains(color),
                "{name}: {}",
                style.presentation
            );
        }
        let mut print_images = Vec::new();
        let (print_effects, _) = local_effects(
            &document,
            &Default::default(),
            "about:blank",
            &mut print_images,
            1024 * 1024,
            Some((&native, &Default::default())),
            true,
        )
        .unwrap();
        let printed = &print_images[print_effects["mask"]];
        for name in ["red", "blue"] {
            let owner = pack(dom.find_by_id(name).unwrap());
            let style = printed
                .svg_instances
                .iter()
                .find(|instance| instance.source == source && instance.ancestry == [owner])
                .unwrap();
            assert!(
                style.presentation.contains("rgb(0, 255, 0)"),
                "print {name}: {}",
                style.presentation
            );
        }
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
