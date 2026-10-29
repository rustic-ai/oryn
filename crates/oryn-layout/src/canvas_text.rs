//! Native canvas text shaping shared by measurement and eventual glyph painting.
use super::{LayoutError, TextEngine, length, style_value};
use oryn_common::v4::{PaintFontResource, PaintGlyphRun, ViewportState};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
};
use std::collections::BTreeMap;
use std::sync::Arc;
pub mod baselines;
pub mod font_inputs;
pub(crate) mod font_bytes;
pub mod font_selection;
mod ink;
pub mod metrics;
pub mod outlines;

#[derive(Default)]
struct Cancellation(Option<Arc<dyn Fn() -> bool + Send + Sync>>);
impl std::fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CanvasCancellation")
    }
}

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
    pub first_available_font: font_selection::MetricFont,
    cancellation: Cancellation,
    _font_byte_owner: Option<font_bytes::Owner>,
}

impl CanvasTextShape {
    pub fn measure_text(&self, text: &str, baseline: metrics::Baseline, alignment: f64) -> Result<metrics::Metrics, LayoutError> {
        use icu_properties::{CodePointMapData, PropertyNamesShort, props::Script};
        self.checkpoint()?;
        let scripts = CodePointMapData::<Script>::new();
        let mut script = Script::Latin;
        for (index, ch) in text.chars().enumerate() {
            if index % 1024 == 0 { self.checkpoint()?; }
            let candidate = scripts.get(ch);
            if !matches!(candidate, Script::Common | Script::Inherited | Script::Unknown) {
                script = candidate;
                break;
            }
        }
        // OpenType's legacy script tags differ from ISO 15924 for these
        // scripts; BASE uses these tags, not the newer GSUB shaping tags.
        let bytes = match script {
            Script::Hiragana | Script::Katakana => *b"kana",
            Script::Lao => *b"lao ", Script::Yi => *b"yi  ",
            Script::Nko => *b"nko ", Script::Vai => *b"vai ",
            _ => {
                let names = PropertyNamesShort::<Script>::new();
                let name = names.get(script).unwrap_or("Latn");
                let mut tag = [b' '; 4];
                if name.len() == 4 { tag.copy_from_slice(name.as_bytes()); }
                tag[0].make_ascii_lowercase();
                tag
            }
        };
        self.measure(skrifa::raw::types::Tag::new(&bytes), baseline, alignment)
    }
    fn checkpoint(&self) -> Result<(), LayoutError> {
        super::layout_checkpoint(&self.cancellation.0)
    }
    pub fn measure(
        &self,
        script: skrifa::raw::types::Tag,
        baseline: metrics::Baseline,
        alignment: f64,
    ) -> Result<metrics::Metrics, LayoutError> {
        self.checkpoint()?;
        self.project_metrics(
            self.first_available_font.baseline_positions(script)?,
            baseline,
            alignment,
        )
    }
    /// Horizontal BASE data for the selected neutral face, if present. This
    /// connects the native parser to original font bytes without synthesizing
    /// absent baselines or pretending neutral selection is first-available.
    pub fn neutral_baselines(
        &self,
        script: skrifa::raw::types::Tag,
    ) -> Result<Option<BTreeMap<skrifa::raw::types::Tag, f64>>, LayoutError> {
        self.checkpoint()?;
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
        self.checkpoint()?;
        use skrifa::{GlyphId, outline::DrawSettings};
        let mut union: Option<[f64; 4]> = None;
        for run in &self.glyph_runs {
            self.checkpoint()?;
            if !run.font_size.is_finite()
                || run.font_size < 0.
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
            if run.font_size == 0. {
                continue;
            }
            let coords = run
                .normalized_coords
                .iter()
                .map(|v| NormalizedCoord::from_bits(*v))
                .collect::<Vec<_>>();
            let outlines = font.outline_glyphs();
            for glyph in &run.glyphs {
                self.checkpoint()?;
                let outline = outlines.get(GlyphId::new(glyph.id)).ok_or_else(|| {
                    LayoutError::InvalidRenderedTree("canvas glyph outline is missing".into())
                })?;
                let mut pen = ink::Pen::new(f64::from(run.skew_degrees).to_radians().tan());
                pen.cancellation = self.cancellation.0.clone();
                outline
                    .draw(
                        DrawSettings::unhinted(
                            Size::new(run.font_size as f32),
                            LocationRef::new(&coords),
                        ),
                        &mut pen,
                    )
                    .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
                if pen.was_cancelled {
                    return Err(LayoutError::Cancelled);
                }
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
        self.checkpoint()?;
        self.glyph_runs
            .iter()
            .map(|run| {
                self.checkpoint()?;
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
    shape_controlled(text, font_style, viewport, fonts, None)
}

pub fn shape_controlled(
    text: &str,
    font_style: &BTreeMap<String, String>,
    viewport: ViewportState,
    fonts: &[(String, Vec<u8>, String, bool, bool)],
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
) -> Result<CanvasTextShape, LayoutError> {
    shape_with_font_byte_admission(text, font_style, viewport, fonts, cancelled, None, None)
}

/// Admit actual copies of bundled/document font bytes before Fontique and
/// paint-resource publication. This does not account opaque Parley scratch.
pub fn shape_font_bytes_admitted(
    text: &str,
    font_style: &BTreeMap<String, String>,
    viewport: ViewportState,
    fonts: &[(String, Vec<u8>, String, bool, bool)],
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    admission: Arc<dyn crate::cssom::native_import_graph::ImportAdmission + Send + Sync>,
) -> Result<CanvasTextShape, LayoutError> {
    shape_font_bytes_admitted_with_language(text, font_style, viewport, fonts, cancelled, admission, None)
}
pub fn shape_font_bytes_admitted_with_language(
    text: &str, font_style: &BTreeMap<String, String>, viewport: ViewportState,
    fonts: &[(String, Vec<u8>, String, bool, bool)], cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    admission: Arc<dyn crate::cssom::native_import_graph::ImportAdmission + Send + Sync>, language: Option<&str>,
) -> Result<CanvasTextShape, LayoutError> {
    shape_with_font_byte_admission(text, font_style, viewport, fonts, cancelled, Some(font_bytes::Owner::new(admission)), language)
}
fn shape_with_font_byte_admission(
    text: &str,
    font_style: &BTreeMap<String, String>,
    viewport: ViewportState,
    fonts: &[(String, Vec<u8>, String, bool, bool)],
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    font_byte_owner: Option<font_bytes::Owner>,
    language: Option<&str>,
) -> Result<CanvasTextShape, LayoutError> {
    super::layout_checkpoint(&cancelled)?;
    let admitted_text = font_byte_owner.as_ref().map(|owner| owner.normalize_text(text, &cancelled))
        .transpose()?;
    // Canvas replaces each ASCII whitespace character independently. It does
    // not collapse runs, trim edges or combine CRLF into one space.
    let mut normalized = String::new();
    normalized
        .try_reserve_exact(if admitted_text.is_some() { 0 } else { text.len() })
        .map_err(|_| LayoutError::InvalidRenderedTree("canvas text allocation failed".into()))?;
    if admitted_text.is_none() {
    for (index, ch) in text.chars().enumerate() {
        if index % 1024 == 0 {
            super::layout_checkpoint(&cancelled)?;
        }
        normalized.push(if matches!(ch, '\t' | '\n' | '\u{c}' | '\r' | ' ') {
            ' '
        } else {
            ch
        });
    }
    }
    let text = admitted_text.as_ref().map_or(normalized.as_str(), |owned| owned.value.as_str());
    super::layout_checkpoint(&cancelled)?;
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
    let mut engine = TextEngine::new_with_font_byte_admission(fonts, font_byte_owner)?;
    engine.cancelled = cancelled.clone();
    // Synthetic Canvas text has native provenance owner zero. Locale reaches
    // Parley's existing ranged shaping path, not a width postprocessor.
    engine.languages.insert(0, language.and_then(|language| fontique::Language::parse(language).ok()));
    super::layout_checkpoint(&cancelled)?;
    let first_available_font = font_selection::select(&mut engine, &style, font_size)?;
    let spacing_metrics = first_available_font.spacing_metrics()?;
    for property in ["letter-spacing", "word-spacing"] {
        if let Some(value) = style.get(property).filter(|value| value.as_str() != "normal") {
            let resolved = oryn_common::css_math::resolve_length_with_font_metrics(value,
                oryn_common::css_math::LengthContext { percentage_basis: font_size, font_size,
                    root_font_size: 16., viewport_width: viewport.width, viewport_height: viewport.height }, Some(spacing_metrics))
                .ok_or_else(|| LayoutError::InvalidRenderedTree("unresolved Canvas spacing length".into()))?;
            style.insert(property.into(), format!("{resolved}px"));
        }
    }
    super::layout_checkpoint(&cancelled)?;
    let shaped = engine.shape_styled(text, 1_000_000_000., font_size, &style, viewport);
    super::layout_checkpoint(&cancelled)?;
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
    super::layout_checkpoint(&cancelled)?;
    Ok(CanvasTextShape {
        width: shaped.width,
        baseline: shaped.baselines.first().copied().unwrap_or(0.),
        glyph_runs: shaped.glyph_runs.into_iter().flatten().collect(),
        fonts: engine.paint_fonts,
        neutral_font_metrics,
        font_size,
        neutral_font_run: neutral.glyph_runs.into_iter().flatten().next(),
        first_available_font,
        cancellation: Cancellation(cancelled),
        _font_byte_owner: engine.canvas_font_bytes,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn canvas_font_metric_spacing_uses_selected_face_and_nested_math() {
        let shape = |spacing: &str, size: &str| super::shape("abc", &std::collections::BTreeMap::from([
            ("font-size".into(), size.into()), ("letter-spacing".into(), spacing.into()),
        ]), oryn_common::v4::ViewportState::default(), &[]).unwrap();
        for size in ["10px", "20px"] {
            let base = shape("0px", size);
            let metrics = base.first_available_font.spacing_metrics().unwrap();
            for (unit, value) in [("ex", metrics.x_height), ("ch", metrics.zero_advance), ("ic", metrics.ideographic_advance), ("cap", metrics.cap_height)] {
                let spaced = shape(&format!("1{unit}"), size);
                assert!((spaced.width - base.width - value * 3.).abs() < 0.01, "{size} {unit}");
                assert_ne!(spaced.glyph_runs, base.glyph_runs);
            }
            let calc = shape("calc(2ex + min(1ch, 3px))", size);
            assert!((calc.width - base.width - (2. * metrics.x_height + metrics.zero_advance.min(3.)) * 3.).abs() < 0.01);
        }
        let cancelled = std::sync::Arc::new(|| true);
        assert!(matches!(super::shape_controlled("abc", &std::collections::BTreeMap::from([("letter-spacing".into(), "1ex".into())]), oryn_common::v4::ViewportState::default(), &[], Some(cancelled)), Err(super::LayoutError::Cancelled)));
    }
    #[test]
    fn canvas_spacing_uses_native_glyph_advances_and_current_font_size() {
        let measure = |text: &str, font: &str, letter: &str, word: &str| {
            super::shape(text, &std::collections::BTreeMap::from([
                ("font-size".into(), font.into()),
                ("letter-spacing".into(), letter.into()),
                ("word-spacing".into(), word.into()),
            ]), oryn_common::v4::ViewportState::default(), &[]).unwrap()
        };
        for font in ["10px", "20px"] {
            let base = measure("Hello World", font, "0px", "0px");
            for (value, delta) in [("3px", 33.), ("-2px", -22.), ("1in", 1056.)] {
                let spaced = measure("Hello World", font, value, "0px");
                assert!((spaced.width - base.width - delta).abs() < 0.01, "{font} {value}: {} vs {}", spaced.width, base.width);
                assert_ne!(spaced.glyph_runs, base.glyph_runs, "spacing must move real glyphs");
            }
            let size = if font == "10px" { 10. } else { 20. };
            assert!((measure("Hello World", font, "1em", "0px").width - base.width - size * 11.).abs() < 0.01);
            let base = measure("Hello World, again", font, "0px", "0px");
            assert!((measure("Hello World, again", font, "0px", "1em").width - base.width - size * 2.).abs() < 0.01);
            assert!((measure("a\r\nb", font, "0px", "3px").width - measure("a\r\nb", font, "0px", "0px").width - 6.).abs() < 0.01);
            assert_eq!(measure("", font, "3px", "5px").width, 0.);
        }
    }
    use super::*;
    #[test]
    fn canvas_concrete_font_copies_retain_charges_and_reject_each_denial() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Charge { live: Arc<AtomicUsize>, bytes: usize }
        impl Drop for Charge { fn drop(&mut self) { self.live.fetch_sub(self.bytes, Ordering::SeqCst); } }
        struct Admission { live: Arc<AtomicUsize>, calls: AtomicUsize, deny: usize }
        impl crate::cssom::native_import_graph::ImportAdmission for Admission {
            fn checkpoint(&self) -> Result<(), String> { Ok(()) }
            fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == self.deny { return Err("copy admission denied".into()); }
                self.live.fetch_add(bytes, Ordering::SeqCst);
                Ok(Arc::new(Charge { live: self.live.clone(), bytes }))
            }
        }
        let make = |deny| Arc::new(Admission { live: Arc::new(AtomicUsize::new(0)), calls: AtomicUsize::new(0), deny });
        let fonts = vec![("Captured".into(), crate::DEFAULT_FONT_BYTES.to_vec(), "font-weight:400".into(), false, false)];
        let style = BTreeMap::from([("font-family".into(), "Captured".into()), ("font-size".into(), "20px".into())]);
        let owner = make(usize::MAX);
        let shape = shape_font_bytes_admitted("Ag", &style, ViewportState::default(), &fonts, None, owner.clone()).unwrap();
        let calls = owner.calls.load(Ordering::SeqCst);
        assert!(calls >= 6, "bundled, document and exported face copies each need metadata and byte admission");
        assert!(owner.live.load(Ordering::SeqCst) >= crate::DEFAULT_FONT_BYTES.len() * 3);
        assert!(shape.width > 0.);
        drop(shape);
        assert_eq!(owner.live.load(Ordering::SeqCst), 0);
        for denial in 0..calls {
            let owner = make(denial);
            assert!(shape_font_bytes_admitted("Ag", &style, ViewportState::default(), &fonts, None, owner.clone()).is_err(), "denial {denial}");
            assert_eq!(owner.live.load(Ordering::SeqCst), 0, "denial {denial} leaked a charge");
        }
    }
    #[test]
    fn native_canvas_zero_size_font_has_zero_metrics_and_no_ink() {
        let style = BTreeMap::from([
            ("font-size".into(), "0px".into()),
            ("font-family".into(), "sans-serif".into()),
        ]);
        let shaped = shape("Ag", &style, ViewportState::default(), &[]).unwrap();
        assert_eq!(shaped.font_size, 0.);
        assert_eq!(shaped.width, 0.);
        assert_eq!(shaped.ink_bounds().unwrap(), None);
        struct NoOutline;
        impl outlines::OutlinePen for NoOutline {
            fn move_to(&mut self, _: f32, _: f32) {
                panic!("zero-size outline");
            }
            fn line_to(&mut self, _: f32, _: f32) {
                panic!("zero-size outline");
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
                panic!("zero-size outline");
            }
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
                panic!("zero-size outline");
            }
            fn close(&mut self) {
                panic!("zero-size outline");
            }
        }
        for (run_index, run) in shaped.glyph_runs.iter().enumerate() {
            for glyph_index in 0..run.glyphs.len() {
                shaped
                    .draw_glyph_outline(run_index, glyph_index, &mut NoOutline)
                    .unwrap();
            }
        }
        use metrics::Baseline::*;
        for baseline in [Top, Hanging, Middle, Alphabetic, Ideographic, Bottom] {
            let metrics = shaped
                .measure(skrifa::raw::types::Tag::new(b"latn"), baseline, 0.)
                .unwrap();
            assert_eq!(
                [
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
                    metrics.ideographic_baseline
                ],
                [0.; 12]
            );
        }
    }
    #[test]
    fn canvas_text_cancellation_stops_input_and_retained_measurement() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let cancelled = Arc::new(AtomicBool::new(true));
        let flag = cancelled.clone();
        let callback: Arc<dyn Fn() -> bool + Send + Sync> =
            Arc::new(move || flag.load(Ordering::Relaxed));
        assert!(matches!(
            shape_controlled(
                "Ag",
                &BTreeMap::new(),
                ViewportState::default(),
                &[],
                Some(callback.clone())
            ),
            Err(LayoutError::Cancelled)
        ));
        cancelled.store(false, Ordering::Relaxed);
        let shaped = shape_controlled(
            "Ag",
            &BTreeMap::new(),
            ViewportState::default(),
            &[],
            Some(callback),
        )
        .unwrap();
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(shaped.ink_bounds().unwrap_err(), LayoutError::Cancelled);
        assert_eq!(shaped.font_metrics().unwrap_err(), LayoutError::Cancelled);
        assert_eq!(
            shaped
                .measure(
                    skrifa::raw::types::Tag::new(b"latn"),
                    metrics::Baseline::Alphabetic,
                    0.
                )
                .unwrap_err(),
            LayoutError::Cancelled
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let callback: Arc<dyn Fn() -> bool + Send + Sync> =
            Arc::new(move || count.fetch_add(1, Ordering::Relaxed) >= 3);
        assert!(matches!(
            shape_controlled(
                &"a".repeat(8192),
                &BTreeMap::new(),
                ViewportState::default(),
                &[],
                Some(callback)
            ),
            Err(LayoutError::Cancelled)
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }
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
        let script = skrifa::raw::types::Tag::new(b"latn");
        let measured = single
            .measure(script, metrics::Baseline::Alphabetic, 0.)
            .unwrap();
        assert_eq!(measured.width, single.width);
        assert!((measured.em_height_ascent + measured.em_height_descent - 20.).abs() < 1e-9);
        let top = single.measure(script, metrics::Baseline::Top, 0.).unwrap();
        assert_eq!(top.em_height_ascent, 0.);
        let middle = single
            .measure(script, metrics::Baseline::Middle, 0.)
            .unwrap();
        assert!((middle.em_height_ascent - 10.).abs() < 1e-9);
        assert!((middle.em_height_descent - 10.).abs() < 1e-9);
        let empty_metrics = empty
            .measure(script, metrics::Baseline::Alphabetic, 0.)
            .unwrap();
        assert_eq!(empty_metrics.actual_bounding_box_ascent, 0.);
        assert!(empty_metrics.font_bounding_box_ascent > 0.);
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
