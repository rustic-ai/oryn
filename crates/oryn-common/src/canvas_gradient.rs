//! Native canvas color-stop ramp. CSS parsing and opaque JS identity are owned
//! by the binding; raster consumers receive unpremultiplied sRGB samples.

#[derive(Debug, Clone, Default)]
pub struct GradientStops {
    stops: Vec<(f64, [u8; 4])>,
}

impl GradientStops {
    /// Equal offsets retain insertion order, creating an abrupt color boundary.
    /// Invalid offsets leave the ramp unchanged; the binding chooses the WebIDL
    /// conversion/IndexSizeError precedence before invoking this operation.
    pub fn insert(&mut self, offset: f64, color: [u8; 4]) -> bool {
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
            return false;
        }
        let index = self.stops.partition_point(|stop| stop.0 <= offset);
        self.stops.insert(index, (offset, color));
        true
    }

    /// Outside the first/last stop, extend its color. No stops is transparent.
    /// Interpolate color and alpha separately, not premultiplied color: a fully
    /// transparent colored stop must still affect intermediate chromaticity.
    pub fn sample(&self, offset: f64) -> [u8; 4] {
        if offset.is_nan() || self.stops.is_empty() {
            return [0; 4];
        }
        let upper = self.stops.partition_point(|stop| stop.0 <= offset);
        if upper == 0 {
            return self.stops[0].1;
        }
        let (left_offset, left) = self.stops[upper - 1];
        let Some(&(right_offset, right)) = self.stops.get(upper) else {
            return left;
        };
        let amount = (offset - left_offset) / (right_offset - left_offset);
        std::array::from_fn(|index| {
            (f64::from(left[index]) + (f64::from(right[index]) - f64::from(left[index])) * amount)
                .round()
                .clamp(0.0, 255.0) as u8
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_and_alpha_interpolate_without_premultiplication() {
        let mut ramp = GradientStops::default();
        assert!(ramp.insert(1.0, [0, 0, 255, 255]));
        assert!(ramp.insert(0.0, [255, 255, 0, 0]));
        assert_eq!(ramp.sample(0.5), [128, 128, 128, 128]);
        assert_eq!(ramp.sample(0.25), [191, 191, 64, 64]);
    }
    #[test]
    fn duplicate_offsets_keep_order_and_extend_endpoint_colors() {
        let mut ramp = GradientStops::default();
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        ramp.insert(0.5, red);
        ramp.insert(0.5, blue);
        assert_eq!(ramp.sample(0.49), red);
        assert_eq!(ramp.sample(0.5), blue);
        assert_eq!(ramp.sample(0.51), blue);
        assert_eq!(ramp.sample(f64::NEG_INFINITY), red);
        assert_eq!(ramp.sample(f64::INFINITY), blue);
    }
    #[test]
    fn invalid_stop_offsets_do_not_mutate_the_ramp() {
        let mut ramp = GradientStops::default();
        assert_eq!(ramp.sample(0.5), [0; 4]);
        ramp.insert(0.5, [1, 2, 3, 4]);
        for offset in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(!ramp.insert(offset, [255; 4]));
        }
        assert_eq!(ramp.sample(0.5), [1, 2, 3, 4]);
        assert_eq!(ramp.stops.len(), 1);
    }
}
