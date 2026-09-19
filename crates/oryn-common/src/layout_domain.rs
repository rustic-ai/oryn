//! Finite native scroll-overflow domain shared by geometry and consumers.
//!
//! Taffy 0.14 and Parley 0.11 store coordinates in IEEE binary32. Every integer
//! CSS pixel remains representable over a span of 2^24 pixels. A symmetric
//! domain of radius 2^23 keeps both endpoints and their entire signed distance
//! within that span. This is also inside Stylo's app_units 0.7.8 coordinate
//! envelope: ((2^30)-1)/60 CSS px. This conservative native policy is a resource
//! and representation limit, not a claim that projective infinity is finite.
use crate::v4::CssRect;
pub const MAX_SCROLL_COORDINATE: f64 = 8_388_608.;
pub const SCROLL_DOMAIN: CssRect = CssRect {
    x: -MAX_SCROLL_COORDINATE,
    y: -MAX_SCROLL_COORDINATE,
    width: 2. * MAX_SCROLL_COORDINATE,
    height: 2. * MAX_SCROLL_COORDINATE,
};
/// Clamp an axis-aligned overflow rectangle to the native domain. Empty regions
/// outside the domain collapse at its nearest edge; NaN is never admitted.
pub fn clip_scroll_rect(rect: CssRect) -> Option<CssRect> {
    if [
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        rect.right(),
        rect.bottom(),
    ]
    .iter()
    .any(|v| !v.is_finite())
        || rect.width < 0.
        || rect.height < 0.
    {
        return None;
    }
    let x = rect.x.clamp(-MAX_SCROLL_COORDINATE, MAX_SCROLL_COORDINATE);
    let y = rect.y.clamp(-MAX_SCROLL_COORDINATE, MAX_SCROLL_COORDINATE);
    let right = rect
        .right()
        .clamp(-MAX_SCROLL_COORDINATE, MAX_SCROLL_COORDINATE);
    let bottom = rect
        .bottom()
        .clamp(-MAX_SCROLL_COORDINATE, MAX_SCROLL_COORDINATE);
    Some(CssRect {
        x,
        y,
        width: (right - x).max(0.),
        height: (bottom - y).max(0.),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_span_retains_backend_integer_pixel_resolution() {
        let radius = MAX_SCROLL_COORDINATE as f32;
        assert_eq!((radius + radius - 1.) as f64, SCROLL_DOMAIN.width - 1.);
        assert!(MAX_SCROLL_COORDINATE < (((1u64 << 30) - 1) as f64 / 60.));
    }
    #[test]
    fn overflow_clamps_both_signed_edges_without_nonfinite_values() {
        assert_eq!(
            clip_scroll_rect(CssRect {
                x: -1e20,
                y: -1e20,
                width: 2e20,
                height: 2e20
            }),
            Some(SCROLL_DOMAIN)
        );
        assert!(
            clip_scroll_rect(CssRect {
                x: f64::NAN,
                ..SCROLL_DOMAIN
            })
            .is_none()
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectedBounds {
    pub rect: CssRect,
    pub unbounded: bool,
    pub saturated: bool,
}
/// Project a native z=0 rectangle into the documented finite coordinate domain.
/// Retain the camera-crossing signal separately: callers must not turn clipped
/// representable geometry into an assertion of mathematical boundedness.
pub fn projected_bounds(matrix: [f64; 16], rect: CssRect) -> Option<ProjectedBounds> {
    if matrix.iter().any(|v| !v.is_finite())
        || [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .any(|v| !v.is_finite())
        || rect.width < 0.
        || rect.height < 0.
    {
        return None;
    }
    let mut polygon = [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), rect.bottom()),
        (rect.x, rect.bottom()),
    ]
    .into_iter()
    .map(|(x, y)| {
        [
            matrix[0] * x + matrix[4] * y + matrix[12],
            matrix[1] * x + matrix[5] * y + matrix[13],
            matrix[3] * x + matrix[7] * y + matrix[15],
        ]
    })
    .collect::<Vec<_>>();
    if polygon.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let unbounded = polygon.iter().any(|p| p[2] <= 0.) && polygon.iter().any(|p| p[2] > 0.);
    let saturated = unbounded
        || polygon.iter().any(|p| {
            p[2] > 0.
                && (p[0].abs() > MAX_SCROLL_COORDINATE * p[2]
                    || p[1].abs() > MAX_SCROLL_COORDINATE * p[2])
        });
    for plane in [
        [0., 0., 1.],
        [1., 0., MAX_SCROLL_COORDINATE],
        [-1., 0., MAX_SCROLL_COORDINATE],
        [0., 1., MAX_SCROLL_COORDINATE],
        [0., -1., MAX_SCROLL_COORDINATE],
    ] {
        if polygon.is_empty() {
            break;
        }
        let distance = |p: [f64; 3]| plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2];
        let mut output = Vec::new();
        let mut previous = *polygon.last()?;
        let mut before = distance(previous);
        for current in polygon {
            let after = distance(current);
            if (before >= 0.) != (after >= 0.) {
                let scale = before.abs().max(after.abs());
                let (a, b) = (before / scale, after / scale);
                let t = a / (a - b);
                output.push(std::array::from_fn(|i| {
                    previous[i] + t * (current[i] - previous[i])
                }));
            }
            if after >= 0. {
                output.push(current);
            }
            previous = current;
            before = after;
        }
        polygon = output;
    }
    let (mut left, mut top, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in polygon {
        if p[2] <= 0. {
            continue;
        }
        let (x, y) = (p[0] / p[2], p[1] / p[2]);
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    let rect = if left.is_finite() {
        clip_scroll_rect(CssRect {
            x: left,
            y: top,
            width: (right - left).max(0.),
            height: (bottom - top).max(0.),
        })?
    } else {
        CssRect {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
        }
    };
    Some(ProjectedBounds {
        rect,
        unbounded,
        saturated,
    })
}
#[cfg(test)]
mod projection_tests {
    use super::*;
    #[test]
    fn camera_crossing_retains_ray_to_native_boundary() {
        let mut matrix = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        matrix[3] = -0.02;
        let projected = projected_bounds(
            matrix,
            CssRect {
                x: 0.,
                y: 10.,
                width: 100.,
                height: 10.,
            },
        )
        .unwrap();
        assert!(projected.unbounded && projected.saturated);
        assert_eq!(projected.rect.x, 0.);
        assert!((projected.rect.right() - MAX_SCROLL_COORDINATE).abs() < 0.001);
        assert_eq!(projected.rect.y, 10.);
    }
}
