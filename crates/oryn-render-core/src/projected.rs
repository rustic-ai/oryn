//! Inverse-mapped paint on the source z=0 plane. Sampling tests homogeneous w
//! before shading, so boxes crossing the perspective plane retain visible parts.
use super::*;

fn oriented_glyph_bounds(
    rect: CssRect,
    origin: (f64, f64),
    orientation: oryn_common::v4::PaintGlyphOrientation,
) -> CssRect {
    use oryn_common::v4::PaintGlyphOrientation::*;
    match orientation {
        Upright => CssRect {
            x: origin.0 + rect.x,
            y: origin.1 + rect.y,
            ..rect
        },
        Clockwise => CssRect {
            x: origin.0 - rect.bottom(),
            y: origin.1 + rect.x,
            width: rect.height,
            height: rect.width,
        },
        CounterClockwise => CssRect {
            x: origin.0 + rect.y,
            y: origin.1 - rect.right(),
            width: rect.height,
            height: rect.width,
        },
    }
}
fn glyph_source_point(
    x: f64,
    y: f64,
    orientation: oryn_common::v4::PaintGlyphOrientation,
) -> (f64, f64) {
    use oryn_common::v4::PaintGlyphOrientation::*;
    match orientation {
        Upright => (x, y),
        Clockwise => (y, -x),
        CounterClockwise => (-y, x),
    }
}

struct Mapping {
    forward: [f64; 9],
    inverse: [f64; 9],
}
pub(super) fn density_for_transform(matrix: [f64; 16], output: CssRect) -> Option<(f64, f64)> {
    let density = Mapping::new(matrix)?.density_bound(output);
    (density.0.is_finite() && density.1.is_finite() && density.0 > 0. && density.1 > 0.)
        .then_some(density)
}
impl Mapping {
    fn new(m: [f64; 16]) -> Option<Self> {
        let h = [m[0], m[4], m[12], m[1], m[5], m[13], m[3], m[7], m[15]];
        if !h.iter().all(|v| v.is_finite()) {
            return None;
        }
        let magnitude = h
            .iter()
            .fold(0.0_f64, |maximum, value| maximum.max(value.abs()));
        if magnitude == 0. {
            return None;
        }
        // Positive homogeneous normalization preserves the camera half-plane.
        let h = h.map(|v| v / magnitude);
        let mut rows = [[0.; 6]; 3];
        for row in 0..3 {
            for column in 0..3 {
                rows[row][column] = h[row * 3 + column];
            }
            rows[row][row + 3] = 1.;
        }
        for column in 0..3 {
            let pivot = (column..3).max_by(|left, right| {
                rows[*left][column]
                    .abs()
                    .total_cmp(&rows[*right][column].abs())
            })?;
            rows.swap(column, pivot);
            let divisor = rows[column][column];
            if divisor == 0. {
                return None;
            }
            for entry in &mut rows[column] {
                *entry /= divisor;
            }
            let pivot_row = rows[column];
            for row in 0..3 {
                if row == column {
                    continue;
                }
                let factor = rows[row][column];
                for entry in 0..6 {
                    rows[row][entry] -= factor * pivot_row[entry];
                }
            }
        }
        let inverse = std::array::from_fn(|index| rows[index / 3][index % 3 + 3]);
        inverse.iter().all(|v| v.is_finite()).then_some(Self {
            forward: h,
            inverse,
        })
    }

    fn source(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let m = self.inverse;
        let w = m[6] * x + m[7] * y + m[8];
        if w == 0. {
            return None;
        }
        let (x, y) = (
            (m[0] * x + m[1] * y + m[2]) / w,
            (m[3] * x + m[4] * y + m[5]) / w,
        );
        let forward_w = self.forward[6] * x + self.forward[7] * y + self.forward[8];
        (x.is_finite() && y.is_finite() && forward_w > 0.).then_some((x, y))
    }
    fn scale_at(&self, x: f64, y: f64) -> (f64, f64) {
        let m = self.forward;
        let w = m[6] * x + m[7] * y + m[8];
        let px = (m[0] * x + m[1] * y + m[2]) / w;
        let py = (m[3] * x + m[4] * y + m[5]) / w;
        (
            ((m[0] - px * m[6]) / w).hypot((m[3] - py * m[6]) / w),
            ((m[1] - px * m[7]) / w).hypot((m[4] - py * m[7]) / w),
        )
    }
    /// Conservative Jacobian bounds over a finite output rectangle. Expressing
    /// the forward derivative in destination coordinates avoids sampling past
    /// a camera horizon or missing a peak between a small set of samples.
    fn density_bound(&self, rect: CssRect) -> (f64, f64) {
        let m = self.forward;
        let q = self.inverse;
        let factor = [
            (rect.x, rect.y),
            (rect.right(), rect.y),
            (rect.x, rect.bottom()),
            (rect.right(), rect.bottom()),
        ]
        .into_iter()
        .map(|(x, y)| (q[6] * x + q[7] * y + q[8]).abs())
        .fold(0f64, f64::max);
        let bound = |a: f64, b: f64, coefficient: f64| {
            let x = (a - rect.x * coefficient)
                .abs()
                .max((a - rect.right() * coefficient).abs());
            let y = (b - rect.y * coefficient)
                .abs()
                .max((b - rect.bottom() * coefficient).abs());
            factor * x.hypot(y)
        };
        (bound(m[0], m[3], m[6]), bound(m[1], m[4], m[7]))
    }
    fn bounds(&self, rect: CssRect, scale: f64, width: u32, height: u32) -> (u32, u32, u32, u32) {
        if !scale.is_finite() || scale <= 0. {
            return (0, 0, 0, 0);
        }
        let m = self.forward;
        let mut polygon = [
            (rect.x, rect.y),
            (rect.right(), rect.y),
            (rect.right(), rect.bottom()),
            (rect.x, rect.bottom()),
        ]
        .into_iter()
        .map(|(x, y)| {
            [
                m[0] * x + m[1] * y + m[2],
                m[3] * x + m[4] * y + m[5],
                m[6] * x + m[7] * y + m[8],
            ]
        })
        .collect::<Vec<_>>();
        if polygon.iter().flatten().any(|value| !value.is_finite()) {
            return (0, 0, width, height);
        }
        // Clip before dividing by w. The finite target viewport closes every
        // visible ray crossing the camera plane, without an arbitrary epsilon
        // or an enormous allocation based on nearly infinite projected corners.
        for plane in [
            [0., 0., 1.],
            [1., 0., 0.],
            [-1., 0., width as f64 / scale],
            [0., 1., 0.],
            [0., -1., height as f64 / scale],
        ] {
            if polygon.is_empty() {
                return (0, 0, 0, 0);
            }
            let distance = |p: [f64; 3]| plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2];
            let mut output = Vec::new();
            let mut previous = *polygon.last().unwrap();
            let mut before = distance(previous);
            for current in polygon {
                let after = distance(current);
                if (before >= 0.) != (after >= 0.) {
                    let magnitude = before.abs().max(after.abs());
                    let a = before / magnitude;
                    let b = after / magnitude;
                    let t = a / (a - b);
                    output.push(std::array::from_fn(|i| {
                        previous[i] + t * (current[i] - previous[i])
                    }));
                }
                if after >= 0. {
                    output.push(current);
                }
                previous = current;
                before = after;
            }
            polygon = output;
        }
        let mut bounds = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for point in polygon {
            if point[2] <= 0. {
                continue;
            }
            let (x, y) = (point[0] / point[2], point[1] / point[2]);
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
        if !bounds.0.is_finite() {
            return (0, 0, 0, 0);
        }
        pixel_bounds(
            CssRect {
                x: bounds.0,
                y: bounds.1,
                width: bounds.2 - bounds.0,
                height: bounds.3 - bounds.1,
            },
            scale,
            width,
            height,
        )
    }
}

struct Clip {
    mapping: Mapping,
    rect: CssRect,
    x: bool,
    y: bool,
    shape: Option<oryn_common::basic_shape::Shape>,
}
impl Clip {
    fn contains(&self, x: f64, y: f64) -> bool {
        let Some((x, y)) = self.mapping.source(x, y) else {
            return false;
        };
        self.shape.as_ref().is_none_or(|shape| shape.contains(x, y))
            && (!self.x || x >= self.rect.x && x < self.rect.right())
            && (!self.y || y >= self.rect.y && y < self.rect.bottom())
    }
}

fn shade(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    scale: f64,
    mapping: &Mapping,
    clips: &[Clip],
    rect: CssRect,
    sample: impl FnMut(f64, f64) -> [u8; 4],
) {
    shade_region(
        pixels,
        width,
        height,
        scale,
        mapping,
        clips,
        Some(rect),
        sample,
    );
}
fn shade_region(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    scale: f64,
    mapping: &Mapping,
    clips: &[Clip],
    rect: Option<CssRect>,
    mut sample: impl FnMut(f64, f64) -> [u8; 4],
) {
    if rect.is_some_and(|rect| rect.width <= 0. || rect.height <= 0.) {
        return;
    }
    let (x0, y0, x1, y1) = rect
        .map(|rect| mapping.bounds(rect, scale, width, height))
        .unwrap_or((0, 0, width, height));
    for y in y0..y1 {
        for x in x0..x1 {
            let mut sums = [0.; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f64 + (sx as f64 + 0.5) / 4.) / scale;
                    let py = (y as f64 + (sy as f64 + 0.5) / 4.) / scale;
                    if !clips.iter().all(|clip| clip.contains(px, py)) {
                        continue;
                    }
                    let Some((u, v)) = mapping.source(px, py) else {
                        continue;
                    };
                    if rect.is_some_and(|rect| {
                        u < rect.x || v < rect.y || u >= rect.right() || v >= rect.bottom()
                    }) {
                        continue;
                    }
                    let rgba = sample(u, v);
                    let alpha = rgba[3] as f64 / 255.;
                    for channel in 0..3 {
                        sums[channel] += rgba[channel] as f64 * alpha;
                    }
                    sums[3] += alpha;
                }
            }
            if sums[3] > 0. {
                let rgba = [
                    (sums[0] / sums[3]).round() as u8,
                    (sums[1] / sums[3]).round() as u8,
                    (sums[2] / sums[3]).round() as u8,
                    (sums[3] * 255. / 16.).round() as u8,
                ];
                blend(pixels, ((y * width + x) * 4) as usize, rgba);
            }
        }
    }
}

#[cfg(test)]
fn sample_image(image: &image::RgbaImage, x: f64, y: f64) -> [u8; 4] {
    sample_image_mode(image, x, y, ImageRendering::Auto, (1., 1.))
}

pub(super) fn sample_image_mode(
    image: &image::RgbaImage,
    x: f64,
    y: f64,
    rendering: ImageRendering,
    scale: (f64, f64),
) -> [u8; 4] {
    if image.width() == 0 || image.height() == 0 {
        return [0; 4];
    }
    if rendering == ImageRendering::CrispEdges {
        return image
            .get_pixel(
                x.round().clamp(0., (image.width() - 1) as f64) as u32,
                y.round().clamp(0., (image.height() - 1) as f64) as u32,
            )
            .0;
    }
    let factor = |scale: f64| {
        if rendering == ImageRendering::Pixelated && scale.is_finite() {
            scale.round().max(1.)
        } else {
            1.
        }
    };
    let (kx, ky) = (factor(scale.0), factor(scale.1));
    // Bilinear samples of a virtual integer nearest-neighbor enlargement.
    // Source indices are computed directly, avoiding an intermediate bitmap.
    let x = ((x + 0.5) * kx - 0.5).clamp(0., image.width() as f64 * kx - 1.);
    let y = ((y + 0.5) * ky - 0.5).clamp(0., image.height() as f64 * ky - 1.);
    let (vx, vy) = (x.floor(), y.floor());
    let (fx, fy) = (x - vx, y - vy);
    let (x0, y0) = ((vx / kx).floor() as u32, (vy / ky).floor() as u32);
    let x1 = (((vx + 1.) / kx).floor() as u32).min(image.width() - 1);
    let y1 = (((vy + 1.) / ky).floor() as u32).min(image.height() - 1);
    let mut sums = [0.; 4];
    for (ix, iy, weight) in [
        (x0, y0, (1. - fx) * (1. - fy)),
        (x1, y0, fx * (1. - fy)),
        (x0, y1, (1. - fx) * fy),
        (x1, y1, fx * fy),
    ] {
        let rgba = image.get_pixel(ix, iy).0;
        let alpha = rgba[3] as f64 / 255. * weight;
        for channel in 0..3 {
            sums[channel] += rgba[channel] as f64 * alpha;
        }
        sums[3] += alpha;
    }
    if sums[3] == 0. {
        return [0; 4];
    }
    [
        (sums[0] / sums[3]).round() as u8,
        (sums[1] / sums[3]).round() as u8,
        (sums[2] / sums[3]).round() as u8,
        (sums[3] * 255.).round() as u8,
    ]
}

#[derive(Default)]
pub(super) struct RasterCache {
    images: std::collections::BTreeMap<usize, std::sync::Arc<image::RgbaImage>>,
    image_bytes: usize,
    shadows: std::collections::BTreeMap<Vec<u64>, std::sync::Arc<shadows::ShadowBitmap>>,
    shadow_bytes: usize,
    svg_trees: std::collections::BTreeMap<(usize, [u64; 5]), std::sync::Arc<resvg::usvg::Tree>>,
    svg_images:
        std::collections::BTreeMap<(usize, u32, u32, [u64; 5]), std::sync::Arc<image::RgbaImage>>,
    svg_bytes: usize,
    glyphs: std::collections::BTreeMap<(char, u32), std::sync::Arc<(fontdue::Metrics, Vec<u8>)>>,
    glyph_bytes: usize,
    fonts: std::collections::BTreeMap<String, std::sync::Arc<Font>>,
    indexed_glyphs: std::collections::BTreeMap<
        (String, u32, u32, bool, u32, Vec<i16>),
        std::sync::Arc<(fontdue::Metrics, Vec<u8>)>,
    >,
}
impl RasterCache {
    fn shadow(
        &mut self,
        rect: CssRect,
        radii: [f64; 8],
        shadow: &BoxShadow,
        scale: f64,
    ) -> Result<std::sync::Arc<shadows::ShadowBitmap>, RenderError> {
        let mut key = vec![
            rect.x.to_bits(),
            rect.y.to_bits(),
            rect.width.to_bits(),
            rect.height.to_bits(),
            shadow.offset[0].to_bits(),
            shadow.offset[1].to_bits(),
            shadow.blur.to_bits(),
            shadow.spread.to_bits(),
            u64::from(shadow.inset),
            scale.to_bits(),
        ];
        key.extend(radii.map(f64::to_bits));
        if let Some(mask) = self.shadows.get(&key) {
            return Ok(mask.clone());
        }
        let mask = std::sync::Arc::new(shadows::raster(rect, radii, shadow, scale)?);
        if self.shadow_bytes.saturating_add(mask.alpha.len()) > 64 * 1024 * 1024
            || self.shadows.len() >= 256
        {
            self.shadows.clear();
            self.shadow_bytes = 0;
        }
        self.shadow_bytes += mask.alpha.len();
        self.shadows.insert(key, mask.clone());
        Ok(mask)
    }
    fn shaped_font(
        &mut self,
        key: &str,
        snapshot: &PaintSnapshot,
    ) -> Result<std::sync::Arc<Font>, RenderError> {
        if let Some(font) = self.fonts.get(key) {
            return Ok(font.clone());
        }
        let source = snapshot
            .fonts
            .get(key)
            .ok_or_else(|| RenderError::Font(format!("missing shaped font {key}")))?;
        let font = std::sync::Arc::new(
            Font::from_bytes(
                source.bytes.as_slice(),
                FontSettings {
                    collection_index: source.collection_index,
                    ..FontSettings::default()
                },
            )
            .map_err(|error| RenderError::Font(error.to_string()))?,
        );
        // Keep parsing reuse bounded across many independent faces in a page.
        if self.fonts.len() >= 64 {
            self.fonts.clear();
        }
        self.fonts.insert(key.to_owned(), font.clone());
        Ok(font)
    }
    fn indexed_glyph(
        &mut self,
        snapshot: &PaintSnapshot,
        run: &oryn_common::v4::PaintGlyphRun,
        id: u32,
        size: f32,
    ) -> Result<std::sync::Arc<(fontdue::Metrics, Vec<u8>)>, RenderError> {
        if !run.scale_x.is_finite() || run.scale_x <= 0. {
            return Err(RenderError::Font("invalid glyph horizontal scale".into()));
        }
        if !size.is_finite() || size <= 0. || size > 1_000_000. {
            return Err(RenderError::Font("invalid shaped font size".into()));
        }
        let key = (
            run.font_key.clone(),
            id,
            size.to_bits(),
            run.embolden,
            run.skew_degrees.to_bits(),
            run.normalized_coords.clone(),
        );
        if let Some(glyph) = self.indexed_glyphs.get(&key) {
            return Ok(glyph.clone());
        }
        let glyph = if run.embolden || run.skew_degrees != 0. || !run.normalized_coords.is_empty() {
            let source = snapshot.fonts.get(&run.font_key).ok_or_else(|| {
                RenderError::Font(format!("missing shaped font {}", run.font_key))
            })?;
            crate::glyph_outline::rasterize(source, run, id, size)?
        } else {
            let font = self.shaped_font(&run.font_key, snapshot)?;
            let id = u16::try_from(id)
                .ok()
                .filter(|id| *id < font.glyph_count())
                .ok_or_else(|| {
                    RenderError::Font(format!("glyph {id} outside shaped font {}", run.font_key))
                })?;
            let metrics = font.metrics_indexed(id, size);
            if metrics
                .width
                .checked_mul(metrics.height)
                .is_none_or(|pixels| pixels > 16 * 1024 * 1024)
            {
                return Err(RenderError::Font(
                    "shaped glyph exceeds bitmap budget".into(),
                ));
            }
            font.rasterize_indexed(id, size)
        };
        let glyph = std::sync::Arc::new(glyph);
        let bytes = glyph.1.len();
        const LIMIT: usize = 16 * 1024 * 1024;
        if bytes <= LIMIT {
            if self.glyph_bytes.saturating_add(bytes) > LIMIT
                || self.glyphs.len() + self.indexed_glyphs.len() >= 4096
            {
                self.glyphs.clear();
                self.indexed_glyphs.clear();
                self.glyph_bytes = 0;
            }
            self.glyph_bytes += bytes;
            self.indexed_glyphs.insert(key, glyph.clone());
        }
        Ok(glyph)
    }

    fn image(
        &mut self,
        index: usize,
        source: &oryn_common::v4::PaintImageResource,
    ) -> Result<std::sync::Arc<image::RgbaImage>, RenderError> {
        if let Some(image) = self.images.get(&index) {
            return Ok(image.clone());
        }
        let image = std::sync::Arc::new(decode_image(source)?.to_rgba8());
        let bytes = image.as_raw().len();
        const LIMIT: usize = 64 * 1024 * 1024;
        if bytes <= LIMIT {
            if self.image_bytes.saturating_add(bytes) > LIMIT || self.images.len() >= 4096 {
                self.images.clear();
                self.image_bytes = 0;
            }
            self.image_bytes += bytes;
            self.images.insert(index, image.clone());
        }
        Ok(image)
    }
    fn svg(
        &mut self,
        index: usize,
        snapshot: &PaintSnapshot,
        width: u32,
        height: u32,
        environment: SvgEnvironment,
    ) -> Result<std::sync::Arc<image::RgbaImage>, RenderError> {
        let key = (index, width, height, environment.key());
        let tree_key = (index, environment.key());
        if let Some(image) = self.svg_images.get(&key) {
            return Ok(image.clone());
        }
        let tree = if let Some(tree) = self.svg_trees.get(&tree_key) {
            tree.clone()
        } else {
            let source = snapshot
                .images
                .get(index)
                .ok_or_else(|| RenderError::Encode("missing SVG resource".into()))?;
            let _svg_request = crate::svg_coverage::svg_request();
            let tree = std::sync::Arc::new(crate::svg::parse_with_environment(
                source,
                snapshot,
                environment,
            )?);
            if self.svg_trees.len() >= 32 {
                self.svg_trees.clear();
            }
            self.svg_trees.insert(tree_key, tree.clone());
            tree
        };
        let image = std::sync::Arc::new(crate::svg::raster(&tree, width, height)?);
        let bytes = image.as_raw().len();
        if self.svg_bytes.saturating_add(bytes) > 64 * 1024 * 1024 || self.svg_images.len() >= 64 {
            self.svg_images.clear();
            self.svg_bytes = 0;
        }
        self.svg_bytes += bytes;
        self.svg_images.insert(key, image.clone());
        Ok(image)
    }
    fn glyph(
        &mut self,
        font: &Font,
        character: char,
        size: f32,
    ) -> Result<std::sync::Arc<(fontdue::Metrics, Vec<u8>)>, RenderError> {
        if !size.is_finite() || size <= 0. || size > 1_000_000. {
            return Err(RenderError::Font("invalid legacy glyph size".into()));
        }
        let key = (character, size.to_bits());
        if let Some(glyph) = self.glyphs.get(&key) {
            return Ok(glyph.clone());
        }
        let metrics = font.metrics(character, size);
        if metrics
            .width
            .checked_mul(metrics.height)
            .is_none_or(|pixels| pixels > 16 * 1024 * 1024)
        {
            return Err(RenderError::Font(
                "legacy glyph exceeds bitmap budget".into(),
            ));
        }
        let glyph = std::sync::Arc::new(font.rasterize(character, size));
        let bytes = glyph.1.len();
        const LIMIT: usize = 16 * 1024 * 1024;
        if bytes <= LIMIT {
            if self.glyph_bytes.saturating_add(bytes) > LIMIT
                || self.glyphs.len() + self.indexed_glyphs.len() >= 4096
            {
                self.glyphs.clear();
                self.indexed_glyphs.clear();
                self.glyph_bytes = 0;
            }
            self.glyph_bytes += bytes;
            self.glyphs.insert(key, glyph.clone());
        }
        Ok(glyph)
    }
}

fn multiply(left: [f64; 16], right: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|index| {
        let row = index % 4;
        let column = index / 4;
        (0..4)
            .map(|k| left[k * 4 + row] * right[column * 4 + k])
            .sum()
    })
}

fn decoration_gaps(
    snapshot: &PaintSnapshot,
    fragment: &oryn_common::v4::PaintTextFragment,
    run: &oryn_common::v4::PaintGlyphRun,
    stroke: &crate::decorations::Stroke,
    scale: f64,
    cache: &mut RasterCache,
) -> Result<Vec<(f64, f64)>, RenderError> {
    if !stroke.skip_ink {
        return Ok(Vec::new());
    }
    let mut gaps = Vec::new();
    let pad = stroke.thickness.min(run.font_size / 5.);
    for positioned in &run.glyphs {
        let glyph =
            cache.indexed_glyph(snapshot, run, positioned.id, (run.font_size * scale) as f32)?;
        let (metrics, bitmap) = glyph.as_ref();
        let origin = (
            fragment.rect.x + positioned.x,
            fragment.rect.y + positioned.y,
        );
        let local = CssRect {
            x: metrics.xmin as f64 / scale * run.scale_x,
            y: -(metrics.height as f64 + metrics.ymin as f64) / scale,
            width: metrics.width as f64 / scale * run.scale_x,
            height: metrics.height as f64 / scale,
        };
        let bounds = oriented_glyph_bounds(local, origin, run.orientation);
        if bounds.right() < stroke.rect.x
            || bounds.x > stroke.rect.right()
            || bounds.bottom() < stroke.rect.y
            || bounds.y > stroke.rect.bottom()
        {
            continue;
        }
        // Aggregate only actual opaque samples intersecting the decoration band.
        // This retains counters and spaces instead of excluding each glyph box.
        let mut occupied = std::collections::BTreeSet::new();
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                if bitmap[row * metrics.width + col] < 32 {
                    continue;
                }
                let point = oriented_glyph_bounds(
                    CssRect {
                        x: local.x + col as f64 / scale * run.scale_x,
                        y: local.y + row as f64 / scale,
                        width: run.scale_x / scale,
                        height: 1. / scale,
                    },
                    origin,
                    run.orientation,
                );
                let intersects = if stroke.vertical {
                    point.right() > stroke.rect.x && point.x < stroke.rect.right()
                } else {
                    point.bottom() > stroke.rect.y && point.y < stroke.rect.bottom()
                };
                if intersects {
                    let inline = if stroke.vertical { point.y } else { point.x };
                    occupied.insert((inline * scale).floor() as i64);
                }
            }
        }
        let mut interval: Option<(i64, i64)> = None;
        for value in occupied {
            match interval {
                Some((start, end)) if value <= end + 1 => interval = Some((start, value)),
                Some((start, end)) => {
                    gaps.push((start as f64 / scale - pad, (end + 1) as f64 / scale + pad));
                    interval = Some((value, value));
                }
                None => interval = Some((value, value)),
            }
        }
        if let Some((start, end)) = interval {
            gaps.push((start as f64 / scale - pad, (end + 1) as f64 / scale + pad));
        }
    }
    gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for gap in gaps {
        if let Some(last) = merged.last_mut() {
            if gap.0 <= last.1 {
                last.1 = last.1.max(gap.1);
                continue;
            }
        }
        merged.push(gap);
    }
    Ok(merged)
}

fn text_clip_mask(
    snapshot: &PaintSnapshot,
    owner: u64,
    current: [f64; 16],
) -> Result<([f64; 16], Vec<PaintCommand>), RenderError> {
    let root = snapshot
        .nodes
        .iter()
        .find(|node| node.node_id == owner)
        .ok_or_else(|| RenderError::Encode("text clip owner missing".into()))?;
    let identity = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    let base = root
        .projection
        .as_ref()
        .map(|projection| projection.matrix)
        .unwrap_or(identity);
    let Some(base) = Mapping::new(base) else {
        return Ok((identity, Vec::new()));
    };
    let Some(current) = Mapping::new(current) else {
        return Ok((identity, Vec::new()));
    };
    let h: [f64; 9] = std::array::from_fn(|index| {
        let row = index / 3;
        let column = index % 3;
        (0..3)
            .map(|k| current.forward[row * 3 + k] * base.inverse[k * 3 + column])
            .sum()
    });
    let extra = [
        h[0], h[3], 0., h[6], h[1], h[4], 0., h[7], 0., 0., 1., 0., h[2], h[5], 0., h[8],
    ];
    let mut children =
        std::collections::BTreeMap::<u64, Vec<&oryn_common::v4::PaintNodeSnapshot>>::new();
    for node in &snapshot.nodes {
        if let Some(parent) = node.parent_id {
            children.entry(parent).or_default().push(node);
        }
    }
    let mut pending = vec![root];
    let mut visited = std::collections::BTreeSet::new();
    let mut commands = Vec::new();
    while let Some(node) = pending.pop() {
        if !visited.insert(node.node_id) || visited.len() > 100_000 {
            return Err(RenderError::Encode(
                "cyclic or oversized text clip subtree".into(),
            ));
        }
        if let Some(children) = children.get(&node.node_id) {
            pending.extend(children.iter().copied());
        }
        if node
            .style
            .get("visibility")
            .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
            || node.geometry.availability != GeometryAvailability::Available
        {
            continue;
        }
        let fragments = node
            .projection
            .as_ref()
            .map(|projection| projection.text.as_slice())
            .unwrap_or(node.text.as_slice());
        let mut glyphs = Vec::new();
        for source in fragments {
            if source.text.is_empty() {
                continue;
            }
            let mut fragment = source.clone();
            fragment.color = "white".into();
            for run in &mut fragment.glyph_runs {
                run.color = Some([255; 4]);
                run.line_box = None;
                run.text_box = None;
                run.background = None;
                run.decorations.clear();
            }
            glyphs.push(PaintCommand::Text {
                fragment,
                color: [255; 4],
            });
        }
        if glyphs.is_empty() {
            continue;
        }
        if let Some(projection) = &node.projection {
            let mut clips = projection.clips.clone();
            clips.extend(projection.text_clips.iter().cloned());
            commands.push(PaintCommand::Projected {
                matrix: projection.text_matrix.unwrap_or(projection.matrix),
                clips,
                commands: glyphs,
            });
        } else {
            commands.extend(glyphs);
        }
    }
    Ok((extra, commands))
}

enum PreparedBackground<'a> {
    Gradient(&'a BackgroundGradient),
    Solid([u8; 4]),
    Image(std::sync::Arc<image::RgbaImage>, ImageRendering),
    CrossFade(Vec<(f64, PreparedBackground<'a>)>),
}
impl<'a> PreparedBackground<'a> {
    fn new(
        source: &'a BackgroundSource,
        snapshot: &PaintSnapshot,
        tile: CssRect,
        scale: f64,
        cache: &mut RasterCache,
        budget: &mut usize,
        depth: usize,
    ) -> Result<Self, RenderError> {
        if depth > 16 {
            return Err(RenderError::Encode("cross-fade nesting exceeds 16".into()));
        }
        *budget = budget
            .saturating_add(std::mem::size_of::<(f64, Self)>() + 2 * std::mem::size_of::<usize>());
        if *budget > 64 * 1024 * 1024 {
            return Err(RenderError::Encode(
                "cross-fade tree/resource budget exceeded".into(),
            ));
        }
        Ok(match source {
            BackgroundSource::Gradient(gradient) => Self::Gradient(gradient),
            BackgroundSource::Solid(color) => Self::Solid(*color),
            BackgroundSource::Image {
                resource,
                svg,
                rendering,
                environment,
            } => {
                let image = if *svg {
                    let width = (tile.width * scale).ceil();
                    let height = (tile.height * scale).ceil();
                    if !width.is_finite()
                        || !height.is_finite()
                        || width < 0.
                        || height < 0.
                        || width > u32::MAX as f64
                        || height > u32::MAX as f64
                    {
                        return Err(RenderError::Encode(
                            "invalid cross-fade SVG dimensions".into(),
                        ));
                    }
                    cache.svg(
                        *resource,
                        snapshot,
                        width.max(1.) as u32,
                        height.max(1.) as u32,
                        *environment,
                    )?
                } else {
                    let source = snapshot.images.get(*resource).ok_or_else(|| {
                        RenderError::Encode("missing cross-fade image resource".into())
                    })?;
                    cache.image(*resource, source)?
                };
                *budget = budget.saturating_add(image.as_raw().len());
                if *budget > 64 * 1024 * 1024 {
                    return Err(RenderError::Encode(
                        "cross-fade decoded resource budget exceeded".into(),
                    ));
                }
                Self::Image(image, *rendering)
            }
            BackgroundSource::CrossFade(parts) => {
                if parts.is_empty()
                    || parts.len() > 256
                    || parts
                        .iter()
                        .any(|(weight, _)| !weight.is_finite() || !(0.0..=1.0).contains(weight))
                    || parts.iter().map(|(weight, _)| weight).sum::<f64>() > 1.000001
                {
                    return Err(RenderError::Encode("invalid cross-fade weights".into()));
                }
                let mut prepared = Vec::new();
                for (weight, source) in parts {
                    if *weight > 0. {
                        prepared.push((
                            *weight,
                            Self::new(source, snapshot, tile, scale, cache, budget, depth + 1)?,
                        ));
                    }
                }
                Self::CrossFade(prepared)
            }
        })
    }
    fn sample(&self, x: f64, y: f64, tile: CssRect, density: (f64, f64)) -> [u8; 4] {
        match self {
            Self::Gradient(gradient) => gradient.sample(x, y),
            Self::Solid(color) => *color,
            Self::Image(image, rendering) => sample_image_mode(
                image,
                (x - tile.x) / tile.width * image.width() as f64 - 0.5,
                (y - tile.y) / tile.height * image.height() as f64 - 0.5,
                *rendering,
                (
                    density.0 * tile.width / image.width() as f64,
                    density.1 * tile.height / image.height() as f64,
                ),
            ),
            Self::CrossFade(parts) => {
                let mut channels = [0.; 4];
                for (weight, part) in parts {
                    let rgba = part.sample(x, y, tile, density);
                    let alpha = rgba[3] as f64 / 255. * weight;
                    for i in 0..3 {
                        channels[i] += rgba[i] as f64 * alpha;
                    }
                    channels[3] += alpha;
                }
                if channels[3] <= 0. {
                    [0; 4]
                } else {
                    [
                        (channels[0] / channels[3]).round().clamp(0., 255.) as u8,
                        (channels[1] / channels[3]).round().clamp(0., 255.) as u8,
                        (channels[2] / channels[3]).round().clamp(0., 255.) as u8,
                        (channels[3] * 255.).round().clamp(0., 255.) as u8,
                    ]
                }
            }
        }
    }
}

pub(super) fn paint(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    scale: f64,
    matrix: [f64; 16],
    clip_records: &[oryn_common::v4::PaintClip],
    commands: &[PaintCommand],
    snapshot: &PaintSnapshot,
    font: &Font,
    cache: &mut RasterCache,
) -> Result<(), RenderError> {
    let Some(mapping) = Mapping::new(matrix) else {
        return Ok(());
    };
    let mut clips = Vec::new();
    for clip in clip_records {
        let Some(mapping) = Mapping::new(clip.matrix) else {
            return Ok(());
        };
        let shape = clip
            .shape
            .as_ref()
            .map(|shape| shape.refined_for_projection(clip.matrix, 1. / (scale * 256.)))
            .transpose()
            .map_err(RenderError::Encode)?;
        clips.push(Clip {
            mapping,
            rect: clip.rect,
            x: clip.clip_x,
            y: clip.clip_y,
            shape,
        });
    }
    for command in commands {
        match command {
            PaintCommand::TextClip {
                owner,
                fragment,
                commands,
            } => {
                let (extra, mask_commands) = text_clip_mask(snapshot, *owner, matrix)?;
                if mask_commands.is_empty() {
                    continue;
                }
                let mut content = Vec::new();
                for command in commands {
                    content.push(match command {
                        PaintCommand::FillRect { color, .. }
                        | PaintCommand::FillRoundedRect { color, .. } => PaintCommand::Canvas {
                            color: *color,
                            layers: Vec::new(),
                        },
                        PaintCommand::Background { tile, source, .. } => PaintCommand::Canvas {
                            color: [0; 4],
                            layers: vec![(tile.clone(), source.clone())],
                        },
                        command => command.clone(),
                    });
                }
                let mut layer = vec![0; pixels.len()];
                let mut mask = vec![0; pixels.len()];
                let mut mask_clips = clip_records.to_vec();
                if let Some(rect) = fragment {
                    mask_clips.push(oryn_common::v4::PaintClip {
                        shape: None,
                        owner: None,
                        scroll_x: false,
                        scroll_y: false,
                        matrix,
                        rect: *rect,
                        clip_x: true,
                        clip_y: true,
                    });
                }
                paint(
                    &mut layer,
                    width,
                    height,
                    scale,
                    matrix,
                    clip_records,
                    &content,
                    snapshot,
                    font,
                    cache,
                )?;
                paint(
                    &mut mask,
                    width,
                    height,
                    scale,
                    extra,
                    &mask_clips,
                    &mask_commands,
                    snapshot,
                    font,
                    cache,
                )?;
                for (pixel, mask) in layer.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    pixel[3] = ((u16::from(pixel[3]) * u16::from(mask[3]) + 127) / 255) as u8;
                }
                composite_layer(pixels, &layer, 1.);
            }
            PaintCommand::Canvas { color, layers } => {
                if color[3] > 0 {
                    shade_region(
                        pixels,
                        width,
                        height,
                        scale,
                        &mapping,
                        &clips,
                        None,
                        |_, _| *color,
                    );
                }
                for (tile, source) in layers {
                    match source {
                        BackgroundSource::Solid(_) | BackgroundSource::CrossFade(_) => {
                            let density = mapping.density_bound(CssRect {
                                x: 0.,
                                y: 0.,
                                width: width as f64 / scale,
                                height: height as f64 / scale,
                            });
                            let prepared = PreparedBackground::new(
                                source,
                                snapshot,
                                tile.rect,
                                scale * density.0.max(density.1),
                                cache,
                                &mut 0,
                                0,
                            )?;
                            shade_region(
                                pixels,
                                width,
                                height,
                                scale,
                                &mapping,
                                &clips,
                                None,
                                |x, y| {
                                    let (a, b) = mapping.scale_at(x, y);
                                    tile.map(x, y)
                                        .map(|(x, y)| {
                                            prepared.sample(x, y, tile.rect, (a * scale, b * scale))
                                        })
                                        .unwrap_or([0; 4])
                                },
                            );
                        }
                        BackgroundSource::Gradient(gradient) => shade_region(
                            pixels,
                            width,
                            height,
                            scale,
                            &mapping,
                            &clips,
                            None,
                            |x, y| {
                                tile.map(x, y)
                                    .map(|(x, y)| gradient.sample(x, y))
                                    .unwrap_or([0; 4])
                            },
                        ),
                        BackgroundSource::Image {
                            resource,
                            svg,
                            rendering,
                            environment,
                        } => {
                            let source = snapshot.images.get(*resource).ok_or_else(|| {
                                RenderError::Encode("missing captured canvas image".into())
                            })?;
                            let image = if *svg {
                                let density = mapping.density_bound(CssRect {
                                    x: 0.,
                                    y: 0.,
                                    width: width as f64 / scale,
                                    height: height as f64 / scale,
                                });
                                let w = (tile.rect.width * scale * density.0).ceil();
                                let h = (tile.rect.height * scale * density.1).ceil();
                                if !w.is_finite()
                                    || !h.is_finite()
                                    || w > u32::MAX as f64
                                    || h > u32::MAX as f64
                                {
                                    return Err(RenderError::PixelLimit {
                                        actual: usize::MAX,
                                        limit: crate::svg::MAX_PIXELS,
                                    });
                                }
                                cache.svg(
                                    *resource,
                                    snapshot,
                                    w.max(1.) as u32,
                                    h.max(1.) as u32,
                                    *environment,
                                )?
                            } else {
                                cache.image(*resource, source)?
                            };
                            shade_region(
                                pixels,
                                width,
                                height,
                                scale,
                                &mapping,
                                &clips,
                                None,
                                |x, y| {
                                    let (sx, sy) = mapping.scale_at(x, y);
                                    let Some((x, y)) = tile.map(x, y) else {
                                        return [0; 4];
                                    };
                                    sample_image_mode(
                                        &image,
                                        (x - tile.rect.x) / tile.rect.width * image.width() as f64
                                            - 0.5,
                                        (y - tile.rect.y) / tile.rect.height
                                            * image.height() as f64
                                            - 0.5,
                                        *rendering,
                                        (
                                            sx * scale * tile.rect.width / image.width() as f64,
                                            sy * scale * tile.rect.height / image.height() as f64,
                                        ),
                                    )
                                },
                            );
                        }
                    }
                }
            }
            PaintCommand::EmbeddedDocument { rect, frame } => {
                if rect.width <= 0. || rect.height <= 0. {
                    continue;
                }
                let _frame_scope = crate::svg_coverage::frame_scope(
                    snapshot
                        .frames
                        .get(*frame)
                        .ok_or_else(|| {
                            RenderError::InvalidPrint("semantic frame index out of bounds".into())
                        })?
                        .owner_node,
                );
                let child = crate::frames::snapshot(snapshot, *frame)?;
                let bounds = mapping.bounds(*rect, scale, width, height);
                if bounds.0 >= bounds.2 || bounds.1 >= bounds.3 {
                    continue;
                }
                // Measure only the visible camera half-plane. A source corner
                // on the horizon is outside this finite raster viewport and
                // must not request an infinite child surface.
                let mut density = 0_f64;
                for px in [
                    bounds.0 as f64,
                    (bounds.0 as f64 + bounds.2 as f64) / 2.,
                    bounds.2 as f64,
                ] {
                    for py in [
                        bounds.1 as f64,
                        (bounds.1 as f64 + bounds.3 as f64) / 2.,
                        bounds.3 as f64,
                    ] {
                        if let Some((x, y)) = mapping.source(px / scale, py / scale) {
                            if x >= rect.x && x <= rect.right() && y >= rect.y && y <= rect.bottom()
                            {
                                let (a, b) = mapping.scale_at(x, y);
                                density = density.max(
                                    (a * rect.width / child.viewport.width)
                                        .max(b * rect.height / child.viewport.height),
                                );
                            }
                        }
                    }
                }
                for (x, y) in [
                    (rect.x, rect.y),
                    (rect.right(), rect.y),
                    (rect.right(), rect.bottom()),
                    (rect.x, rect.bottom()),
                ] {
                    let m = mapping.forward;
                    let w = m[6] * x + m[7] * y + m[8];
                    if w > 0. {
                        let (px, py) = (
                            (m[0] * x + m[1] * y + m[2]) / w,
                            (m[3] * x + m[4] * y + m[5]) / w,
                        );
                        if px >= 0.
                            && px <= width as f64 / scale
                            && py >= 0.
                            && py <= height as f64 / scale
                        {
                            let (a, b) = mapping.scale_at(x, y);
                            density = density.max(
                                (a * rect.width / child.viewport.width)
                                    .max(b * rect.height / child.viewport.height),
                            );
                        }
                    }
                }

                let image = crate::frames::raster(child, (scale * density).max(0.01))?;
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    *rect,
                    |x, y| {
                        sample_image_mode(
                            &image,
                            (x - rect.x) / rect.width * image.width() as f64 - 0.5,
                            (y - rect.y) / rect.height * image.height() as f64 - 0.5,
                            ImageRendering::Auto,
                            (1., 1.),
                        )
                    },
                );
            }
            PaintCommand::SvgMask {
                rect,
                resource,
                fragment,
                mode,
                environment,
            } => {
                let _environment = environment.enter();
                let density = mapping.density_bound(CssRect {
                    x: 0.,
                    y: 0.,
                    width: width as f64 / scale,
                    height: height as f64 / scale,
                });
                let image = crate::masks::svg_image(
                    snapshot,
                    *resource,
                    fragment,
                    *rect,
                    mode,
                    scale * density.0.max(density.1),
                )?;
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    *rect,
                    |x, y| {
                        sample_image_mode(
                            &image,
                            (x - rect.x) / rect.width * image.width() as f64 - 0.5,
                            (y - rect.y) / rect.height * image.height() as f64 - 0.5,
                            ImageRendering::Auto,
                            (1., 1.),
                        )
                    },
                );
            }
            PaintCommand::BackdropFilter {
                source,
                current_color,
                plane,
                rect,
                radii,
                environment,
            } => {
                let _environment = environment.enter();
                let _budget =
                    crate::filters::RasterBudget::enter(width as usize * height as usize * 3)?;
                let mut png = Vec::new();
                image::codecs::png::PngEncoder::new(&mut png)
                    .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
                    .map_err(|e| RenderError::Encode(e.to_string()))?;
                let mut captured = snapshot.clone();
                let resource = captured.images.len();
                captured.images.push(oryn_common::v4::PaintImageResource {
                    svg_document_snapshot: false,
                    content_type: None,
                    url: "oryn-private-backdrop:".into(),
                    url_aliases: Vec::new(),
                    media_type: "image/png".into(),
                    bytes: png,
                });
                let q = Mapping::new(matrix)
                    .ok_or_else(|| RenderError::Encode("singular backdrop plane".into()))?
                    .inverse;
                let inverse = [
                    q[0], q[3], 0., q[6], q[1], q[4], 0., q[7], 0., 0., 1., 0., q[2], q[5], 0.,
                    q[8],
                ];
                let command = PaintCommand::Filter {
                    environment: *environment,
                    reference_box: Some(*rect),
                    source: source.clone(),
                    current_color: current_color.clone(),
                    plane: *plane,
                    commands: vec![PaintCommand::Projected {
                        matrix: inverse,
                        clips: Vec::new(),
                        commands: vec![PaintCommand::Image {
                            rect: CssRect {
                                x: 0.,
                                y: 0.,
                                width: width as f64 / scale,
                                height: height as f64 / scale,
                            },
                            resource,
                            rendering: ImageRendering::Auto,
                            clip: None,
                        }],
                    }],
                };
                let mut filtered = vec![0; pixels.len()];
                let mut private_cache = RasterCache::default();
                paint(
                    &mut filtered,
                    width,
                    height,
                    scale,
                    matrix,
                    &[],
                    &[command],
                    &captured,
                    font,
                    &mut private_cache,
                )?;
                let mut coverage = vec![0; pixels.len()];
                paint(
                    &mut coverage,
                    width,
                    height,
                    scale,
                    multiply(matrix, *plane),
                    clip_records,
                    &[PaintCommand::FillRoundedRect {
                        rect: *rect,
                        color: [255; 4],
                        radii: *radii,
                    }],
                    snapshot,
                    font,
                    cache,
                )?;
                for ((pixel, filtered), coverage) in pixels
                    .chunks_exact_mut(4)
                    .zip(filtered.chunks_exact(4))
                    .zip(coverage.chunks_exact(4))
                {
                    let coverage = f64::from(coverage[3]) / 255.;
                    let before = f64::from(pixel[3]) * (1. - coverage);
                    let after = f64::from(filtered[3]) * coverage;
                    let alpha = before + after;
                    for i in 0..3 {
                        pixel[i] = if alpha > 0. {
                            ((f64::from(pixel[i]) * before + f64::from(filtered[i]) * after)
                                / alpha)
                                .round()
                                .clamp(0., 255.) as u8
                        } else {
                            0
                        };
                    }
                    pixel[3] = alpha.round().clamp(0., 255.) as u8;
                }
            }
            PaintCommand::Mask { layers, commands } => {
                let _budget =
                    crate::filters::RasterBudget::enter(width as usize * height as usize * 3)?;
                let mut layer = vec![0; pixels.len()];
                paint(
                    &mut layer,
                    width,
                    height,
                    scale,
                    matrix,
                    clip_records,
                    commands,
                    snapshot,
                    font,
                    cache,
                )?;
                let mut coverage = vec![0.; width as usize * height as usize];
                for (index, mask) in layers.iter().enumerate() {
                    let mut mask_pixels = vec![0; pixels.len()];
                    paint(
                        &mut mask_pixels,
                        width,
                        height,
                        scale,
                        matrix,
                        clip_records,
                        &mask.commands,
                        snapshot,
                        font,
                        cache,
                    )?;
                    for (alpha, pixel) in coverage.iter_mut().zip(mask_pixels.chunks_exact(4)) {
                        let source = crate::masks::alpha(pixel, mask.luminance);
                        *alpha = if index == 0 {
                            source
                        } else {
                            crate::masks::combine(source, *alpha, mask.composite)
                        };
                    }
                }
                for (pixel, alpha) in layer.chunks_exact_mut(4).zip(coverage) {
                    pixel[3] = (f64::from(pixel[3]) * alpha).round().clamp(0., 255.) as u8;
                }
                composite_layer(pixels, &layer, 1.);
            }
            PaintCommand::Filter {
                reference_box,
                source,
                current_color,
                plane,
                commands,
                environment,
            } => {
                let _environment = environment.enter();
                let base = Mapping::new(*plane)
                    .ok_or_else(|| RenderError::Encode("singular native filter plane".into()))?;
                let q = base.inverse;
                let inverse = [
                    q[0], q[3], 0., q[6], q[1], q[4], 0., q[7], 0., 0., 1., 0., q[2], q[5], 0.,
                    q[8],
                ];
                let effect_matrix = multiply(matrix, *plane);
                let effect_mapping = Mapping::new(effect_matrix)
                    .ok_or_else(|| RenderError::Encode("invalid projected filter plane".into()))?;
                let density = effect_mapping.density_bound(CssRect {
                    x: 0.,
                    y: 0.,
                    width: width as f64 / scale,
                    height: height as f64 / scale,
                });
                let resolution = scale * density.0.max(density.1);
                if !resolution.is_finite() || resolution <= 0. {
                    return Err(RenderError::Encode("invalid filter raster density".into()));
                }
                let local_commands = vec![PaintCommand::Projected {
                    matrix: inverse,
                    clips: Vec::new(),
                    commands: commands.clone(),
                }];
                let ink = ink_bounds(snapshot, &local_commands, resolution)?;
                let Some(source_bounds) = reference_box.or(ink) else {
                    continue;
                };
                let mut bounds = ink
                    .map(|ink| effect_union(ink, source_bounds))
                    .unwrap_or(source_bounds);
                let padding =
                    crate::filters::padding(snapshot, source, current_color, source_bounds)?;
                bounds.x -= padding;
                bounds.y -= padding;
                bounds.width += padding * 2.;
                bounds.height += padding * 2.;
                let wide = (bounds.width * resolution).ceil();
                let high = (bounds.height * resolution).ceil();
                if !wide.is_finite()
                    || !high.is_finite()
                    || wide <= 0.
                    || high <= 0.
                    || wide > u32::MAX as f64
                    || high > u32::MAX as f64
                {
                    return Err(RenderError::Encode("invalid filter source extent".into()));
                }
                let (wide, high) = (wide as u32, high as u32);
                let count =
                    (wide as usize)
                        .checked_mul(high as usize)
                        .ok_or(RenderError::PixelLimit {
                            actual: usize::MAX,
                            limit: 4 * 1024 * 1024,
                        })?;
                let _budget = crate::filters::RasterBudget::enter(count)?;
                let mut layer = vec![0; count * 4];
                let translation = [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., -bounds.x, -bounds.y, 0., 1.,
                ];
                paint(
                    &mut layer,
                    wide,
                    high,
                    resolution,
                    multiply(translation, inverse),
                    &[],
                    commands,
                    snapshot,
                    font,
                    cache,
                )?;
                let filtered = crate::filters::apply_source(
                    snapshot,
                    &layer,
                    wide,
                    high,
                    source,
                    current_color,
                    resolution,
                    bounds,
                    source_bounds,
                )?;
                let image = image::RgbaImage::from_raw(wide, high, filtered)
                    .ok_or_else(|| RenderError::Encode("filter raster dimensions".into()))?;
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &effect_mapping,
                    &clips,
                    bounds,
                    |x, y| {
                        sample_image_mode(
                            &image,
                            (x - bounds.x) * resolution - 0.5,
                            (y - bounds.y) * resolution - 0.5,
                            ImageRendering::Auto,
                            (1., 1.),
                        )
                    },
                );
            }
            PaintCommand::Opacity { opacity, commands } => {
                let mut layer = vec![0; pixels.len()];
                paint(
                    &mut layer,
                    width,
                    height,
                    scale,
                    matrix,
                    clip_records,
                    commands,
                    snapshot,
                    font,
                    cache,
                )?;
                composite_layer(pixels, &layer, *opacity);
            }
            PaintCommand::Background {
                rect,
                radii,
                tile,
                source,
            } => match source {
                BackgroundSource::Solid(_) | BackgroundSource::CrossFade(_) => {
                    let density = mapping.density_bound(CssRect {
                        x: 0.,
                        y: 0.,
                        width: width as f64 / scale,
                        height: height as f64 / scale,
                    });
                    let prepared = PreparedBackground::new(
                        source,
                        snapshot,
                        tile.rect,
                        scale * density.0.max(density.1),
                        cache,
                        &mut 0,
                        0,
                    )?;
                    shade(
                        pixels,
                        width,
                        height,
                        scale,
                        &mapping,
                        &clips,
                        *rect,
                        |x, y| {
                            if !borders::inside(*rect, *radii, x, y) {
                                return [0; 4];
                            }
                            let (a, b) = mapping.scale_at(x, y);
                            tile.map(x, y)
                                .map(|(x, y)| {
                                    prepared.sample(x, y, tile.rect, (a * scale, b * scale))
                                })
                                .unwrap_or([0; 4])
                        },
                    );
                }
                BackgroundSource::Gradient(gradient) => shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    *rect,
                    |x, y| {
                        if !borders::inside(*rect, *radii, x, y) {
                            return [0; 4];
                        }
                        tile.map(x, y)
                            .map(|(x, y)| gradient.sample(x, y))
                            .unwrap_or([0; 4])
                    },
                ),
                BackgroundSource::Image {
                    resource,
                    svg,
                    rendering,
                    environment,
                } => {
                    let Some(source) = snapshot.images.get(*resource) else {
                        return Err(RenderError::Encode(
                            "missing captured background resource".into(),
                        ));
                    };
                    if tile.rect.width <= 0. || tile.rect.height <= 0. {
                        continue;
                    }
                    let image = if *svg {
                        let (left, top, right, bottom) =
                            mapping.bounds(*rect, scale, width, height);
                        let density = mapping.density_bound(CssRect {
                            x: left as f64 / scale,
                            y: top as f64 / scale,
                            width: (right - left) as f64 / scale,
                            height: (bottom - top) as f64 / scale,
                        });
                        let w = (tile.rect.width * scale * density.0).ceil();
                        let h = (tile.rect.height * scale * density.1).ceil();
                        if !w.is_finite()
                            || !h.is_finite()
                            || w > u32::MAX as f64
                            || h > u32::MAX as f64
                        {
                            return Err(RenderError::PixelLimit {
                                actual: usize::MAX,
                                limit: crate::svg::MAX_PIXELS,
                            });
                        }
                        cache.svg(
                            *resource,
                            snapshot,
                            w.max(1.) as u32,
                            h.max(1.) as u32,
                            *environment,
                        )?
                    } else {
                        cache.image(*resource, source)?
                    };
                    shade(
                        pixels,
                        width,
                        height,
                        scale,
                        &mapping,
                        &clips,
                        *rect,
                        |x, y| {
                            if !borders::inside(*rect, *radii, x, y) {
                                return [0; 4];
                            }
                            let Some((tx, ty)) = tile.map(x, y) else {
                                return [0; 4];
                            };
                            let (sx, sy) = mapping.scale_at(x, y);
                            sample_image_mode(
                                &image,
                                (tx - tile.rect.x) / tile.rect.width * image.width() as f64 - 0.5,
                                (ty - tile.rect.y) / tile.rect.height * image.height() as f64 - 0.5,
                                *rendering,
                                (
                                    sx * scale * tile.rect.width / image.width() as f64,
                                    sy * scale * tile.rect.height / image.height() as f64,
                                ),
                            )
                        },
                    );
                }
            },
            PaintCommand::BoxShadow {
                rect,
                shadow,
                radii,
            } => {
                let area = shadows::bounds(*rect, shadow);
                let (left, top, right, bottom) = mapping.bounds(area, scale, width, height);
                if left >= right || top >= bottom {
                    continue;
                }
                let density = [
                    (rect.x, rect.y),
                    (rect.right(), rect.y),
                    (rect.x, rect.bottom()),
                    (rect.right(), rect.bottom()),
                ]
                .into_iter()
                .map(|(x, y)| {
                    let (a, b) = mapping.scale_at(x, y);
                    a.max(b)
                })
                .fold(0f64, f64::max);
                let mask = cache.shadow(*rect, *radii, shadow, scale * density)?;
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    area,
                    |x, y| {
                        if borders::inside(*rect, *radii, x, y) != shadow.inset {
                            return [0; 4];
                        }
                        let mut color = shadow.color;
                        color[3] = ((u16::from(color[3]) * u16::from(mask.sample(x, y)) + 127)
                            / 255) as u8;
                        color
                    },
                );
            }
            PaintCommand::Gradient {
                rect,
                gradient,
                radii,
            } => shade(
                pixels,
                width,
                height,
                scale,
                &mapping,
                &clips,
                *rect,
                |x, y| {
                    if borders::inside(*rect, *radii, x, y) {
                        gradient.sample(x, y)
                    } else {
                        [0; 4]
                    }
                },
            ),
            PaintCommand::LinearGradient {
                rect,
                gradient,
                radii,
            } => shade(
                pixels,
                width,
                height,
                scale,
                &mapping,
                &clips,
                *rect,
                |x, y| {
                    if borders::inside(*rect, *radii, x, y) {
                        gradient.sample(x, y)
                    } else {
                        [0; 4]
                    }
                },
            ),
            PaintCommand::FillRoundedRect { rect, color, radii } => shade(
                pixels,
                width,
                height,
                scale,
                &mapping,
                &clips,
                *rect,
                |x, y| {
                    if borders::inside(*rect, *radii, x, y) {
                        *color
                    } else {
                        [0; 4]
                    }
                },
            ),
            PaintCommand::FillRect { rect, color } => shade(
                pixels,
                width,
                height,
                scale,
                &mapping,
                &clips,
                *rect,
                |_, _| *color,
            ),
            PaintCommand::BorderEdges { rect, edges, radii } => {
                let colors = edges
                    .each_ref()
                    .map(|edge| parse_color(&edge.color).unwrap_or([0, 0, 0, 255]));
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    *rect,
                    |x, y| borders::sample(*rect, edges, *radii, colors, x, y),
                );
            }
            PaintCommand::Border {
                rect,
                widths,
                color,
            } => shade(
                pixels,
                width,
                height,
                scale,
                &mapping,
                &clips,
                *rect,
                |x, y| {
                    if x < rect.x + widths[3]
                        || x >= rect.right() - widths[1]
                        || y < rect.y + widths[0]
                        || y >= rect.bottom() - widths[2]
                    {
                        *color
                    } else {
                        [0; 4]
                    }
                },
            ),
            PaintCommand::BuiltinImage { rect, clip, image } => {
                if rect.width <= 0. || rect.height <= 0. {
                    continue;
                }
                let density = [
                    (rect.x, rect.y),
                    (rect.right(), rect.y),
                    (rect.x, rect.bottom()),
                    (rect.right(), rect.bottom()),
                    (rect.x + rect.width / 2., rect.y + rect.height / 2.),
                ]
                .into_iter()
                .map(|(x, y)| mapping.scale_at(x, y))
                .fold((0_f64, 0_f64), |(x, y), (a, b)| (x.max(a), y.max(b)));
                let w = (rect.width * scale * density.0).ceil();
                let h = (rect.height * scale * density.1).ceil();
                let image = crate::builtin_image::raster(image, w, h)?;
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    *rect,
                    |x, y| {
                        if clip.is_some_and(|clip| {
                            x < clip.x || x >= clip.right() || y < clip.y || y >= clip.bottom()
                        }) {
                            return [0; 4];
                        }
                        sample_image_mode(
                            &image,
                            (x - rect.x) / rect.width * image.width() as f64 - 0.5,
                            (y - rect.y) / rect.height * image.height() as f64 - 0.5,
                            ImageRendering::Auto,
                            (1., 1.),
                        )
                    },
                );
            }
            PaintCommand::SvgImage {
                rect,
                clip,
                resource,
                environment,
            } => {
                if rect.width <= 0. || rect.height <= 0. {
                    continue;
                }
                // Rasterize vector geometry at the projected pixel density;
                // CSS enlargement must not upscale an intrinsic-size bitmap.
                let density = [
                    (rect.x, rect.y),
                    (rect.right(), rect.y),
                    (rect.x, rect.bottom()),
                    (rect.right(), rect.bottom()),
                    (rect.x + rect.width / 2., rect.y + rect.height / 2.),
                ]
                .into_iter()
                .map(|(x, y)| mapping.scale_at(x, y))
                .fold((0_f64, 0_f64), |(x, y), (a, b)| (x.max(a), y.max(b)));
                let w = (rect.width * scale * density.0).ceil();
                let h = (rect.height * scale * density.1).ceil();
                if !w.is_finite() || !h.is_finite() || w > u32::MAX as f64 || h > u32::MAX as f64 {
                    return Err(RenderError::PixelLimit {
                        actual: usize::MAX,
                        limit: crate::svg::MAX_PIXELS,
                    });
                }
                let image = cache.svg(
                    *resource,
                    snapshot,
                    w.max(1.) as u32,
                    h.max(1.) as u32,
                    *environment,
                )?;
                shade(
                    pixels,
                    width,
                    height,
                    scale,
                    &mapping,
                    &clips,
                    *rect,
                    |x, y| {
                        if clip.is_some_and(|clip| {
                            x < clip.x || x >= clip.right() || y < clip.y || y >= clip.bottom()
                        }) {
                            return [0; 4];
                        }
                        sample_image_mode(
                            &image,
                            (x - rect.x) / rect.width * image.width() as f64 - 0.5,
                            (y - rect.y) / rect.height * image.height() as f64 - 0.5,
                            ImageRendering::Auto,
                            (1., 1.),
                        )
                    },
                );
            }
            PaintCommand::Image {
                rect,
                resource,
                rendering,
                clip,
            } => {
                if let Some(source) = snapshot.images.get(*resource) {
                    let image = cache.image(*resource, source)?;
                    shade(
                        pixels,
                        width,
                        height,
                        scale,
                        &mapping,
                        &clips,
                        *rect,
                        |x, y| {
                            if clip.is_some_and(|clip| {
                                x < clip.x || x >= clip.right() || y < clip.y || y >= clip.bottom()
                            }) {
                                return [0; 4];
                            }
                            let (sx, sy) = mapping.scale_at(x, y);
                            sample_image_mode(
                                &image,
                                (x - rect.x) / rect.width * image.width() as f64 - 0.5,
                                (y - rect.y) / rect.height * image.height() as f64 - 0.5,
                                *rendering,
                                (
                                    sx * scale * rect.width / image.width() as f64,
                                    sy * scale * rect.height / image.height() as f64,
                                ),
                            )
                        },
                    );
                }
            }
            PaintCommand::Text { fragment, color } => {
                if !fragment.glyph_runs.is_empty() {
                    for run in &fragment.glyph_runs {
                        let boxes = crate::text_boxes::commands(snapshot, fragment, run)?;
                        if !boxes.is_empty() {
                            paint(
                                pixels,
                                width,
                                height,
                                scale,
                                matrix,
                                clip_records,
                                &boxes,
                                snapshot,
                                font,
                                cache,
                            )?;
                        }
                        if let Some((rect, color)) = crate::decorations::background(fragment, run)?
                        {
                            shade(
                                pixels,
                                width,
                                height,
                                scale,
                                &mapping,
                                &clips,
                                rect,
                                |_, _| color,
                            );
                        }
                    }
                    let mut decoration_masks: Vec<(crate::decorations::Stroke, Vec<(f64, f64)>)> =
                        Vec::new();
                    for run in &fragment.glyph_runs {
                        for stroke in crate::decorations::strokes(snapshot, fragment, run)? {
                            let gaps =
                                decoration_gaps(snapshot, fragment, run, &stroke, scale, cache)?;
                            if let Some((_, existing)) = decoration_masks
                                .iter_mut()
                                .find(|(existing, _)| *existing == stroke)
                            {
                                existing.extend(gaps);
                            } else {
                                decoration_masks.push((stroke, gaps));
                            }
                        }
                    }
                    for (_, gaps) in &mut decoration_masks {
                        gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
                        let mut merged: Vec<(f64, f64)> = Vec::new();
                        for gap in gaps.drain(..) {
                            if let Some(last) = merged.last_mut() {
                                if gap.0 <= last.1 {
                                    last.1 = last.1.max(gap.1);
                                    continue;
                                }
                            }
                            merged.push(gap);
                        }
                        *gaps = merged;
                    }
                    let paint_decoration = |pixels: &mut [u8], after: bool| {
                        for (stroke, gaps) in &decoration_masks {
                            if stroke.after != after {
                                continue;
                            }
                            shade(
                                pixels,
                                width,
                                height,
                                scale,
                                &mapping,
                                &clips,
                                stroke.rect,
                                |x, y| {
                                    let inline = if stroke.vertical { y } else { x };
                                    let index = gaps.partition_point(|gap| gap.1 <= inline);
                                    if gaps.get(index).is_some_and(|gap| inline >= gap.0) {
                                        [0; 4]
                                    } else {
                                        stroke.sample(x, y)
                                    }
                                },
                            );
                        }
                    };
                    paint_decoration(pixels, false);
                    for run in &fragment.glyph_runs {
                        let run_color = run.color.unwrap_or(*color);
                        for positioned in &run.glyphs {
                            let glyph = cache.indexed_glyph(
                                snapshot,
                                run,
                                positioned.id,
                                (run.font_size * scale) as f32,
                            )?;
                            let (metrics, bitmap) = glyph.as_ref();
                            if metrics.width == 0 || metrics.height == 0 {
                                continue;
                            }
                            let origin = (
                                fragment.rect.x + positioned.x,
                                fragment.rect.y + positioned.y,
                            );
                            let unrotated = CssRect {
                                x: metrics.xmin as f64 / scale * run.scale_x,
                                y: -(metrics.height as f64 + metrics.ymin as f64) / scale,
                                width: metrics.width as f64 / scale * run.scale_x,
                                height: metrics.height as f64 / scale,
                            };
                            let rect = oriented_glyph_bounds(unrotated, origin, run.orientation);
                            shade(
                                pixels,
                                width,
                                height,
                                scale,
                                &mapping,
                                &clips,
                                rect,
                                |x, y| {
                                    let (x, y) = glyph_source_point(
                                        x - origin.0,
                                        y - origin.1,
                                        run.orientation,
                                    );
                                    let ix = (((x - unrotated.x) * scale / run.scale_x).floor()
                                        as usize)
                                        .min(metrics.width - 1);
                                    let iy = (((y - unrotated.y) * scale).floor() as usize)
                                        .min(metrics.height - 1);
                                    let mut rgba = run_color;
                                    rgba[3] = (rgba[3] as u16
                                        * bitmap[iy * metrics.width + ix] as u16
                                        / 255) as u8;
                                    rgba
                                },
                            );
                        }
                    }
                    paint_decoration(pixels, true);
                    continue;
                }
                // Legacy snapshots did not transport shaped glyphs.
                let mut pen = fragment.rect.x;
                let baseline = fragment.rect.y + fragment.font_size;
                let size = (fragment.font_size * scale) as f32;
                for character in fragment.text.chars() {
                    if matches!(character, '\n' | '\r') {
                        continue;
                    }
                    if character == '\t' {
                        pen += font.metrics(' ', size).advance_width as f64 / scale * 4.;
                        continue;
                    }
                    let glyph = cache.glyph(font, character, size)?;
                    let (metrics, bitmap) = glyph.as_ref();
                    let rect = CssRect {
                        x: pen + metrics.xmin as f64 / scale,
                        y: baseline - (metrics.height as f64 + metrics.ymin as f64) / scale,
                        width: metrics.width as f64 / scale,
                        height: metrics.height as f64 / scale,
                    };
                    shade(
                        pixels,
                        width,
                        height,
                        scale,
                        &mapping,
                        &clips,
                        rect,
                        |x, y| {
                            let ix =
                                (((x - rect.x) * scale).floor() as usize).min(metrics.width - 1);
                            let iy =
                                (((y - rect.y) * scale).floor() as usize).min(metrics.height - 1);
                            let mut rgba = *color;
                            rgba[3] = (rgba[3] as u16 * bitmap[iy * metrics.width + ix] as u16
                                / 255) as u8;
                            rgba
                        },
                    );
                    pen += metrics.advance_width as f64 / scale;
                }
            }
            PaintCommand::Projected {
                matrix: local,
                clips: local_clips,
                commands,
            } => {
                let composed = multiply(matrix, *local);
                let mut nested_clips = clip_records.to_vec();
                nested_clips.extend(local_clips.iter().cloned().map(|mut clip| {
                    clip.matrix = multiply(matrix, clip.matrix);
                    clip
                }));
                paint(
                    pixels,
                    width,
                    height,
                    scale,
                    composed,
                    &nested_clips,
                    commands,
                    snapshot,
                    font,
                    cache,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glyph_bounds_are_checked_before_allocating_rasters() {
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let mut cache = RasterCache::default();
        assert!(cache.glyph(&font, 'A', f32::INFINITY).is_err());
        assert!(cache.glyph(&font, 'W', 1_000_000.).is_err());
        assert!(cache.glyph(&font, 'A', 16.).is_ok());
    }
    #[test]
    fn glyph_orientation_rotates_coverage_about_physical_baseline() {
        use oryn_common::v4::{
            PaintFontResource, PaintGlyph, PaintGlyphOrientation, PaintGlyphRun, PaintTextFragment,
        };
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 64.;
        snapshot.viewport.height = 64.;
        snapshot.fonts.insert(
            "face".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let glyph = font.lookup_glyph_index('F') as u32;
        let render = |orientation| {
            let scene = DisplayList {
                contract_version: crate::RENDER_CONTRACT_VERSION,
                width: 64,
                height: 64,
                diagnostics: Vec::new(),
                commands: vec![PaintCommand::Text {
                    color: [0, 0, 0, 255],
                    fragment: PaintTextFragment {
                        flow_line: None,
                        flow_rect: None,
                        clusters: Vec::new(),
                        text: "F".into(),
                        rect: CssRect {
                            x: 0.,
                            y: 0.,
                            width: 64.,
                            height: 64.,
                        },
                        font_family: oryn_layout::DEFAULT_FONT_FAMILY.into(),
                        font_size: 20.,
                        color: "black".into(),
                        glyph_runs: vec![PaintGlyphRun {
                            line_box: None,
                            text_box: None,
                            background: None,
                            decoration_rect: None,
                            scale_x: 1.,
                            vertical: false,
                            decorations: Vec::new(),
                            orientation,
                            font_key: "face".into(),
                            font_size: 20.,
                            color: None,
                            embolden: false,
                            skew_degrees: 0.,
                            normalized_coords: Vec::new(),
                            glyphs: vec![PaintGlyph {
                                source: None,
                                id: glyph,
                                x: 32.,
                                y: 32.,
                                advance: 0.,
                            }],
                        }],
                    },
                }],
            };
            crate::rasterize(&snapshot, &scene, RenderOptions::default()).unwrap()
        };
        let upright = render(PaintGlyphOrientation::Upright);
        for (orientation, clockwise) in [
            (PaintGlyphOrientation::Clockwise, true),
            (PaintGlyphOrientation::CounterClockwise, false),
        ] {
            let rotated = render(orientation);
            for y in 0..64 {
                for x in 0..64 {
                    let (sx, sy) = if clockwise { (y, 63 - x) } else { (63 - y, x) };
                    assert_eq!(
                        rotated[(y * 64 + x) * 4 + 3],
                        upright[(sy * 64 + sx) * 4 + 3]
                    );
                }
            }
        }
    }

    #[test]
    fn shaped_glyph_cache_separates_face_synthesis_and_variation_coordinates() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.fonts.insert(
            "face".into(),
            oryn_common::v4::PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let id = font.lookup_glyph_index('A') as u32;
        let mut run = oryn_common::v4::PaintGlyphRun {
            line_box: None,
            text_box: None,
            background: None,
            decoration_rect: None,
            scale_x: 1.,
            vertical: false,
            decorations: Vec::new(),
            orientation: oryn_common::v4::PaintGlyphOrientation::Upright,
            font_key: "face".into(),
            font_size: 16.,
            color: None,
            glyphs: vec![],
            embolden: false,
            skew_degrees: 0.,
            normalized_coords: vec![],
        };
        let mut cache = RasterCache::default();
        let ordinary = cache.indexed_glyph(&snapshot, &run, id, 16.).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &ordinary,
            &cache.indexed_glyph(&snapshot, &run, id, 16.).unwrap()
        ));
        run.embolden = true;
        let bold = cache.indexed_glyph(&snapshot, &run, id, 16.).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&ordinary, &bold));
        run.embolden = false;
        run.normalized_coords = vec![0];
        let default_axis = cache.indexed_glyph(&snapshot, &run, id, 16.).unwrap();
        run.normalized_coords = vec![16384];
        let changed_axis = cache.indexed_glyph(&snapshot, &run, id, 16.).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&default_axis, &changed_axis));
        assert_eq!(cache.indexed_glyphs.len(), 4);
    }

    #[test]
    fn glyph_cache_reuses_raster_size_without_aliasing_other_sizes() {
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let mut cache = RasterCache::default();
        let first = cache.glyph(&font, 'A', 16.).unwrap();
        let again = cache.glyph(&font, 'A', 16.).unwrap();
        let larger = cache.glyph(&font, 'A', 24.).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &again));
        assert!(!std::sync::Arc::ptr_eq(&first, &larger));
        assert_eq!(cache.glyph_bytes, first.1.len() + larger.1.len());
        let detached = first.clone();
        cache.glyphs.clear();
        cache.glyph_bytes = 0;
        assert!(!detached.1.is_empty());
    }

    #[test]
    fn rendering_modes_select_integer_pixelated_and_crisp_sampling() {
        let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert_eq!(
            sample_image_mode(&image, 0.25, 0., ImageRendering::Auto, (2., 2.)),
            [191, 0, 64, 255]
        );
        assert_eq!(
            sample_image_mode(&image, 0.25, 0., ImageRendering::CrispEdges, (2., 2.)),
            [255, 0, 0, 255]
        );
        assert_eq!(
            sample_image_mode(&image, 0.25, 0., ImageRendering::Pixelated, (2., 2.)),
            [255, 0, 0, 255]
        );
        assert_eq!(
            sample_image_mode(&image, 0.5, 0., ImageRendering::Pixelated, (2.4, 2.4)),
            [128, 0, 128, 255]
        );
        assert_eq!(
            sample_image_mode(&image, 0.75, 0., ImageRendering::Pixelated, (2., 2.)),
            [0, 0, 255, 255]
        );
    }

    #[test]
    fn image_interpolation_ignores_color_of_transparent_texels() {
        let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 0]).unwrap();
        assert_eq!(sample_image(&image, 0.5, 0.), [255, 0, 0, 128]);
        assert_eq!(sample_image(&image, -0.5, 0.), [255, 0, 0, 255]);
        assert_eq!(sample_image(&image, 1.5, 0.), [0, 0, 0, 0]);
    }

    #[test]
    fn translucent_source_over_preserves_straight_colors() {
        let mut pixel = [0; 4];
        blend(&mut pixel, 0, [255, 0, 0, 128]);
        assert_eq!(pixel, [255, 0, 0, 128]);
        blend(&mut pixel, 0, [0, 0, 255, 128]);
        assert_eq!(pixel, [85, 0, 170, 192]);
        blend(&mut pixel, 0, [0, 255, 0, 0]);
        assert_eq!(pixel, [85, 0, 170, 192]);
    }
    const IDENTITY: [f64; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    #[test]
    fn native_shape_clip_masks_pixels_in_its_own_plane() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 8.,
            height: 8.,
        };
        let shape = oryn_common::basic_shape::parse("circle(4px at center)", rect, 16., (8., 8.))
            .unwrap()
            .unwrap();
        let mapping = Mapping::new(IDENTITY).unwrap();
        let clip = Clip {
            mapping: Mapping::new(IDENTITY).unwrap(),
            rect,
            x: true,
            y: true,
            shape: Some(shape),
        };
        let mut pixels = vec![0; 8 * 8 * 4];
        shade(&mut pixels, 8, 8, 1., &mapping, &[clip], rect, |_, _| {
            [0, 255, 0, 255]
        });
        assert_eq!(pixels[3], 0);
        assert_eq!(pixels[(4 * 8 + 4) * 4 + 3], 255);
    }
    #[test]
    fn ancestor_clip_is_evaluated_in_its_own_transformed_space() {
        let mapping = Mapping::new(IDENTITY).unwrap();
        let mut translated = IDENTITY;
        translated[12] = 1.;
        let clip = Clip {
            shape: None,
            mapping: Mapping::new(translated).unwrap(),
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            x: true,
            y: false,
        };
        let mut pixels = vec![0; 4 * 2 * 4];
        shade(
            &mut pixels,
            4,
            2,
            1.,
            &mapping,
            &[clip],
            CssRect {
                x: 0.,
                y: 0.,
                width: 4.,
                height: 2.,
            },
            |_, _| [0, 255, 0, 255],
        );
        for y in 0..2 {
            for x in 0..4 {
                assert_eq!(pixels[(y * 4 + x) * 4 + 3], if x == 1 { 255 } else { 0 });
            }
        }
    }

    #[test]
    fn inverse_mapping_keeps_visible_half_of_horizon_crossing() {
        let mut matrix = IDENTITY;
        matrix[3] = -1.; // w=1-x: the source rectangle crosses the horizon x=1.
        let mapping = Mapping::new(matrix).unwrap();
        let mut pixels = vec![0; 4 * 2 * 4];
        shade(
            &mut pixels,
            4,
            2,
            1.,
            &mapping,
            &[],
            CssRect {
                x: 0.,
                y: 0.,
                width: 2.,
                height: 2.,
            },
            |_, _| [0, 255, 0, 255],
        );
        for pixel in pixels.chunks_exact(4) {
            assert_eq!(pixel, [0, 255, 0, 255]);
        }
        assert!(mapping.source(-3., 0.).is_none());
    }
    #[test]
    fn projected_rotated_rectangle_does_not_fill_its_bounding_box() {
        let mut matrix = IDENTITY;
        let angle = std::f64::consts::FRAC_PI_4;
        matrix[0] = angle.cos();
        matrix[1] = angle.sin();
        matrix[4] = -angle.sin();
        matrix[5] = angle.cos();
        matrix[12] = 2.;
        let mapping = Mapping::new(matrix).unwrap();
        let mut pixels = vec![0; 4 * 4 * 4];
        shade(
            &mut pixels,
            4,
            4,
            1.,
            &mapping,
            &[],
            CssRect {
                x: 0.,
                y: 0.,
                width: 2.,
                height: 2.,
            },
            |_, _| [255, 0, 0, 255],
        );
        assert_eq!(pixels[3], 0);
        assert!(pixels[(1 * 4 + 1) * 4 + 3] > 0);
        assert_eq!(pixels[(3 * 4 + 3) * 4 + 3], 0);
    }
    #[test]
    fn homogeneous_scale_does_not_overflow_matrix_inversion() {
        for factor in [1e200, 1e-200] {
            let mapping = Mapping::new(IDENTITY.map(|v| v * factor)).unwrap();
            assert_eq!(mapping.source(3., 7.), Some((3., 7.)));
        }
        let mut exchanged = IDENTITY;
        exchanged[0] = 0.;
        exchanged[5] = 0.;
        exchanged[1] = 1.;
        exchanged[4] = 1.;
        assert_eq!(
            Mapping::new(exchanged).unwrap().source(3., 7.),
            Some((7., 3.))
        );
        let behind = Mapping::new(IDENTITY.map(|v| -v)).unwrap();
        assert!(behind.source(3., 7.).is_none());
    }

    #[test]
    fn singular_projection_has_no_coverage() {
        let mut matrix = IDENTITY;
        matrix[0] = 0.;
        assert!(Mapping::new(matrix).is_none());
    }
}

/// Finite physical paint bounds, including real shaped glyph ink. This is used
/// to allocate offscreen native HTML composition surfaces without truncating
/// overflow-visible descendants to their formatting viewport.
pub(super) fn ink_bounds(
    snapshot: &PaintSnapshot,
    commands: &[PaintCommand],
    scale: f64,
) -> Result<Option<CssRect>, RenderError> {
    fn union(a: CssRect, b: CssRect) -> CssRect {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        CssRect {
            x,
            y,
            width: a.right().max(b.right()) - x,
            height: a.bottom().max(b.bottom()) - y,
        }
    }
    fn project(rect: CssRect, m: [f64; 16]) -> Result<CssRect, RenderError> {
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (x, y) in [
            (rect.x, rect.y),
            (rect.right(), rect.y),
            (rect.right(), rect.bottom()),
            (rect.x, rect.bottom()),
        ] {
            let w = m[3] * x + m[7] * y + m[15];
            if !w.is_finite() || w <= 0. {
                return Err(RenderError::InvalidPrint(
                    "offscreen paint crosses an unbounded perspective plane".into(),
                ));
            }
            let (x, y) = (
                (m[0] * x + m[4] * y + m[12]) / w,
                (m[1] * x + m[5] * y + m[13]) / w,
            );
            if !x.is_finite() || !y.is_finite() {
                return Err(RenderError::InvalidPrint(
                    "offscreen paint has nonfinite bounds".into(),
                ));
            }
            min.0 = min.0.min(x);
            min.1 = min.1.min(y);
            max.0 = max.0.max(x);
            max.1 = max.1.max(y);
        }
        Ok(CssRect {
            x: min.0,
            y: min.1,
            width: max.0 - min.0,
            height: max.1 - min.1,
        })
    }
    fn walk(
        snapshot: &PaintSnapshot,
        commands: &[PaintCommand],
        matrix: [f64; 16],
        clips: &[oryn_common::v4::PaintClip],
        scale: f64,
        font: &Font,
        cache: &mut RasterCache,
        total: &mut Option<CssRect>,
        depth: usize,
    ) -> Result<(), RenderError> {
        if depth > 256 {
            return Err(RenderError::InvalidPrint(
                "offscreen paint exceeds depth 256".into(),
            ));
        }
        for command in commands {
            let mut boxes = Vec::new();
            match command {
                PaintCommand::BackdropFilter { rect, plane, .. } => {
                    let rect = project(*rect, multiply(matrix, *plane))?;
                    *total = Some(total.map(|old| union(old, rect)).unwrap_or(rect));
                    continue;
                }
                PaintCommand::Mask { commands, .. } => {
                    walk(
                        snapshot,
                        commands,
                        matrix,
                        clips,
                        scale,
                        font,
                        cache,
                        total,
                        depth + 1,
                    )?;
                    continue;
                }
                PaintCommand::Filter {
                    reference_box,
                    source,
                    current_color,
                    plane,
                    commands,
                    environment,
                } => {
                    let _environment = environment.enter();
                    let base = Mapping::new(*plane)
                        .ok_or_else(|| RenderError::Encode("singular filter paint plane".into()))?;
                    let q = base.inverse;
                    let inverse = [
                        q[0], q[3], 0., q[6], q[1], q[4], 0., q[7], 0., 0., 1., 0., q[2], q[5], 0.,
                        q[8],
                    ];
                    let mut bounds = None;
                    walk(
                        snapshot,
                        commands,
                        inverse,
                        &[],
                        scale,
                        font,
                        cache,
                        &mut bounds,
                        depth + 1,
                    )?;
                    if let Some(reference) = reference_box.or(bounds) {
                        let rect = bounds
                            .map(|ink| effect_union(ink, reference))
                            .unwrap_or(reference);
                        let padding =
                            crate::filters::padding(snapshot, source, current_color, reference)?;
                        let expanded = CssRect {
                            x: rect.x - padding,
                            y: rect.y - padding,
                            width: rect.width + padding * 2.,
                            height: rect.height + padding * 2.,
                        };
                        let expanded = project(expanded, multiply(matrix, *plane))?;
                        *total = Some(total.map(|old| union(old, expanded)).unwrap_or(expanded));
                    }
                    continue;
                }
                PaintCommand::Opacity { opacity, commands } => {
                    if *opacity > 0. {
                        walk(
                            snapshot,
                            commands,
                            matrix,
                            clips,
                            scale,
                            font,
                            cache,
                            total,
                            depth + 1,
                        )?;
                    }
                    continue;
                }
                PaintCommand::Projected {
                    matrix: local,
                    clips: local_clips,
                    commands,
                } => {
                    let mut nested = clips.to_vec();
                    nested.extend(local_clips.iter().cloned().map(|mut clip| {
                        clip.matrix = multiply(matrix, clip.matrix);
                        clip
                    }));
                    walk(
                        snapshot,
                        commands,
                        multiply(matrix, *local),
                        &nested,
                        scale,
                        font,
                        cache,
                        total,
                        depth + 1,
                    )?;
                    continue;
                }
                PaintCommand::TextClip { owner, .. } => {
                    let (extra, mask) = text_clip_mask(snapshot, *owner, matrix)?;
                    walk(
                        snapshot,
                        &mask,
                        extra,
                        clips,
                        scale,
                        font,
                        cache,
                        total,
                        depth + 1,
                    )?;
                    continue;
                }
                PaintCommand::Canvas { .. } => {
                    let rect = CssRect {
                        x: 0.,
                        y: 0.,
                        width: snapshot.viewport.width,
                        height: snapshot.viewport.height,
                    };
                    *total = Some(total.map(|current| union(current, rect)).unwrap_or(rect));
                    continue;
                }
                PaintCommand::BoxShadow { rect, shadow, .. } => {
                    boxes.push(shadows::bounds(*rect, shadow))
                }
                PaintCommand::FillRect { rect, color }
                | PaintCommand::FillRoundedRect { rect, color, .. } => {
                    if color[3] > 0 {
                        boxes.push(*rect);
                    }
                }
                PaintCommand::EmbeddedDocument { rect, .. }
                | PaintCommand::Background { rect, .. }
                | PaintCommand::Border { rect, .. }
                | PaintCommand::BorderEdges { rect, .. }
                | PaintCommand::LinearGradient { rect, .. }
                | PaintCommand::Gradient { rect, .. }
                | PaintCommand::Image { rect, .. }
                | PaintCommand::SvgMask { rect, .. }
                | PaintCommand::SvgImage { rect, .. }
                | PaintCommand::BuiltinImage { rect, .. } => boxes.push(*rect),
                PaintCommand::Text { fragment, .. } => {
                    if fragment.glyph_runs.is_empty() {
                        let mut pen = fragment.rect.x;
                        let size = (fragment.font_size * scale) as f32;
                        for ch in fragment.text.chars() {
                            if matches!(ch, '\n' | '\r') {
                                continue;
                            }
                            if ch == '\t' {
                                pen +=
                                    f64::from(font.metrics(' ', size).advance_width) / scale * 4.;
                                continue;
                            }
                            let glyph = cache.glyph(font, ch, size)?;
                            let metrics = &glyph.0;
                            boxes.push(CssRect {
                                x: pen + f64::from(metrics.xmin) / scale,
                                y: fragment.rect.y + fragment.font_size
                                    - (metrics.height as f64 + f64::from(metrics.ymin)) / scale,
                                width: metrics.width as f64 / scale,
                                height: metrics.height as f64 / scale,
                            });
                            pen += f64::from(metrics.advance_width) / scale;
                        }
                    } else {
                        for run in &fragment.glyph_runs {
                            let text_boxes = crate::text_boxes::commands(snapshot, fragment, run)?;
                            if !text_boxes.is_empty() {
                                if let Some(rect) = ink_bounds(snapshot, &text_boxes, scale)? {
                                    boxes.push(rect);
                                }
                            }
                            if let Some((rect, color)) =
                                crate::decorations::background(fragment, run)?
                            {
                                if color[3] != 0 {
                                    boxes.push(rect);
                                }
                            }
                            boxes.extend(
                                crate::decorations::strokes(snapshot, fragment, run)?
                                    .into_iter()
                                    .map(|stroke| stroke.rect),
                            );
                            for glyph in &run.glyphs {
                                let rendered = cache.indexed_glyph(
                                    snapshot,
                                    run,
                                    glyph.id,
                                    (run.font_size * scale) as f32,
                                )?;
                                let metrics = &rendered.0;
                                boxes.push(oriented_glyph_bounds(
                                    CssRect {
                                        x: f64::from(metrics.xmin) / scale * run.scale_x,
                                        y: -(metrics.height as f64 + f64::from(metrics.ymin))
                                            / scale,
                                        width: metrics.width as f64 / scale * run.scale_x,
                                        height: metrics.height as f64 / scale,
                                    },
                                    (fragment.rect.x + glyph.x, fragment.rect.y + glyph.y),
                                    run.orientation,
                                ));
                            }
                        }
                    }
                }
            }
            for rect in boxes {
                if rect.width <= 0. || rect.height <= 0. {
                    continue;
                }
                let mut rect = project(rect, matrix)?;
                for clip in clips {
                    if clip.clip_x && clip.clip_y {
                        let bound = project(clip.rect, clip.matrix)?;
                        let x = rect.x.max(bound.x);
                        let y = rect.y.max(bound.y);
                        rect = CssRect {
                            x,
                            y,
                            width: (rect.right().min(bound.right()) - x).max(0.),
                            height: (rect.bottom().min(bound.bottom()) - y).max(0.),
                        };
                    } else if clip.matrix[1] == 0.
                        && clip.matrix[4] == 0.
                        && clip.matrix[3] == 0.
                        && clip.matrix[7] == 0.
                    {
                        let bound = project(clip.rect, clip.matrix)?;
                        if clip.clip_x {
                            let right = rect.right().min(bound.right());
                            rect.x = rect.x.max(bound.x);
                            rect.width = (right - rect.x).max(0.);
                        }
                        if clip.clip_y {
                            let bottom = rect.bottom().min(bound.bottom());
                            rect.y = rect.y.max(bound.y);
                            rect.height = (bottom - rect.y).max(0.);
                        }
                    }
                }
                if rect.width > 0. && rect.height > 0. {
                    *total = Some(total.map(|current| union(current, rect)).unwrap_or(rect));
                }
            }
        }
        Ok(())
    }
    let font = Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default())
        .map_err(|error| RenderError::Font(error.to_string()))?;
    let mut total = None;
    walk(
        snapshot,
        commands,
        [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        &[],
        scale,
        &font,
        &mut RasterCache::default(),
        &mut total,
        0,
    )?;
    Ok(total)
}

#[cfg(test)]
mod horizon_bounds_tests {
    use super::*;
    #[test]
    fn entirely_behind_camera_has_empty_paint_bounds() {
        let mapping = Mapping::new([
            -1., 0., 0., 0., 0., -1., 0., 0., 0., 0., 1., 0., 0., 0., 0., -1.,
        ])
        .unwrap();
        assert_eq!(
            mapping.bounds(
                CssRect {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.
                },
                1.,
                100,
                100
            ),
            (0, 0, 0, 0)
        );
    }
    #[test]
    fn horizon_crossing_is_clipped_in_homogeneous_viewport() {
        let mapping = Mapping::new([
            1., 0., 0., -0.02, 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ])
        .unwrap();
        let rect = CssRect {
            x: 0.,
            y: 10.,
            width: 100.,
            height: 10.,
        };
        let bounds = mapping.bounds(rect, 1., 100, 100);
        assert_eq!((bounds.0, bounds.2), (0, 100));
        assert!((9..=10).contains(&bounds.1));
        assert!((60..=61).contains(&bounds.3));
        // x/(1-.02x)=100 gives native x=100/3; the far y=20
        // projects to 60. The behind-camera half cannot enlarge this area.
        assert!(mapping.source(99., 30.).is_some());
    }
    #[test]
    fn native_hit_projection_and_raster_camera_halfplane_agree() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        let mut node = crate::tests::paint_node(1, None, "div", rect);
        let projection = node.projection.as_mut().unwrap();
        projection.matrix[3] = 0.05;
        let mapping = Mapping::new(projection.matrix).unwrap();
        let bounds = oryn_common::layout_domain::projected_bounds(projection.matrix, rect).unwrap();
        assert!(bounds.rect.right() <= oryn_common::layout_domain::MAX_SCROLL_COORDINATE);
        for (x, y) in [(0., 0.), (10., 5.), (19., 20.), (20., 20.), (30., 5.)] {
            let native = projection.viewport_to_local(x, y);
            let raster = mapping.source(x, y);
            assert_eq!(native.is_some(), raster.is_some());
            if let (Some(a), Some(b)) = (native, raster) {
                assert!((a.0 - b.0).abs() < 0.001);
                assert!((a.1 - b.1).abs() < 0.001);
            }
        }
    }

    #[test]
    fn projective_density_bound_contains_visible_derivatives() {
        let matrix = [
            2., 0.2, 0., 0.002, 0.3, 1.5, 0., -0.001, 0., 0., 1., 0., 10., 20., 0., 1.,
        ];
        let mapping = Mapping::new(matrix).unwrap();
        let viewport = CssRect {
            x: 0.,
            y: 0.,
            width: 320.,
            height: 240.,
        };
        let bound = mapping.density_bound(viewport);
        for x in 0..=16 {
            for y in 0..=12 {
                if let Some((x, y)) = mapping.source(x as f64 * 20., y as f64 * 20.) {
                    let density = mapping.scale_at(x, y);
                    assert!(density.0 <= bound.0 + 1e-9);
                    assert!(density.1 <= bound.1 + 1e-9);
                }
            }
        }
        let affine = Mapping::new([
            2., 0., 0., 0., 0., 3., 0., 0., 0., 0., 1., 0., 10., 20., 0., 1.,
        ])
        .unwrap();
        let exact = affine.density_bound(viewport);
        assert!((exact.0 - 2.).abs() < 1e-9);
        assert!((exact.1 - 3.).abs() < 1e-9);
    }
    #[test]
    fn cross_fade_tree_nodes_share_decoded_resource_budget() {
        let source = BackgroundSource::Solid([255; 4]);
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let mut budget = 64 * 1024 * 1024;
        assert!(
            PreparedBackground::new(
                &source,
                &snapshot,
                rect,
                1.,
                &mut RasterCache::default(),
                &mut budget,
                0
            )
            .is_err()
        );
    }
    #[test]
    fn cross_fade_retains_missing_alpha_instead_of_opaque_color_average() {
        let source = BackgroundSource::CrossFade(vec![
            (0.25, BackgroundSource::Solid([255, 0, 0, 255])),
            (0.25, BackgroundSource::Solid([0, 0, 255, 255])),
        ]);
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let prepared = PreparedBackground::new(
            &source,
            &snapshot,
            rect,
            1.,
            &mut RasterCache::default(),
            &mut 0,
            0,
        )
        .unwrap();
        assert_eq!(prepared.sample(5., 5., rect, (1., 1.)), [128, 0, 128, 128]);
    }

    #[test]
    fn background_clip_text_uses_transformed_descendant_glyph_ink_even_when_text_is_transparent() {
        use oryn_common::v4::{
            PaintFontResource, PaintGlyph, PaintGlyphOrientation, PaintGlyphRun,
        };
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.fonts.insert(
            "mask-font".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font =
            Font::from_bytes(oryn_layout::DEFAULT_FONT_BYTES, FontSettings::default()).unwrap();
        let fragment = PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: "F".into(),
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 20.,
                height: 25.,
            },
            font_family: "native".into(),
            font_size: 20.,
            color: "transparent".into(),
            glyph_runs: vec![PaintGlyphRun {
                line_box: None,
                text_box: None,
                background: None,
                decoration_rect: None,
                scale_x: 1.,
                vertical: false,
                decorations: Vec::new(),
                orientation: PaintGlyphOrientation::Upright,
                font_key: "mask-font".into(),
                font_size: 20.,
                color: Some([0; 4]),
                embolden: false,
                skew_degrees: 0.,
                normalized_coords: Vec::new(),
                glyphs: vec![PaintGlyph {
                    source: None,
                    id: u32::from(font.lookup_glyph_index('F')),
                    x: 0.,
                    y: 20.,
                    advance: 12.,
                }],
            }],
        };
        let mut owner = crate::tests::paint_node(
            1,
            None,
            "div",
            CssRect {
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
        );
        owner.style.insert("background-clip".into(), "text".into());
        owner.style.insert("background-color".into(), "red".into());
        let mut child = crate::tests::paint_node(2, Some(1), "span", fragment.rect);
        child.projection.as_mut().unwrap().matrix[12] = 10.;
        child.projection.as_mut().unwrap().text_matrix = None;
        child
            .projection
            .as_mut()
            .unwrap()
            .text
            .push(fragment.clone());
        child.text.push(fragment);
        snapshot.nodes = vec![owner, child];
        let options = RenderOptions::default();
        let scene = build_display_list(&snapshot, options).unwrap();
        let pixels = rasterize(&snapshot, &scene, options).unwrap();
        let red = pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[0] > 200 && pixel[1] < 100 && pixel[2] < 100)
            .map(|(index, _)| index % scene.width as usize)
            .collect::<Vec<_>>();
        assert!(red.len() > 10);
        assert!(red.iter().all(|x| *x >= 10));
        assert_eq!(&pixels[0..4], &[255, 255, 255, 255]);
        let tree = crate::print::vector::tree(&snapshot, &scene, options).unwrap();
        assert!(
            tree.to_string(&resvg::usvg::WriteOptions::default())
                .contains("data:image/png")
        );
    }
}

fn effect_union(a: CssRect, b: CssRect) -> CssRect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    CssRect {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}

#[cfg(test)]
mod text_background_tests {
    use super::*;
    use oryn_common::v4::{PaintGlyphOrientation, PaintGlyphRun, PaintTextFragment};
    #[test]
    fn typed_run_background_rasterizes_and_rejects_nonfinite_geometry() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.viewport.width = 16.;
        snapshot.viewport.height = 16.;
        let mut fragment = PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: " ".into(),
            rect: CssRect {
                x: 2.,
                y: 3.,
                width: 8.,
                height: 7.,
            },
            font_family: "native".into(),
            font_size: 12.,
            color: "transparent".into(),
            glyph_runs: vec![PaintGlyphRun {
                line_box: None,
                text_box: None,
                background: Some([255, 0, 0, 255]),
                decoration_rect: Some(CssRect {
                    x: 0.,
                    y: 0.,
                    width: 8.,
                    height: 7.,
                }),
                scale_x: 1.,
                vertical: false,
                decorations: Vec::new(),
                orientation: PaintGlyphOrientation::Upright,
                embolden: false,
                skew_degrees: 0.,
                normalized_coords: Vec::new(),
                color: None,
                font_key: String::new(),
                font_size: 12.,
                glyphs: Vec::new(),
            }],
        };
        let scene = DisplayList {
            contract_version: crate::RENDER_CONTRACT_VERSION,
            width: 16,
            height: 16,
            diagnostics: Vec::new(),
            commands: vec![PaintCommand::Text {
                fragment: fragment.clone(),
                color: [0; 4],
            }],
        };
        let pixels = crate::rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        assert_eq!(
            &pixels[(4 * 16 + 3) * 4..(4 * 16 + 3) * 4 + 4],
            &[255, 0, 0, 255]
        );
        let content = CssRect {
            x: 0.,
            y: 0.,
            width: 8.,
            height: 7.,
        };
        let border = CssRect {
            x: -1.,
            y: -1.,
            width: 10.,
            height: 9.,
        };
        fragment.glyph_runs[0].background = None;
        fragment.glyph_runs[0].text_box = Some(oryn_common::v4::PaintTextBox {
            owner: 1,
            model: oryn_common::v4::BoxModel {
                content_box: content,
                padding_box: content,
                border_box: border,
                padding: Default::default(),
                margin: Default::default(),
                border: oryn_common::v4::EdgeSizes {
                    top: 1.,
                    right: 1.,
                    bottom: 1.,
                    left: 1.,
                },
            },
            style: std::collections::BTreeMap::from([
                ("background-color".into(), "red".into()),
                ("border-top-style".into(), "solid".into()),
                ("border-top-color".into(), "blue".into()),
            ]),
        });
        let scene = DisplayList {
            commands: vec![PaintCommand::Text {
                fragment: fragment.clone(),
                color: [0; 4],
            }],
            ..scene
        };
        let pixels = crate::rasterize(&snapshot, &scene, RenderOptions::default()).unwrap();
        assert_eq!(
            &pixels[(4 * 16 + 3) * 4..(4 * 16 + 3) * 4 + 4],
            &[255, 0, 0, 255]
        );
        assert_eq!(
            &pixels[(2 * 16 + 3) * 4..(2 * 16 + 3) * 4 + 4],
            &[0, 0, 255, 255]
        );
        fragment.glyph_runs[0]
            .text_box
            .as_mut()
            .unwrap()
            .model
            .border_box
            .width = f64::INFINITY;
        assert!(
            crate::text_boxes::commands(&snapshot, &fragment, &fragment.glyph_runs[0]).is_err()
        );
        fragment.glyph_runs[0].background = Some([255, 0, 0, 255]);
        fragment.glyph_runs[0]
            .decoration_rect
            .as_mut()
            .unwrap()
            .width = f64::NAN;
        assert!(crate::decorations::background(&fragment, &fragment.glyph_runs[0]).is_err());
    }
}

#[cfg(test)]
mod svg_environment_cache_tests {
    use super::*;
    #[test]
    fn same_resource_separate_consumers_and_preference_mutations_never_reuse_wrong_svg() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.images.push(oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url_aliases:Vec::new(),url:"https://example.test/shared.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><style>rect{fill:red}@media(prefers-color-scheme:dark){rect{fill:blue}}@media(prefers-reduced-motion:reduce){rect{fill:lime}}</style><rect width="10" height="10"/></svg>"#.to_vec()});
        let mut cache = RasterCache::default();
        let light = SvgEnvironment {
            width: 10.,
            height: 10.,
            ..SvgEnvironment::default()
        };
        let dark = SvgEnvironment {
            dark: true,
            ..light
        };
        let reduce = SvgEnvironment {
            reduced_motion: true,
            ..light
        };
        for (environment, expected) in [
            (light, [255, 0, 0, 255]),
            (dark, [0, 0, 255, 255]),
            (reduce, [0, 255, 0, 255]),
            (light, [255, 0, 0, 255]),
        ] {
            let image = cache.svg(0, &snapshot, 20, 20, environment).unwrap();
            assert_eq!(image.get_pixel(5, 5).0, expected);
        }
        assert_eq!(cache.svg_trees.len(), 3);
        assert_eq!(cache.svg_images.len(), 3);
        for environment in [
            SvgEnvironment {
                width: 20.,
                ..light
            },
            SvgEnvironment {
                device_pixel_ratio: 2.,
                ..light
            },
        ] {
            cache.svg(0, &snapshot, 20, 20, environment).unwrap();
        }
        assert_eq!(cache.svg_trees.len(), 5);
    }
}
