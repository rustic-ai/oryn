//! Synchronous native drag subtree paint. Called on the owning V8 thread;
//! resources are decoded snapshots supplied by that realm, never fetched here.

/// Inputs retained together for capture svg subtree.
struct DragSvgCaptureInput<'a> {
    document: &'a ParsedDocument,
    root: NodeId,
    input: &'a oryn_layout::LayoutInput,
    layout: &'a oryn_layout::LayoutOutput,
    sample: &'a oryn_layout::svg_instances::StyleSample,
    base: &'a str,
    viewport: ViewportState,
    images: &'a mut Vec<PaintImageResource>,
    objects: &'a mut Vec<SvgForeignObjectSnapshot>,
    fonts: &'a mut BTreeMap<String, PaintFontResource>,
    diagnostics: &'a mut Vec<String>,
    bytes: &'a mut usize,
    depth: usize,
    inherited: Option<(&'a oryn_render_core::CapturedSvgInstances, &'a [u64])>,
    decoded: &'a BTreeMap<NodeId, usize>,
    paint_keys: Option<&'a BTreeSet<u64>>,
}
use super::*;
use oryn_common::v4::*;
use oryn_js::drag_image::{Capture, MAX_BYTES, MAX_PIXELS, Snapshot};
pub(super) fn capture(
    provider: &LiveLayoutProvider,
    request: &Capture,
    custom_states: &BTreeMap<NodeId, BTreeSet<String>>,
    scroll: (f64, f64),
    element_scrolls: &BTreeMap<NodeId, (f64, f64)>,
) -> Result<Snapshot, String> {
    capture_filtered(
        provider,
        request,
        custom_states,
        scroll,
        element_scrolls,
        None,
    )
    .map(|(snapshot, _)| snapshot)
}
pub(super) fn capture_filtered(
    provider: &LiveLayoutProvider,
    request: &Capture,
    custom_states: &BTreeMap<NodeId, BTreeSet<String>>,
    scroll: (f64, f64),
    element_scrolls: &BTreeMap<NodeId, (f64, f64)>,
    paint_keys: Option<&BTreeSet<u64>>,
) -> Result<(Snapshot, Option<bool>), String> {
    if std::thread::current().id() != provider.owner_thread {
        return Err("drag capture entered on a foreign thread".into());
    }
    let (tag, tree, revision) = {
        let dom = provider.dom.borrow();
        if dom
            .arena()
            .node_document(request.source)
            .map_err(|e| e.to_string())?
            != Some(request.document)
        {
            return Err("foreign drag image source".into());
        }
        let NodeKind::Element { name } = &dom
            .arena()
            .get(request.source)
            .map_err(|e| e.to_string())?
            .kind
        else {
            return Err("drag image source must be an element".into());
        };
        (
            name.local.to_string(),
            dom.flat_tree_elements(request.source)
                .map_err(|e| e.to_string())?,
            dom.mutation_cursor(),
        )
    };
    if tag == "img"
        && request.polygons.is_empty()
        && request
            .resources
            .iter()
            .any(|resource| resource.node == request.source && resource.svg.is_none())
    {
        let image = request
            .resources
            .iter()
            .find(|image| image.node == request.source)
            .ok_or("drag image is not decoded")?;
        let snapshot = Snapshot {
            width: image.width,
            height: image.height,
            png: image.png.clone(),
            bounds: CssRect {
                x: 0.,
                y: 0.,
                width: image.width as f64,
                height: image.height as f64,
            },
            scale: 1.,
            hotspot: (request.hotspot.0 as f64, request.hotspot.1 as f64),
        };
        snapshot.validate()?;
        return Ok((snapshot, None));
    }
    let (layout_input, layout, style_sample) = provider.native_layout_with_input(
        Some(request.document),
        custom_states,
        scroll,
        element_scrolls,
    )?;
    let mut keys = tree
        .iter()
        .map(|(node, _)| pack(*node))
        .collect::<BTreeSet<_>>();
    let mut images = Vec::new();
    let mut image_by_node = BTreeMap::new();
    let mut bytes = 0usize;
    for resource in &request.resources {
        if !keys.contains(&pack(resource.node)) {
            continue;
        }
        let data = resource.svg.as_ref().unwrap_or(&resource.png);
        bytes = bytes.saturating_add(data.len());
        if bytes > MAX_BYTES {
            return Err("drag resource budget".into());
        }
        image_by_node.insert(resource.node, images.len());
        images.push(PaintImageResource {
            svg_document_snapshot: false,
            svg_instances: Vec::new(),
            content_type: None,
            url: resource.url.clone(),
            url_aliases: resource.url_aliases.clone(),
            media_type: if resource.svg.is_some() {
                "image/svg+xml"
            } else {
                "image/png"
            }
            .into(),
            bytes: data.clone(),
        });
    }
    for image in &request.css_resources {
        bytes = bytes
            .checked_add(
                crate::svg_capture::resource_bytes(image)
                    .ok_or("drag CSS resource byte overflow")?,
            )
            .ok_or("drag CSS resource byte overflow")?;
        if bytes > MAX_BYTES {
            return Err("drag CSS resource budget".into());
        }
        images.push(image.clone());
    }
    let (arena, _) = provider.dom.borrow().snapshot();
    let document = ParsedDocument {
        dom: arena,
        document: request.document,
        errors: Vec::new(),
        quirks_mode: html5ever::interface::QuirksMode::NoQuirks,
        events: Vec::new(),
    };
    let base = document
        .dom
        .get(request.document)
        .map_err(|error| error.to_string())?
        .document_metadata
        .as_ref()
        .and_then(|metadata| metadata.url.clone())
        .unwrap_or_else(|| "about:blank".into());
    let mut foreign_objects = Vec::new();
    let mut fonts = layout.paint_fonts.clone();
    let mut diagnostics = layout.diagnostics.clone();
    let mut nodes = Vec::new();
    let mut bounds = None;
    let mut svg_descendants = BTreeSet::new();
    for (node, parent) in tree {
        if svg_descendants.contains(&node) {
            continue;
        }
        let key = pack(node);
        let Some(mut geometry) = layout.geometry.get(&key).cloned() else {
            continue;
        };
        if geometry.availability != GeometryAvailability::Available {
            continue;
        }
        let Some(style) = layout.computed_styles.get(&key) else {
            continue;
        };
        let projection = layout.paint_projections.get(&key).cloned();
        let (tag, svg) = {
            let dom = provider.dom.borrow();
            let NodeKind::Element { name } =
                &dom.arena().get(node).map_err(|e| e.to_string())?.kind
            else {
                continue;
            };
            (
                name.local.to_string(),
                name.ns.as_ref() == "http://www.w3.org/2000/svg" && name.local.as_ref() == "svg",
            )
        };
        let mut image = image_by_node.get(&node).copied();
        if tag == "video" {
            image = capture_video_resource(&document, node, &mut images, &mut bytes)?;
        }
        if tag == "img" && image.is_none() && paint_keys.is_none_or(|keys| keys.contains(&key)) {
            return Err("drag subtree contains an undecoded image".into());
        }
        if svg {
            let (index, descendants) = capture_svg_subtree(DragSvgCaptureInput {
                document: &document,
                root: node,
                input: &layout_input,
                layout: &layout,
                sample: &style_sample,
                base: &base,
                viewport: provider.viewport.get(),
                images: &mut images,
                objects: &mut foreign_objects,
                fonts: &mut fonts,
                diagnostics: &mut diagnostics,
                bytes: &mut bytes,
                depth: 0,
                inherited: None,
                decoded: &image_by_node,
                paint_keys,
            })?;
            image = index;
            svg_descendants.extend(descendants);
            if image.is_none() {
                continue;
            }
        }
        if let Some(p) = &projection {
            let model = if p.fragment_boxes.is_empty() {
                vec![p.box_model.clone()]
            } else {
                p.fragment_boxes.clone()
            };
            for model in model {
                if let Some(rect) = projected(model.border_box, p.matrix) {
                    bounds = Some(union(bounds, rect));
                }
            }
        } else if let Some(model) = &geometry.box_model {
            bounds = Some(union(bounds, model.border_box));
        }
        geometry.in_viewport = true;
        // Subtree capture may be offscreen. Keep actual ancestor overflow clips,
        // but viewport presentation clipping is applied only by the crop.
        nodes.push(PaintNodeSnapshot {
            builtin_image: None,
            backdrop: layout.backdrops.get(&key).cloned().map(Box::new),
            control: layout.control_states.get(&key).cloned(),
            backdrop_style: layout
                .pseudo_styles
                .get(&key)
                .and_then(|pseudos| pseudos.get("backdrop"))
                .map(|style| (**style).clone()),
            top_layer: provider.dom.borrow().top_layer().position(|entry| {
                pack(entry.node) == key
                    && !matches!(
                        entry.kind,
                        oryn_dom::TopLayerKind::SelectPicker
                            | oryn_dom::TopLayerKind::TemporalPicker
                            | oryn_dom::TopLayerKind::PermissionPrompt
                    )
            }),
            node_id: key,
            parent_id: layout
                .paint_parent_overrides
                .get(&key)
                .copied()
                .unwrap_or_else(|| parent.map(pack).filter(|parent| keys.contains(parent))),
            tag,
            geometry,
            projection,
            style: (**style).clone(),
            text: if image.is_some() {
                Vec::new()
            } else {
                layout.text_fragments.get(&key).cloned().unwrap_or_default()
            },
            image_resource: image,
        });
    }
    for generated in &layout.generated_nodes {
        if !generated
            .parent_id
            .is_some_and(|owner| keys.contains(&owner))
        {
            continue;
        }
        if !keys.insert(generated.node_id) {
            return Err("generated drag paint identity collision".into());
        }
        let mut node = generated.clone();
        node.geometry.in_viewport = true;
        if let Some(url) = layout.generated_image_urls.get(&node.node_id) {
            node.image_resource = images
                .iter()
                .position(|image| image.url == *url || image.url_aliases.contains(url));
            if node.image_resource.is_none()
                && paint_keys.is_none_or(|keys| keys.contains(&node.node_id))
            {
                return Err("generated drag image is not decoded".into());
            }
        }
        if let Some(model) = &node.geometry.box_model {
            bounds = Some(union(bounds, model.border_box));
        }
        nodes.push(node);
    }
    let bounds = if request.polygons.is_empty() {
        bounds.ok_or("drag source has no rendered bounds")?
    } else {
        let mut bounds = None;
        for polygon in &request.polygons {
            let xs = [polygon[0], polygon[2], polygon[4], polygon[6]];
            let ys = [polygon[1], polygon[3], polygon[5], polygon[7]];
            let x = xs.into_iter().fold(f64::INFINITY, f64::min);
            let y = ys.into_iter().fold(f64::INFINITY, f64::min);
            bounds = Some(union(
                bounds,
                CssRect {
                    x,
                    y,
                    width: xs.into_iter().fold(f64::NEG_INFINITY, f64::max) - x,
                    height: ys.into_iter().fold(f64::NEG_INFINITY, f64::max) - y,
                },
            ));
        }
        bounds.ok_or("empty selected drag image")?
    };
    if fonts
        .values()
        .fold(0usize, |bytes, font| bytes.saturating_add(font.bytes.len()))
        > 32 * 1024 * 1024
    {
        return Err("drag font snapshot budget".into());
    }
    let viewport = provider.viewport.get();
    let (local_effects, effect_resources) = crate::svg_capture::local_effects_with_style_sample(
        crate::svg_capture::LocalSvgEffectsInput {
            document: &document,
            styles: &layout.computed_styles,
            base: &base,
            images: &mut images,
            limit: MAX_BYTES,
            native: Some((&layout_input, &oryn_layout::LayoutLimits::default())),
            print_media: false,
            sample: Some(&style_sample),
        },
    )
    .map_err(|error| error.to_string())?;
    for resource in effect_resources {
        if !resource.starts_with("data:")
            && !images
                .iter()
                .any(|image| image.url == resource || image.url_aliases.contains(&resource))
        {
            return Err("local effect dependency is not ready in native capture".into());
        }
    }
    let mut snapshot = PaintSnapshot {
        local_effects,
        visual_viewport: None,
        frames: Vec::new(),
        viewport_scroll: None,
        contract_version: CONTRACT_VERSION,
        page: PageInfo {
            page_id: PageId(0),
            opener_page_id: None,
            url: base.clone(),
            title: String::new(),
            document_generation: request.generation,
        },
        revision: Revision(revision.0),
        style_revision: StyleRevision(revision.0),
        layout_revision: LayoutRevision(revision.0),
        viewport,
        scroll: ScrollState {
            x: scroll.0,
            y: scroll.1,
            ..Default::default()
        },
        nodes,
        images,
        fonts,
        svg_foreign_objects: foreign_objects,
        diagnostics,
    };
    if let Some(inventory) = provider
        .paint_inventory
        .borrow()
        .as_ref()
        .filter(|inventory| {
            inventory.document == request.document
                && inventory.generation == request.generation
                && inventory.epoch == request.epoch
        })
    {
        // Resources retain their original resolved URLs. Geometry can only be
        // reused when current native layout and viewport/scroll still agree.
        let same_layout = inventory.mutation == revision
            && inventory.snapshot.viewport == viewport
            && inventory.snapshot.scroll.x == scroll.0
            && inventory.snapshot.scroll.y == scroll.1
            && snapshot
                .fonts
                .iter()
                .all(|(key, font)| inventory.snapshot.fonts.get(key) == Some(font))
            && svg_inventory_matches(
                &snapshot.images,
                &snapshot.svg_foreign_objects,
                &inventory.snapshot.images,
                &inventory.snapshot.svg_foreign_objects,
            )
            && snapshot.nodes.iter().all(|node| {
                inventory.snapshot.nodes.iter().any(|old| {
                    old.node_id == node.node_id
                        && {
                            let mut geometry = old.geometry.clone();
                            geometry.in_viewport = true;
                            geometry == node.geometry
                        }
                        && old.projection == node.projection
                        && old.style == node.style
                        && (node.tag != "video"
                            || video_resource_matches(
                                node.image_resource,
                                old.image_resource,
                                &snapshot.images,
                                &inventory.snapshot.images,
                            ))
                })
            });
        if same_layout {
            let mut cached = inventory.snapshot.clone();
            cached.nodes.retain(|node| keys.contains(&node.node_id));
            let images = cached
                .nodes
                .iter()
                .filter_map(|node| node.image_resource)
                .collect::<BTreeSet<_>>();
            retain_svg_object_dependencies(&mut cached.svg_foreign_objects, &images);
            let owners = cached
                .nodes
                .iter()
                .chain(
                    cached
                        .svg_foreign_objects
                        .iter()
                        .flat_map(|object| object.nodes.iter()),
                )
                .map(|node| node.node_id)
                .collect::<BTreeSet<_>>();
            cached
                .frames
                .retain(|frame| owners.contains(&frame.owner_node));
            // Canvas pixels change without a DOM mutation. Replace corresponding
            // inventory resources from this call's freshly captured native bytes.
            for node in &cached.nodes {
                if let Some(index) = node.image_resource
                    && let Some(resource) = request
                        .resources
                        .iter()
                        .find(|resource| pack(resource.node) == node.node_id)
                    && let Some(image) = cached.images.get_mut(index)
                {
                    image.url = resource.url.clone();
                    image.url_aliases = resource.url_aliases.clone();
                    image.media_type = if resource.svg.is_some() {
                        "image/svg+xml"
                    } else {
                        "image/png"
                    }
                    .into();
                    image.bytes = resource.svg.as_ref().unwrap_or(&resource.png).clone();
                }
            }
            cached.viewport_scroll = None;
            snapshot = cached;
        } else {
            for image in &inventory.snapshot.images {
                if !snapshot
                    .images
                    .iter()
                    .any(|existing| existing.url == image.url)
                {
                    snapshot.images.push(image.clone());
                }
            }
        }
    }
    if let Some(paint_keys) = paint_keys {
        let parents = snapshot
            .nodes
            .iter()
            .map(|node| (node.node_id, node.parent_id))
            .collect::<BTreeMap<_, _>>();
        let mut relevant = paint_keys.clone();
        for key in paint_keys {
            let mut current = parents.get(key).copied().flatten();
            let mut seen = BTreeSet::new();
            while let Some(key) = current {
                if !seen.insert(key) {
                    return Err("permission paint ancestor cycle".into());
                }
                relevant.insert(key);
                current = parents.get(&key).copied().flatten();
            }
        }
        for node in &snapshot.nodes {
            if paint_keys.contains(&node.node_id)
                && node.tag == "iframe"
                && !snapshot
                    .frames
                    .iter()
                    .any(|frame| frame.owner_node == node.node_id)
            {
                return Err("permission paint requires current native frame inventory".into());
            }
            if relevant.contains(&node.node_id)
                && (node
                    .style
                    .get("backdrop-filter")
                    .is_some_and(|value| value != "none")
                    || ["mask-border-source"]
                        .iter()
                        .any(|name| node.style.get(*name).is_some_and(|value| value != "none")))
            {
                return Err("permission paint needs unsupported native mask-border or isolated backdrop input".into());
            }
        }
        for node in &mut snapshot.nodes {
            if !paint_keys.contains(&node.node_id) {
                suppress_unselected_paint(node);
            }
        }
        snapshot
            .frames
            .retain(|frame| paint_keys.contains(&frame.owner_node));
    }
    // Inventory reuse and fallback may add resources after initial capture.
    // Enforce the same aggregate limits on the final renderer input.
    validate_capture_resources(
        &snapshot.images,
        &snapshot.fonts,
        MAX_BYTES,
        32 * 1024 * 1024,
    )?;
    let intrinsic = if tag == "img" && request.polygons.is_empty() {
        request
            .resources
            .iter()
            .find(|resource| resource.node == request.source)
    } else {
        None
    };
    let (artifact, has_ink) = if let Some(resource) = intrinsic {
        let index = snapshot
            .nodes
            .iter()
            .find(|node| node.node_id == pack(request.source))
            .and_then(|node| node.image_resource)
            .ok_or("missing intrinsic drag resource")?;
        oryn_render_core::render_drag_resource(
            &snapshot,
            index,
            resource.width,
            resource.height,
            MAX_PIXELS,
            MAX_BYTES,
        )
        .map(|artifact| (artifact, None))
    } else {
        oryn_render_core::render_drag_png_with_coverage(
            &snapshot,
            bounds,
            &request.polygons,
            viewport.device_pixel_ratio,
            MAX_PIXELS,
            MAX_BYTES,
        )
        .map(|(artifact, has_ink)| (artifact, Some(has_ink)))
    }
    .map_err(|e| e.to_string())?;
    let (bounds, scale) = if let Some(resource) = intrinsic {
        (
            CssRect {
                x: 0.,
                y: 0.,
                width: resource.width as f64,
                height: resource.height as f64,
            },
            1.,
        )
    } else {
        (bounds, viewport.device_pixel_ratio)
    };
    if provider.dom.borrow().mutation_cursor() != revision {
        return Err("drag source mutated during native capture".into());
    }
    let result = Snapshot {
        width: artifact.width,
        height: artifact.height,
        png: artifact.bytes,
        bounds,
        scale,
        hotspot: (
            request.hotspot.0 as f64 * scale,
            request.hotspot.1 as f64 * scale,
        ),
    };
    result.validate()?;
    Ok((result, has_ink))
}
fn pack(node: NodeId) -> u64 {
    (u64::from(node.generation) << 32) | u64::from(node.slot)
}
fn union(before: Option<CssRect>, rect: CssRect) -> CssRect {
    if let Some(before) = before {
        let x = before.x.min(rect.x);
        let y = before.y.min(rect.y);
        CssRect {
            x,
            y,
            width: before.right().max(rect.right()) - x,
            height: before.bottom().max(rect.bottom()) - y,
        }
    } else {
        rect
    }
}
fn projected(rect: CssRect, m: [f64; 16]) -> Option<CssRect> {
    let mut bounds = None;
    for (x, y) in [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ] {
        let w = m[3] * x + m[7] * y + m[15];
        if !w.is_finite() || w <= 0. {
            return None;
        }
        let (x, y) = (
            (m[0] * x + m[4] * y + m[12]) / w,
            (m[1] * x + m[5] * y + m[13]) / w,
        );
        bounds = Some(union(
            bounds,
            CssRect {
                x,
                y,
                width: 0.,
                height: 0.,
            },
        ));
    }
    bounds
}

// Synchronous composition uses only the realm's already-admitted resources.
// Each native reference receives its own HTML layout and nested SVG capture.
fn capture_svg_subtree(
    params: DragSvgCaptureInput<'_>,
) -> Result<(Option<usize>, BTreeSet<NodeId>), String> {
    let DragSvgCaptureInput {
        document,
        root,
        input,
        layout,
        sample,
        base,
        viewport,
        images,
        objects,
        fonts,
        diagnostics,
        bytes,
        depth,
        inherited,
        decoded,
        paint_keys,
    } = params;

    if depth >= 8 || objects.len() >= 256 {
        return Err("drag SVG composition depth/object budget".into());
    }
    let key = pack(root);
    let model = layout
        .paint_projections
        .get(&key)
        .map(|p| &p.box_model)
        .or_else(|| layout.geometry.get(&key).and_then(|g| g.box_model.as_ref()));
    let size = model.map(|m| (m.content_box.width, m.content_box.height));
    if size.is_some_and(|(w, h)| w.is_finite() && h.is_finite() && (w <= 0. || h <= 0.))
        || layout
            .computed_styles
            .get(&key)
            .and_then(|s| s.get("display"))
            .is_some_and(|s| s == "none")
    {
        return Ok((
            None,
            crate::svg_capture::descendants(document, root).map_err(|e| e.to_string())?,
        ));
    }
    let limits = oryn_layout::LayoutLimits::default();
    let captured = crate::svg_capture::capture_with_paint_selection(
        crate::svg_capture::SelectedSvgCaptureInput {
            document,
            root,
            styles: &layout.computed_styles,
            base,
            size,
            max_bytes: MAX_BYTES.saturating_sub(*bytes),
            native: (input, &limits),
            inherited,
            print_media: false,
            sample: Some(sample),
            paint_keys,
        },
    )
    .map_err(|e| e.to_string())?;
    for resource in &captured.resources {
        require_loaded_svg_resource(resource, images)?;
    }
    if let Some(projection) = &captured.instances {
        fonts.extend(projection.text_fonts.resources.clone());
        diagnostics.extend(projection.text_fonts.diagnostics.clone());
    }
    let resource_bytes =
        crate::svg_capture::resource_bytes(&captured.resource).ok_or("drag SVG byte overflow")?;
    *bytes = bytes
        .checked_add(resource_bytes)
        .filter(|n| *n <= MAX_BYTES)
        .ok_or("drag SVG resource budget")?;
    let image_index = images.len();
    images.push(captured.resource);
    let jobs = captured
        .foreign_objects
        .iter()
        .map(|(id, identity)| (*id, identity.clone(), captured.foreign_rects[id], None))
        .chain(captured.foreign_instances.iter().map(|instance| {
            (
                instance.source,
                instance.identity.clone(),
                instance.rect,
                Some(instance.ancestry.clone()),
            )
        }))
        .collect::<Vec<_>>();
    let empty_selection = BTreeSet::new();
    for (foreign, identity, rect, ancestry) in jobs {
        let job_keys = instance_paint_keys(paint_keys, ancestry.as_deref(), &empty_selection);
        if rect.width <= 0. || rect.height <= 0. {
            continue;
        }
        if objects.len() >= 256 {
            return Err("drag SVG foreignObject budget".into());
        }
        let viewport = ViewportState {
            width: rect.width,
            height: rect.height,
            device_pixel_ratio: viewport.device_pixel_ratio,
        };
        let mut styles = layout.computed_styles.clone();
        let mut pseudos = layout.pseudo_styles.clone();
        if let Some(ancestry) = &ancestry {
            let projection = captured
                .instances
                .as_ref()
                .ok_or("drag foreign instance lacks projection")?;
            for (key, style) in &projection.computed {
                if &key.ancestry != ancestry {
                    continue;
                }
                styles.insert(key.source, style.clone());
                pseudos.remove(&key.source);
                if let Some(value) = projection.pseudos.get(key) {
                    pseudos.insert(key.source, value.clone());
                }
            }
        }
        let local = oryn_layout::compute_subtree_with_styles_and_pseudos(
            input,
            pack(foreign),
            viewport,
            &styles,
            &pseudos,
            &limits,
        )
        .map_err(|e| e.to_string())?;
        let object_index = objects.len();
        objects.push(SvgForeignObjectSnapshot {
            image_resource: image_index,
            element_id: identity,
            viewport,
            nodes: Vec::new(),
        });
        let mut suppressed = BTreeSet::new();
        let mut nodes = Vec::new();
        for source in &input.nodes {
            if suppressed.contains(&source.id) {
                continue;
            }
            let Some(geometry) = local.geometry.get(&source.id) else {
                continue;
            };
            let Some(style) = local.computed_styles.get(&source.id) else {
                continue;
            };
            let native_id = NodeId {
                slot: source.id as u32,
                generation: (source.id >> 32) as u32,
            };
            let mut image_resource = decoded.get(&native_id).copied();
            if source.namespace == "http://www.w3.org/1999/xhtml" && source.tag == "video" {
                image_resource = capture_video_resource(document, native_id, images, bytes)?;
            }
            if source.namespace == "http://www.w3.org/2000/svg" && source.tag == "svg" {
                let id = NodeId {
                    slot: source.id as u32,
                    generation: (source.id >> 32) as u32,
                };
                let (index, descendants) = capture_svg_subtree(DragSvgCaptureInput {
                    document,
                    root: id,
                    input,
                    layout: &local,
                    sample,
                    base,
                    viewport,
                    images,
                    objects,
                    fonts,
                    diagnostics,
                    bytes,
                    depth: depth + 1,
                    inherited: captured
                        .instances
                        .as_ref()
                        .map(|p| (p, ancestry.as_deref().unwrap_or(&[]))),
                    decoded,
                    paint_keys: job_keys,
                })?;
                suppressed.extend(descendants.into_iter().map(pack));
                if index.is_none() {
                    continue;
                }
                image_resource = index;
            }
            if source.namespace == "http://www.w3.org/1999/xhtml"
                && source.tag == "img"
                && image_resource.is_none()
                && let Some(src) = source.attributes.get("src")
            {
                let url = url::Url::parse(src)
                    .or_else(|_| url::Url::parse(base).and_then(|b| b.join(src)))
                    .map_err(|e| e.to_string())?;
                image_resource = Some(
                    require_loaded_svg_resource(url.as_str(), images)?
                        .ok_or("drag foreignObject image is not decoded")?,
                );
            }
            require_loaded_style_resources(style, base, images)?;
            nodes.push(PaintNodeSnapshot {
                builtin_image: None,
                backdrop: None,
                control: local.control_states.get(&source.id).cloned(),
                backdrop_style: None,
                top_layer: None,
                node_id: source.id,
                parent_id: source.parent.filter(|id| local.geometry.contains_key(id)),
                tag: source.tag.clone(),
                geometry: geometry.clone(),
                projection: local.paint_projections.get(&source.id).cloned(),
                style: (**style).clone(),
                text: if image_resource.is_some() {
                    Vec::new()
                } else {
                    local
                        .text_fragments
                        .get(&source.id)
                        .cloned()
                        .unwrap_or_default()
                },
                image_resource,
            });
        }
        let mut ids = nodes.iter().map(|n| n.node_id).collect::<BTreeSet<_>>();
        for generated in &local.generated_nodes {
            if !generated.parent_id.is_some_and(|id| ids.contains(&id)) {
                continue;
            }
            if nodes.len() >= limits.max_nodes || !ids.insert(generated.node_id) {
                return Err("drag foreignObject generated identity/node budget".into());
            }
            require_loaded_style_resources(&generated.style, base, images)?;
            let mut node = generated.clone();
            if let Some(url) = local.generated_image_urls.get(&node.node_id) {
                node.image_resource = Some(
                    require_loaded_svg_resource(url, images)?
                        .ok_or("drag generated image is not decoded")?,
                );
            }
            nodes.push(node);
        }
        if let Some(keys) = job_keys {
            for node in &mut nodes {
                if !keys.contains(&node.node_id) {
                    suppress_unselected_paint(node);
                }
            }
        }
        objects[object_index].nodes = nodes;
        fonts.extend(local.paint_fonts);
        diagnostics.extend(local.diagnostics);
    }
    Ok((Some(image_index), captured.descendants))
}

// A use instance is painted by its outer native use owner; definition nodes
// need not themselves appear in the permission stacking-order selection.
fn instance_paint_keys<'a>(
    keys: Option<&'a BTreeSet<u64>>,
    ancestry: Option<&[u64]>,
    empty: &'a BTreeSet<u64>,
) -> Option<&'a BTreeSet<u64>> {
    match (keys, ancestry.and_then(|ancestry| ancestry.first())) {
        (Some(keys), Some(owner)) => {
            if keys.contains(owner) {
                None
            } else {
                Some(empty)
            }
        }
        _ => keys,
    }
}

fn suppress_unselected_paint(node: &mut PaintNodeSnapshot) {
    // The SVG resource already filters individual native descendants. Keep its
    // image transport even when this container's own paint is not selected.
    let svg_container = node.tag == "svg" && node.image_resource.is_some();
    if !svg_container {
        node.geometry.availability = GeometryAvailability::Hidden;
        node.image_resource = None;
    }
    for (property, value) in [
        ("background-color", "transparent"),
        ("background-image", "none"),
        ("box-shadow", "none"),
        ("outline-style", "none"),
        ("border-top-style", "none"),
        ("border-right-style", "none"),
        ("border-bottom-style", "none"),
        ("border-left-style", "none"),
        ("border-image-source", "none"),
    ] {
        node.style.insert(property.into(), value.into());
    }
    node.backdrop = None;
    node.backdrop_style = None;
    node.text.clear();
    node.builtin_image = None;
    node.control = None;
}

// Video pixels and poster state are owned by the native DOM, independently
// of the decoded image/canvas maps supplied by the binding capture request.
fn capture_video_resource(
    document: &ParsedDocument,
    node: NodeId,
    images: &mut Vec<PaintImageResource>,
    bytes: &mut usize,
) -> Result<Option<usize>, String> {
    let record = document.dom.get(node).map_err(|error| error.to_string())?;
    let poster = record.showing_poster && record.media_poster.is_some();
    let data = if poster {
        record.media_poster.as_ref()
    } else {
        record.media_frame.as_ref()
    };
    let Some(data) = data else {
        return Ok(None);
    };
    let resource = PaintImageResource {
        svg_document_snapshot: false,
        svg_instances: Vec::new(),
        content_type: None,
        url: format!("oryn:media-frame/{}", pack(node)),
        url_aliases: Vec::new(),
        media_type: if poster && record.media_poster_is_svg {
            "image/svg+xml"
        } else {
            "image/png"
        }
        .into(),
        bytes: data.clone(),
    };
    if let Some(index) = images.iter().position(|image| image == &resource) {
        return Ok(Some(index));
    }
    *bytes = bytes
        .checked_add(resource.bytes.len())
        .filter(|total| *total <= MAX_BYTES)
        .ok_or("drag native video resource budget")?;
    let index = images.len();
    images.push(resource);
    Ok(Some(index))
}

fn video_resource_matches(
    current: Option<usize>,
    cached: Option<usize>,
    images: &[PaintImageResource],
    cached_images: &[PaintImageResource],
) -> bool {
    match (current, cached) {
        (None, None) => true,
        (Some(current), Some(cached)) => match (images.get(current), cached_images.get(cached)) {
            (Some(current), Some(cached)) => current == cached,
            _ => false,
        },
        _ => false,
    }
}

// An HTML subtree can contain another SVG with its own HTML subtrees. Retain
// the transitive image dependencies before dropping unrelated page objects.
fn retain_svg_object_dependencies(
    objects: &mut Vec<SvgForeignObjectSnapshot>,
    roots: &BTreeSet<usize>,
) {
    let mut reachable = roots.clone();
    let mut pending = roots.iter().copied().collect::<Vec<_>>();
    while let Some(image) = pending.pop() {
        for object in objects
            .iter()
            .filter(|object| object.image_resource == image)
        {
            for dependency in object.nodes.iter().filter_map(|node| node.image_resource) {
                if reachable.insert(dependency) {
                    pending.push(dependency);
                }
            }
        }
    }
    objects.retain(|object| reachable.contains(&object.image_resource));
}

fn validate_capture_resources(
    images: &[PaintImageResource],
    fonts: &BTreeMap<String, PaintFontResource>,
    image_limit: usize,
    font_limit: usize,
) -> Result<(), String> {
    images
        .iter()
        .try_fold(0usize, |total, image| {
            total
                .checked_add(crate::svg_capture::resource_bytes(image)?)
                .filter(|total| *total <= image_limit)
        })
        .ok_or("drag final image resource budget")?;
    fonts
        .values()
        .try_fold(0usize, |total, font| {
            total
                .checked_add(font.bytes.len())
                .filter(|total| *total <= font_limit)
        })
        .ok_or("drag final font resource budget")?;
    Ok(())
}

// Node geometry alone cannot detect animation or text changes inside a
// suppressed SVG subtree, or new native video pixels. Compare native content
// and remap resource indices.
fn svg_inventory_matches(
    images: &[PaintImageResource],
    objects: &[SvgForeignObjectSnapshot],
    cached_images: &[PaintImageResource],
    cached_objects: &[SvgForeignObjectSnapshot],
) -> bool {
    images
        .iter()
        .enumerate()
        .filter(|(_, image)| {
            image.svg_document_snapshot || image.url.starts_with("oryn:media-frame/")
        })
        .all(|(index, image)| {
            let Some(cached_index) = cached_images.iter().position(|cached| cached == image) else {
                return false;
            };
            let current = objects
                .iter()
                .filter(|object| object.image_resource == index)
                .collect::<Vec<_>>();
            let cached = cached_objects
                .iter()
                .filter(|object| object.image_resource == cached_index)
                .collect::<Vec<_>>();
            current.len() == cached.len()
                && current.iter().all(|object| {
                    let mut remapped = (**object).clone();
                    remapped.image_resource = cached_index;
                    cached.contains(&&remapped)
                })
        })
}

fn require_loaded_svg_resource(
    url: &str,
    images: &[PaintImageResource],
) -> Result<Option<usize>, String> {
    let index = images.iter().position(|image| {
        !image.svg_document_snapshot
            && (image.url == url || image.url_aliases.iter().any(|alias| alias == url))
    });
    if index.is_none() && !url.starts_with("data:") {
        return Err(format!("drag SVG dependency is not decoded: {url}"));
    }
    Ok(index)
}

fn require_loaded_style_resources(
    style: &BTreeMap<String, String>,
    base: &str,
    images: &[PaintImageResource],
) -> Result<(), String> {
    for name in ["background-image", "border-image-source", "mask-image"] {
        let Some(value) = style.get(name) else {
            continue;
        };
        for source in oryn_layout::cssom::resource_urls(value) {
            if source.starts_with('#') {
                continue;
            }
            let url = url::Url::parse(&source)
                .or_else(|_| url::Url::parse(base).and_then(|b| b.join(&source)))
                .map_err(|e| e.to_string())?;
            require_loaded_svg_resource(url.as_str(), images)?;
        }
    }
    Ok(())
}

/// Installed only at a native capture completion boundary, never from script.
/// The revision stamp is the real DOM cursor, not the semantic report revision.
pub(super) struct Inventory {
    document: NodeId,
    generation: u64,
    epoch: u64,
    mutation: MutationCursor,
    snapshot: PaintSnapshot,
}
pub(super) fn install(
    provider: &LiveLayoutProvider,
    document: NodeId,
    generation: u64,
    epoch: u64,
    snapshot: &PaintSnapshot,
) -> Result<(), String> {
    if std::thread::current().id() != provider.owner_thread {
        return Err("paint inventory foreign thread".into());
    }
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|count| *count <= 64 * 1024 * 1024)
                .ok_or_else(|| std::io::Error::other("paint inventory byte limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(0), snapshot).map_err(|error| error.to_string())?;
    let mutation = provider.dom.borrow().mutation_cursor();
    *provider.paint_inventory.borrow_mut() = Some(Inventory {
        document,
        generation,
        epoch,
        mutation,
        snapshot: snapshot.clone(),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection_pixels(markup: &str, selected: Option<&[&str]>) -> image::RgbaImage {
        let document = oryn_dom::html::parse(markup);
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let keys = selected.map(|names| {
            names
                .iter()
                .map(|name| pack(dom.find_by_id(name).unwrap()))
                .collect::<BTreeSet<_>>()
        });
        let mut input = crate::runtime::layout_input(
            &document,
            Default::default(),
            Default::default(),
            PreferredColorScheme::Light,
            ReducedMotion::NoPreference,
            &[],
        );
        input.document_base_url = "https://example.test/page".into();
        for node in &mut input.nodes {
            node.document_url = Some(input.document_base_url.clone());
        }
        let mut engine = oryn_layout::StyleEngine::default();
        let layout = engine
            .compute_at(&input, &oryn_layout::LayoutLimits::default(), 0.)
            .unwrap();
        let sample = engine.instance_style_sample().unwrap();
        let mut images = Vec::new();
        let mut objects = Vec::new();
        let mut fonts = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let mut bytes = 0;
        let (index, _) = capture_svg_subtree(DragSvgCaptureInput {
            document: &document,
            root,
            input: &input,
            layout: &layout,
            sample: &sample,
            base: "https://example.test/page",
            viewport: input.viewport,
            images: &mut images,
            objects: &mut objects,
            fonts: &mut fonts,
            diagnostics: &mut diagnostics,
            bytes: &mut bytes,
            depth: 0,
            inherited: None,
            decoded: &BTreeMap::new(),
            paint_keys: keys.as_ref(),
        })
        .unwrap();
        let snapshot = PaintSnapshot {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id: PageId(0),
                opener_page_id: None,
                url: "https://example.test/page".into(),
                title: String::new(),
                document_generation: 0,
            },
            revision: Revision(0),
            style_revision: StyleRevision(0),
            layout_revision: LayoutRevision(0),
            local_effects: BTreeMap::new(),
            visual_viewport: None,
            viewport_scroll: None,
            frames: Vec::new(),
            viewport: input.viewport,
            scroll: ScrollState::default(),
            nodes: Vec::new(),
            images,
            fonts,
            svg_foreign_objects: objects,
            diagnostics,
        };
        let artifact = oryn_render_core::render_drag_resource(
            &snapshot,
            index.unwrap(),
            20,
            10,
            MAX_PIXELS,
            MAX_BYTES,
        )
        .unwrap();
        image::load_from_memory(&artifact.bytes).unwrap().to_rgba8()
    }

    #[test]
    fn selected_html_and_nested_svg_pixels_survive_unselected_containers() {
        let fixtures = [
            r##"<svg id="s" width="20" height="10"><g opacity=".5"><foreignObject width="20" height="10"><div xmlns="http://www.w3.org/1999/xhtml" style="position:relative;width:20px;height:10px"><div id="earlier" style="position:absolute;left:0;top:0;width:10px;height:10px;background:red"></div><div id="later" style="position:absolute;left:10px;top:0;width:10px;height:10px;background:blue"></div></div></foreignObject></g></svg>"##,
            r##"<svg id="s" width="20" height="10"><foreignObject width="20" height="10"><div xmlns="http://www.w3.org/1999/xhtml" style="width:20px;height:10px"><svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><g opacity=".5"><rect id="earlier" width="10" height="10" fill="red"/><rect id="later" x="10" width="10" height="10" fill="blue"/></g></svg></div></foreignObject></svg>"##,
        ];
        for markup in fixtures {
            let filtered = selection_pixels(markup, Some(&["later"]));
            assert_eq!(
                filtered.get_pixel(5, 5).0[3],
                0,
                "unselected sibling painted"
            );
            let blue = filtered.get_pixel(15, 5).0;
            assert_eq!(&blue[..3], &[0, 0, 255]);
            assert!(
                (127..=128).contains(&blue[3]),
                "ancestor opacity lost: {blue:?}"
            );
            let unfiltered = selection_pixels(markup, None);
            let red = unfiltered.get_pixel(5, 5).0;
            assert_eq!(&red[..3], &[255, 0, 0]);
            assert!((127..=128).contains(&red[3]));
            assert_eq!(unfiltered.get_pixel(15, 5), filtered.get_pixel(15, 5));
        }
    }

    #[test]
    fn native_permission_provider_sees_selected_html_inside_unselected_svg() {
        let parsed = oryn_dom::html::parse(
            r##"<style>html,body{margin:0;padding:0}</style><div id="root" style="width:20px;height:10px"><svg width="20" height="10"><foreignObject width="20" height="10"><div xmlns="http://www.w3.org/1999/xhtml" style="position:relative;width:20px;height:10px"><div id="earlier" style="position:absolute;left:0;top:0;width:10px;height:10px;background:red"></div><div id="later" style="position:absolute;left:10px;top:0;width:10px;height:10px;background:blue"></div></div></foreignObject></svg></div>"##,
        );
        let document = parsed.document;
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let root = dom.borrow().find_by_id("root").unwrap();
        let later = dom.borrow().find_by_id("later").unwrap();
        let provider = Rc::new(LiveLayoutProvider {
            style_lock: oryn_layout::cssom::new_native_style_lock(),
            retained_stylesheets: Rc::new(RefCell::new(RetainedStylesheets::default())),
            native_css_handles: Rc::new(RefCell::new(NativeCssHandleRegistry::default())),
            stylesheet_graph_revision: Cell::new(0),
            native_document_identity: Cell::new(None),
            shared_retained_admission: RefCell::new(None),
            native_retained_allocation: RefCell::new(None),
            retained_admission: RefCell::new(None),
            retained_cascade_key: RefCell::new(None),
            retained_cascade_key_charge: RefCell::new(None),
            retained_revision_key: RefCell::new(None),
            retained_revision_key_charge: RefCell::new(None),
            style_engine: RefCell::new(Default::default()),
            animation_time: Cell::new(0.),
            animation_revision: Cell::new(0),
            font_revision: Cell::new(0),
            layout_generation: Cell::new(0),
            layout_environment: RefCell::new(None),
            owner_thread: std::thread::current().id(),
            paint_inventory: RefCell::new(None),
            fonts: RefCell::new(Vec::new()),
            dom,
            document,
            viewport: Cell::new(oryn_common::v4::ViewportState::default()),
            scrollbar_width: Cell::new(0.),
            native_media: Cell::new((false, false)),
        });
        provider.set_viewport(20., 10.);
        let mut request = Capture {
            css_resources: Vec::new(),
            resources: Vec::new(),
            document,
            source: root,
            generation: 0,
            epoch: 0,
            hotspot: (0, 0),
            polygons: vec![[10., 0., 20., 0., 20., 10., 10., 10.]],
        };
        let selected = BTreeSet::from([pack(later)]);
        let coverage = |request: &Capture, keys: &BTreeSet<u64>| {
            let (captured, has_ink) = capture_filtered(
                &provider,
                request,
                &BTreeMap::new(),
                (0., 0.),
                &BTreeMap::new(),
                Some(keys),
            )
            .unwrap();
            let decoded_ink = oryn_render_core::captured_png_has_ink(
                &captured.png,
                captured.width,
                captured.height,
            )
            .unwrap();
            assert_eq!(has_ink, Some(decoded_ink));
            let coverage = provider
                .permission_paint_coverage(
                    request,
                    keys,
                    &BTreeMap::new(),
                    (0., 0.),
                    &BTreeMap::new(),
                )
                .unwrap();
            assert_eq!(coverage, Some(decoded_ink));
            coverage
        };
        assert_eq!(coverage(&request, &selected), Some(true));
        request.polygons = vec![[0., 0., 10., 0., 10., 10., 0., 10.]];
        assert_eq!(coverage(&request, &selected), Some(false));
        assert_eq!(coverage(&request, &BTreeSet::new()), Some(false));
    }

    #[test]
    fn filtered_vector_and_html_pixels_keep_unselected_clip_and_filter_definitions() {
        for content in [
            "<rect id='chosen' x='10' width='10' height='10' fill='blue'/>",
            "<foreignObject width='20' height='10'><div xmlns='http://www.w3.org/1999/xhtml' style='position:relative;width:20px;height:10px'><div id='chosen' style='position:absolute;left:10px;top:0;width:10px;height:10px;background:blue'></div></div></foreignObject>",
        ] {
            let markup = format!(
                "<svg id='s' width='20' height='10'><defs><clipPath id='clip'><rect x='10' width='5' height='10'/></clipPath><filter id='half' filterUnits='userSpaceOnUse' x='0' y='0' width='20' height='10' color-interpolation-filters='sRGB'><feComponentTransfer><feFuncA type='linear' slope='.5'/></feComponentTransfer></filter></defs><g clip-path='url(#clip)' filter='url(#half)'>{content}</g></svg>"
            );
            let filtered = selection_pixels(&markup, Some(&["chosen"]));
            let visible = filtered.get_pixel(12, 5).0;
            assert_eq!(&visible[..3], &[0, 0, 255]);
            assert!(
                (127..=128).contains(&visible[3]),
                "native ancestor filter lost: {visible:?}"
            );
            assert_eq!(filtered.get_pixel(17, 5).0[3], 0, "ancestor clipping lost");
            assert_eq!(filtered.get_pixel(5, 5).0[3], 0);
            assert_eq!(filtered, selection_pixels(&markup, None));
        }
    }

    #[test]
    fn selected_use_pixels_do_not_require_definition_membership() {
        for definition in [
            "<defs><rect id='shape' width='10' height='10' fill='currentColor'/></defs>",
            "<rect id='shape' width='10' height='10' fill='currentColor'/>",
        ] {
            let markup = format!(
                "<svg id='s' width='20' height='10'>{definition}<use href='#shape' style='color:red'/><use id='chosen' href='#shape' x='10' style='color:blue'/></svg>"
            );
            let pixels = selection_pixels(&markup, Some(&["chosen"]));
            assert_eq!(pixels.get_pixel(5, 5).0[3], 0);
            assert_eq!(pixels.get_pixel(15, 5).0, [0, 0, 255, 255]);
        }
    }

    #[test]
    fn filtered_svg_container_keeps_selected_descendants_and_effects() {
        let mut node: PaintNodeSnapshot = serde_json::from_value(serde_json::json!({
            "node_id":1, "tag":"svg", "geometry":{"availability":"available"}, "image_resource":0,
            "style":{"opacity":"0.5", "clip-path":"url(#clip)", "filter":"url(#filter)",
                "background-color":"red", "border-top-style":"solid", "box-shadow":"0 0 4px black"},
        }))
        .unwrap();
        suppress_unselected_paint(&mut node);
        assert_eq!(node.image_resource, Some(0));
        assert_eq!(node.geometry.availability, GeometryAvailability::Available);
        assert_eq!(node.style["opacity"], "0.5");
        assert_eq!(node.style["clip-path"], "url(#clip)");
        assert_eq!(node.style["filter"], "url(#filter)");
        assert_eq!(node.style["background-color"], "transparent");
        assert_eq!(node.style["border-top-style"], "none");
        assert_eq!(node.style["box-shadow"], "none");
        node.tag = "div".into();
        suppress_unselected_paint(&mut node);
        assert_eq!(node.image_resource, None);
        assert_eq!(node.geometry.availability, GeometryAvailability::Hidden);
    }

    #[test]
    fn instance_selection_follows_use_owner_not_definition_membership() {
        let keys = BTreeSet::from([10, 30]);
        let empty = BTreeSet::new();
        assert_eq!(instance_paint_keys(Some(&keys), None, &empty), Some(&keys));
        assert!(instance_paint_keys(Some(&keys), Some(&[10, 20]), &empty).is_none());
        assert_eq!(
            instance_paint_keys(Some(&keys), Some(&[20, 30]), &empty),
            Some(&empty)
        );
        assert!(instance_paint_keys(None, Some(&[20]), &empty).is_none());
    }

    #[test]
    fn drag_inventory_retains_nested_svg_objects_and_terminates_cycles() {
        let object = |image: usize, dependency: Option<usize>| SvgForeignObjectSnapshot {
            image_resource: image,
            element_id: format!("object-{image}"),
            viewport: ViewportState::default(),
            nodes: dependency
                .into_iter()
                .map(|dependency| {
                    serde_json::from_value(serde_json::json!({
                "node_id": image as u64 + 1, "tag":"svg", "geometry":{"availability":"available"},
                "style":{}, "image_resource":dependency,
            })).unwrap()
                })
                .collect(),
        };
        // Root 2 owns HTML which embeds SVG 5, then SVG 8. The back edge must
        // terminate; disconnected object 9 must not survive subtree capture.
        let mut objects = vec![
            object(8, Some(2)),
            object(9, None),
            object(5, Some(8)),
            object(2, Some(5)),
        ];
        retain_svg_object_dependencies(&mut objects, &BTreeSet::from([2]));
        assert_eq!(
            objects
                .iter()
                .map(|object| object.image_resource)
                .collect::<Vec<_>>(),
            vec![8, 5, 2]
        );
        retain_svg_object_dependencies(&mut objects, &BTreeSet::new());
        assert!(objects.is_empty());
    }

    #[test]
    fn native_drag_video_tracks_poster_frame_and_budget_without_recharging_instances() {
        let mut document = oryn_dom::html::parse("<video id='v'></video>");
        let node = document
            .clone()
            .into_runtime()
            .unwrap()
            .find_by_id("v")
            .unwrap();
        {
            let record = document.dom.get_mut(node).unwrap();
            record.media_frame = Some(vec![1, 2, 3]);
            record.media_poster = Some(b"<svg/>".to_vec());
            record.media_poster_is_svg = true;
            record.showing_poster = true;
        }
        let mut images = Vec::new();
        let mut bytes = 0;
        let poster = capture_video_resource(&document, node, &mut images, &mut bytes)
            .unwrap()
            .unwrap();
        assert_eq!(images[poster].media_type, "image/svg+xml");
        assert_eq!(images[poster].bytes, b"<svg/>");
        assert!(!images[poster].svg_document_snapshot);
        assert_eq!(bytes, 6);
        assert_eq!(
            capture_video_resource(&document, node, &mut images, &mut bytes).unwrap(),
            Some(poster)
        );
        assert_eq!(bytes, 6);
        document.dom.get_mut(node).unwrap().showing_poster = false;
        let frame = capture_video_resource(&document, node, &mut images, &mut bytes)
            .unwrap()
            .unwrap();
        assert_ne!(frame, poster);
        assert_eq!(images[frame].bytes, vec![1, 2, 3]);
        assert_eq!(images[frame].media_type, "image/png");
        assert_eq!(bytes, 9);
        assert!(!svg_inventory_matches(
            &[images[frame].clone()],
            &[],
            &[images[poster].clone()],
            &[]
        ));
        assert!(svg_inventory_matches(
            &[images[frame].clone()],
            &[],
            &[images[frame].clone()],
            &[]
        ));
        let mut full = MAX_BYTES - 2;
        let mut fresh = Vec::new();
        assert!(capture_video_resource(&document, node, &mut fresh, &mut full).is_err());
        assert!(fresh.is_empty());
        assert_eq!(full, MAX_BYTES - 2);
        document.dom.get_mut(node).unwrap().media_frame = None;
        assert_eq!(
            capture_video_resource(&document, node, &mut images, &mut bytes).unwrap(),
            None
        );
        assert!(!video_resource_matches(None, Some(frame), &images, &images));
        assert!(!video_resource_matches(Some(frame), None, &images, &images));
        assert!(video_resource_matches(None, None, &images, &images));
        assert!(video_resource_matches(
            Some(frame),
            Some(0),
            &images,
            &[images[frame].clone()]
        ));
        assert!(!video_resource_matches(
            Some(frame),
            Some(poster),
            &images,
            &images
        ));
        assert!(!video_resource_matches(
            Some(images.len()),
            Some(images.len()),
            &images,
            &images
        ));
        document.dom.get_mut(node).unwrap().showing_poster = true;
        assert_eq!(
            capture_video_resource(&document, node, &mut images, &mut bytes).unwrap(),
            Some(poster)
        );
    }

    #[test]
    fn final_drag_budget_counts_instance_metadata_and_inventory_additions() {
        let image = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: vec![SvgInstanceSnapshot {
                ancestry: vec![1],
                source: 2,
                presentation: "fill:red".into(),
                content: SvgInstanceContent::Reference("target".into()),
            }],
            content_type: None,
            url: "about:blank".into(),
            url_aliases: Vec::new(),
            media_type: "image/svg+xml".into(),
            bytes: b"<svg/>".to_vec(),
        };
        let expected = 6 + 8 + 8 + 8 + 6;
        assert_eq!(crate::svg_capture::resource_bytes(&image), Some(expected));
        let fonts = BTreeMap::from([(
            "face".into(),
            PaintFontResource {
                bytes: vec![0; 4],
                collection_index: 0,
            },
        )]);
        assert!(
            validate_capture_resources(std::slice::from_ref(&image), &fonts, expected, 4).is_ok()
        );
        assert!(
            validate_capture_resources(std::slice::from_ref(&image), &fonts, expected - 1, 4)
                .is_err()
        );
        assert!(
            validate_capture_resources(std::slice::from_ref(&image), &fonts, expected, 3).is_err()
        );
        // A fallback inventory adds a second individually acceptable resource.
        assert!(validate_capture_resources(&[image.clone(), image], &fonts, expected, 4).is_err());
    }

    #[test]
    fn svg_dependency_requires_fetched_identity_even_when_snapshot_url_matches() {
        let mut image = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: Vec::new(),
            content_type: None,
            url: "https://example.test/a.svg".into(),
            url_aliases: vec!["https://example.test/alias.svg".into()],
            media_type: "image/svg+xml".into(),
            bytes: b"<svg/>".to_vec(),
        };
        assert!(require_loaded_svg_resource(&image.url, &[image.clone()]).is_err());
        assert!(require_loaded_svg_resource(&image.url_aliases[0], &[image.clone()]).is_err());
        image.svg_document_snapshot = false;
        assert_eq!(
            require_loaded_svg_resource(&image.url_aliases[0], &[image.clone()]).unwrap(),
            Some(0)
        );
        assert_eq!(
            require_loaded_svg_resource("data:image/svg+xml,%3Csvg/%3E", &[]).unwrap(),
            None
        );
    }

    #[test]
    fn svg_inventory_reuse_checks_nested_content_and_remaps_resource_indices() {
        let image = PaintImageResource {
            svg_document_snapshot: true,
            svg_instances: Vec::new(),
            content_type: None,
            url: "https://example.test/page".into(),
            url_aliases: Vec::new(),
            media_type: "image/svg+xml".into(),
            bytes: b"<svg/>".to_vec(),
        };
        let object = SvgForeignObjectSnapshot {
            image_resource: 0,
            element_id: "native-1".into(),
            viewport: ViewportState {
                width: 10.,
                height: 10.,
                device_pixel_ratio: 1.,
            },
            nodes: Vec::new(),
        };
        let mut unrelated = image.clone();
        unrelated.svg_document_snapshot = false;
        let mut cached_object = object.clone();
        cached_object.image_resource = 1;
        assert!(svg_inventory_matches(
            std::slice::from_ref(&image),
            std::slice::from_ref(&object),
            &[unrelated.clone(), image.clone()],
            &[cached_object.clone()]
        ));
        cached_object.viewport.width = 20.;
        assert!(!svg_inventory_matches(
            std::slice::from_ref(&image),
            std::slice::from_ref(&object),
            &[unrelated, image.clone()],
            &[cached_object]
        ));
        let mut animated = image.clone();
        animated.bytes = b"<svg fill='red'/>".to_vec();
        assert!(!svg_inventory_matches(&[animated], &[], &[image], &[]));
    }

    #[test]
    fn drag_svg_capture_keeps_reference_styles_and_independent_html_jobs() {
        let document = oryn_dom::html::parse(
            r##"<svg id="s" width="20" height="10"><defs><foreignObject id="f" width="10" height="10"><div xmlns="http://www.w3.org/1999/xhtml" style="background:currentColor;width:10px;height:10px"></div></foreignObject></defs><use href="#f" style="color:red"/><use href="#f" x="10" style="color:blue"/></svg>"##,
        );
        let dom = document.clone().into_runtime().unwrap();
        let root = dom.find_by_id("s").unwrap();
        let input = crate::runtime::layout_input(
            &document,
            Default::default(),
            Default::default(),
            PreferredColorScheme::Light,
            ReducedMotion::NoPreference,
            &[],
        );
        let mut engine = oryn_layout::StyleEngine::default();
        let layout = engine
            .compute_at(&input, &oryn_layout::LayoutLimits::default(), 0.)
            .unwrap();
        let sample = engine.instance_style_sample().unwrap();
        let mut images = Vec::new();
        let mut objects = Vec::new();
        let mut fonts = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let mut bytes = 0;
        let (index, descendants) = capture_svg_subtree(DragSvgCaptureInput {
            document: &document,
            root,
            input: &input,
            layout: &layout,
            sample: &sample,
            base: "https://example.test/page",
            viewport: input.viewport,
            images: &mut images,
            objects: &mut objects,
            fonts: &mut fonts,
            diagnostics: &mut diagnostics,
            bytes: &mut bytes,
            depth: 0,
            inherited: None,
            decoded: &BTreeMap::new(),
            paint_keys: None,
        })
        .unwrap();
        assert_eq!(index, Some(0));
        assert!(!descendants.is_empty());
        assert!(images[0].svg_document_snapshot);
        assert!(!images[0].svg_instances.is_empty());
        let colors = objects
            .iter()
            .flat_map(|object| object.nodes.iter())
            .filter(|node| node.tag == "div")
            .filter_map(|node| node.style.get("color"))
            .cloned()
            .collect::<BTreeSet<_>>();
        assert!(colors.contains("rgb(255, 0, 0)"));
        assert!(colors.contains("rgb(0, 0, 255)"));
        let identities = objects
            .iter()
            .map(|object| &object.element_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(identities.len(), objects.len());
        assert!(objects.iter().all(|object| object.image_resource == 0));
        let first_use = input
            .nodes
            .iter()
            .find(|node| node.tag == "use")
            .unwrap()
            .id;
        let selected = BTreeSet::from([first_use]);
        let mut filtered_images = Vec::new();
        let mut filtered_objects = Vec::new();
        let mut filtered_fonts = BTreeMap::new();
        let mut filtered_diagnostics = Vec::new();
        let mut filtered_bytes = 0;
        capture_svg_subtree(DragSvgCaptureInput {
            document: &document,
            root,
            input: &input,
            layout: &layout,
            sample: &sample,
            base: "https://example.test/page",
            viewport: input.viewport,
            images: &mut filtered_images,
            objects: &mut filtered_objects,
            fonts: &mut filtered_fonts,
            diagnostics: &mut filtered_diagnostics,
            bytes: &mut filtered_bytes,
            depth: 0,
            inherited: None,
            decoded: &BTreeMap::new(),
            paint_keys: Some(&selected),
        })
        .unwrap();
        let painted_colors = filtered_objects
            .iter()
            .flat_map(|object| object.nodes.iter())
            .filter(|node| {
                node.tag == "div" && node.geometry.availability == GeometryAvailability::Available
            })
            .filter_map(|node| node.style.get("color"))
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            painted_colors,
            BTreeSet::from(["rgb(255, 0, 0)".to_owned()])
        );
        assert!(!svg_inventory_matches(
            &filtered_images,
            &filtered_objects,
            &images,
            &objects
        ));
    }

    #[test]
    fn transformed_drag_bounds_keep_rotation_and_translation() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 20.,
        };
        let matrix = [
            0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 30., 40., 0., 1.,
        ];
        let value = projected(rect, matrix).unwrap();
        assert_eq!(
            (value.x, value.y, value.width, value.height),
            (10., 40., 20., 10.)
        );
    }
    #[test]
    fn projection_rejects_eye_plane_crossing() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 20.,
        };
        let mut matrix = [0.; 16];
        matrix[0] = 1.;
        matrix[5] = 1.;
        matrix[15] = -1.;
        assert!(projected(rect, matrix).is_none());
    }
    #[test]
    fn shear_uses_original_source_coordinates_for_both_axes() {
        let rect = CssRect {
            x: 2.,
            y: 3.,
            width: 4.,
            height: 5.,
        };
        let matrix = [
            2., 3., 0., 0., 1., 4., 0., 0., 0., 0., 1., 0., 7., 11., 0., 1.,
        ];
        let value = projected(rect, matrix).unwrap();
        assert_eq!(
            (value.x, value.y, value.width, value.height),
            (14., 29., 13., 32.)
        );
    }
}
