//! Continuous projected-box coverage. Half-plane clipping examines the complete
//! region, not a finite set of hit-test samples. Complex paint may conservatively
//! obstruct a region; invalid/non-invertible geometry cannot prove visibility.
use crate::v4::{CssRect, PaintProjection};
pub type Polygon = Vec<[f64; 2]>;
pub fn rectangle(rect: CssRect) -> Polygon {
    vec![
        [rect.x, rect.y],
        [rect.right(), rect.y],
        [rect.right(), rect.bottom()],
        [rect.x, rect.bottom()],
    ]
}
pub fn area(polygon: &[[f64; 2]]) -> f64 {
    if polygon.len() < 3 {
        return 0.;
    }
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        .abs()
        / 2.
}
fn clip(input: Polygon, line: [f64; 3]) -> Result<Polygon, String> {
    if input.len() > 1024 || !line.iter().all(|n| n.is_finite()) {
        return Err("visibility geometry bound".into());
    }
    let mut output = Vec::new();
    if input.is_empty() {
        return Ok(output);
    }
    let evaluate = |point: [f64; 2]| line[0] * point[0] + line[1] * point[1] + line[2];
    let mut previous = *input.last().unwrap();
    let mut before = evaluate(previous);
    for point in input {
        let after = evaluate(point);
        if !before.is_finite() || !after.is_finite() {
            return Err("non-finite visibility plane".into());
        }
        if (before >= 0.) != (after >= 0.) {
            let t = before / (before - after);
            output.push([
                previous[0] + t * (point[0] - previous[0]),
                previous[1] + t * (point[1] - previous[1]),
            ]);
        }
        if after >= 0. {
            output.push(point)
        }
        previous = point;
        before = after;
    }
    Ok(output)
}
fn inverse(matrix: [f64; 16]) -> Result<[[f64; 3]; 3], String> {
    let [a, b, c, d, e, f, g, h, i] = [
        matrix[0], matrix[4], matrix[12], matrix[1], matrix[5], matrix[13], matrix[3], matrix[7],
        matrix[15],
    ];
    if ![a, b, c, d, e, f, g, h, i].iter().all(|n| n.is_finite()) {
        return Err("invalid visibility transform".into());
    }
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if determinant == 0. || !determinant.is_finite() {
        return Err("singular visibility transform".into());
    }
    Ok([
        [e * i - f * h, c * h - b * i, b * f - c * e],
        [f * g - d * i, a * i - c * g, c * d - a * f],
        [d * h - e * g, b * g - a * h, a * e - b * d],
    ]
    .map(|row| row.map(|n| n / determinant)))
}
fn clip_rect(
    mut polygon: Polygon,
    rect: CssRect,
    matrix: [f64; 16],
    x: bool,
    y: bool,
) -> Result<Polygon, String> {
    if (!x && !y) || polygon.is_empty() {
        return Ok(polygon);
    }
    let [u, v, w] = inverse(matrix)?;
    polygon = clip(polygon, w)?;
    for (axis, low, high, enabled) in [(u, rect.x, rect.right(), x), (v, rect.y, rect.bottom(), y)]
    {
        if !enabled {
            continue;
        }
        if !low.is_finite() || !high.is_finite() || high <= low {
            return Ok(Vec::new());
        }
        polygon = clip(polygon, std::array::from_fn(|i| axis[i] - low * w[i]))?;
        polygon = clip(polygon, std::array::from_fn(|i| high * w[i] - axis[i]))?;
    }
    Ok(polygon)
}
pub fn projected(
    rect: CssRect,
    matrix: [f64; 16],
    projection: Option<&PaintProjection>,
    viewport: CssRect,
) -> Result<Polygon, String> {
    let mut polygon = clip_rect(rectangle(viewport), rect, matrix, true, true)?;
    if let Some(projection) = projection {
        if projection.clips.len() > 256 {
            return Err("visibility clip depth exceeded".into());
        }
        for boundary in &projection.clips {
            if boundary.shape.as_ref().is_some_and(|shape|!matches!(shape,crate::basic_shape::Shape::RoundedRect{radii,..} if radii.iter().all(|radius|*radius==[0.,0.]))){return Err("curved or nonconvex clipping requires native raster coverage".into())}
            polygon = clip_rect(
                polygon,
                boundary.rect,
                boundary.matrix,
                boundary.clip_x,
                boundary.clip_y,
            )?;
        }
    }
    Ok(polygon)
}
pub fn regions(projection: &PaintProjection, viewport: CssRect) -> Result<Vec<Polygon>, String> {
    if projection.fragment_boxes.len() > 1024 {
        return Err("visibility fragment bound".into());
    }
    let rects = if projection.fragment_boxes.is_empty() {
        vec![projection.box_model.border_box]
    } else {
        projection
            .fragment_boxes
            .iter()
            .map(|model| model.border_box)
            .collect()
    };
    rects
        .into_iter()
        .map(|rect| projected(rect, projection.matrix, Some(projection), viewport))
        .collect()
}
pub fn intersect(mut subject: Polygon, boundary: &[[f64; 2]]) -> Result<Polygon, String> {
    if boundary.len() < 3 {
        return Ok(Vec::new());
    }
    let winding = boundary
        .iter()
        .zip(boundary.iter().cycle().skip(1))
        .take(boundary.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        .signum();
    for (a, b) in boundary
        .iter()
        .zip(boundary.iter().cycle().skip(1))
        .take(boundary.len())
    {
        subject = clip(
            subject,
            [
                (a[1] - b[1]) * winding,
                (b[0] - a[0]) * winding,
                (a[0] * b[1] - b[0] * a[1]) * winding,
            ],
        )?;
    }
    Ok(subject)
}
pub fn overlaps(a: &[Polygon], b: &[Polygon]) -> Result<bool, String> {
    for left in a {
        for right in b {
            if area(&intersect(left.clone(), right)?) > f64::EPSILON * 64. * area(left).max(1.) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub fn fully_unclipped(projection: &PaintProjection, viewport: CssRect) -> Result<bool, String> {
    let rects = if projection.fragment_boxes.is_empty() {
        vec![projection.box_model.border_box]
    } else {
        projection
            .fragment_boxes
            .iter()
            .map(|model| model.border_box)
            .collect()
    };
    for rect in rects {
        let corners = rectangle(rect);
        let mut full = Vec::new();
        for [x, y] in corners {
            let w = projection.matrix[3] * x + projection.matrix[7] * y + projection.matrix[15];
            if w <= 0. || !w.is_finite() {
                return Ok(false);
            }
            let point = [
                (projection.matrix[0] * x + projection.matrix[4] * y + projection.matrix[12]) / w,
                (projection.matrix[1] * x + projection.matrix[5] * y + projection.matrix[13]) / w,
            ];
            if !point.iter().all(|n| n.is_finite()) {
                return Ok(false);
            }
            full.push(point);
        }
        let original = area(&full);
        let visible = area(&projected(
            rect,
            projection.matrix,
            Some(projection),
            viewport,
        )?);
        if original <= 0. || original - visible > f64::EPSILON * 64. * original.max(1.) {
            return Ok(false);
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    const I: [f64; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    fn projection(rect: CssRect) -> PaintProjection {
        PaintProjection {
            scroll: None,
            fragment_boxes: Vec::new(),
            border_segments: Vec::new(),
            border_edges: None,
            text_matrix: None,
            text_clips: Vec::new(),
            matrix: I,
            box_model: crate::v4::BoxModel {
                border_box: rect,
                padding_box: rect,
                content_box: rect,
                margin: Default::default(),
                padding: Default::default(),
                border: Default::default(),
            },
            text: Vec::new(),
            clips: Vec::new(),
        }
    }
    #[test]
    fn thin_occluder_between_former_hit_points_is_detected() {
        let viewport = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let target = regions(&projection(viewport), viewport).unwrap();
        let strip = regions(
            &projection(CssRect {
                x: 24.,
                y: 20.,
                width: 0.01,
                height: 10.,
            }),
            viewport,
        )
        .unwrap();
        assert!(overlaps(&target, &strip).unwrap());
        let outside = regions(
            &projection(CssRect {
                x: 100.,
                y: 0.,
                width: 2.,
                height: 100.,
            }),
            viewport,
        )
        .unwrap();
        assert!(!overlaps(&target, &outside).unwrap());
    }
    #[test]
    fn full_region_detects_one_axis_ancestor_clip_and_preserves_transformed_disjointness() {
        let viewport = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let mut target = projection(CssRect {
            x: 10.,
            y: 10.,
            width: 60.,
            height: 60.,
        });
        assert!(fully_unclipped(&target, viewport).unwrap());
        target.clips.push(crate::v4::PaintClip {
            shape: None,
            owner: None,
            scroll_x: false,
            scroll_y: false,
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 50.,
                height: 0.,
            },
            matrix: I,
            clip_x: true,
            clip_y: false,
        });
        assert!(!fully_unclipped(&target, viewport).unwrap());
        assert_eq!(area(&regions(&target, viewport).unwrap()[0]), 2400.);
        let mut moved = projection(CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        });
        moved.matrix[12] = 80.;
        assert!(
            !overlaps(
                &regions(&target, viewport).unwrap(),
                &regions(&moved, viewport).unwrap()
            )
            .unwrap()
        );
    }
    #[test]
    fn singular_or_camera_crossing_target_cannot_be_certified_visible() {
        let viewport = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let mut target = projection(viewport);
        target.matrix[0] = 0.;
        assert!(regions(&target, viewport).is_err());
        target.matrix = I;
        target.matrix[3] = -0.02;
        assert!(!fully_unclipped(&target, viewport).unwrap());
    }
}
