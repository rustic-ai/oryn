//! Tight unhinted outline bounds after synthetic skew, not control-point bounds.
use skrifa::outline::OutlinePen;

#[derive(Default)]
pub(super) struct Pen {
    pub bounds: Option<[f64; 4]>,
    pub skew: f64,
    pub invalid: bool,
    pub cancellation: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
    pub was_cancelled: bool,
    current: [f64; 2],
    start: [f64; 2],
}
impl Pen {
    fn stopped(&mut self) -> bool {
        self.was_cancelled |= self.cancellation.as_ref().is_some_and(|check| check());
        self.was_cancelled
    }
    pub(super) fn new(skew: f64) -> Self {
        Self {
            skew,
            ..Self::default()
        }
    }
    fn point(&self, x: f32, y: f32) -> [f64; 2] {
        [f64::from(x) + self.skew * f64::from(y), f64::from(y)]
    }
    fn include(&mut self, p: [f64; 2]) {
        if p.iter().any(|v| !v.is_finite()) {
            self.invalid = true;
            return;
        }
        let b = self.bounds.get_or_insert([p[0], p[1], p[0], p[1]]);
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    fn curve(&mut self, p: [[f64; 2]; 4]) {
        self.include(p[0]);
        self.include(p[3]);
        for axis in 0..2 {
            let a = -p[0][axis] + 3. * p[1][axis] - 3. * p[2][axis] + p[3][axis];
            let b = 2. * (p[0][axis] - 2. * p[1][axis] + p[2][axis]);
            let c = p[1][axis] - p[0][axis];
            let mut roots = Vec::new();
            if a == 0. {
                if b != 0. {
                    roots.push(-c / b);
                }
            } else {
                let d = b * b - 4. * a * c;
                if d >= 0. {
                    // Stable quadratic roots avoid cancellation at nearly
                    // linear segments. A double zero root needs no division.
                    let q = -0.5 * (b + d.sqrt().copysign(b));
                    if q == 0. {
                        roots.push(-b / (2. * a));
                    } else {
                        roots.extend([q / a, c / q]);
                    }
                }
            }
            for t in roots.into_iter().filter(|t| *t > 0. && *t < 1.) {
                let u = 1. - t;
                self.include(std::array::from_fn(|i| {
                    u * u * u * p[0][i]
                        + 3. * u * u * t * p[1][i]
                        + 3. * u * t * t * p[2][i]
                        + t * t * t * p[3][i]
                }));
            }
        }
        self.current = p[3];
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.stopped() {
            return;
        }
        self.current = self.point(x, y);
        self.start = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.stopped() {
            return;
        }
        let next = self.point(x, y);
        if next != self.current {
            self.include(self.current);
            self.include(next);
        }
        self.current = next;
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if self.stopped() {
            return;
        }
        let control = self.point(cx, cy);
        let end = self.point(x, y);
        self.curve([
            self.current,
            std::array::from_fn(|i| self.current[i] + (control[i] - self.current[i]) * 2. / 3.),
            std::array::from_fn(|i| end[i] + (control[i] - end[i]) * 2. / 3.),
            end,
        ]);
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        if self.stopped() {
            return;
        }
        self.curve([
            self.current,
            self.point(ax, ay),
            self.point(bx, by),
            self.point(x, y),
        ]);
    }
    fn close(&mut self) {
        if self.stopped() {
            return;
        }
        if self.current != self.start {
            self.include(self.current);
            self.include(self.start);
        }
        self.current = self.start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tight_bounds_use_curve_extrema_not_control_points() {
        let mut pen = Pen::default();
        pen.move_to(0., 0.);
        pen.quad_to(10., 20., 20., 0.);
        let b = pen.bounds.unwrap();
        assert!((b[3] - 10.).abs() < 1e-10);
        let mut pen = Pen::default();
        pen.move_to(0., 0.);
        pen.curve_to(0., 20., 20., 20., 20., 0.);
        assert!((pen.bounds.unwrap()[3] - 15.).abs() < 1e-10);
    }
    #[test]
    fn skew_and_empty_contours_do_not_invent_control_bounds() {
        let mut empty = Pen::default();
        empty.move_to(100., 100.);
        empty.close();
        assert_eq!(empty.bounds, None);
        let mut pen = Pen::new(1.);
        pen.move_to(0., 0.);
        pen.quad_to(0., 20., 0., 0.);
        let b = pen.bounds.unwrap();
        assert!((b[2] - 10.).abs() < 1e-10 && (b[3] - 10.).abs() < 1e-10);
        pen.line_to(f32::INFINITY, 0.);
        assert!(pen.invalid);
    }
}
