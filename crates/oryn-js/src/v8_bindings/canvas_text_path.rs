//! Admitted original glyph paths for the native canvas raster consumer.
use super::*;
use kurbo::{Affine, BezPath, PathEl, Point};
use oryn_layout::canvas_text::{CanvasTextShape, outlines::OutlinePen};
use oryn_layout::cssom::native_import_graph::ImportAdmission;

#[derive(Clone, Copy)]
pub(super) enum Alignment {
    Left,
    Right,
    Center,
    Start,
    End,
}
impl Alignment {
    pub(super) fn fraction(self, rtl: bool) -> f64 {
        match self {
            Self::Left => 0.,
            Self::Right => 1.,
            Self::Center => 0.5,
            Self::Start => {
                if rtl {
                    1.
                } else {
                    0.
                }
            }
            Self::End => {
                if rtl {
                    0.
                } else {
                    1.
                }
            }
        }
    }
}

/// Translate the native line's alphabetic baseline to the requested canvas
/// anchor, compressing horizontally before applying the canvas CTM. Metrics
/// themselves stay uncompressed and independent of the CTM. Invalid drawing
/// arguments are a no-op; malformed native geometry is an error.
pub(super) fn text_placement(
    shape: &CanvasTextShape,
    metrics: &oryn_layout::canvas_text::metrics::Metrics,
    alignment: Alignment,
    rtl: bool,
    point: [f64; 2],
    max_width: Option<f64>,
    transform: [f64; 6],
) -> Result<Option<[f64; 6]>, V8BindingError> {
    if point.iter().any(|v| !v.is_finite()) || max_width.is_some_and(|v| v.is_nan() || v <= 0.) {
        return Ok(None);
    }
    if !shape.width.is_finite()
        || shape.width < 0.
        || !shape.baseline.is_finite()
        || !metrics.alphabetic_baseline.is_finite()
        || transform.iter().any(|v| !v.is_finite())
    {
        return Err(V8BindingError::Message(
            "invalid native canvas text geometry".into(),
        ));
    }
    let scale = max_width
        .filter(|v| *v < shape.width)
        .map(|v| v / shape.width)
        .unwrap_or(1.);
    let matrix = Affine::new(transform)
        * Affine::translate((point[0], point[1] - metrics.alphabetic_baseline))
        * Affine::scale_non_uniform(scale, 1.)
        * Affine::translate((-shape.width * alignment.fraction(rtl), -shape.baseline));
    let coefficients = matrix.as_coeffs();
    if coefficients.iter().any(|v| !v.is_finite()) {
        return Err(V8BindingError::Message(
            "canvas text placement overflow".into(),
        ));
    }
    Ok(Some(coefficients))
}

pub(super) struct GlyphPath {
    pub(super) outline: BezPath,
    pub(super) placement: Affine,
    pub(super) embolden_radius: f64,
    _allocation: Option<std::sync::Arc<dyn Send + Sync>>,
}
struct Pen<'a> {
    elements: Vec<PathEl>,
    allocation: Option<std::sync::Arc<dyn Send + Sync>>,
    admission: &'a dyn ImportAdmission,
    error: Option<String>,
    skew: f64,
}
impl Pen<'_> {
    fn point(&self, x: f32, y: f32) -> Point {
        Point::new(f64::from(x) + self.skew * f64::from(y), f64::from(y))
    }
    fn push(&mut self, element: PathEl) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.append(element) {
            self.error = Some(error);
        }
    }
    fn append(&mut self, element: PathEl) -> Result<(), String> {
        self.admission.checkpoint()?;
        let finite = match element {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => p.is_finite(),
            PathEl::QuadTo(a, b) => a.is_finite() && b.is_finite(),
            PathEl::CurveTo(a, b, c) => a.is_finite() && b.is_finite() && c.is_finite(),
            PathEl::ClosePath => true,
        };
        if !finite {
            return Err("non-finite canvas glyph path".into());
        }
        if self.elements.len() == self.elements.capacity() {
            let capacity = self
                .elements
                .capacity()
                .checked_mul(2)
                .filter(|n| *n > 0)
                .unwrap_or(32);
            if capacity <= self.elements.len() {
                return Err("canvas glyph path capacity overflow".into());
            }
            let bytes = capacity
                .checked_mul(std::mem::size_of::<PathEl>())
                .and_then(|n| n.checked_add(std::mem::size_of::<GlyphPath>()))
                .ok_or("canvas glyph path bytes overflow")?;
            // Keep the incumbent allocation/Vec until replacement is admitted.
            let allocation = self.admission.reserve(bytes)?;
            self.admission.checkpoint()?;
            let mut replacement = Vec::new();
            replacement
                .try_reserve_exact(capacity)
                .map_err(|_| "canvas glyph path allocation")?;
            if replacement.capacity() > capacity {
                return Err("canvas glyph path exceeded admission".into());
            }
            replacement.append(&mut self.elements);
            self.elements = replacement;
            self.allocation = Some(allocation);
        }
        self.elements.push(element);
        Ok(())
    }
}
impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.push(PathEl::MoveTo(self.point(x, y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(PathEl::LineTo(self.point(x, y)));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.push(PathEl::QuadTo(self.point(cx, cy), self.point(x, y)));
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.push(PathEl::CurveTo(
            self.point(ax, ay),
            self.point(bx, by),
            self.point(x, y),
        ));
    }
    fn close(&mut self) {
        self.push(PathEl::ClosePath);
    }
}

pub(super) fn compile(
    shape: &CanvasTextShape,
    run_index: usize,
    glyph_index: usize,
    text_placement: [f64; 6],
    admission: &dyn ImportAdmission,
) -> Result<GlyphPath, V8BindingError> {
    admission.checkpoint().map_err(V8BindingError::Message)?;
    let run = shape
        .glyph_runs
        .get(run_index)
        .ok_or(V8BindingError::Allocation("canvas glyph run"))?;
    let glyph = run
        .glyphs
        .get(glyph_index)
        .ok_or(V8BindingError::Allocation("canvas glyph"))?;
    if text_placement.iter().any(|v| !v.is_finite())
        || !run.scale_x.is_finite()
        || run.scale_x <= 0.
        || !run.skew_degrees.is_finite()
        || run.skew_degrees.abs() >= 90.
    {
        return Err(V8BindingError::Message(
            "invalid canvas glyph placement".into(),
        ));
    }
    let placement =
        Affine::new(text_placement) * Affine::new([run.scale_x, 0., 0., -1., glyph.x, glyph.y]);
    if placement.as_coeffs().iter().any(|v| !v.is_finite()) {
        return Err(V8BindingError::Message(
            "canvas glyph transform overflow".into(),
        ));
    }
    let mut pen = Pen {
        elements: Vec::new(),
        allocation: None,
        admission,
        error: None,
        skew: f64::from(run.skew_degrees).to_radians().tan(),
    };
    let result = shape.draw_glyph_outline(run_index, glyph_index, &mut pen);
    if let Some(error) = pen.error {
        return Err(V8BindingError::Message(error));
    }
    result.map_err(|e| V8BindingError::Message(e.to_string()))?;
    admission.checkpoint().map_err(V8BindingError::Message)?;
    Ok(GlyphPath {
        outline: BezPath::from_vec(pen.elements),
        placement,
        embolden_radius: if run.embolden {
            run.font_size / 48.
        } else {
            0.
        },
        _allocation: pen.allocation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[test]
    fn native_canvas_glyph_paths_place_all_anchors_before_canvas_transform() {
        use oryn_layout::canvas_text::metrics::{Baseline, BaselinePositions};
        let shape =
            oryn_layout::canvas_text::shape("Ag", &BTreeMap::new(), Default::default(), &[])
                .unwrap();
        let positions = BaselinePositions {
            font_ascent: 12.,
            font_descent: 4.,
            em_over: 12.,
            em_under: -4.,
            hanging: 10.,
            ideographic_under: -3.,
        };
        let ctm = [2., 0.5, 0.25, 3., 7., 11.];
        for baseline in [
            Baseline::Top,
            Baseline::Hanging,
            Baseline::Middle,
            Baseline::Alphabetic,
            Baseline::Ideographic,
            Baseline::Bottom,
        ] {
            let metrics = shape.project_metrics(positions, baseline, 0.).unwrap();
            for rtl in [false, true] {
                for alignment in [
                    Alignment::Left,
                    Alignment::Right,
                    Alignment::Center,
                    Alignment::Start,
                    Alignment::End,
                ] {
                    let matrix = Affine::new(
                        text_placement(
                            &shape,
                            &metrics,
                            alignment,
                            rtl,
                            [100., 50.],
                            Some(shape.width / 2.),
                            ctm,
                        )
                        .unwrap()
                        .unwrap(),
                    );
                    let native_anchor = Point::new(
                        shape.width * alignment.fraction(rtl),
                        shape.baseline - positions.anchor(baseline),
                    );
                    let actual = matrix * native_anchor;
                    let expected = Affine::new(ctm) * Point::new(100., 50.);
                    assert!(actual.distance(expected) < 1e-9);
                    let advance =
                        matrix * Point::new(native_anchor.x + shape.width, native_anchor.y);
                    let expected = Affine::new(ctm) * Point::new(100. + shape.width / 2., 50.);
                    assert!(advance.distance(expected) < 1e-9);
                }
            }
            for width in [f64::NAN, 0., -1.] {
                assert!(
                    text_placement(
                        &shape,
                        &metrics,
                        Alignment::Left,
                        false,
                        [0., 0.],
                        Some(width),
                        CANVAS_IDENTITY
                    )
                    .unwrap()
                    .is_none()
                );
            }
            assert!(
                text_placement(
                    &shape,
                    &metrics,
                    Alignment::Left,
                    false,
                    [f64::INFINITY, 0.],
                    None,
                    CANVAS_IDENTITY
                )
                .unwrap()
                .is_none()
            );
            assert!(
                text_placement(
                    &shape,
                    &metrics,
                    Alignment::Left,
                    false,
                    [0., 0.],
                    Some(f64::INFINITY),
                    CANVAS_IDENTITY
                )
                .unwrap()
                .is_some()
            );
        }
    }
    struct Lease(Arc<AtomicUsize>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct TrackedAdmission {
        live: Arc<AtomicUsize>,
        checkpoints: AtomicUsize,
        deny_at: usize,
    }
    impl ImportAdmission for TrackedAdmission {
        fn checkpoint(&self) -> Result<(), String> {
            if self.checkpoints.fetch_add(1, Ordering::SeqCst) >= self.deny_at {
                Err("owner revoked".into())
            } else {
                Ok(())
            }
        }
        fn reserve(&self, _: usize) -> Result<Arc<dyn Send + Sync>, String> {
            self.live.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Lease(self.live.clone())))
        }
    }
    #[test]
    fn native_canvas_glyph_paths_retain_and_release_admission() {
        let shape = oryn_layout::canvas_text::shape("A", &BTreeMap::new(), Default::default(), &[])
            .unwrap();
        let admission = TrackedAdmission {
            live: Arc::new(AtomicUsize::new(0)),
            checkpoints: AtomicUsize::new(0),
            deny_at: usize::MAX,
        };
        let glyph = compile(&shape, 0, 0, CANVAS_IDENTITY, &admission).unwrap();
        assert_eq!(admission.live.load(Ordering::SeqCst), 1);
        drop(glyph);
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
        // Revoke after allocation admission, while outline commands are flowing.
        let admission = TrackedAdmission {
            deny_at: 4,
            ..admission
        };
        admission.checkpoints.store(0, Ordering::SeqCst);
        assert!(compile(&shape, 0, 0, CANVAS_IDENTITY, &admission).is_err());
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn native_canvas_glyph_paths_fail_closed_on_growth_and_invalid_geometry() {
        let admission = TrackedAdmission {
            live: Arc::new(AtomicUsize::new(0)),
            checkpoints: AtomicUsize::new(0),
            deny_at: usize::MAX,
        };
        let mut pen = Pen {
            elements: Vec::new(),
            allocation: None,
            admission: &admission,
            error: None,
            skew: 0.,
        };
        for i in 0..65 {
            pen.line_to(i as f32, 0.);
        }
        assert!(pen.error.is_none());
        assert_eq!(pen.elements.len(), 65);
        assert_eq!(admission.live.load(Ordering::SeqCst), 1);
        pen.line_to(f32::NAN, 0.);
        assert!(pen.error.is_some());
        pen.close();
        assert_eq!(pen.elements.len(), 65);
        drop(pen);
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
    }
    struct Admission(bool);
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<std::sync::Arc<dyn Send + Sync>, String> {
            if self.0 {
                Ok(std::sync::Arc::new(()))
            } else {
                Err("denied".into())
            }
        }
    }
    #[test]
    fn native_canvas_glyph_paths_match_measured_ink_and_fail_closed_on_admission() {
        let shape = oryn_layout::canvas_text::shape(
            "A",
            &BTreeMap::from([("font-size".into(), "20px".into())]),
            Default::default(),
            &[],
        )
        .unwrap();
        let mut glyph = compile(&shape, 0, 0, CANVAS_IDENTITY, &Admission(true)).unwrap();
        assert_eq!(glyph.embolden_radius, 0.);
        glyph.outline.apply_affine(glyph.placement);
        let b = glyph.outline.bounding_box();
        let measured = shape.ink_bounds().unwrap().unwrap();
        for (actual, expected) in [b.x0, b.y0, b.x1, b.y1].into_iter().zip(measured) {
            assert!((actual - expected).abs() < 1e-8);
        }
        assert!(compile(&shape, 0, 0, CANVAS_IDENTITY, &Admission(false)).is_err());
    }
}
