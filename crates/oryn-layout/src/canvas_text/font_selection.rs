//! CSS first-available selection without a glyph/cmap requirement.
use super::*;
use crate::{text_features, web_font_faces};

#[derive(Debug)]
pub struct MetricFont {
    pub source: PaintFontResource,
    pub normalized_coords: Vec<i16>,
    pub font_size: f64,
    /// CSS em size before size-adjust scales glyphs and font metrics.
    pub em_size: f64,
    pub metrics: skrifa::metrics::Metrics,
}

impl MetricFont {
    /// CSS font-relative units use the selected face at its variation location.
    /// Missing zero/ideograph glyphs use the CSS .5em/1em fallbacks.
    pub fn spacing_metrics(&self) -> Result<oryn_common::css_math::FontMetricLengths, LayoutError> {
        let face = FontRef::from_index(&self.source.bytes, self.source.collection_index)
            .map_err(|error| LayoutError::InvalidRenderedTree(error.to_string()))?;
        let coords = self.normalized_coords.iter().map(|value| NormalizedCoord::from_bits(*value)).collect::<Vec<_>>();
        let glyphs = face.glyph_metrics(Size::new(self.font_size as f32), LocationRef::new(&coords));
        let advance = |ch| face.charmap().map(ch).and_then(|id| glyphs.advance_width(id)).map(f64::from);
        let values = oryn_common::css_math::FontMetricLengths {
            x_height: self.metrics.x_height.map(f64::from).filter(|value| *value > 0.).unwrap_or(self.em_size * 0.5),
            zero_advance: advance('0').unwrap_or(self.em_size * 0.5),
            ideographic_advance: advance('\u{6c34}').unwrap_or(self.em_size),
            cap_height: self.metrics.cap_height.map(f64::from).filter(|value| *value > 0.).unwrap_or(f64::from(self.metrics.ascent)),
        };
        if [values.x_height, values.zero_advance, values.ideographic_advance, values.cap_height].iter().any(|value| !value.is_finite() || *value < 0.) {
            return Err(LayoutError::InvalidRenderedTree("invalid Canvas spacing font metrics".into()));
        }
        Ok(values)
    }
    /// CSS Inline em-box normalization and native BASE anchors. Missing
    /// hanging baseline uses the specification's suggested .6em fallback.
    pub fn baseline_positions(
        &self,
        script: skrifa::raw::types::Tag,
    ) -> Result<super::metrics::BaselinePositions, LayoutError> {
        use skrifa::raw::{TableProvider, types::Tag};
        let face = FontRef::from_index(&self.source.bytes, self.source.collection_index)
            .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
        let units = face
            .head()
            .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?
            .units_per_em();
        if units == 0 {
            return Err(LayoutError::InvalidRenderedTree(
                "zero font units per em".into(),
            ));
        }
        let baselines = face
            .table_data(Tag::new(b"BASE"))
            .map(|table| {
                super::baselines::read_horizontal(
                    table.as_bytes(),
                    script,
                    self.font_size / f64::from(units),
                    &self.normalized_coords,
                )
            })
            .transpose()?
            .flatten()
            .unwrap_or_default();
        positions(
            self.em_size,
            f64::from(self.metrics.ascent),
            -f64::from(self.metrics.descent),
            &baselines,
        )
    }
}

fn positions(
    size: f64,
    ascent: f64,
    descent: f64,
    table: &BTreeMap<skrifa::raw::types::Tag, f64>,
) -> Result<super::metrics::BaselinePositions, LayoutError> {
    use skrifa::raw::types::Tag;
    if [size, ascent, descent].iter().any(|v| !v.is_finite()) || size < 0. {
        return Err(LayoutError::InvalidRenderedTree(
            "invalid canvas baseline metrics".into(),
        ));
    }
    let roman = table.get(&Tag::new(b"romn")).copied().unwrap_or(0.);
    let read = |tag| table.get(&Tag::new(tag)).map(|v| *v - roman);
    let under = read(b"ideo");
    let over = read(b"idtp");
    let center = match (over, under) {
        (Some(over), Some(under)) => Some(over / 2. + under / 2.),
        (Some(over), None) => Some(over - size / 2.),
        (None, Some(under)) => Some(under + size / 2.),
        _ => None,
    };
    let (em_over, em_under) = if let Some(center) = center {
        (center + size / 2., center - size / 2.)
    } else {
        let sum = ascent + descent;
        if !sum.is_finite() {
            return Err(LayoutError::InvalidRenderedTree(
                "canvas ascent/descent overflow".into(),
            ));
        }
        let top = if sum > 0. {
            size * (ascent / sum)
        } else {
            size / 2.
        };
        (top, top - size)
    };
    let p = super::metrics::BaselinePositions {
        font_ascent: ascent,
        font_descent: descent,
        em_over,
        em_under,
        hanging: read(b"hang").unwrap_or(size * 0.6),
        ideographic_under: under.unwrap_or(em_under),
    };
    if [p.em_over, p.em_under, p.hanging, p.ideographic_under]
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err(LayoutError::InvalidRenderedTree(
            "canvas baseline synthesis overflow".into(),
        ));
    }
    Ok(p)
}

pub(super) fn select(
    engine: &mut TextEngine,
    style: &BTreeMap<String, String>,
    size: f64,
) -> Result<MetricFont, LayoutError> {
    let family_list = web_font_faces::families_for_cluster(&engine.web_faces, style, " ");
    let families = crate::cssom::font_family_names(&family_list);
    let attrs = fontique::Attributes::new(
        fontique::FontWidth::parse_css(style_value(style, "font-stretch", "normal"))
            .unwrap_or_default(),
        fontique::FontStyle::parse_css(style_value(style, "font-style", "normal"))
            .unwrap_or_default(),
        fontique::FontWeight::parse_css(style_value(style, "font-weight", "normal"))
            .unwrap_or_default(),
    );
    let mut selected = None;
    {
        let mut query = engine
            .fonts
            .collection
            .query(&mut engine.fonts.source_cache);
        query.set_families(
            families
                .iter()
                .map(|name| fontique::QueryFamily::Named(name.as_str())),
        );
        query.set_attributes(attrs);
        query.matches_with(|font| {
            selected = Some(font.clone());
            fontique::QueryStatus::Stop
        });
    }
    let selected = selected.ok_or_else(|| {
        LayoutError::InvalidRenderedTree("canvas first available font is missing".into())
    })?;
    let descriptor = engine
        .fonts
        .face_overrides
        .iter()
        .find(|(id, _)| *id == selected.family.0)
        .map(|(_, d)| d);
    let item = text_features::variations(style, size);
    let variations =
        parley::font_instance_variations(&selected.synthesis, attrs, Some(&item), descriptor);
    let face = FontRef::from_index(selected.blob.as_ref(), selected.index)
        .map_err(|e| LayoutError::InvalidRenderedTree(e.to_string()))?;
    let location = face.axes().location(
        variations
            .iter()
            .map(|v| (skrifa::raw::types::Tag::new(&v.tag.to_bytes()), v.value)),
    );
    let effective_size = size * f64::from(descriptor.and_then(|d| d.size_adjust).unwrap_or(1.));
    let mut metrics = face.metrics(Size::new(effective_size as f32), location.coords());
    if let Some(d) = descriptor {
        if let Some(ascent) = d.ascent {
            metrics.ascent = ascent * effective_size as f32;
        }
        if let Some(descent) = d.descent {
            metrics.descent = -descent * effective_size as f32;
        }
        if let Some(gap) = d.line_gap {
            metrics.leading = gap * effective_size as f32;
        }
    }
    Ok(MetricFont {
        source: PaintFontResource {
            bytes: selected.blob.as_ref().to_vec(),
            collection_index: selected.index,
        },
        normalized_coords: location.coords().iter().map(|v| v.to_bits()).collect(),
        font_size: effective_size,
        em_size: size,
        metrics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_baseline_synthesis_normalizes_em_and_honors_base_coordinates() {
        use skrifa::raw::types::Tag;
        let no_base = BTreeMap::new();
        let p = positions(40., 10., 10., &no_base).unwrap();
        assert_eq!((p.em_over, p.em_under), (20., -20.));
        assert_eq!(p.hanging, 24.);
        let p = positions(40., 30., 0., &no_base).unwrap();
        assert_eq!((p.em_over, p.em_under), (40., 0.));
        let base = BTreeMap::from([
            (Tag::new(b"romn"), 100.),
            (Tag::new(b"ideo"), 90.),
            (Tag::new(b"hang"), 125.),
        ]);
        let p = positions(40., 10., 10., &base).unwrap();
        assert_eq!((p.em_over, p.em_under), (30., -10.));
        assert_eq!(p.hanging, 25.);
        assert_eq!(p.ideographic_under, -10.);
    }
    #[test]
    fn metric_font_selection_does_not_require_a_space_glyph() {
        let mut bytes = crate::DEFAULT_FONT_BYTES.to_vec();
        let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let mut removed = false;
        for index in 0..count {
            let offset = 12 + index * 16;
            if &bytes[offset..offset + 4] == b"cmap" {
                let cmap =
                    u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
                let tables =
                    u16::from_be_bytes(bytes[cmap + 2..cmap + 4].try_into().unwrap()) as usize;
                let mut offsets = std::collections::BTreeSet::new();
                for i in 0..tables {
                    let record = cmap + 4 + i * 8;
                    offsets.insert(
                        cmap + u32::from_be_bytes(bytes[record + 4..record + 8].try_into().unwrap())
                            as usize,
                    );
                }
                for table in offsets {
                    let format = u16::from_be_bytes(bytes[table..table + 2].try_into().unwrap());
                    if format == 4 {
                        let segments =
                            u16::from_be_bytes(bytes[table + 6..table + 8].try_into().unwrap())
                                as usize
                                / 2;
                        let ends = table + 14;
                        let starts = ends + segments * 2 + 2;
                        let deltas = starts + segments * 2;
                        let ranges = deltas + segments * 2;
                        for j in 0..segments {
                            let start = u16::from_be_bytes(
                                bytes[starts + j * 2..starts + j * 2 + 2]
                                    .try_into()
                                    .unwrap(),
                            );
                            let end = u16::from_be_bytes(
                                bytes[ends + j * 2..ends + j * 2 + 2].try_into().unwrap(),
                            );
                            if start <= 32 && end >= 32 {
                                let range = u16::from_be_bytes(
                                    bytes[ranges + j * 2..ranges + j * 2 + 2]
                                        .try_into()
                                        .unwrap(),
                                ) as usize;
                                let target = if range == 0 {
                                    deltas + j * 2
                                } else {
                                    ranges + j * 2 + range + (32 - start) as usize * 2
                                };
                                let value = if range == 0 { (-32i16) as u16 } else { 0 };
                                bytes[target..target + 2].copy_from_slice(&value.to_be_bytes());
                            }
                        }
                    }
                }
                removed = true;
            }
        }
        assert!(removed);
        assert!(FontRef::new(&bytes).unwrap().charmap().map(' ').is_none());
        let fonts = vec![("MetricOnly".into(), bytes.clone(), "".into(), false, false)];
        let mut engine = TextEngine::new(&fonts);
        let style = BTreeMap::from([("font-family".into(), "MetricOnly".into())]);
        let font = select(&mut engine, &style, 20.).unwrap();
        assert!(
            font.source.bytes == bytes,
            "metric selection fell back based on glyph coverage"
        );
        assert!(font.metrics.ascent > 0.);
        let excluded = vec![(
            "MetricOnly".into(),
            bytes.clone(),
            "unicode-range:U+0041".into(),
            false,
            false,
        )];
        let font = select(&mut TextEngine::new(&excluded), &style, 20.).unwrap();
        assert!(
            font.source.bytes.as_slice() == crate::DEFAULT_FONT_BYTES,
            "space-excluded face must not supply first-available metrics"
        );
        let overridden = vec![(
            "MetricOnly".into(),
            bytes,
            "ascent-override:50%;descent-override:10%;size-adjust:200%".into(),
            false,
            false,
        )];
        let font = select(&mut TextEngine::new(&overridden), &style, 20.).unwrap();
        assert_eq!(font.font_size, 40.);
        assert_eq!(font.metrics.ascent, 20.);
        assert_eq!(font.metrics.descent, -4.);
        let p = font
            .baseline_positions(skrifa::raw::types::Tag::new(b"latn"))
            .unwrap();
        assert!((p.em_over - p.em_under - 20.).abs() < 1e-9);
    }
}
