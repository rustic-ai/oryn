//! Bounded raster decode for native HTML/SVG image resource completion.
//! Compressed response admission alone does not bound decompressed pixels.
use super::*;

pub(super) fn raster(bytes: &[u8]) -> Result<image::RgbaImage, V8BindingError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| V8BindingError::Message("EncodingError".into()))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder()
        .map_err(|_| V8BindingError::Message("EncodingError".into()))?;
    use image::ImageDecoder;
    let (width, height) = decoder.dimensions();
    // Validate RGBA allocation before decoding/conversion. Reuse the native
    // image backing bound; do not grant a new budget to resource images.
    image_data_size(width, height)?;
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| V8BindingError::Message("EncodingError".into()))?;
    Ok(image.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_raster_decoder_rejects_invalid_bytes_and_preserves_pixels() {
        assert!(raster(b"not an image").is_err());
        let pixels = image::RgbaImage::from_pixel(2, 3, image::Rgba([12, 34, 56, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels.clone())
            .write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        assert_eq!(raster(encoded.get_ref()).unwrap(), pixels);
    }

    #[test]
    fn native_raster_decoder_rejects_oversized_rgba_before_conversion() {
        // A narrow monochrome PNG can compress very small while its RGBA
        // representation exceeds the established 32MiB backing bound.
        let pixels = image::GrayImage::new(4096, 2049);
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(pixels)
            .write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        assert!(encoded.get_ref().len() < 16 * 1024 * 1024);
        assert!(raster(encoded.get_ref()).is_err());
    }
}
