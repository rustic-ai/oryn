//! Synchronous native drag subtree paint. Called on the owning V8 thread;
//! resources are decoded snapshots supplied by that realm, never fetched here.
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
}
pub(super) fn capture_filtered(
    provider: &LiveLayoutProvider,
    request: &Capture,
    custom_states: &BTreeMap<NodeId, BTreeSet<String>>,
    scroll: (f64, f64),
    element_scrolls: &BTreeMap<NodeId, (f64, f64)>,
    paint_keys: Option<&BTreeSet<u64>>,
) -> Result<Snapshot, String> {
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
        return Ok(snapshot);
    }
    let layout = provider.native_layout(
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
        bytes = bytes.saturating_add(image.bytes.len());
        if bytes > MAX_BYTES {
            return Err("drag CSS resource budget".into());
        }
        images.push(image.clone());
    }
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
        let mut projection = layout.paint_projections.get(&key).cloned();
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
        if tag == "img" && image.is_none() && paint_keys.is_none_or(|keys| keys.contains(&key)) {
            return Err("drag subtree contains an undecoded image".into());
        }
        if svg {
            // Preserve actual native SVG content and computed styles. The core
            // SVG resolver only sees the supplied already-loaded resource set.
            let (arena, document) = provider.dom.borrow().snapshot();
            let mut copied = ParsedDocument {
                dom: arena,
                document,
                errors: Vec::new(),
                quirks_mode: html5ever::interface::QuirksMode::NoQuirks,
                events: Vec::new(),
            }
            .into_runtime()
            .map_err(|e| e.to_string())?;
            let descendants = copied
                .arena()
                .descendants_inclusive(node)
                .map_err(|e| e.to_string())?;
            for child in &descendants {
                if let Some(style) = layout.computed_styles.get(&pack(*child)) {
                    if matches!(
                        copied.arena().get(*child).map_err(|e| e.to_string())?.kind,
                        NodeKind::Element { .. }
                    ) {
                        let css = style
                            .iter()
                            .map(|(name, value)| format!("{name}:{value}"))
                            .collect::<Vec<_>>()
                            .join(";");
                        copied
                            .set_attribute(*child, "style", css)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            let markup = copied.serialize(node).map_err(|e| e.to_string())?;
            bytes = bytes.saturating_add(markup.len());
            if bytes > MAX_BYTES {
                return Err("drag SVG budget".into());
            }
            image = Some(images.len());
            images.push(PaintImageResource {
                svg_document_snapshot: true,
                content_type: None,
                url: format!("oryn:drag-svg/{key}"),
                url_aliases: Vec::new(),
                media_type: "image/svg+xml".into(),
                bytes: markup.into_bytes(),
            });
            svg_descendants.extend(descendants);
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
    if layout
        .paint_fonts
        .values()
        .fold(0usize, |bytes, font| bytes.saturating_add(font.bytes.len()))
        > 32 * 1024 * 1024
    {
        return Err("drag font snapshot budget".into());
    }
    let viewport = provider.viewport.get();
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
    let (local_effects, effect_resources) = crate::svg_capture::local_effects(
        &document,
        &layout.computed_styles,
        &base,
        &mut images,
        MAX_BYTES,
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
        fonts: layout.paint_fonts.clone(),
        svg_foreign_objects: Vec::new(),
        diagnostics: layout.diagnostics.clone(),
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
            cached
                .svg_foreign_objects
                .retain(|object| images.contains(&object.image_resource));
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
                if let Some(index) = node.image_resource {
                    if let Some(resource) = request
                        .resources
                        .iter()
                        .find(|resource| pack(resource.node) == node.node_id)
                    {
                        if let Some(image) = cached.images.get_mut(index) {
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
                // Keep native context/opacity/filter ancestry but remove this node's
                // own paint, including promoted root canvas backgrounds.
                node.geometry.availability = GeometryAvailability::Hidden;
                node.style
                    .insert("background-color".into(), "transparent".into());
                node.style.insert("background-image".into(), "none".into());
                node.backdrop = None;
                node.backdrop_style = None;
                node.text.clear();
                node.image_resource = None;
                node.builtin_image = None;
                node.control = None;
            }
        }
        snapshot
            .frames
            .retain(|frame| paint_keys.contains(&frame.owner_node));
    }
    let intrinsic = if tag == "img" && request.polygons.is_empty() {
        request
            .resources
            .iter()
            .find(|resource| resource.node == request.source)
    } else {
        None
    };
    let artifact = if let Some(resource) = intrinsic {
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
    } else {
        oryn_render_core::render_drag_png(
            &snapshot,
            bounds,
            &request.polygons,
            viewport.device_pixel_ratio,
            MAX_PIXELS,
            MAX_BYTES,
        )
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
    Ok(result)
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

#[cfg(test)]
mod tests {
    use super::*;
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
