//! Intersections retain complete regions until the final bounds projection.
//! Boundary arrangements preserve disjoint pieces and holes across transforms.
use super::*;
const POINT_LIMIT: usize = 65536;
const WORK_LIMIT: usize = 4_000_000;
const TOLERANCE: f64 = 0.0001;
#[derive(Clone)]
struct Edge {
    a: [f64; 2],
    b: [f64; 2],
    leaf: usize,
}
enum Region {
    Leaf {
        shape: Shape,
        matrix: Box<[f64; 16]>,
    },
    Union(Vec<Region>),
    Intersection(Vec<Region>),
}
fn mul(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|i| {
        let row = i % 4;
        let col = i / 4;
        (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
    })
}
fn point(m: [f64; 16], p: [f64; 2]) -> Option<[f64; 2]> {
    let w = m[3] * p[0] + m[7] * p[1] + m[15];
    if w <= 0. || !w.is_finite() {
        return None;
    }
    let p = [
        (m[0] * p[0] + m[4] * p[1] + m[12]) / w,
        (m[1] * p[0] + m[5] * p[1] + m[13]) / w,
    ];
    p.iter().all(|v| v.is_finite()).then_some(p)
}
fn inverse(m: [f64; 16]) -> Option<[f64; 16]> {
    let [a, b, c, d, e, f, g, h, i] = [m[0], m[4], m[12], m[1], m[5], m[13], m[3], m[7], m[15]];
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if determinant == 0. || !determinant.is_finite() {
        return None;
    }
    let v = [
        e * i - f * h,
        c * h - b * i,
        b * f - c * e,
        f * g - d * i,
        a * i - c * g,
        c * d - a * f,
        d * h - e * g,
        b * g - a * h,
        a * e - b * d,
    ]
    .map(|v| v / determinant);
    Some([
        v[0], v[3], 0., v[6], v[1], v[4], 0., v[7], 0., 0., 1., 0., v[2], v[5], 0., v[8],
    ])
}
impl Region {
    fn cost(&self) -> usize {
        match self {
            Self::Union(regions) | Self::Intersection(regions) => regions
                .iter()
                .fold(0usize, |cost, region| cost.saturating_add(region.cost())),
            Self::Leaf { shape, .. } => match shape {
                Shape::Polygon { points, .. } => points.len().max(1),
                Shape::Path { contours, .. } => contours.iter().map(Vec::len).sum::<usize>().max(1),
                _ => 1,
            },
        }
    }
    fn contains(&self, p: [f64; 2]) -> bool {
        match self {
            Self::Union(regions) => regions.iter().any(|r| r.contains(p)),
            Self::Intersection(regions) => {
                !regions.is_empty() && regions.iter().all(|r| r.contains(p))
            }
            Self::Leaf { shape, matrix } => point(**matrix, p).is_some_and(|p| {
                if shape.contains(p[0], p[1]) {
                    return true;
                }
                let epsilon = f64::EPSILON * 64. * p[0].abs().max(p[1].abs()).max(1.);
                [[-epsilon, 0.], [epsilon, 0.], [0., -epsilon], [0., epsilon]]
                    .iter()
                    .any(|offset| shape.contains(p[0] + offset[0], p[1] + offset[1]))
            }),
        }
    }
}
fn curve(
    points: &mut Vec<[f64; 2]>,
    matrix: [f64; 16],
    center: [f64; 2],
    radius: [f64; 2],
    start: f64,
    end: f64,
    depth: u8,
) -> Result<(), String> {
    let at = |angle: f64| {
        point(
            matrix,
            [
                center[0] + radius[0] * angle.cos(),
                center[1] + radius[1] * angle.sin(),
            ],
        )
        .ok_or_else(|| "clip curve crosses projection horizon".to_owned())
    };
    let a = at(start)?;
    let b = at(end)?;
    let middle = (start + end) / 2.;
    let c = at(middle)?;
    let error = ((a[0] + b[0]) / 2. - c[0]).hypot((a[1] + b[1]) / 2. - c[1]);
    if error <= TOLERANCE {
        if points.len() >= POINT_LIMIT {
            return Err("clip region point budget".into());
        }
        points.push(b);
        return Ok(());
    }
    if depth >= 32 {
        return Err("clip curve precision bound".into());
    }
    curve(points, matrix, center, radius, start, middle, depth + 1)?;
    curve(points, matrix, center, radius, middle, end, depth + 1)
}
fn contours(shape: &Shape, matrix: [f64; 16]) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let project = |points: &[[f64; 2]]| {
        points
            .iter()
            .map(|p| {
                point(matrix, *p)
                    .ok_or_else(|| "clip contour crosses projection horizon".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
    };
    match shape {
        Shape::Polygon { points, .. } => Ok(vec![project(points)?]),
        Shape::Path { contours, .. } => contours.iter().map(|points| project(points)).collect(),
        Shape::Ellipse { center, radius } => {
            if radius[0] <= 0. || radius[1] <= 0. {
                return Ok(Vec::new());
            }
            let mut points = vec![
                point(matrix, [center[0] + radius[0], center[1]]).ok_or("clip ellipse horizon")?,
            ];
            for quadrant in 0..4 {
                curve(
                    &mut points,
                    matrix,
                    *center,
                    *radius,
                    quadrant as f64 * std::f64::consts::FRAC_PI_2,
                    (quadrant + 1) as f64 * std::f64::consts::FRAC_PI_2,
                    0,
                )?;
            }
            Ok(vec![points])
        }
        Shape::RoundedRect { rect, radii } => {
            if rect.width <= 0. || rect.height <= 0. {
                return Ok(Vec::new());
            }
            let mut points = Vec::new();
            for (index, radius) in radii.iter().copied().enumerate() {
                let center = [
                    if matches!(index, 1 | 2) {
                        rect.right() - radius[0]
                    } else {
                        rect.x + radius[0]
                    },
                    if index >= 2 {
                        rect.bottom() - radius[1]
                    } else {
                        rect.y + radius[1]
                    },
                ];
                let start = std::f64::consts::PI + index as f64 * std::f64::consts::FRAC_PI_2;
                let initial = [
                    center[0] + radius[0] * start.cos(),
                    center[1] + radius[1] * start.sin(),
                ];
                points.push(point(matrix, initial).ok_or("clip rounded rectangle horizon")?);
                if radius[0] > 0. && radius[1] > 0. {
                    curve(
                        &mut points,
                        matrix,
                        center,
                        radius,
                        start,
                        start + std::f64::consts::FRAC_PI_2,
                        0,
                    )?
                } else {
                    let corner = [
                        if matches!(index, 1 | 2) {
                            rect.right()
                        } else {
                            rect.x
                        },
                        if index >= 2 { rect.bottom() } else { rect.y },
                    ];
                    points.push(point(matrix, corner).ok_or("clip corner horizon")?);
                }
            }
            Ok(vec![points])
        }
        _ => Err("composite clip must be compiled before contour extraction".into()),
    }
}
fn compile(
    shape: &Shape,
    matrix: [f64; 16],
    edges: &mut Vec<Edge>,
    leaf: &mut usize,
    depth: usize,
) -> Result<Region, String> {
    if depth > 64 {
        return Err("clip expression depth bound".into());
    }
    match shape {
        Shape::Union(shapes) => Ok(Region::Union(
            shapes
                .iter()
                .map(|shape| compile(shape, matrix, edges, leaf, depth + 1))
                .collect::<Result<_, _>>()?,
        )),
        Shape::Intersection(shapes) => Ok(Region::Intersection(
            shapes
                .iter()
                .map(|shape| compile(shape, matrix, edges, leaf, depth + 1))
                .collect::<Result<_, _>>()?,
        )),
        Shape::Affine { matrix: m, shape } => compile(
            shape,
            mul(
                matrix,
                [
                    m[0], m[1], 0., 0., m[2], m[3], 0., 0., 0., 0., 1., 0., m[4], m[5], 0., 1.,
                ],
            ),
            edges,
            leaf,
            depth + 1,
        ),
        _ => {
            let Some(inverse) = inverse(matrix) else {
                return Ok(Region::Union(Vec::new()));
            };
            let shape = shape.refined_for_projection(matrix, TOLERANCE)?;
            let contours = contours(&shape, matrix)?;
            for points in contours {
                if edges.len().saturating_add(points.len()) > POINT_LIMIT {
                    return Err("clip arrangement edge budget".into());
                }
                for (a, b) in points
                    .iter()
                    .zip(points.iter().cycle().skip(1))
                    .take(points.len())
                {
                    edges.push(Edge {
                        a: *a,
                        b: *b,
                        leaf: *leaf,
                    });
                }
            }
            *leaf += 1;
            Ok(Region::Leaf {
                shape,
                matrix: Box::new(inverse),
            })
        }
    }
}
fn crossing(a: &Edge, b: &Edge) -> Option<[f64; 2]> {
    let r = [a.b[0] - a.a[0], a.b[1] - a.a[1]];
    let s = [b.b[0] - b.a[0], b.b[1] - b.a[1]];
    let q = [b.a[0] - a.a[0], b.a[1] - a.a[1]];
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let denominator = cross(r, s);
    if denominator == 0. {
        return None;
    }
    let t = cross(q, s) / denominator;
    let u = cross(q, r) / denominator;
    if !(0. ..=1.).contains(&t) || !(0. ..=1.).contains(&u) {
        return None;
    }
    Some([a.a[0] + t * r[0], a.a[1] + t * r[1]])
}
/// The matrices map each native shape plane into the common observation plane.
/// All constraints remain live until region membership is known; intermediate
/// axis-aligned bounds never replace a clipped region.
pub(super) fn intersect(shapes: &[(Shape, [f64; 16])]) -> Result<Option<CssRect>, String> {
    let mut edges = Vec::new();
    let mut leaf = 0;
    let regions = shapes
        .iter()
        .map(|(shape, matrix)| compile(shape, *matrix, &mut edges, &mut leaf, 0))
        .collect::<Result<Vec<_>, _>>()?;
    let region = Region::Intersection(regions);
    let membership_cost = region.cost().max(1);
    let initial_work = edges
        .len()
        .saturating_mul(membership_cost)
        .saturating_mul(2);
    if initial_work > WORK_LIMIT {
        return Err("clip membership work budget".into());
    }
    let (mut x, mut y, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    {
        let mut include = |point: [f64; 2]| {
            if region.contains(point) {
                x = x.min(point[0]);
                y = y.min(point[1]);
                right = right.max(point[0]);
                bottom = bottom.max(point[1]);
            }
        };
        for edge in &edges {
            include(edge.a);
            include(edge.b)
        }
        edges.sort_by(|a, b| a.a[0].min(a.b[0]).total_cmp(&b.a[0].min(b.b[0])));
        let mut work = initial_work;
        for (index, a) in edges.iter().enumerate() {
            let max_x = a.a[0].max(a.b[0]);
            for b in &edges[index + 1..] {
                if b.a[0].min(b.b[0]) > max_x {
                    break;
                }
                if a.leaf == b.leaf
                    || a.a[1].max(a.b[1]) < b.a[1].min(b.b[1])
                    || b.a[1].max(b.b[1]) < a.a[1].min(a.b[1])
                {
                    continue;
                }
                work = work.saturating_add(1);
                if work > WORK_LIMIT {
                    return Err("clip arrangement work budget".into());
                }
                if let Some(point) = crossing(a, b) {
                    work = work.saturating_add(membership_cost);
                    if work > WORK_LIMIT {
                        return Err("clip membership work budget".into());
                    }
                    include(point)
                }
            }
        }
    }
    Ok((x.is_finite() && y.is_finite()).then_some(CssRect {
        x,
        y,
        width: (right - x).max(0.),
        height: (bottom - y).max(0.),
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: [f64; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];

    #[test]
    fn transformed_regions_do_not_intersect_through_bounding_box_corners() {
        let square = Shape::RoundedRect {
            rect: CssRect {
                x: -1.,
                y: -1.,
                width: 2.,
                height: 2.,
            },
            radii: [[0.; 2]; 4],
        };
        let mut matrix = ID;
        let c = std::f64::consts::FRAC_1_SQRT_2;
        matrix[0] = c;
        matrix[1] = c;
        matrix[4] = -c;
        matrix[5] = c;
        let corner = Shape::RoundedRect {
            rect: CssRect {
                x: 1.2,
                y: 1.2,
                width: 0.1,
                height: 0.1,
            },
            radii: [[0.; 2]; 4],
        };
        assert!(
            intersect(&[(square, matrix), (corner, ID)])
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn compound_hole_survives_multiple_clip_constraints() {
        let outer = vec![[0., 0.], [100., 0.], [100., 100.], [0., 100.]];
        let inner = vec![[25., 25.], [75., 25.], [75., 75.], [25., 75.]];
        let hole = Shape::Path {
            contours: vec![outer, inner],
            even_odd: true,
            source: None,
        };
        let inside = Shape::RoundedRect {
            rect: CssRect {
                x: 40.,
                y: 40.,
                width: 10.,
                height: 10.,
            },
            radii: [[0.; 2]; 4],
        };
        assert!(intersect(&[(hole, ID), (inside, ID)]).unwrap().is_none());
    }
}
