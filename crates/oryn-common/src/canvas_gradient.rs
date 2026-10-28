//! Native canvas color-stop ramp. CSS parsing and opaque JS identity are owned
//! by the binding; raster consumers receive unpremultiplied sRGB samples.

#[derive(Debug, Clone, Default)]
pub struct GradientStops {
    stops: Vec<(f64, [u8; 4])>,
}

#[derive(Debug, Clone, Copy)]
pub enum GradientGeometry {
    Linear {
        start: [f64; 2],
        end: [f64; 2],
    },
    Radial {
        start: [f64; 2],
        start_radius: f64,
        end: [f64; 2],
        end_radius: f64,
    },
    Conic {
        center: [f64; 2],
        angle: f64,
    },
}

impl GradientGeometry {
    /// Gradient-space parameter, before endpoint color extension. No parameter
    /// means the point is outside a radial cone or the geometry is degenerate.
    pub fn parameter_at(self, point: [f64; 2]) -> Option<f64> {
        if !point.iter().all(|value| value.is_finite()) {
            return None;
        }
        match self {
            Self::Linear { start, end } => {
                if !start
                    .iter()
                    .chain(end.iter())
                    .all(|value| value.is_finite())
                    || start == end
                {
                    return None;
                }
                let scale = start
                    .iter()
                    .chain(end.iter())
                    .map(|value| value.abs())
                    .fold(0.0, f64::max);
                let scale = if scale == 0.0 { 1.0 } else { scale };
                let direction = [
                    end[0] / scale - start[0] / scale,
                    end[1] / scale - start[1] / scale,
                ];
                let length = direction[0].hypot(direction[1]);
                let relative = [
                    point[0] / scale - start[0] / scale,
                    point[1] / scale - start[1] / scale,
                ];
                let dot = |index: usize| {
                    if direction[index] == 0.0 {
                        0.0
                    } else {
                        relative[index] * (direction[index] / length)
                    }
                };
                let value = (dot(0) + dot(1)) / length;
                (!value.is_nan()).then_some(value)
            }
            Self::Conic { center, angle } => {
                if !center.iter().all(|value| value.is_finite()) || !angle.is_finite() {
                    return None;
                }
                let value = (point[1] - center[1]).atan2(point[0] - center[0]);
                Some(
                    (value - angle.rem_euclid(std::f64::consts::TAU))
                        .rem_euclid(std::f64::consts::TAU)
                        / std::f64::consts::TAU,
                )
            }
            Self::Radial {
                start,
                start_radius,
                end,
                end_radius,
            } => {
                if !start
                    .iter()
                    .chain(end.iter())
                    .all(|value| value.is_finite())
                    || !start_radius.is_finite()
                    || !end_radius.is_finite()
                    || start_radius < 0.0
                    || end_radius < 0.0
                {
                    return None;
                }
                if start == end {
                    if start_radius == end_radius {
                        return None;
                    }
                    return Some(
                        ((point[0] - start[0]).hypot(point[1] - start[1]) - start_radius)
                            / (end_radius - start_radius),
                    );
                }
                let scale = start
                    .iter()
                    .chain(end.iter())
                    .chain(point.iter())
                    .map(|value| value.abs())
                    .fold(start_radius.max(end_radius), f64::max);
                let q = [
                    point[0] / scale - start[0] / scale,
                    point[1] / scale - start[1] / scale,
                ];
                let d = [
                    end[0] / scale - start[0] / scale,
                    end[1] / scale - start[1] / scale,
                ];
                let r = start_radius / scale;
                let dr = end_radius / scale - r;
                let a = d[0] * d[0] + d[1] * d[1] - dr * dr;
                let b = -2.0 * (q[0] * d[0] + q[1] * d[1] + r * dr);
                let c = q[0] * q[0] + q[1] * q[1] - r * r;
                let roots = if a == 0.0 {
                    if b == 0.0 {
                        return None;
                    }
                    [-c / b, f64::NAN]
                } else {
                    let discriminant = b * b - 4.0 * a * c;
                    if discriminant < 0.0 {
                        return None;
                    }
                    // Stable quadratic roots avoid cancellation of the small root.
                    let numerator = -0.5 * (b + discriminant.sqrt().copysign(b));
                    if numerator == 0.0 {
                        [-b / (2.0 * a), f64::NAN]
                    } else {
                        [numerator / a, c / numerator]
                    }
                };
                roots
                    .into_iter()
                    .filter(|value| !value.is_nan() && r + value * dr >= 0.0)
                    .max_by(f64::total_cmp)
            }
        }
    }
    pub fn sample(self, stops: &GradientStops, point: [f64; 2]) -> [u8; 4] {
        self.parameter_at(point)
            .map_or([0; 4], |offset| stops.sample(offset))
    }
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
    fn linear_projection_is_global_and_coincident_endpoints_are_transparent() {
        let geometry = GradientGeometry::Linear {
            start: [10.0, 20.0],
            end: [30.0, 20.0],
        };
        assert!((geometry.parameter_at([20.0, 80.0]).unwrap() - 0.5).abs() <= 2.0 * f64::EPSILON);
        assert!((geometry.parameter_at([40.0, 20.0]).unwrap() - 1.5).abs() <= 2.0 * f64::EPSILON);
        let empty = GradientGeometry::Linear {
            start: [1.0, 1.0],
            end: [1.0, 1.0],
        };
        assert_eq!(empty.parameter_at([2.0, 2.0]), None);
        let tiny = GradientGeometry::Linear {
            start: [0.0, 0.0],
            end: [f64::MIN_POSITIVE / 1024.0, 0.0],
        };
        assert_eq!(tiny.parameter_at([100.0, 100.0]), Some(f64::INFINITY));
    }
    #[test]
    fn radial_cones_select_last_nonnegative_radius_intersection() {
        let concentric = GradientGeometry::Radial {
            start: [0.0, 0.0],
            start_radius: 0.0,
            end: [0.0, 0.0],
            end_radius: 10.0,
        };
        assert_eq!(concentric.parameter_at([3.0, 4.0]), Some(0.5));
        let cylinder = GradientGeometry::Radial {
            start: [0.0, 0.0],
            start_radius: 1.0,
            end: [2.0, 0.0],
            end_radius: 1.0,
        };
        assert_eq!(cylinder.parameter_at([1.0, 0.0]), Some(1.0));
        assert_eq!(cylinder.parameter_at([1.0, 2.0]), None);
        let identical = GradientGeometry::Radial {
            start: [0.0, 0.0],
            start_radius: 1.0,
            end: [0.0, 0.0],
            end_radius: 1.0,
        };
        assert_eq!(identical.parameter_at([1.0, 0.0]), None);
    }
    #[test]
    fn conic_angles_turn_clockwise_from_positive_x_and_wrap() {
        let conic = GradientGeometry::Conic {
            center: [0.0, 0.0],
            angle: 0.0,
        };
        assert_eq!(conic.parameter_at([1.0, 0.0]), Some(0.0));
        assert_eq!(conic.parameter_at([0.0, 1.0]), Some(0.25));
        assert_eq!(conic.parameter_at([0.0, -1.0]), Some(0.75));
    }
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
