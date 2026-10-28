//! Native canvas pattern tiling and premultiplied image filtering.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Both,
    X,
    Y,
    Neither,
}
impl Repeat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "" | "repeat" => Some(Self::Both),
            "repeat-x" => Some(Self::X),
            "repeat-y" => Some(Self::Y),
            "no-repeat" => Some(Self::Neither),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Bitmap<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}
impl<'a> Bitmap<'a> {
    pub fn new(width: u32, height: u32, pixels: &'a [u8]) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (width != 0 && height != 0 && pixels.len() == expected).then_some(Self {
            width,
            height,
            pixels,
        })
    }
    /// Return premultiplied sRGB channels in 0..255 and alpha in 0..1, matching
    /// native canvas compositing. Filtering must not blend hidden RGB from a
    /// transparent pixel into the visible color at a tile edge.
    pub fn sample(self, repeat: Repeat, x: f64, y: f64, smooth: bool) -> [f64; 4] {
        if !x.is_finite() || !y.is_finite() {
            return [0.0; 4];
        }
        let coordinate = |value: f64, length: u32, repeating: bool| -> Option<usize> {
            if repeating {
                Some(value.rem_euclid(f64::from(length)) as usize)
            } else if value < 0.0 || value >= f64::from(length) {
                None
            } else {
                Some(value as usize)
            }
        };
        let pixel = |x: f64, y: f64| {
            let Some(x) = coordinate(x, self.width, matches!(repeat, Repeat::Both | Repeat::X))
            else {
                return [0.0; 4];
            };
            let Some(y) = coordinate(y, self.height, matches!(repeat, Repeat::Both | Repeat::Y))
            else {
                return [0.0; 4];
            };
            let offset = (y * self.width as usize + x) * 4;
            let alpha = f64::from(self.pixels[offset + 3]) / 255.0;
            [
                f64::from(self.pixels[offset]) * alpha,
                f64::from(self.pixels[offset + 1]) * alpha,
                f64::from(self.pixels[offset + 2]) * alpha,
                alpha,
            ]
        };
        if !smooth {
            return pixel(x.floor(), y.floor());
        }
        let (x, y) = (x - 0.5, y - 0.5);
        let (left, top) = (x.floor(), y.floor());
        let (fx, fy) = (x - left, y - top);
        let mut result = [0.0; 4];
        for (dx, dy, weight) in [
            (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
            (1.0, 0.0, fx * (1.0 - fy)),
            (0.0, 1.0, (1.0 - fx) * fy),
            (1.0, 1.0, fx * fy),
        ] {
            let sample = pixel(left + dx, top + dy);
            for channel in 0..4 {
                result[channel] += sample[channel] * weight;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PIXELS: [u8; 8] = [255, 0, 0, 255, 0, 0, 255, 0];
    #[test]
    fn modes_are_case_sensitive_and_empty_repeats_both_axes() {
        assert_eq!(Repeat::parse(""), Some(Repeat::Both));
        for (mode, expected) in [
            ("repeat", Repeat::Both),
            ("repeat-x", Repeat::X),
            ("repeat-y", Repeat::Y),
            ("no-repeat", Repeat::Neither),
        ] {
            assert_eq!(Repeat::parse(mode), Some(expected));
        }
        for mode in ["REPEAT", "Repeat-X", "repeat ", "null"] {
            assert_eq!(Repeat::parse(mode), None);
        }
    }
    #[test]
    fn repeat_axes_wrap_negative_coordinates_independently() {
        let bitmap = Bitmap::new(2, 1, &PIXELS).unwrap();
        assert_eq!(
            bitmap.sample(Repeat::Both, -1.5, -0.5, false),
            [255.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(bitmap.sample(Repeat::X, -1.5, -0.5, false), [0.0; 4]);
        assert_eq!(
            bitmap.sample(Repeat::Y, 0.5, -0.5, false),
            [255.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(bitmap.sample(Repeat::Neither, 2.5, 0.5, false), [0.0; 4]);
    }
    #[test]
    fn filtering_wraps_tile_edges_and_uses_premultiplied_colors() {
        let bitmap = Bitmap::new(2, 1, &PIXELS).unwrap();
        assert_eq!(
            bitmap.sample(Repeat::Both, 1.0, 0.5, true),
            [127.5, 0.0, 0.0, 0.5]
        );
        assert_eq!(
            bitmap.sample(Repeat::Both, 2.0, 0.5, true),
            [127.5, 0.0, 0.0, 0.5]
        );
        assert_eq!(
            bitmap.sample(Repeat::Neither, 0.0, 0.5, true),
            [127.5, 0.0, 0.0, 0.5]
        );
    }
    #[test]
    fn malformed_bitmaps_and_nonfinite_points_fail_closed() {
        assert!(Bitmap::new(0, 1, &[]).is_none());
        assert!(Bitmap::new(2, 1, &PIXELS[..4]).is_none());
        let bitmap = Bitmap::new(2, 1, &PIXELS).unwrap();
        assert_eq!(bitmap.sample(Repeat::Both, f64::NAN, 0.5, true), [0.0; 4]);
    }
}
