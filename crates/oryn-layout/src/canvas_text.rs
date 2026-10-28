//! Native canvas text shaping shared by measurement and eventual glyph painting.
use super::{LayoutError, TextEngine, length, style_value};
use oryn_common::v4::{PaintFontResource, PaintGlyphRun, ViewportState};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
};
use std::collections::BTreeMap;
pub mod baselines;
mod ink;
pub mod metrics;

/// Original native glyphs and fonts, not estimated character advances.
#[derive(Debug)]
pub struct CanvasTextShape {
    pub width: f64,
    pub baseline: f64,
    pub glyph_runs: Vec<PaintGlyphRun>,
    pub fonts: BTreeMap<String, PaintFontResource>,
    /// Native face selected for a neutral space independently of text ink.
    /// This preserves a metric source for empty strings. CSS first-available-
    /// font selection still needs reconciliation with unicode-range faces.
    pub neutral_font_metrics: Option<skrifa::metrics::Metrics>,
    pub font_size: f64,
    neutral_font_run: Option<PaintGlyphRun>,
}

impl CanvasTextShape {
    /// Horizontal BASE data for the selected neutral face, if present. This
    /// connects the native parser to original font bytes without synthesizing
    /// absent baselines or pretending neutral selection is first-available.
    pub fn neutral_baselines(
        &self,
        script: skrifa::raw::types::Tag,
    ) -> Result<Option<BTreeMap<skrifa::raw::types::Tag, f64>>, LayoutError> {
        use skrifa::raw::{TableProvider, types::Tag};
        let Some(run) = &self.neutral_font_run else {
            return Ok(None);
        };
        let source = self.fonts.get(&run.font_key).ok_or_else(|| {
            LayoutError::InvalidRenderedTree("canvas neutral font is missing".into())
        })?;
        let face = FontRef::from_index(&source.bytes, source.collection_index)
            .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
        let Some(table) = face.table_data(Tag::new(b"BASE")) else {
            return Ok(None);
        };
        let units = face
            .head()
            .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?
            .units_per_em();
        if units == 0 {
            return Err(LayoutError::InvalidRenderedTree(
                "zero font units per em".into(),
            ));
        }
        baselines::read_horizontal(
            table.as_bytes(),
            script,
            run.font_size / f64::from(units),
            &run.normalized_coords,
        )
    }
    /// Ink union in the shaped line's coordinate system (y increases down).
    pub fn ink_bounds(&self) -> Result<Option<[f64; 4]>, LayoutError> {
        use skrifa::{GlyphId, outline::DrawSettings};
        let mut union: Option<[f64; 4]> = None;
        for run in &self.glyph_runs {
            if !run.font_size.is_finite()
                || run.font_size <= 0.
                || run.font_size > f64::from(f32::MAX)
                || !run.scale_x.is_finite()
                || run.scale_x <= 0.
                || !run.skew_degrees.is_finite()
                || run.skew_degrees.abs() >= 90.
                || run
                    .normalized_coords
                    .iter()
                    .any(|v| !(-16384..=16384).contains(v))
                || run
                    .glyphs
                    .iter()
                    .any(|g| !g.x.is_finite() || !g.y.is_finite())
                || run.vertical
            {
                return Err(LayoutError::InvalidRenderedTree(
                    "invalid horizontal canvas glyph instance".into(),
                ));
            }
            let source = self.fonts.get(&run.font_key).ok_or_else(|| {
                LayoutError::InvalidRenderedTree("canvas glyph font is missing".into())
            })?;
            let font = FontRef::from_index(&source.bytes, source.collection_index)
                .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
            let coords = run
                .normalized_coords
                .iter()
                .map(|v| NormalizedCoord::from_bits(*v))
                .collect::<Vec<_>>();
            let outlines = font.outline_glyphs();
            for glyph in &run.glyphs {
                let outline = outlines.get(GlyphId::new(glyph.id)).ok_or_else(|| {
                    LayoutError::InvalidRenderedTree("canvas glyph outline is missing".into())
                })?;
                let mut pen = ink::Pen::new(f64::from(run.skew_degrees).to_radians().tan());
                outline
                    .draw(
                        DrawSettings::unhinted(
                            Size::new(run.font_size as f32),
                            LocationRef::new(&coords),
                        ),
                        &mut pen,
                    )
                    .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
                if pen.invalid {
                    return Err(LayoutError::InvalidRenderedTree(
                        "non-finite canvas outline".into(),
                    ));
                }
                if let Some(b) = pen.bounds {
                    let radius = if run.embolden {
                        run.font_size / 48.
                    } else {
                        0.
                    };
                    let b = [
                        glyph.x + (b[0] - radius) * run.scale_x,
                        glyph.y - b[3] - radius,
                        glyph.x + (b[2] + radius) * run.scale_x,
                        glyph.y - b[1] + radius,
                    ];
                    if b.iter().any(|v| !v.is_finite()) {
                        return Err(LayoutError::InvalidRenderedTree(
                            "canvas glyph bounds overflow".into(),
                        ));
                    }
                    let u = union.get_or_insert(b);
                    u[0] = u[0].min(b[0]);
                    u[1] = u[1].min(b[1]);
                    u[2] = u[2].max(b[2]);
                    u[3] = u[3].max(b[3]);
                }
            }
        }
        Ok(union)
    }
    /// Font-instance metrics in glyph-run order. Fallback faces and variation
    /// coordinates must remain distinct; these are not yet canvas baseline-
    /// adjusted TextMetrics values.
    pub fn font_metrics(&self) -> Result<Vec<skrifa::metrics::Metrics>, LayoutError> {
        self.glyph_runs
            .iter()
            .map(|run| {
                let source = self.fonts.get(&run.font_key).ok_or_else(|| {
                    LayoutError::InvalidRenderedTree("canvas glyph font is missing".into())
                })?;
                let font = FontRef::from_index(&source.bytes, source.collection_index)
                    .map_err(|error| LayoutError::InvalidRenderedTree(error.to_string()))?;
                let coordinates = run
                    .normalized_coords
                    .iter()
                    .map(|value| NormalizedCoord::from_bits(*value))
                    .collect::<Vec<_>>();
                Ok(font.metrics(
                    Size::new(run.font_size as f32),
                    LocationRef::new(&coordinates),
                ))
            })
            .collect()
    }
}

pub fn shape(
    text: &str,
    font_style: &BTreeMap<String, String>,
    viewport: ViewportState,
    fonts: &[(String, Vec<u8>, String, bool, bool)],
) -> Result<CanvasTextShape, LayoutError> {
    // Canvas replaces each ASCII whitespace character independently. It does
    // not collapse runs, trim edges or combine CRLF into one space.
    let text: String = text
        .chars()
        .map(|ch| {
            if matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' ') {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let mut style = font_style.clone();
    style.extend([
        ("white-space".into(), "pre".into()),
        ("white-space-collapse".into(), "preserve".into()),
        ("text-wrap-mode".into(), "nowrap".into()),
        ("text-transform".into(), "none".into()),
        ("writing-mode".into(), "horizontal-tb".into()),
        ("text-align".into(), "left".into()),
    ]);
    let font_size = length(style_value(&style, "font-size", "10px"), 16., 16., viewport);
    let mut engine = TextEngine::new(fonts);
    let shaped = engine.shape_styled(&text, 1_000_000_000., font_size, &style, viewport);
    let neutral = engine.shape_styled(" ", 1_000_000_000., font_size, &style, viewport);
    if let Some(error) = engine.font_demand_error {
        return Err(error);
    }
    let neutral_font_metrics = neutral
        .glyph_runs
        .iter()
        .flatten()
        .next()
        .map(|run| {
            let source = engine.paint_fonts.get(&run.font_key).ok_or_else(|| {
                LayoutError::InvalidRenderedTree("canvas neutral font is missing".into())
            })?;
            let face = FontRef::from_index(&source.bytes, source.collection_index)
                .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
            let coordinates = run
                .normalized_coords
                .iter()
                .map(|v| NormalizedCoord::from_bits(*v))
                .collect::<Vec<_>>();
            Ok(face.metrics(
                Size::new(run.font_size as f32),
                LocationRef::new(&coordinates),
            ))
        })
        .transpose()?;
    Ok(CanvasTextShape {
        width: shaped.width,
        baseline: shaped.baselines.first().copied().unwrap_or(0.),
        glyph_runs: shaped.glyph_runs.into_iter().flatten().collect(),
        fonts: engine.paint_fonts,
        neutral_font_metrics,
        font_size,
        neutral_font_run: neutral.glyph_runs.into_iter().flatten().next(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_canvas_shape_preserves_spaces_and_original_font_glyphs() {
        let style = BTreeMap::from([
            ("font-family".into(), "monospace".into()),
            ("font-size".into(), "20px".into()),
        ]);
        let measure = |text| shape(text, &style, ViewportState::default(), &[]).unwrap();
        let single = measure("a b");
        let empty = measure("");
        assert_eq!(empty.width, 0.);
        assert_eq!(empty.ink_bounds().unwrap(), None);
        assert!(empty.neutral_font_metrics.unwrap().ascent > 0.);
        assert_eq!(empty.neutral_font_metrics, single.neutral_font_metrics);
        assert_eq!(
            empty
                .neutral_baselines(skrifa::raw::types::Tag::new(b"latn"))
                .unwrap(),
            single
                .neutral_baselines(skrifa::raw::types::Tag::new(b"latn"))
                .unwrap()
        );
        let double = measure("a  b");
        assert!(double.width > single.width);
        assert_eq!(measure("a\r\nb").width, double.width);
        assert!(measure(" a b ").width > single.width);
        assert!(single.baseline > 0.);
        let bounds = single.ink_bounds().unwrap().unwrap();
        assert!(bounds[2] > bounds[0] && bounds[3] > bounds[1]);
        assert_eq!(measure("   ").ink_bounds().unwrap(), None);
        let mut translated = measure("a b");
        for run in &mut translated.glyph_runs {
            for glyph in &mut run.glyphs {
                glyph.x += 23.;
                glyph.y += 17.;
            }
        }
        let shifted = translated.ink_bounds().unwrap().unwrap();
        for i in 0..4 {
            assert!((shifted[i] - bounds[i] - if i % 2 == 0 { 23. } else { 17. }).abs() < 1e-9);
        }
        translated.glyph_runs[0].skew_degrees = f32::NAN;
        assert!(translated.ink_bounds().is_err());
        assert!(!single.glyph_runs.is_empty());
        let metrics = single.font_metrics().unwrap();
        assert_eq!(metrics.len(), single.glyph_runs.len());
        assert!(
            metrics
                .iter()
                .all(|metric| metric.units_per_em > 0 && metric.ascent > 0.)
        );
        let doubled = shape(
            "a b",
            &BTreeMap::from([
                ("font-family".into(), "monospace".into()),
                ("font-size".into(), "40px".into()),
            ]),
            ViewportState::default(),
            &[],
        )
        .unwrap();
        assert_eq!(
            doubled.font_metrics().unwrap()[0].ascent,
            metrics[0].ascent * 2.
        );
        for run in &single.glyph_runs {
            assert!(!run.glyphs.is_empty());
            assert!(!single.fonts[&run.font_key].bytes.is_empty());
        }
    }
}
