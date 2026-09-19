//! Native decoration metrics and continuous line pattern coverage.
use crate::RenderError;
use oryn_common::v4::{CssRect, PaintGlyphRun, PaintSnapshot, PaintTextFragment};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
};
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stroke {
    pub rect: CssRect,
    pub vertical: bool,
    pub thickness: f64,
    pub style: String,
    pub color: [u8; 4],
    pub skip_ink: bool,
    pub after: bool,
}
impl Stroke {
    pub fn sample(&self, x: f64, y: f64) -> [u8; 4] {
        let (inline, block) = if self.vertical {
            (y - self.rect.y, x - self.rect.x)
        } else {
            (x - self.rect.x, y - self.rect.y)
        };
        let t = self.thickness;
        let covered = match self.style.as_str() {
            "double" => block < t || block >= 2. * t,
            "dashed" => inline.rem_euclid(6. * t) < 3. * t,
            "dotted" => {
                let a = inline.rem_euclid(2. * t) - t;
                let b = block - t / 2.;
                a * a + b * b <= t * t / 4.
            }
            "wavy" => {
                let center = 1.5 * t + t * (inline * std::f64::consts::TAU / (4. * t)).sin();
                (block - center).abs() <= t / 2.
            }
            _ => true,
        };
        if covered { self.color } else { [0; 4] }
    }
}
pub(super) fn background(
    fragment: &PaintTextFragment,
    run: &PaintGlyphRun,
) -> Result<Option<(CssRect, [u8; 4])>, RenderError> {
    let Some(color) = run.background else {
        return Ok(None);
    };
    let rect = run
        .decoration_rect
        .ok_or_else(|| RenderError::Font("text background has no native run box".into()))?;
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        || rect.width < 0.
        || rect.height < 0.
    {
        return Err(RenderError::Font("invalid text background run box".into()));
    }
    Ok(Some((
        CssRect {
            x: fragment.rect.x + rect.x,
            y: fragment.rect.y + rect.y,
            ..rect
        },
        color,
    )))
}
pub(super) fn strokes(
    snapshot: &PaintSnapshot,
    fragment: &PaintTextFragment,
    run: &PaintGlyphRun,
) -> Result<Vec<Stroke>, RenderError> {
    if run.decorations.is_empty() || run.glyphs.is_empty() {
        return Ok(Vec::new());
    }
    let source = snapshot
        .fonts
        .get(&run.font_key)
        .ok_or_else(|| RenderError::Font("decoration font missing".into()))?;
    let font = FontRef::from_index(&source.bytes, source.collection_index)
        .map_err(|e| RenderError::Font(e.to_string()))?;
    let coords = run
        .normalized_coords
        .iter()
        .map(|c| NormalizedCoord::from_bits(*c))
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    for decoration in &run.decorations {
        if !decoration.font_size.is_finite()
            || decoration.font_size <= 0.
            || decoration.font_size > 1_000_000.
        {
            return Err(RenderError::Font("invalid decoration font size".into()));
        }
        let metrics = font.metrics(
            Size::new(decoration.font_size as f32),
            LocationRef::new(&coords),
        );
        let fallback = decoration.font_size / 16.;
        let thickness = decoration.thickness.unwrap_or_else(|| {
            metrics
                .underline
                .map(|m| f64::from(m.thickness))
                .unwrap_or(fallback)
        });
        if !thickness.is_finite() || thickness < 0. || thickness > 1_000_000. {
            return Err(RenderError::Font("invalid decoration thickness".into()));
        }
        if thickness == 0. {
            continue;
        }
        let extent = if matches!(decoration.style.as_str(), "double" | "wavy") {
            thickness * 3.
        } else {
            thickness
        };
        if run.decoration_rect.is_some_and(|rect| {
            ![rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
                || rect.width < 0.
                || rect.height < 0.
        }) {
            return Err(RenderError::Font("invalid decoration run box".into()));
        }
        let start = run
            .decoration_rect
            .map(|rect| if run.vertical { rect.y } else { rect.x })
            .unwrap_or_else(|| {
                run.glyphs
                    .iter()
                    .map(|g| if run.vertical { g.y } else { g.x })
                    .fold(f64::INFINITY, f64::min)
            });
        let end = run
            .decoration_rect
            .map(|rect| {
                if run.vertical {
                    rect.bottom()
                } else {
                    rect.right()
                }
            })
            .unwrap_or_else(|| {
                run.glyphs
                    .iter()
                    .map(|g| {
                        if run.vertical {
                            g.y + g.advance
                        } else {
                            g.x + g.advance
                        }
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            });
        let baseline = if run.vertical {
            run.glyphs[0].x
        } else {
            run.glyphs[0].y
        };
        for line in decoration.line.split_whitespace() {
            let after = line == "line-through";
            let block = match line {
                "underline" => decoration.underline_offset.unwrap_or_else(|| {
                    if decoration.position.split_whitespace().any(|p| p == "under") {
                        f64::from(metrics.descent).abs()
                    } else {
                        metrics
                            .underline
                            .map(|m| -f64::from(m.offset))
                            .unwrap_or(decoration.font_size / 8.)
                    }
                }),
                "overline" => -f64::from(metrics.ascent) - extent,
                "line-through" => metrics
                    .strikeout
                    .map(|m| -f64::from(m.offset))
                    .unwrap_or(-decoration.font_size * 0.3),
                _ => continue,
            };
            if !block.is_finite() {
                return Err(RenderError::Font("invalid decoration offset".into()));
            }
            let rect = if run.vertical {
                let right = decoration.position.split_whitespace().any(|p| p == "right");
                // Upright vertical glyph origins are on the ideographic center;
                // place under/over on opposite sides of that native center.
                let box_rect = run.decoration_rect.unwrap_or(CssRect {
                    x: baseline - decoration.font_size * 0.5,
                    y: start,
                    width: decoration.font_size,
                    height: end - start,
                });
                let offset = decoration.underline_offset.unwrap_or(0.);
                let edge = if after {
                    box_rect.x + box_rect.width / 2. - extent / 2.
                } else if (line == "underline") == right {
                    box_rect.right() + offset
                } else {
                    box_rect.x - offset - extent
                };
                CssRect {
                    x: fragment.rect.x + edge,
                    y: fragment.rect.y + start,
                    width: extent,
                    height: (end - start).max(0.),
                }
            } else {
                CssRect {
                    x: fragment.rect.x + start,
                    y: fragment.rect.y + baseline + block,
                    width: (end - start).max(0.),
                    height: extent,
                }
            };
            output.push(Stroke {
                rect,
                vertical: run.vertical,
                thickness,
                style: decoration.style.clone(),
                color: decoration.color,
                skip_ink: decoration.skip_ink && !after,
                after,
            });
        }
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patterns_have_gaps_and_double_has_two_lines() {
        let mut stroke = Stroke {
            rect: CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 6.,
            },
            vertical: false,
            thickness: 2.,
            style: "double".into(),
            color: [255, 0, 0, 255],
            skip_ink: false,
            after: false,
        };
        assert_eq!(stroke.sample(1., 1.)[3], 255);
        assert_eq!(stroke.sample(1., 3.)[3], 0);
        assert_eq!(stroke.sample(1., 5.)[3], 255);
        stroke.style = "dashed".into();
        assert_eq!(stroke.sample(2., 1.)[3], 255);
        assert_eq!(stroke.sample(8., 1.)[3], 0);
        stroke.style = "wavy".into();
        assert_eq!(stroke.sample(0., 3.)[3], 255);
        assert_eq!(stroke.sample(0., 0.)[3], 0);
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use oryn_common::v4::{
        PaintFontResource, PaintGlyph, PaintGlyphOrientation, PaintTextDecoration,
    };
    #[test]
    fn native_font_metrics_and_upright_vertical_decoration_have_real_paint_bounds() {
        let mut snapshot = crate::tests::empty_snapshot();
        snapshot.fonts.insert(
            "face".into(),
            PaintFontResource {
                bytes: oryn_layout::DEFAULT_FONT_BYTES.to_vec(),
                collection_index: 0,
            },
        );
        let font = fontdue::Font::from_bytes(
            oryn_layout::DEFAULT_FONT_BYTES,
            fontdue::FontSettings::default(),
        )
        .unwrap();
        let mut run = PaintGlyphRun {
            line_box: None,
            text_box: None,
            background: None,
            decoration_rect: None,
            scale_x: 1.,
            vertical: false,
            decorations: vec![PaintTextDecoration {
                line: "underline overline line-through".into(),
                style: "solid".into(),
                color: [255, 0, 0, 255],
                thickness: Some(2.),
                underline_offset: None,
                position: "auto".into(),
                skip_ink: true,
                font_size: 20.,
            }],
            orientation: PaintGlyphOrientation::Upright,
            embolden: false,
            skew_degrees: 0.,
            normalized_coords: Vec::new(),
            color: None,
            font_key: "face".into(),
            font_size: 20.,
            glyphs: vec![PaintGlyph {
                source: None,
                id: u32::from(font.lookup_glyph_index('g')),
                x: 0.,
                y: 20.,
                advance: 12.,
            }],
        };
        let fragment = PaintTextFragment {
            flow_line: None,
            flow_rect: None,
            clusters: Vec::new(),
            text: "g".into(),
            rect: CssRect {
                x: 5.,
                y: 5.,
                width: 12.,
                height: 24.,
            },
            font_family: "face".into(),
            font_size: 20.,
            color: "black".into(),
            glyph_runs: vec![],
        };
        let horizontal = strokes(&snapshot, &fragment, &run).unwrap();
        assert_eq!(horizontal.len(), 3);
        assert!(horizontal[0].rect.y > 25.);
        assert!(horizontal[1].rect.y < 25.);
        assert!(horizontal[2].after);
        run.vertical = true;
        run.glyphs[0].x = 10.;
        run.glyphs[0].y = 0.;
        let vertical = strokes(&snapshot, &fragment, &run).unwrap();
        assert_eq!(vertical[0].rect.height, 12.);
        assert_eq!(vertical[0].rect.width, 2.);
        assert!(vertical[0].rect.x < 15.);
    }
}
