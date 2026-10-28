//! Original unhinted glyph outlines for the canvas raster consumer.
use super::*;
pub use skrifa::outline::OutlinePen;
use skrifa::{GlyphId, outline::DrawSettings};

struct ControlledPen<'a, P> {
    target: &'a mut P,
    shape: &'a CanvasTextShape,
    cancelled: bool,
}
impl<P: OutlinePen> ControlledPen<'_, P> {
    fn active(&mut self) -> bool {
        self.cancelled |= self.shape.checkpoint().is_err();
        !self.cancelled
    }
}
impl<P: OutlinePen> OutlinePen for ControlledPen<'_, P> {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.active() {
            self.target.move_to(x, y);
        }
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.active() {
            self.target.line_to(x, y);
        }
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if self.active() {
            self.target.quad_to(cx, cy, x, y);
        }
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        if self.active() {
            self.target.curve_to(ax, ay, bx, by, x, y);
        }
    }
    fn close(&mut self) {
        if self.active() {
            self.target.close();
        }
    }
}

impl CanvasTextShape {
    /// Emits original font-local coordinates (y up). The raster consumer must
    /// apply glyph x/y, run scale/skew/emboldening and the canvas anchor/CTM.
    /// Treat the destination as provisional until this returns success.
    pub fn draw_glyph_outline(
        &self,
        run_index: usize,
        glyph_index: usize,
        target: &mut impl OutlinePen,
    ) -> Result<(), LayoutError> {
        self.checkpoint()?;
        let run = self
            .glyph_runs
            .get(run_index)
            .ok_or_else(|| LayoutError::InvalidRenderedTree("canvas outline run index".into()))?;
        let glyph = run
            .glyphs
            .get(glyph_index)
            .ok_or_else(|| LayoutError::InvalidRenderedTree("canvas outline glyph index".into()))?;
        if !run.font_size.is_finite()
            || run.font_size <= 0.
            || run.font_size > f64::from(f32::MAX)
            || run
                .normalized_coords
                .iter()
                .any(|v| !(-16384..=16384).contains(v))
        {
            return Err(LayoutError::InvalidRenderedTree(
                "invalid canvas outline instance".into(),
            ));
        }
        let source = self.fonts.get(&run.font_key).ok_or_else(|| {
            LayoutError::InvalidRenderedTree("canvas outline font is missing".into())
        })?;
        let face = FontRef::from_index(&source.bytes, source.collection_index)
            .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
        let outlines = face.outline_glyphs();
        let glyph = outlines.get(GlyphId::new(glyph.id)).ok_or_else(|| {
            LayoutError::InvalidRenderedTree("canvas glyph outline is missing".into())
        })?;
        let coordinates = run
            .normalized_coords
            .iter()
            .map(|v| NormalizedCoord::from_bits(*v))
            .collect::<Vec<_>>();
        let mut pen = ControlledPen {
            target,
            shape: self,
            cancelled: false,
        };
        glyph
            .draw(
                DrawSettings::unhinted(
                    Size::new(run.font_size as f32),
                    LocationRef::new(&coordinates),
                ),
                &mut pen,
            )
            .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
        if pen.cancelled {
            return Err(LayoutError::Cancelled);
        }
        self.checkpoint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emitted_original_glyph_outline_matches_native_ink_and_rejects_bad_indices() {
        let shape = super::super::shape(
            "A",
            &BTreeMap::from([("font-size".into(), "20px".into())]),
            ViewportState::default(),
            &[],
        )
        .unwrap();
        let mut pen = super::super::ink::Pen::default();
        shape.draw_glyph_outline(0, 0, &mut pen).unwrap();
        let b = pen.bounds.unwrap();
        let run = &shape.glyph_runs[0];
        let glyph = &run.glyphs[0];
        let placed = [
            glyph.x + b[0],
            glyph.y - b[3],
            glyph.x + b[2],
            glyph.y - b[1],
        ];
        assert_eq!(Some(placed), shape.ink_bounds().unwrap());
        assert!(shape.draw_glyph_outline(usize::MAX, 0, &mut pen).is_err());
        assert!(shape.draw_glyph_outline(0, usize::MAX, &mut pen).is_err());
    }
}
