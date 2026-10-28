//! Canvas anchor projection, independent of font baseline extraction.
use super::{CanvasTextShape, LayoutError};

#[derive(Clone, Copy, Debug)]
pub enum Baseline {
    Top,
    Hanging,
    Middle,
    Alphabetic,
    Ideographic,
    Bottom,
}

/// All positions are CSS pixels upwards from the alphabetic baseline.
/// Callers must resolve these from the native first-available font/inline box;
/// neutral-space metrics are not sufficient for unicode-range selection.
#[derive(Clone, Copy, Debug)]
pub struct BaselinePositions {
    pub font_ascent: f64,
    pub font_descent: f64,
    pub em_over: f64,
    pub em_under: f64,
    pub hanging: f64,
    pub ideographic_under: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub width: f64,
    pub actual_bounding_box_left: f64,
    pub actual_bounding_box_right: f64,
    pub font_bounding_box_ascent: f64,
    pub font_bounding_box_descent: f64,
    pub actual_bounding_box_ascent: f64,
    pub actual_bounding_box_descent: f64,
    pub em_height_ascent: f64,
    pub em_height_descent: f64,
    pub hanging_baseline: f64,
    pub alphabetic_baseline: f64,
    pub ideographic_baseline: f64,
}

impl BaselinePositions {
    pub fn anchor(self, baseline: Baseline) -> f64 {
        match baseline {
            Baseline::Top => self.em_over,
            Baseline::Hanging => self.hanging,
            Baseline::Middle => self.em_over / 2. + self.em_under / 2.,
            Baseline::Alphabetic => 0.,
            Baseline::Ideographic => self.ideographic_under,
            Baseline::Bottom => self.em_under,
        }
    }
}

impl CanvasTextShape {
    /// `alignment` is the resolved x anchor in the untransformed shaped line:
    /// left=0, center=width/2, right=width; resolve start/end using direction.
    pub fn project_metrics(
        &self,
        positions: BaselinePositions,
        baseline: Baseline,
        alignment: f64,
    ) -> Result<Metrics, LayoutError> {
        if [
            positions.font_ascent,
            positions.font_descent,
            positions.em_over,
            positions.em_under,
            positions.hanging,
            positions.ideographic_under,
            alignment,
            self.width,
            self.baseline,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err(LayoutError::InvalidRenderedTree(
                "non-finite canvas metric input".into(),
            ));
        }
        let anchor = positions.anchor(baseline);
        let ink = self.ink_bounds()?;
        let (left, right, ascent, descent) = ink
            .map(|b| {
                (
                    alignment - b[0],
                    b[2] - alignment,
                    self.baseline - b[1] - anchor,
                    b[3] - self.baseline + anchor,
                )
            })
            .unwrap_or((0., 0., 0., 0.));
        let metrics = Metrics {
            width: self.width,
            actual_bounding_box_left: left,
            actual_bounding_box_right: right,
            font_bounding_box_ascent: positions.font_ascent - anchor,
            font_bounding_box_descent: positions.font_descent + anchor,
            actual_bounding_box_ascent: ascent,
            actual_bounding_box_descent: descent,
            em_height_ascent: positions.em_over - anchor,
            em_height_descent: anchor - positions.em_under,
            hanging_baseline: positions.hanging - anchor,
            alphabetic_baseline: -anchor,
            ideographic_baseline: positions.ideographic_under - anchor,
        };
        if [
            metrics.width,
            metrics.actual_bounding_box_left,
            metrics.actual_bounding_box_right,
            metrics.font_bounding_box_ascent,
            metrics.font_bounding_box_descent,
            metrics.actual_bounding_box_ascent,
            metrics.actual_bounding_box_descent,
            metrics.em_height_ascent,
            metrics.em_height_descent,
            metrics.hanging_baseline,
            metrics.alphabetic_baseline,
            metrics.ideographic_baseline,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err(LayoutError::InvalidRenderedTree(
                "canvas metric projection overflow".into(),
            ));
        }
        Ok(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::v4::ViewportState;
    use std::collections::BTreeMap;
    #[test]
    fn canvas_anchor_projection_preserves_signed_distances_and_ink() {
        let shape = super::super::shape(
            "Ag",
            &BTreeMap::from([("font-size".into(), "40px".into())]),
            ViewportState::default(),
            &[],
        )
        .unwrap();
        let p = BaselinePositions {
            font_ascent: 30.,
            font_descent: 10.,
            em_over: 30.,
            em_under: -10.,
            hanging: 25.,
            ideographic_under: -6.25,
        };
        let a = shape.project_metrics(p, Baseline::Alphabetic, 0.).unwrap();
        assert!(shape.project_metrics(p, Baseline::Top, f64::NAN).is_err());
        let overflowing = BaselinePositions {
            font_ascent: f64::MAX,
            em_over: -f64::MAX,
            ..p
        };
        assert!(
            shape
                .project_metrics(overflowing, Baseline::Top, 0.)
                .is_err()
        );
        for (baseline, anchor) in [
            (Baseline::Top, 30.),
            (Baseline::Hanging, 25.),
            (Baseline::Middle, 10.),
            (Baseline::Alphabetic, 0.),
            (Baseline::Ideographic, -6.25),
            (Baseline::Bottom, -10.),
        ] {
            let m = shape
                .project_metrics(p, baseline, shape.width / 2.)
                .unwrap();
            assert_eq!(m.width, a.width);
            assert_eq!(m.em_height_ascent + m.em_height_descent, 40.);
            assert_eq!(m.alphabetic_baseline, -anchor);
            assert_eq!(m.hanging_baseline, 25. - anchor);
            assert_eq!(m.ideographic_baseline, -6.25 - anchor);
            assert!(
                (m.actual_bounding_box_ascent - a.actual_bounding_box_ascent + anchor).abs() < 1e-9
            );
            assert!(
                (m.actual_bounding_box_left - a.actual_bounding_box_left - shape.width / 2.).abs()
                    < 1e-9
            );
        }
    }
}
