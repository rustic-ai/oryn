//! Native current paths. Coordinates are transformed when commands are added.

/// Inputs for ellipse.
pub(super) struct EllipseInputs {
    pub(super) center: (f64, f64),
    pub(super) radii: (f64, f64),
    pub(super) rotation: f64,
    pub(super) start: f64,
    pub(super) end: f64,
    pub(super) counterclockwise: bool,
    pub(super) matrix: [f64; 6],
}
use kurbo::{BezPath, PathEl, Point, Shape};

#[derive(Clone, Debug, Default)]
pub(super) struct CanvasPath {
    path: BezPath,
    current: Option<Point>,
    start: Option<Point>,
}
impl CanvasPath {
    pub(super) fn from_svg(value: &str) -> Self {
        super::canvas_path_svg::parse(value)
    }
    fn append(&mut self, path: &BezPath, matrix: [f64; 6]) {
        let transform = kurbo::Affine::new(matrix);
        for element in path.elements() {
            match *element {
                PathEl::MoveTo(p) => self.move_to(transform * p),
                PathEl::LineTo(p) => self.line_to(transform * p),
                PathEl::QuadTo(a, b) => self.quad_to(transform * a, transform * b),
                PathEl::CurveTo(a, b, c) => {
                    self.curve_to(transform * a, transform * b, transform * c)
                }
                PathEl::ClosePath => self.close(),
            }
        }
    }
    pub(super) fn add_path(&mut self, other: &Self, matrix: [f64; 6]) {
        self.append(&other.path, matrix);
    }
    pub(super) fn transformed(&self, matrix: [f64; 6]) -> Self {
        let mut result = Self::default();
        result.add_path(self, matrix);
        result
    }
    pub(super) fn move_to(&mut self, point: Point) {
        if !point.is_finite() {
            return;
        }
        self.path.move_to(point);
        self.current = Some(point);
        self.start = Some(point);
    }
    pub(super) fn line_to(&mut self, point: Point) {
        if !point.is_finite() {
            return;
        }
        if self.current.is_none() {
            self.move_to(point);
            return;
        }
        self.path.line_to(point);
        self.current = Some(point);
    }
    pub(super) fn quad_to(&mut self, control: Point, end: Point) {
        if !control.is_finite() || !end.is_finite() {
            return;
        }
        if self.current.is_none() {
            self.move_to(control);
        }
        self.path.quad_to(control, end);
        self.current = Some(end);
    }
    pub(super) fn curve_to(&mut self, first: Point, second: Point, end: Point) {
        if !first.is_finite() || !second.is_finite() || !end.is_finite() {
            return;
        }
        if self.current.is_none() {
            self.move_to(first);
        }
        self.path.curve_to(first, second, end);
        self.current = Some(end);
    }
    pub(super) fn close(&mut self) {
        if let Some(start) = self.start {
            self.path.close_path();
            self.move_to(start);
        }
    }
    pub(super) fn rectangle(&mut self, points: [Point; 4]) {
        if !points.iter().all(|p| p.is_finite()) {
            return;
        }
        self.move_to(points[0]);
        for point in &points[1..] {
            self.line_to(*point);
        }
        self.close();
    }
    pub(super) fn round_rectangle(
        &mut self,
        rect: [f64; 4],
        mut radii: [(f64, f64); 4],
        matrix: [f64; 6],
    ) {
        let [x, y, width, height] = rect;
        let w = width.abs();
        let h = height.abs();
        let mut scale = 1.0_f64;
        for (extent, sum) in [
            (w, radii[0].0 + radii[1].0),
            (w, radii[3].0 + radii[2].0),
            (h, radii[0].1 + radii[3].1),
            (h, radii[1].1 + radii[2].1),
        ] {
            if sum > 0.0 {
                scale = scale.min(extent / sum);
            }
        }
        for radius in &mut radii {
            radius.0 *= scale;
            radius.1 *= scale;
        }
        let sx = if width < 0.0 { -1.0 } else { 1.0 };
        let sy = if height < 0.0 { -1.0 } else { 1.0 };
        let transform = [
            matrix[0] * sx,
            matrix[1] * sx,
            matrix[2] * sy,
            matrix[3] * sy,
            matrix[0] * x + matrix[2] * y + matrix[4],
            matrix[1] * x + matrix[3] * y + matrix[5],
        ];
        if !transform.iter().all(|n| n.is_finite()) {
            return;
        }
        let map = |x, y| kurbo::Affine::new(transform) * Point::new(x, y);
        let [tl, tr, br, bl] = radii;
        let half_pi = std::f64::consts::FRAC_PI_2;
        self.move_to(map(tl.0, 0.0));
        self.line_to(map(w - tr.0, 0.0));
        self.ellipse(EllipseInputs {
            center: (w - tr.0, tr.1),
            radii: tr,
            rotation: 0.0,
            start: -half_pi,
            end: 0.0,
            counterclockwise: false,
            matrix: transform,
        });
        self.line_to(map(w, h - br.1));
        self.ellipse(EllipseInputs {
            center: (w - br.0, h - br.1),
            radii: br,
            rotation: 0.0,
            start: 0.0,
            end: half_pi,
            counterclockwise: false,
            matrix: transform,
        });
        self.line_to(map(bl.0, h));
        self.ellipse(EllipseInputs {
            center: (bl.0, h - bl.1),
            radii: bl,
            rotation: 0.0,
            start: half_pi,
            end: 2.0 * half_pi,
            counterclockwise: false,
            matrix: transform,
        });
        self.line_to(map(0.0, tl.1));
        self.ellipse(EllipseInputs {
            center: (tl.0, tl.1),
            radii: tl,
            rotation: 0.0,
            start: 2.0 * half_pi,
            end: 3.0 * half_pi,
            counterclockwise: false,
            matrix: transform,
        });
        self.close();
        self.move_to(map(0.0, 0.0));
    }
    pub(super) fn ellipse(&mut self, params: EllipseInputs) {
        let EllipseInputs {
            center,
            radii,
            rotation,
            start,
            end,
            counterclockwise,
            matrix,
        } = params;

        let tau = std::f64::consts::TAU;
        let difference = end - start;
        let sweep = if !counterclockwise && difference >= tau {
            tau
        } else if counterclockwise && -difference >= tau {
            -tau
        } else if counterclockwise {
            -(-difference).rem_euclid(tau)
        } else {
            difference.rem_euclid(tau)
        };
        let start = start.rem_euclid(tau);
        let (sin, cos) = rotation.sin_cos();
        let point = |angle: f64| {
            let (s, c) = angle.sin_cos();
            (
                center.0 + radii.0 * c * cos - radii.1 * s * sin,
                center.1 + radii.0 * c * sin + radii.1 * s * cos,
            )
        };
        let derivative = |angle: f64| {
            let (s, c) = angle.sin_cos();
            (
                -radii.0 * s * cos - radii.1 * c * sin,
                -radii.0 * s * sin + radii.1 * c * cos,
            )
        };
        let map = |(x, y)| {
            Point::new(
                matrix[0] * x + matrix[2] * y + matrix[4],
                matrix[1] * x + matrix[3] * y + matrix[5],
            )
        };
        self.line_to(map(point(start)));
        if sweep == 0.0 {
            return;
        }
        let count = (sweep.abs() / (std::f64::consts::PI / 16.0)).ceil() as usize;
        let step = sweep / count as f64;
        for index in 0..count {
            let a = start + index as f64 * step;
            let b = a + step;
            let p = point(a);
            let q = point(b);
            let d = derivative(a);
            let e = derivative(b);
            let factor = 4.0 / 3.0 * (step / 4.0).tan();
            self.curve_to(
                map((p.0 + factor * d.0, p.1 + factor * d.1)),
                map((q.0 - factor * e.0, q.1 - factor * e.1)),
                map(q),
            );
        }
    }
    pub(super) fn arc_to(
        &mut self,
        corner: Point,
        next: Point,
        radius: f64,
        matrix: [f64; 6],
        inverse: Option<[f64; 6]>,
    ) {
        let transform = kurbo::Affine::new(matrix);
        let Some(current) = self.current else {
            self.move_to(transform * corner);
            return;
        };
        let Some(inverse) = inverse else {
            return;
        };
        let previous = kurbo::Affine::new(inverse) * current;
        let first = previous - corner;
        let second = next - corner;
        let first_length = first.hypot();
        let second_length = second.hypot();
        if radius == 0.0 || first_length == 0.0 || second_length == 0.0 {
            self.line_to(transform * corner);
            return;
        }
        let u = first / first_length;
        let v = second / second_length;
        let cross = u.x * v.y - u.y * v.x;
        if cross == 0.0 {
            self.line_to(transform * corner);
            return;
        }
        let dot = (u.x * v.x + u.y * v.y).clamp(-1.0, 1.0);
        let distance = radius * (1.0 + dot) / cross.abs();
        let first_tangent = corner + u * distance;
        let second_tangent = corner + v * distance;
        let center = first_tangent + kurbo::Vec2::new(-u.y, u.x) * (radius * cross.signum());
        if !first_tangent.is_finite() || !second_tangent.is_finite() || !center.is_finite() {
            return;
        }
        let start = (first_tangent.y - center.y).atan2(first_tangent.x - center.x);
        let end = (second_tangent.y - center.y).atan2(second_tangent.x - center.x);
        self.ellipse(EllipseInputs {
            center: (center.x, center.y),
            radii: (radius, radius),
            rotation: 0.0,
            start,
            end,
            counterclockwise: cross > 0.0,
            matrix,
        });
    }
    pub(super) fn closed(&self) -> BezPath {
        let mut result = BezPath::new();
        let mut open = false;
        for element in self.path.elements() {
            if matches!(element, PathEl::MoveTo(_)) && open {
                result.close_path();
            }
            result.push(*element);
            open = !matches!(element, PathEl::ClosePath);
        }
        if open {
            result.close_path();
        }
        result
    }
    pub(super) fn stroked(
        &self,
        style: &kurbo::Stroke,
        matrix: [f64; 6],
        inverse: [f64; 6],
    ) -> BezPath {
        // The current path is already in canvas coordinates. Stroke dimensions
        // are resolved in the user space active at the time of stroke().
        let mut path = self.path.clone();
        path.apply_affine(kurbo::Affine::new(inverse));
        let mut style = style.clone();
        if style.dash_pattern.iter().all(|n| *n == 0.0) {
            style.dash_pattern.clear();
        }
        let scale = matrix[0]
            .hypot(matrix[1])
            .max(matrix[2].hypot(matrix[3]))
            .max(1.0);
        let mut outline = kurbo::stroke(
            path.elements().iter().copied(),
            &style,
            &kurbo::StrokeOpts::default(),
            0.01 / scale,
        );
        outline.apply_affine(kurbo::Affine::new(matrix));
        outline
    }
}

pub(super) fn contains(path: &BezPath, x: f64, y: f64, evenodd: bool) -> bool {
    let winding = path.winding(Point::new(x, y));
    if evenodd {
        winding % 2 != 0
    } else {
        winding != 0
    }
}
pub(super) fn coverage(path: &BezPath, x: u32, y: u32, evenodd: bool) -> f64 {
    let mut covered = 0;
    for row in 0..8 {
        for column in 0..8 {
            if contains(
                path,
                x as f64 + (column as f64 + 0.5) / 8.0,
                y as f64 + (row as f64 + 0.5) / 8.0,
                evenodd,
            ) {
                covered += 1;
            }
        }
    }
    covered as f64 / 64.0
}

#[derive(Clone, Debug)]
pub(super) struct Clip {
    pub path: BezPath,
    pub evenodd: bool,
}
pub(super) fn inside_clips(clips: &[Clip], x: f64, y: f64) -> bool {
    clips
        .iter()
        .all(|clip| contains(&clip.path, x, y, clip.evenodd))
}
pub(super) fn coverage_in_clip(
    path: &BezPath,
    x: u32,
    y: u32,
    evenodd: bool,
    clips: &[Clip],
) -> (f64, f64) {
    if clips.is_empty() {
        return (coverage(path, x, y, evenodd), 1.0);
    }
    let mut clipped = 0;
    let mut covered = 0;
    for row in 0..8 {
        for column in 0..8 {
            let px = x as f64 + (column as f64 + 0.5) / 8.0;
            let py = y as f64 + (row as f64 + 0.5) / 8.0;
            if inside_clips(clips, px, py) {
                clipped += 1;
                if contains(path, px, py, evenodd) {
                    covered += 1;
                }
            }
        }
    }
    (
        if clipped == 0 {
            0.0
        } else {
            covered as f64 / clipped as f64
        },
        clipped as f64 / 64.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tangent_arcs_connect_segments_and_handle_degenerate_inputs() {
        let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut path = CanvasPath::default();
        path.move_to((0.0, 0.0).into());
        path.arc_to(
            (10.0, 0.0).into(),
            (10.0, 10.0).into(),
            2.0,
            identity,
            Some(identity),
        );
        let end = path.current.unwrap();
        assert!((end.x - 10.0).abs() < 1e-12 && (end.y - 2.0).abs() < 1e-12);
        let PathEl::LineTo(tangent) = path.path.elements()[1] else {
            panic!("missing tangent connection")
        };
        assert!((tangent.x - 8.0).abs() < 1e-12 && tangent.y.abs() < 1e-12);
        let mut empty = CanvasPath::default();
        empty.arc_to(
            (3.0, 4.0).into(),
            (8.0, 9.0).into(),
            2.0,
            identity,
            Some(identity),
        );
        assert_eq!(empty.current, Some(Point::new(3.0, 4.0)));
        empty.arc_to(
            (6.0, 4.0).into(),
            (9.0, 4.0).into(),
            2.0,
            identity,
            Some(identity),
        );
        assert_eq!(empty.current, Some(Point::new(6.0, 4.0)));
        empty.arc_to(
            (7.0, 8.0).into(),
            (0.0, 0.0).into(),
            0.0,
            identity,
            Some(identity),
        );
        assert_eq!(empty.current, Some(Point::new(7.0, 8.0)));
    }

    #[test]
    fn filling_closes_subpaths_without_mutating_current_path() {
        let mut path = CanvasPath::default();
        path.move_to((0.0, 0.0).into());
        path.line_to((4.0, 0.0).into());
        path.line_to((4.0, 4.0).into());
        assert!(contains(&path.closed(), 3.0, 1.0, false));
        assert!(!contains(&path.closed(), 1.0, 3.0, false));
        path.line_to((0.0, 4.0).into());
        assert!(contains(&path.closed(), 1.0, 3.0, false));
    }
    #[test]
    fn winding_rules_and_ellipse_direction() {
        let mut path = CanvasPath::default();
        for _ in 0..2 {
            path.rectangle([
                (0.0, 0.0).into(),
                (4.0, 0.0).into(),
                (4.0, 4.0).into(),
                (0.0, 4.0).into(),
            ]);
        }
        assert!(contains(&path.closed(), 2.0, 2.0, false));
        assert!(!contains(&path.closed(), 2.0, 2.0, true));
        let mut circle = CanvasPath::default();
        circle.ellipse(EllipseInputs {
            center: (5.0, 5.0),
            radii: (3.0, 2.0),
            rotation: 0.0,
            start: 0.0,
            end: std::f64::consts::TAU,
            counterclockwise: false,
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        });
        assert!(contains(&circle.closed(), 5.0, 5.0, false));
        assert!(!contains(&circle.closed(), 5.0, 8.0, false));
    }
}
