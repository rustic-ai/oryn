//! Bounded SVG path parsing and adaptive native contour construction.
use svgtypes::{PathParser, PathSegment};
const LIMIT: usize = 65536;

type Point = [f64; 2];
fn midpoint(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]
}
fn append(output: &mut Vec<Point>, point: Point) -> Result<(), String> {
    if output.len() >= LIMIT || !point.iter().all(|v| v.is_finite()) {
        return Err("native path coordinate budget".into());
    }
    output.push(point);
    Ok(())
}
fn distance(point: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if length == 0. {
        (point[0] - a[0]).hypot(point[1] - a[1])
    } else {
        ((point[0] - a[0]) * dy - (point[1] - a[1]) * dx).abs() / length
    }
}
fn cubic(
    output: &mut Vec<Point>,
    a: Point,
    b: Point,
    c: Point,
    d: Point,
    depth: u8,
    tolerance: f64,
) -> Result<(), String> {
    let polygon_length = (b[0] - a[0]).hypot(b[1] - a[1])
        + (c[0] - b[0]).hypot(c[1] - b[1])
        + (d[0] - c[0]).hypot(d[1] - c[1]);
    let chord = (d[0] - a[0]).hypot(d[1] - a[1]);
    if distance(b, a, d).max(distance(c, a, d)) <= tolerance && polygon_length - chord <= tolerance
    {
        return append(output, d);
    }
    if depth >= 32 {
        return Err("native path subdivision bound".into());
    }
    let ab = midpoint(a, b);
    let bc = midpoint(b, c);
    let cd = midpoint(c, d);
    let abc = midpoint(ab, bc);
    let bcd = midpoint(bc, cd);
    let middle = midpoint(abc, bcd);
    cubic(output, a, ab, abc, middle, depth + 1, tolerance)?;
    cubic(output, middle, bcd, cd, d, depth + 1, tolerance)
}
struct ArcParameters {
    radius: Point,
    rotation: f64,
    large: bool,
    sweep: bool,
}
fn arc(
    output: &mut Vec<Point>,
    start: Point,
    end: Point,
    parameters: ArcParameters,
    tolerance: f64,
) -> Result<(), String> {
    let ArcParameters {
        radius: [mut rx, mut ry],
        rotation,
        large,
        sweep,
    } = parameters;
    if start == end {
        return Ok(());
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx == 0. || ry == 0. {
        return append(output, end);
    }
    let phi = rotation.to_radians();
    let (cos, sin) = (phi.cos(), phi.sin());
    let (dx, dy) = ((start[0] - end[0]) / 2., (start[1] - end[1]) / 2.);
    let (x, y) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let scale = (x / rx).powi(2) + (y / ry).powi(2);
    if scale > 1. {
        rx *= scale.sqrt();
        ry *= scale.sqrt();
    }
    let denominator = (rx * y).powi(2) + (ry * x).powi(2);
    let numerator = (rx * ry).powi(2) - denominator;
    let sign = if large == sweep { -1. } else { 1. };
    let factor = sign * (numerator.max(0.) / denominator).sqrt();
    let (cx, cy) = (factor * rx * y / ry, -factor * ry * x / rx);
    let center = [
        cos * cx - sin * cy + (start[0] + end[0]) / 2.,
        sin * cx + cos * cy + (start[1] + end[1]) / 2.,
    ];
    let u = [(x - cx) / rx, (y - cy) / ry];
    let v = [(-x - cx) / rx, (-y - cy) / ry];
    let theta = u[1].atan2(u[0]);
    let mut delta = (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]);
    if sweep && delta < 0. {
        delta += std::f64::consts::TAU
    } else if !sweep && delta > 0. {
        delta -= std::f64::consts::TAU
    }
    let radius = rx.max(ry);
    let step = 2. * (1. - tolerance / radius).clamp(-1., 1.).acos();
    if step <= 0. || !step.is_finite() {
        return Err("native arc precision bound".into());
    }
    let count = (delta.abs() / step).ceil().max(1.);
    if !count.is_finite() || count > LIMIT as f64 {
        return Err("native arc subdivision bound".into());
    }
    for index in 1..=count as usize {
        if index == count as usize {
            append(output, end)?
        } else {
            let angle = theta + delta * index as f64 / count;
            append(
                output,
                [
                    center[0] + cos * rx * angle.cos() - sin * ry * angle.sin(),
                    center[1] + sin * rx * angle.cos() + cos * ry * angle.sin(),
                ],
            )?
        }
    }
    Ok(())
}
pub(super) fn parse(source: &str) -> Result<Vec<Vec<Point>>, String> {
    parse_tolerance(source, 0.001)
}
pub(super) fn parse_tolerance(source: &str, tolerance: f64) -> Result<Vec<Vec<Point>>, String> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err("native path tolerance invalid".into());
    }
    let mut contours = Vec::new();
    let mut points = Vec::new();
    let mut current = [0.; 2];
    let mut start = current;
    let mut previous_cubic = None;
    let mut previous_quadratic = None;
    let mut total = 0usize;
    for segment in PathParser::from(source) {
        let segment = segment.map_err(|error| format!("invalid native SVG path: {error}"))?;
        let point = |abs: bool, x: f64, y: f64| {
            if abs {
                [x, y]
            } else {
                [current[0] + x, current[1] + y]
            }
        };
        let mut next_cubic = None;
        let mut next_quadratic = None;
        match segment {
            PathSegment::MoveTo { abs, x, y } => {
                if !points.is_empty() {
                    total += points.len();
                    contours.push(std::mem::take(&mut points));
                }
                current = point(abs, x, y);
                start = current;
                append(&mut points, current)?;
            }
            PathSegment::LineTo { abs, x, y } => {
                current = point(abs, x, y);
                append(&mut points, current)?;
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                current = [if abs { x } else { current[0] + x }, current[1]];
                append(&mut points, current)?;
            }
            PathSegment::VerticalLineTo { abs, y } => {
                current = [current[0], if abs { y } else { current[1] + y }];
                append(&mut points, current)?;
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let b = point(abs, x1, y1);
                let c = point(abs, x2, y2);
                let d = point(abs, x, y);
                cubic(&mut points, current, b, c, d, 0, tolerance)?;
                current = d;
                next_cubic = Some(c);
            }
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
                let b = previous_cubic.map_or(current, |p: Point| {
                    [2. * current[0] - p[0], 2. * current[1] - p[1]]
                });
                let c = point(abs, x2, y2);
                let d = point(abs, x, y);
                cubic(&mut points, current, b, c, d, 0, tolerance)?;
                current = d;
                next_cubic = Some(c);
            }
            PathSegment::Quadratic { abs, x1, y1, x, y } => {
                let control = point(abs, x1, y1);
                let d = point(abs, x, y);
                let b = [
                    current[0] + (control[0] - current[0]) * 2. / 3.,
                    current[1] + (control[1] - current[1]) * 2. / 3.,
                ];
                let c = [
                    d[0] + (control[0] - d[0]) * 2. / 3.,
                    d[1] + (control[1] - d[1]) * 2. / 3.,
                ];
                cubic(&mut points, current, b, c, d, 0, tolerance)?;
                current = d;
                next_quadratic = Some(control);
            }
            PathSegment::SmoothQuadratic { abs, x, y } => {
                let control = previous_quadratic.map_or(current, |p: Point| {
                    [2. * current[0] - p[0], 2. * current[1] - p[1]]
                });
                let d = point(abs, x, y);
                let b = [
                    current[0] + (control[0] - current[0]) * 2. / 3.,
                    current[1] + (control[1] - current[1]) * 2. / 3.,
                ];
                let c = [
                    d[0] + (control[0] - d[0]) * 2. / 3.,
                    d[1] + (control[1] - d[1]) * 2. / 3.,
                ];
                cubic(&mut points, current, b, c, d, 0, tolerance)?;
                current = d;
                next_quadratic = Some(control);
            }
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let end = point(abs, x, y);
                arc(
                    &mut points,
                    current,
                    end,
                    ArcParameters {
                        radius: [rx, ry],
                        rotation: x_axis_rotation,
                        large: large_arc,
                        sweep,
                    },
                    tolerance,
                )?;
                current = end;
            }
            PathSegment::ClosePath { .. } => {
                append(&mut points, start)?;
                current = start;
            }
        }
        if total.saturating_add(points.len()) > LIMIT || contours.len() > 4096 {
            return Err("native path total contour bound".into());
        }
        previous_cubic = next_cubic;
        previous_quadratic = next_quadratic;
    }
    if !points.is_empty() {
        contours.push(points)
    }
    Ok(contours)
}
