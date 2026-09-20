//! Outline rasterization for shaped variation instances and synthetic faces.
use crate::RenderError;
use oryn_common::v4::{PaintFontResource, PaintGlyphRun};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
};

#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}
impl Point {
    fn mid(self, other: Self) -> Self {
        Self {
            x: (self.x + other.x) * 0.5,
            y: (self.y + other.y) * 0.5,
        }
    }
}
fn distance(point: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let norm = dx * dx + dy * dy;
    let t = if norm == 0. {
        0.
    } else {
        ((point.x - a.x) * dx + (point.y - a.y) * dy) / norm
    }
    .clamp(0., 1.);
    (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy)
}
#[derive(Default)]
struct Pen {
    edges: Vec<(Point, Point)>,
    current: Point,
    start: Point,
    skew: f64,
    failed: bool,
}
impl Pen {
    fn point(&self, x: f32, y: f32) -> Point {
        Point {
            x: x as f64 + self.skew * y as f64,
            y: y as f64,
        }
    }
    fn edge(&mut self, a: Point, b: Point) {
        if self.failed {
            return;
        }
        if ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite()) || self.edges.len() >= 65536 {
            self.failed = true;
            return;
        }
        if a.x != b.x || a.y != b.y {
            self.edges.push((a, b));
        }
    }
    fn quad(&mut self, a: Point, b: Point, c: Point, depth: u8) {
        if self.failed {
            return;
        }
        if distance(b, a, c) <= 1. / 16. {
            self.edge(a, c);
            return;
        }
        if depth == 20 {
            self.failed = true;
            return;
        }
        let ab = a.mid(b);
        let bc = b.mid(c);
        let mid = ab.mid(bc);
        self.quad(a, ab, mid, depth + 1);
        self.quad(mid, bc, c, depth + 1);
    }
    fn cubic(&mut self, a: Point, b: Point, c: Point, d: Point, depth: u8) {
        if self.failed {
            return;
        }
        if distance(b, a, d).max(distance(c, a, d)) <= 1. / 16. {
            self.edge(a, d);
            return;
        }
        if depth == 20 {
            self.failed = true;
            return;
        }
        let ab = a.mid(b);
        let bc = b.mid(c);
        let cd = c.mid(d);
        let abc = ab.mid(bc);
        let bcd = bc.mid(cd);
        let mid = abc.mid(bcd);
        self.cubic(a, ab, abc, mid, depth + 1);
        self.cubic(mid, bcd, cd, d, depth + 1);
    }
    fn contains(&self, p: Point, radius: f64) -> bool {
        let mut winding = 0i32;
        for &(a, b) in &self.edges {
            if radius > 0. && distance(p, a, b) <= radius {
                return true;
            }
            let cross = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
            if a.y <= p.y {
                if b.y > p.y && cross > 0. {
                    winding += 1;
                }
            } else if b.y <= p.y && cross < 0. {
                winding -= 1;
            }
        }
        winding != 0
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.current = self.point(x, y);
        self.start = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let next = self.point(x, y);
        self.edge(self.current, next);
        self.current = next;
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let next = self.point(x, y);
        self.quad(self.current, self.point(cx, cy), next, 0);
        self.current = next;
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let next = self.point(x, y);
        self.cubic(
            self.current,
            self.point(ax, ay),
            self.point(bx, by),
            next,
            0,
        );
        self.current = next;
    }
    fn close(&mut self) {
        self.edge(self.current, self.start);
        self.current = self.start;
    }
}

pub(super) fn rasterize(
    source: &PaintFontResource,
    run: &PaintGlyphRun,
    id: u32,
    size: f32,
) -> Result<(fontdue::Metrics, Vec<u8>), RenderError> {
    if !size.is_finite()
        || size <= 0.
        || !run.skew_degrees.is_finite()
        || run.skew_degrees.abs() >= 90.
        || run
            .normalized_coords
            .iter()
            .any(|coord| !(-16384..=16384).contains(coord))
    {
        return Err(RenderError::Font("invalid glyph instance".into()));
    }
    let font = FontRef::from_index(&source.bytes, source.collection_index)
        .map_err(|error| RenderError::Font(error.to_string()))?;
    let outlines = font.outline_glyphs();
    let glyph = outlines
        .get(GlyphId::new(id))
        .ok_or_else(|| RenderError::Font(format!("missing outline glyph {id}")))?;
    let coords = run
        .normalized_coords
        .iter()
        .map(|coord| NormalizedCoord::from_bits(*coord))
        .collect::<Vec<_>>();
    let mut pen = Pen {
        skew: (run.skew_degrees as f64).to_radians().tan(),
        ..Default::default()
    };
    glyph
        .draw(
            DrawSettings::unhinted(Size::new(size), LocationRef::new(&coords)),
            &mut pen,
        )
        .map_err(|error| RenderError::Font(error.to_string()))?;
    if pen.failed {
        return Err(RenderError::Font(
            "glyph outline complexity limit exceeded".into(),
        ));
    }
    rasterize_pen(&pen, if run.embolden { size as f64 / 48. } else { 0. })
}

fn rasterize_pen(pen: &Pen, radius: f64) -> Result<(fontdue::Metrics, Vec<u8>), RenderError> {
    if pen.edges.is_empty() {
        return Ok((fontdue::Metrics::default(), Vec::new()));
    }
    let mut min = Point {
        x: f64::INFINITY,
        y: f64::INFINITY,
    };
    let mut max = Point {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
    };
    for &(a, b) in &pen.edges {
        for p in [a, b] {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
    }
    let left = (min.x - radius).floor();
    let bottom = (min.y - radius).floor();
    let right = (max.x + radius).ceil();
    let top = (max.y + radius).ceil();
    if ![left, bottom, right, top]
        .iter()
        .all(|v| v.is_finite() && *v >= i32::MIN as f64 && *v <= i32::MAX as f64)
    {
        return Err(RenderError::Font(
            "glyph bounds outside raster coordinate range".into(),
        ));
    }
    let width = (right - left) as usize;
    let height = (top - bottom) as usize;
    let count = width
        .checked_mul(height)
        .filter(|count| *count <= 16 * 1024 * 1024)
        .ok_or_else(|| RenderError::Font("glyph bitmap exceeds 16 MiB".into()))?;
    let mut bitmap = vec![0; count];
    for y in 0..height {
        for x in 0..width {
            let mut coverage = 0u16;
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = Point {
                        x: left + x as f64 + (sx as f64 + 0.5) / 4.,
                        y: top - y as f64 - (sy as f64 + 0.5) / 4.,
                    };
                    coverage += u16::from(pen.contains(p, radius));
                }
            }
            bitmap[y * width + x] = ((coverage * 255 + 8) / 16) as u8;
        }
    }
    Ok((
        fontdue::Metrics {
            xmin: left as i32,
            ymin: bottom as i32,
            width,
            height,
            ..Default::default()
        },
        bitmap,
    ))
}

/// Preserve native quadratic/cubic outline segments for vector print. Shaping,
/// glyph selection, variation coordinates and synthesis come from the captured
/// run, so PDF generation never re-shapes the fragment string.
pub(super) fn svg_path(
    source: &PaintFontResource,
    run: &PaintGlyphRun,
    id: u32,
) -> Result<String, RenderError> {
    struct VectorPen {
        path: String,
        count: usize,
        invalid: bool,
        skew: f64,
    }
    impl VectorPen {
        fn point(&mut self, x: f32, y: f32) -> (f64, f64) {
            let point = (f64::from(x) + self.skew * f64::from(y), f64::from(y));
            self.count += 1;
            if self.count > 65536 || !point.0.is_finite() || !point.1.is_finite() {
                self.invalid = true;
            }
            point
        }
    }
    impl OutlinePen for VectorPen {
        fn move_to(&mut self, x: f32, y: f32) {
            if self.invalid {
                return;
            }
            let (x, y) = self.point(x, y);
            self.path.push_str(&format!("M{x:.8} {y:.8}"));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            if self.invalid {
                return;
            }
            let (x, y) = self.point(x, y);
            self.path.push_str(&format!("L{x:.8} {y:.8}"));
        }
        fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
            if self.invalid {
                return;
            }
            let (a, b) = self.point(a, b);
            let (x, y) = self.point(x, y);
            self.path.push_str(&format!("Q{a:.8} {b:.8} {x:.8} {y:.8}"));
        }
        fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
            if self.invalid {
                return;
            }
            let (a, b) = self.point(a, b);
            let (c, d) = self.point(c, d);
            let (x, y) = self.point(x, y);
            self.path
                .push_str(&format!("C{a:.8} {b:.8} {c:.8} {d:.8} {x:.8} {y:.8}"));
        }
        fn close(&mut self) {
            if !self.invalid {
                self.path.push('Z');
            }
        }
    }
    if !run.font_size.is_finite()
        || run.font_size <= 0.
        || run.font_size > 1_000_000.
        || !run.skew_degrees.is_finite()
        || run.skew_degrees.abs() >= 90.
        || run
            .normalized_coords
            .iter()
            .any(|coord| !(-16384..=16384).contains(coord))
    {
        return Err(RenderError::Font("invalid vector glyph instance".into()));
    }
    let font = FontRef::from_index(&source.bytes, source.collection_index)
        .map_err(|e| RenderError::Font(e.to_string()))?;
    let outlines = font.outline_glyphs();
    let glyph = outlines
        .get(GlyphId::new(id))
        .ok_or_else(|| RenderError::Font(format!("missing outline glyph {id}")))?;
    let coords = run
        .normalized_coords
        .iter()
        .map(|coord| NormalizedCoord::from_bits(*coord))
        .collect::<Vec<_>>();
    let mut pen = VectorPen {
        path: String::new(),
        count: 0,
        invalid: false,
        skew: f64::from(run.skew_degrees).to_radians().tan(),
    };
    glyph
        .draw(
            DrawSettings::unhinted(Size::new(run.font_size as f32), LocationRef::new(&coords)),
            &mut pen,
        )
        .map_err(|e| RenderError::Font(e.to_string()))?;
    if pen.invalid || pen.path.len() > 4 * 1024 * 1024 {
        return Err(RenderError::Font(
            "vector glyph complexity exceeds bounds".into(),
        ));
    }
    Ok(pen.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run() -> PaintGlyphRun {
        PaintGlyphRun {
            line_box: None,
            text_box: None,
            background: None,
            decoration_rect: None,
            scale_x: 1.,
            vertical: false,
            decorations: Vec::new(),
            orientation: oryn_common::v4::PaintGlyphOrientation::Upright,
            font_key: "fixture".into(),
            font_size: 64.,
            color: None,
            glyphs: vec![],
            embolden: false,
            skew_degrees: 0.,
            normalized_coords: vec![],
        }
    }
    #[test]
    fn vector_outlines_preserve_curves_and_native_variation_instance() {
        let source = PaintFontResource {
            bytes: include_bytes!("../tests/assets/variabletest_matching.ttf").to_vec(),
            collection_index: 0,
        };
        let font = FontRef::from_index(&source.bytes, 0).unwrap();
        let glyph = font.charmap().map('P').unwrap().to_u32();
        let mut instance = run();
        instance.normalized_coords = font
            .axes()
            .location(&[("wght", 900.)])
            .coords()
            .iter()
            .map(|c| c.to_bits())
            .collect();
        let native = svg_path(&source, &instance, glyph).unwrap();
        let reference =
            svg_path(&source, &run(), font.charmap().map(',').unwrap().to_u32()).unwrap();
        // Outline pens preserve the font's contour order. Equivalent contours
        // may start at different vertices or reverse their winding; compare the
        // actual vector geometry through an independent SVG renderer instead.
        fn coverage(path: &str, scale: u32) -> Vec<u8> {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"80\"><path d=\"{path}\" fill=\"black\" transform=\"translate(2 70) scale(1 -1)\"/></svg>"
            );
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
            let mut pixmap = resvg::tiny_skia::Pixmap::new(128 * scale, 80 * scale).unwrap();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
                &mut pixmap.as_mut(),
            );
            pixmap.data().to_vec()
        }
        let mut light = run();
        light.normalized_coords = font
            .axes()
            .location(&[("wght", 100.)])
            .coords()
            .iter()
            .map(|c| c.to_bits())
            .collect();
        let light_path = svg_path(&source, &light, glyph).unwrap();
        let light_reference =
            svg_path(&source, &run(), font.charmap().map('/').unwrap().to_u32()).unwrap();
        for scale in [1, 3] {
            let actual = coverage(&native, scale);
            assert!(actual.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
            assert_eq!(actual, coverage(&reference, scale));
            assert_eq!(
                coverage(&light_path, scale),
                coverage(&light_reference, scale)
            );
            assert_ne!(
                actual,
                coverage(&light_path, scale),
                "variation must change vector geometry"
            );
        }
        let curved = PaintFontResource {
            bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
            collection_index: 0,
        };
        let curved_font = FontRef::from_index(&curved.bytes, 0).unwrap();
        let curve_path = svg_path(
            &curved,
            &run(),
            curved_font.charmap().map('S').unwrap().to_u32(),
        )
        .unwrap();
        assert!(
            curve_path.contains('Q') || curve_path.contains('C'),
            "native curve segments must remain vector curves"
        );
        assert!(
            coverage(&curve_path, 1)
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
        instance.font_size = f64::MAX;
        assert!(svg_path(&source, &instance, glyph).is_err());
    }
    #[test]
    fn variable_outline_matches_static_reference_glyphs_at_weight_extremes() {
        let source = PaintFontResource {
            bytes: include_bytes!("../tests/assets/variabletest_matching.ttf").to_vec(),
            collection_index: 0,
        };
        let font = FontRef::from_index(&source.bytes, 0).unwrap();
        let variable = font.charmap().map('P').unwrap().to_u32();
        for (weight, reference) in [(100., '/'), (900., ',')] {
            let mut instance = run();
            instance.normalized_coords = font
                .axes()
                .location(&[("wght", weight)])
                .coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect();
            let (actual, a) = rasterize(&source, &instance, variable, 64.).unwrap();
            let (expected, b) = rasterize(
                &source,
                &run(),
                font.charmap().map(reference).unwrap().to_u32(),
                64.,
            )
            .unwrap();
            assert_eq!(
                (actual.xmin, actual.ymin, actual.width, actual.height),
                (
                    expected.xmin,
                    expected.ymin,
                    expected.width,
                    expected.height
                )
            );
            assert_eq!(a, b, "weight {weight}");
        }
    }
    #[test]
    fn synthetic_faces_change_real_outline_ink_and_validate_instances() {
        let source = PaintFontResource {
            bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
            collection_index: 0,
        };
        let font = FontRef::from_index(&source.bytes, 0).unwrap();
        let id = font.charmap().map('A').unwrap().to_u32();
        let (_, normal) = rasterize(&source, &run(), id, 32.).unwrap();
        let mut bold = run();
        bold.embolden = true;
        let (_, heavy) = rasterize(&source, &bold, id, 32.).unwrap();
        assert!(
            heavy.iter().map(|v| *v as u64).sum::<u64>()
                > normal.iter().map(|v| *v as u64).sum::<u64>()
        );
        let mut italic = run();
        italic.skew_degrees = 14.;
        assert_ne!(rasterize(&source, &italic, id, 32.).unwrap().1, normal);
        italic.skew_degrees = f32::NAN;
        assert!(rasterize(&source, &italic, id, 32.).is_err());
    }

    #[test]
    fn winding_preserves_counters_and_emboldening_expands_ink() {
        let mut pen = Pen::default();
        pen.move_to(0., 0.);
        pen.line_to(8., 0.);
        pen.line_to(8., 8.);
        pen.line_to(0., 8.);
        pen.close();
        pen.move_to(2., 2.);
        pen.line_to(2., 6.);
        pen.line_to(6., 6.);
        pen.line_to(6., 2.);
        pen.close();
        let (normal, bitmap) = rasterize_pen(&pen, 0.).unwrap();
        assert_eq!((normal.width, normal.height), (8, 8));
        assert_eq!(bitmap[0], 255);
        assert_eq!(bitmap[3 * 8 + 3], 0);
        let (bold, heavy) = rasterize_pen(&pen, 0.5).unwrap();
        assert!(bold.width > normal.width);
        assert!(
            heavy.iter().map(|v| *v as u64).sum::<u64>()
                > bitmap.iter().map(|v| *v as u64).sum::<u64>()
        );
    }
}
