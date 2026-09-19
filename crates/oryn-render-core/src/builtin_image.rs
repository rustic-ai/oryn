//! Native control images never enter the author URL resource namespace.
use crate::RenderError;
use oryn_common::builtin_image::BuiltinImage;
const MAX_PIXELS: usize = 1024 * 1024;
pub(super) fn raster(
    image: &BuiltinImage,
    width: f64,
    height: f64,
) -> Result<image::RgbaImage, RenderError> {
    if !width.is_finite() || !height.is_finite() {
        return Err(RenderError::PixelLimit {
            actual: usize::MAX,
            limit: MAX_PIXELS,
        });
    }
    let width = width.ceil().max(1.);
    let height = height.ceil().max(1.);
    if !width.is_finite()
        || !height.is_finite()
        || width > MAX_PIXELS as f64
        || height > MAX_PIXELS as f64
        || width * height > MAX_PIXELS as f64
    {
        return Err(RenderError::PixelLimit {
            actual: usize::MAX,
            limit: MAX_PIXELS,
        });
    }
    let bytes = image.svg().map_err(|detail| RenderError::ImageDecode {
        url: "native control image".into(),
        detail,
    })?;
    let mut options = resvg::usvg::Options::default();
    // No path, network, data-image, captured-resource, or font resolver can be
    // selected by an author URL. The source is constructed from a typed enum.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
    let tree = resvg::usvg::Tree::from_data(&bytes, &options).map_err(|error| {
        RenderError::ImageDecode {
            url: "native control image".into(),
            detail: error.to_string(),
        }
    })?;
    crate::svg::raster(&tree, width.ceil() as u32, height.ceil() as u32)
}
#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::builtin_image::PermissionIconKind;
    fn icon(fill: [u8; 4], stroke: [u8; 4]) -> BuiltinImage {
        BuiltinImage::PermissionIcon {
            kind: PermissionIconKind::Location,
            fill,
            stroke,
            stroke_width: 2.,
        }
    }
    #[test]
    fn typed_native_paint_honors_colors_and_bounds_without_resources() {
        let red = raster(&icon([255, 0, 0, 255], [0; 4]), 24., 24.).unwrap();
        let blue = raster(&icon([0, 0, 255, 255], [0; 4]), 24., 24.).unwrap();
        let stroked = raster(&icon([255, 0, 0, 255], [0, 255, 0, 255]), 24., 24.).unwrap();
        assert!(red.pixels().any(|pixel| pixel[0] > 0 && pixel[3] > 0));
        assert_ne!(red, blue);
        assert_ne!(red, stroked);
        assert!(raster(&icon([0; 4], [0; 4]), 2048., 2048.).is_err());
        assert!(raster(&icon([0; 4], [0; 4]), f64::INFINITY, 24.).is_err());
    }
}
