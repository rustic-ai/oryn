//! Secure image-mode SVG decoding for native bitmap consumers. No embedding
//! document resources, styles, scripts or filesystem access are available.
use super::*;
use oryn_common::v4::{LayoutRevision, PageId, PageInfo, Revision, ScrollState, StyleRevision};

pub fn decode_svg_bitmap(
    source: &oryn_common::v4::PaintImageResource,
    max_pixels: usize,
) -> Result<image::RgbaImage, RenderError> {
    if source.svg_document_snapshot || !source.svg_instances.is_empty() {
        return Err(RenderError::ImageDecode {
            url: source.url.clone(),
            detail: "bitmap source must be a secure SVG image, not a document snapshot".into(),
        });
    }
    // Empty resource context is intentional: an SVG image cannot resolve URLs
    // against an embedding document's captured resource inventory.
    let environment = SvgEnvironment::default();
    let snapshot = PaintSnapshot {
        local_effects: Default::default(),
        visual_viewport: None,
        frames: Vec::new(),
        viewport_scroll: None,
        contract_version: oryn_common::v4::CONTRACT_VERSION,
        page: PageInfo {
            page_id: PageId(0),
            opener_page_id: None,
            url: source.url.clone(),
            title: String::new(),
            document_generation: 0,
        },
        revision: Revision(0),
        style_revision: StyleRevision(0),
        layout_revision: LayoutRevision(0),
        viewport: environment.viewport(),
        scroll: ScrollState::default(),
        nodes: Vec::new(),
        svg_foreign_objects: Vec::new(),
        images: Vec::new(),
        fonts: Default::default(),
        diagnostics: Vec::new(),
    };
    let tree = svg::parse_with_environment(source, &snapshot, environment)?;
    let size = tree.size().to_int_size();
    let pixels = (size.width() as usize)
        .checked_mul(size.height() as usize)
        .unwrap_or(usize::MAX);
    if pixels > max_pixels {
        return Err(RenderError::PixelLimit {
            actual: pixels,
            limit: max_pixels,
        });
    }
    svg::raster(&tree, size.width(), size.height())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(xml: &str) -> oryn_common::v4::PaintImageResource {
        oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            svg_instances: Vec::new(),
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://example.test/image.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: xml.as_bytes().to_vec(),
        }
    }
    #[test]
    fn secure_svg_bitmap_has_intrinsic_pixels_and_straight_alpha() {
        let image = decode_svg_bitmap(&source("<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'><rect width='2' height='1' fill='red' opacity='.5'/></svg>"), 2).unwrap();
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 128]);
    }
    #[test]
    fn secure_svg_bitmap_rejects_document_inputs_and_pixel_overflow() {
        let mut source = source("<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>");
        assert!(matches!(
            decode_svg_bitmap(&source, 3),
            Err(RenderError::PixelLimit { .. })
        ));
        source.svg_document_snapshot = true;
        assert!(decode_svg_bitmap(&source, 4).is_err());
        assert!(decode_svg_bitmap(&self::source("not SVG"), 4).is_err());
    }
}
