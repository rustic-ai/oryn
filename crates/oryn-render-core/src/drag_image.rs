//! Rasterize an already-captured native subtree. No live page or resource fetch.
use super::*;
/// Mask polygons and crop coordinates are in the captured viewport coordinate
/// system; all existing paint transforms/clips remain in the display list.
pub fn render_drag_png(
    snapshot: &PaintSnapshot,
    bounds: CssRect,
    polygons: &[[f64; 8]],
    scale: f64,
    max_pixels: usize,
    max_bytes: usize,
) -> Result<RenderedArtifact, RenderError> {
    render_drag_png_with_coverage(snapshot, bounds, polygons, scale, max_pixels, max_bytes)
        .map(|(artifact, _)| artifact)
}

/// Return the encoded artifact and exact alpha coverage from its final masked
/// raster. Native visibility consumers need not decode the PNG they just made.
/// Encoding and byte limits still run before either result is returned.
pub fn render_drag_png_with_coverage(
    snapshot: &PaintSnapshot,
    bounds: CssRect,
    polygons: &[[f64; 8]],
    scale: f64,
    max_pixels: usize,
    max_bytes: usize,
) -> Result<(RenderedArtifact, bool), RenderError> {
    if ![bounds.x, bounds.y, bounds.width, bounds.height, scale]
        .iter()
        .all(|value| value.is_finite())
        || bounds.width <= 0.
        || bounds.height <= 0.
        || scale <= 0.
    {
        return Err(RenderError::Encode("invalid drag image bounds".into()));
    }
    let width = (bounds.width * scale).ceil();
    let height = (bounds.height * scale).ceil();
    if width > 4096. || height > 4096. {
        return Err(RenderError::PixelLimit {
            actual: (width * height).min(usize::MAX as f64) as usize,
            limit: max_pixels,
        });
    }
    let (width, height) = (width as u32, height as u32);
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(RenderError::PixelLimit {
            actual: usize::MAX,
            limit: max_pixels,
        })?;
    if pixels > max_pixels
        || pixels
            .checked_mul(4)
            .is_none_or(|bytes| bytes > 64 * 1024 * 1024)
    {
        return Err(RenderError::PixelLimit {
            actual: pixels,
            limit: max_pixels,
        });
    }
    if polygons.len() > 32768 || polygons.iter().flatten().any(|value| !value.is_finite()) {
        return Err(RenderError::Encode("invalid drag selection mask".into()));
    }
    // Drag bounds/selection polygons use layout CSS coordinates. A document's
    // visual camera must not move the crop; embedded documents retain theirs.
    let mut local = snapshot.clone();
    local.visual_viewport = None;
    local.viewport_scroll = None;
    let snapshot = &local;
    let options = RenderOptions {
        scale,
        max_pixels,
        background: [0; 4],
    };
    // Display-list construction owns no raster allocation. Its original viewport
    // dimensions are replaced by the bounded crop before rasterization.
    let mut scene = build_display_list(
        snapshot,
        RenderOptions {
            max_pixels: usize::MAX,
            ..options
        },
    )?;
    scene.width = width;
    scene.height = height;
    let mut matrix = [0.; 16];
    matrix[0] = 1.;
    matrix[5] = 1.;
    matrix[10] = 1.;
    matrix[15] = 1.;
    matrix[12] = -bounds.x;
    matrix[13] = -bounds.y;
    scene.commands = vec![PaintCommand::Projected {
        matrix,
        clips: Vec::new(),
        commands: scene.commands,
    }];
    let mut rgba = rasterize(snapshot, &scene, options)?;
    if !polygons.is_empty() {
        for y in 0..height {
            for x in 0..width {
                let point = (
                    bounds.x + (x as f64 + 0.5) / scale,
                    bounds.y + (y as f64 + 0.5) / scale,
                );
                if !polygons.iter().any(|polygon| inside(*polygon, point)) {
                    let offset = ((y as usize) * width as usize + x as usize) * 4;
                    rgba[offset..offset + 4].fill(0);
                }
            }
        }
    }
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    if bytes.len() > max_bytes {
        return Err(RenderError::Encode("encoded drag image limit".into()));
    }
    let has_ink = rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0);
    Ok((
        RenderedArtifact {
            bytes,
            width,
            height,
            diagnostics: scene.diagnostics,
        },
        has_ink,
    ))
}
fn inside(polygon: [f64; 8], point: (f64, f64)) -> bool {
    let mut positive = false;
    let mut negative = false;
    for edge in 0..4 {
        let next = (edge + 1) % 4;
        let cross = (polygon[next * 2] - polygon[edge * 2]) * (point.1 - polygon[edge * 2 + 1])
            - (polygon[next * 2 + 1] - polygon[edge * 2 + 1]) * (point.0 - polygon[edge * 2]);
        positive |= cross > 1e-8;
        negative |= cross < -1e-8;
    }
    !(positive && negative) && (positive || negative)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mask_rejects_rotated_bounding_box_corners() {
        let polygon = [5., 0., 10., 5., 5., 10., 0., 5.];
        assert!(inside(polygon, (5., 5.)));
        assert!(!inside(polygon, (0.5, 0.5)));
    }
    #[test]
    fn crop_limits_precede_raster_allocation() {
        let snapshot = crate::tests::empty_snapshot();
        assert!(
            render_drag_png(
                &snapshot,
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 5000.,
                    height: 5000.
                },
                &[],
                1.,
                16_000_000,
                8 * 1024 * 1024
            )
            .is_err()
        );
    }
}

/// Intrinsic image drag feedback is independent of CSS sizing, borders and
/// transforms. SVG still resolves solely against the immutable paint snapshot.
pub fn render_drag_resource(
    snapshot: &PaintSnapshot,
    index: usize,
    width: u32,
    height: u32,
    max_pixels: usize,
    max_bytes: usize,
) -> Result<RenderedArtifact, RenderError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(RenderError::PixelLimit {
            actual: usize::MAX,
            limit: max_pixels,
        })?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 || pixels > max_pixels {
        return Err(RenderError::PixelLimit {
            actual: pixels,
            limit: max_pixels,
        });
    }
    let resource = snapshot
        .images
        .get(index)
        .ok_or_else(|| RenderError::Encode("missing intrinsic drag resource".into()))?;
    let rgba = if resource.media_type == "image/svg+xml" {
        let tree = crate::svg::parse(resource, snapshot)?;
        crate::svg::raster(&tree, width, height)?
    } else {
        let decoded = image::load_from_memory(&resource.bytes)
            .map_err(|error| RenderError::ImageDecode {
                url: resource.url.clone(),
                detail: error.to_string(),
            })?
            .to_rgba8();
        if decoded.width() != width || decoded.height() != height {
            return Err(RenderError::Encode(
                "intrinsic drag dimensions changed".into(),
            ));
        }
        decoded
    };
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            rgba.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    if bytes.len() > max_bytes {
        return Err(RenderError::Encode(
            "intrinsic drag encoded byte limit".into(),
        ));
    }
    Ok(RenderedArtifact {
        bytes,
        width,
        height,
        diagnostics: Vec::new(),
    })
}

#[cfg(test)]
mod raster_regressions {
    use super::*;
    #[test]
    fn coverage_matches_masked_png_and_keeps_encoded_budget() {
        let mut snapshot = crate::tests::empty_snapshot();
        let bounds = CssRect {
            x: 0.,
            y: 0.,
            width: 4.,
            height: 4.,
        };
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                width: 2.,
                ..bounds
            },
        );
        node.style.insert("background-color".into(), "red".into());
        snapshot.nodes.push(node);
        for (polygon, expected) in [
            ([0., 0., 2., 0., 2., 4., 0., 4.], true),
            ([2., 0., 4., 0., 4., 4., 2., 4.], false),
        ] {
            let (artifact, has_ink) =
                render_drag_png_with_coverage(&snapshot, bounds, &[polygon], 1., 16, 4096).unwrap();
            assert_eq!(has_ink, expected);
            assert_eq!(
                has_ink,
                crate::captured_png_has_ink(&artifact.bytes, artifact.width, artifact.height)
                    .unwrap(),
            );
            assert!(
                render_drag_png_with_coverage(&snapshot, bounds, &[polygon], 1., 16, 0,).is_err()
            );
        }
        snapshot.nodes[0].style.insert("opacity".into(), "0".into());
        let (artifact, has_ink) =
            render_drag_png_with_coverage(&snapshot, bounds, &[], 1., 16, 4096).unwrap();
        assert!(!has_ink);
        assert!(
            !crate::captured_png_has_ink(&artifact.bytes, artifact.width, artifact.height,)
                .unwrap()
        );
    }
    #[test]
    fn intrinsic_svg_uses_captured_source_without_network() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://example.test/picture.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1" fill="red"/></svg>"#.to_vec()});
        let image = render_drag_resource(&snapshot, 0, 2, 1, 16, 4096).unwrap();
        let pixels = image::load_from_memory(&image.bytes).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!((image.width, image.height), (2, 1));
    }
    #[test]
    fn svg_processing_mode_distinguishes_disabled_requests_from_missing_native_admissions() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false, svg_instances: Vec::new(),content_type:None,url:"https://example.test/picture.svg".into(),url_aliases:Vec::new(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><image href="missing.png" width="2" height="2"/></svg>"#.to_vec()});
        let secure = render_drag_resource(&snapshot, 0, 2, 2, 16, 4096).unwrap();
        assert!(
            image::load_from_memory(&secure.bytes)
                .unwrap()
                .to_rgba8()
                .pixels()
                .all(|pixel| pixel[3] == 0)
        );
        snapshot.images[0].svg_document_snapshot = true;
        assert!(render_drag_resource(&snapshot, 0, 2, 2, 16, 4096).is_err());
    }
    #[test]
    fn layout_crop_does_not_apply_document_visual_camera_twice() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.visual_viewport = Some(oryn_common::visual_viewport::Geometry {
            scrollport: None,
            layout_width: 64.,
            layout_height: 32.,
            scale: 2.,
            offset_x: 10.,
            offset_y: 5.,
        });
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 20.,
                y: 10.,
                width: 4.,
                height: 4.,
            },
        );
        node.style.insert("background-color".into(), "red".into());
        snapshot.nodes.push(node);
        let image = render_drag_png(
            &snapshot,
            CssRect {
                x: 20.,
                y: 10.,
                width: 4.,
                height: 4.,
            },
            &[],
            1.,
            16,
            4096,
        )
        .unwrap();
        let pixels = image::load_from_memory(&image.bytes).unwrap().to_rgba8();
        assert!(pixels.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]));
    }
    #[test]
    fn offscreen_crop_is_transparent_outside_native_selection() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut node = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 100.,
                y: 100.,
                width: 4.,
                height: 4.,
            },
        );
        node.style
            .insert("background-color".into(), "rgb(255, 0, 0)".into());
        snapshot.nodes.push(node);
        let image = render_drag_png(
            &snapshot,
            CssRect {
                x: 100.,
                y: 100.,
                width: 4.,
                height: 4.,
            },
            &[[102., 100., 104., 102., 102., 104., 100., 102.]],
            1.,
            16,
            4096,
        )
        .unwrap();
        let pixels = image::load_from_memory(&image.bytes).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0).0[3], 0);
        assert_eq!(pixels.get_pixel(2, 2).0, [255, 0, 0, 255]);
    }
}
