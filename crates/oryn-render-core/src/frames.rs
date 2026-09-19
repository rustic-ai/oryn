//! Embedded documents keep resources and scrollbar coordinates in their own
//! snapshot. The enclosing iframe contributes only placement and clipping.
use super::*;
pub(super) fn validate(snapshot: &PaintSnapshot) -> Result<(), RenderError> {
    let mut pending = vec![(snapshot, 0usize)];
    let mut count = 0usize;
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((snapshot, depth)) = pending.pop() {
        if snapshot.local_effects.len() > 256
            || snapshot.local_effects.iter().any(|(id, index)| {
                id.len() > 64 * 1024
                    || snapshot
                        .images
                        .get(*index)
                        .is_none_or(|image| image.media_type != "image/svg+xml")
            })
        {
            return Err(RenderError::Encode(
                "invalid native local effect bindings".into(),
            ));
        }
        if depth > 32 {
            return Err(RenderError::Encode("frame paint depth exceeds 32".into()));
        }
        nodes = nodes.saturating_add(snapshot.nodes.len()).saturating_add(
            snapshot
                .svg_foreign_objects
                .iter()
                .map(|object| object.nodes.len())
                .sum::<usize>(),
        );
        bytes = bytes
            .saturating_add(
                snapshot
                    .images
                    .iter()
                    .map(|image| image.bytes.len())
                    .sum::<usize>(),
            )
            .saturating_add(
                snapshot
                    .fonts
                    .values()
                    .map(|font| font.bytes.len())
                    .sum::<usize>(),
            );
        if nodes > 100_000 || bytes > 128 * 1024 * 1024 {
            return Err(RenderError::Encode(
                "nested frame paint resource limit exceeded".into(),
            ));
        }
        let mut owners = std::collections::BTreeSet::new();
        for frame in &snapshot.frames {
            count += 1;
            if count > 256
                || !owners.insert(frame.owner_node)
                || !snapshot
                    .nodes
                    .iter()
                    .chain(
                        snapshot
                            .svg_foreign_objects
                            .iter()
                            .flat_map(|object| object.nodes.iter()),
                    )
                    .any(|node| {
                        node.node_id == frame.owner_node
                            && matches!(node.tag.as_str(), "iframe" | "frame")
                    })
            {
                return Err(RenderError::Encode(
                    "invalid or duplicate frame capture owner".into(),
                ));
            }
            let viewport = frame.snapshot.viewport;
            if !viewport.width.is_finite()
                || !viewport.height.is_finite()
                || viewport.width <= 0.
                || viewport.height <= 0.
                || viewport.width > 16384.
                || viewport.height > 16384.
            {
                return Err(RenderError::Encode("invalid child frame viewport".into()));
            }
            pending.push((&frame.snapshot, depth + 1));
        }
    }
    Ok(())
}
pub(super) fn snapshot(
    parent: &PaintSnapshot,
    index: usize,
) -> Result<&PaintSnapshot, RenderError> {
    parent
        .frames
        .get(index)
        .map(|frame| frame.snapshot.as_ref())
        .ok_or_else(|| RenderError::Encode("missing embedded frame capture".into()))
}
pub(super) fn scene(
    snapshot: &PaintSnapshot,
    scale: f64,
) -> Result<(DisplayList, RenderOptions), RenderError> {
    let semantic = crate::svg_coverage::text_capture_active();
    let options = RenderOptions {
        scale,
        max_pixels: 16 * 1024 * 1024,
        background: if semantic { [0; 4] } else { [255; 4] },
    };
    let mut scene = build_display_list(snapshot, options)?;
    if semantic {
        scene.commands = crate::svg_coverage::text_only(&scene.commands);
    }
    if !scene.diagnostics.is_empty() {
        return Err(RenderError::Encode(format!(
            "incomplete child frame paint: {}",
            scene.diagnostics.join("; ")
        )));
    }
    Ok((scene, options))
}
pub(super) fn raster(
    snapshot: &PaintSnapshot,
    scale: f64,
) -> Result<image::RgbaImage, RenderError> {
    let (scene, options) = scene(snapshot, scale)?;
    let pixels = rasterize(snapshot, &scene, options)?;
    image::RgbaImage::from_raw(scene.width, scene.height, pixels)
        .ok_or_else(|| RenderError::Encode("invalid embedded frame pixels".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> PaintSnapshot {
        let mut parent = crate::tests::empty_snapshot();
        let mut root = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 64.,
                height: 32.,
            },
        );
        root.style.insert("background-color".into(), "red".into());
        let mut owner = crate::tests::paint_node(
            2,
            Some(1),
            "iframe",
            CssRect {
                x: 10.,
                y: 2.,
                width: 20.,
                height: 12.,
            },
        );
        owner.projection.as_mut().unwrap().matrix[0] = 2.;
        owner.projection.as_mut().unwrap().matrix[5] = 2.;
        parent.nodes = vec![root, owner];
        let mut child = crate::tests::empty_snapshot();
        child.viewport.width = 20.;
        child.viewport.height = 12.;
        let mut root = crate::tests::paint_node(
            1,
            None,
            "html",
            CssRect {
                x: 0.,
                y: 0.,
                width: 16.,
                height: 24.,
            },
        );
        root.style.insert("background-color".into(), "blue".into());
        child.nodes.push(root);
        child.viewport_scroll = Some(oryn_common::v4::PaintViewportScroll {
            scroll: oryn_common::v4::PaintScrollGeometry {
                offset_x: 0.,
                offset_y: 0.,
                scrollport: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 16.,
                    height: 12.,
                },
                scrollable: CssRect {
                    x: 0.,
                    y: 0.,
                    width: 16.,
                    height: 24.,
                },
            },
            style: std::collections::BTreeMap::from([("overflow-y".into(), "auto".into())]),
        });
        parent.frames.push(oryn_common::v4::PaintFrameSnapshot {
            owner_node: 2,
            snapshot: Box::new(child),
        });
        parent
    }
    #[test]
    fn child_viewport_chrome_composes_inside_transformed_iframe() {
        let snapshot = snapshot();
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let at = |x: usize, y: usize| {
            &pixels[(y * scene.width as usize + x) * 4..(y * scene.width as usize + x) * 4 + 4]
        };
        assert_eq!(at(5, 8), [255, 0, 0, 255]);
        assert_eq!(at(24, 8), [0, 0, 255, 255]);
        assert_ne!(at(55, 8), [0, 0, 255, 255]);
        assert_eq!(at(62, 8), [255, 0, 0, 255]);
    }
    #[test]
    fn nested_visual_cameras_compose_only_in_their_own_document() {
        let mut snapshot = snapshot();
        snapshot.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 64.,
            layout_height: 32.,
            scale: 2.,
            offset_x: 10.,
            offset_y: 0.,
        });
        let child = &mut snapshot.frames[0].snapshot;
        child.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 20.,
            layout_height: 12.,
            scale: 2.,
            offset_x: 5.,
            offset_y: 0.,
        });
        let mut marker = crate::tests::paint_node(
            3,
            Some(1),
            "div",
            CssRect {
                x: 6.,
                y: 1.,
                width: 2.,
                height: 2.,
            },
        );
        marker
            .style
            .insert("background-color".into(), "lime".into());
        child.nodes.push(marker);
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let at = |x: usize, y: usize| {
            &pixels[(y * scene.width as usize + x) * 4..(y * scene.width as usize + x) * 4 + 4]
        };
        assert_eq!(at(29, 17), [0, 255, 0, 255]);
        assert_ne!(at(21, 13), [0, 255, 0, 255]);
    }
    #[test]
    fn duplicate_owner_and_unbounded_frame_tree_are_rejected() {
        let mut duplicate = snapshot();
        duplicate.frames.push(duplicate.frames[0].clone());
        assert!(validate(&duplicate).is_err());
        let mut invalid = snapshot();
        invalid.frames[0].snapshot.viewport.width = f64::INFINITY;
        assert!(validate(&invalid).is_err());
    }
    #[test]
    fn frame_svg_export_remains_vector_and_resource_scoped() {
        let snapshot = snapshot();
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let tree = crate::print::vector::tree(&snapshot, &scene, options).unwrap();
        let xml = tree.to_string(&resvg::usvg::WriteOptions::default());
        assert!(!xml.contains("data:image/png"));
        assert!(xml.contains("#0000ff") || xml.contains("blue"));
    }
    #[test]
    fn equal_urls_in_parent_and_child_keep_distinct_native_bytes() {
        fn resource(color: [u8; 4]) -> oryn_common::v4::PaintImageResource {
            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba(color));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false,
                content_type: None,
                url: "https://same.test/image".into(),
                url_aliases: Vec::new(),
                media_type: "image/png".into(),
                bytes: bytes.into_inner(),
            }
        }
        let mut snapshot = snapshot();
        snapshot.images.push(resource([255, 0, 0, 255]));
        let child = &mut snapshot.frames[0].snapshot;
        child.images.push(resource([0, 255, 0, 255]));
        let mut image = crate::tests::paint_node(
            2,
            Some(1),
            "img",
            CssRect {
                x: 0.,
                y: 0.,
                width: 16.,
                height: 12.,
            },
        );
        image.image_resource = Some(0);
        child.nodes.push(image);
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let offset = (8 * scene.width as usize + 24) * 4;
        assert_eq!(&pixels[offset..offset + 4], &[0, 255, 0, 255]);
    }
}
