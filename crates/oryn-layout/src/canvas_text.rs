//! Native canvas text shaping shared by measurement and eventual glyph painting.
use super::{LayoutError, TextEngine, length, style_value};
use oryn_common::v4::{PaintFontResource, PaintGlyphRun, ViewportState};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
};
use std::collections::BTreeMap;

/// Original native glyphs and fonts, not estimated character advances.
#[derive(Debug)]
pub struct CanvasTextShape {
    pub width: f64,
    pub baseline: f64,
    pub glyph_runs: Vec<PaintGlyphRun>,
    pub fonts: BTreeMap<String, PaintFontResource>,
}

impl CanvasTextShape {
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
    if let Some(error) = engine.font_demand_error {
        return Err(error);
    }
    Ok(CanvasTextShape {
        width: shaped.width,
        baseline: shaped.baselines.first().copied().unwrap_or(0.),
        glyph_runs: shaped.glyph_runs.into_iter().flatten().collect(),
        fonts: engine.paint_fonts,
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
        let double = measure("a  b");
        assert!(double.width > single.width);
        assert_eq!(measure("a\r\nb").width, double.width);
        assert!(measure(" a b ").width > single.width);
        assert!(single.baseline > 0.);
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
