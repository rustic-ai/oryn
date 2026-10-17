//! CSS face descriptors remain separate from the font file's metadata. Each
//! face gets a private family so overlapping unicode ranges cannot leak glyphs.
use super::*;
#[derive(Clone)]
pub(super) struct Face {
    pub family: String,
    pub alias: String,
    pub weight: (f32, f32),
    pub width: (f32, f32),
    pub style: String,
    pub coverage: Vec<(u32, u32)>,
    pub descriptors: String,
    pub pending: bool,
    pub blocked: bool,
}
impl Face {
    pub fn new(index: usize, family: &str, descriptors: &str) -> Self {
        let read = |name| {
            cssom::read_font_face_declaration(descriptors, name, "https://oryn.invalid/", false)
                .map(|view| view.value)
                .unwrap_or_default()
        };
        let range = |value: String, default: f32, parse: fn(&str) -> Option<f32>| {
            let mut values = value.split_whitespace().filter_map(parse);
            let first = values.next().unwrap_or(default);
            (first, values.next().unwrap_or(first))
        };
        Self {
            pending: false,
            blocked: false,
            descriptors: descriptors.into(),
            family: family.into(),
            alias: format!("Oryn private face {index}"),
            weight: range(read("font-weight"), 400., |v| {
                fontique::FontWeight::parse_css(v).map(|v| v.value())
            }),
            width: range(read("font-stretch"), 100., |v| {
                fontique::FontWidth::parse_css(v).map(|v| v.percentage())
            }),
            style: read("font-style"),
            coverage: cssom::font_face_unicode_ranges(descriptors, "https://oryn.invalid/")
                .unwrap_or_else(|_| vec![(0, 0x10ffff)]),
        }
    }
    pub fn override_info(&self) -> fontique::FontInfoOverride<'_> {
        let style = if self.style.starts_with("oblique") {
            format!("oblique {}deg", angle_range(&self.style).0)
        } else if self.style.is_empty() {
            "normal".into()
        } else {
            self.style.clone()
        };
        fontique::FontInfoOverride {
            family_name: Some(&self.alias),
            weight: Some(fontique::FontWeight::new(self.weight.0)),
            width: Some(fontique::FontWidth::from_percentage(self.width.0)),
            style: fontique::FontStyle::parse_css(&style),
            ..Default::default()
        }
    }
    pub(super) fn covers_cluster(&self, cluster: &str) -> bool {
        use unicode_normalization::UnicodeNormalization;
        let includes = |ch: char| {
            matches!(u32::from(ch),0x200c..=0x200d|0xfe00..=0xfe0f|0xe0100..=0xe01ef)
                || self
                    .coverage
                    .iter()
                    .any(|(start, end)| (*start..=*end).contains(&u32::from(ch)))
        };
        cluster.chars().all(includes) || cluster.nfc().all(includes) || cluster.nfd().all(includes)
    }
    pub fn shaping_overrides(&self) -> parley::FontFaceOverrides {
        let read = |name| {
            cssom::read_font_face_declaration(
                &self.descriptors,
                name,
                "https://oryn.invalid/",
                false,
            )
            .map(|view| view.value)
            .unwrap_or_default()
        };
        let percent = |name| {
            read(name)
                .trim_end_matches('%')
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.)
                .map(|value| value / 100.)
        };
        let variations = parley::FontVariation::parse_css_list(&read("font-variation-settings"))
            .filter_map(Result::ok)
            .collect();
        let features = parley::FontFeature::parse_css_list(&read("font-feature-settings"))
            .filter_map(Result::ok)
            .collect();
        let mut axis_limits = vec![
            (*b"wght", self.weight.0, self.weight.1),
            (*b"wdth", self.width.0, self.width.1),
        ];
        if self.style.starts_with("oblique") {
            let (min, max) = angle_range(&self.style);
            axis_limits.push((*b"slnt", -max, -min));
        } else {
            axis_limits.push((*b"slnt", 0., 0.));
        }
        let italic = if self.style == "italic" { 1. } else { 0. };
        axis_limits.push((*b"ital", italic, italic));
        parley::FontFaceOverrides {
            features,
            variations,
            axis_limits,
            ascent: percent("ascent-override"),
            descent: percent("descent-override"),
            line_gap: percent("line-gap-override"),
            size_adjust: percent("size-adjust"),
            language: text_features::language_override(&BTreeMap::from([(
                "font-language-override".into(),
                read("font-language-override"),
            )])),
        }
    }
    pub(super) fn rank(&self, style: &BTreeMap<String, String>) -> [f32; 7] {
        let weight = fontique::FontWeight::parse_css(style_value(style, "font-weight", "400"))
            .unwrap_or_default()
            .value();
        let width = fontique::FontWidth::parse_css(style_value(style, "font-stretch", "100%"))
            .unwrap_or_default()
            .percentage();
        use crate::cssom::native_import_graph::{NativeFontAxes, NativeFontStyle, native_font_axes_rank};
        let parse_style=|text:&str| match text.split_whitespace().next().unwrap_or("normal") {
            "italic"=>NativeFontStyle::Italic,
            "oblique"=>{let(a,b)=angle_range(text);NativeFontStyle::Oblique(a,b)},
            _=>NativeFontStyle::Normal,
        };
        native_font_axes_rank(
            NativeFontAxes {weight:self.weight,width:self.width,style:parse_style(&self.style)},
            NativeFontAxes {weight:(weight,weight),width:(width,width),style:parse_style(style_value(style,"font-style","normal"))},
        )
    }
}
fn angle_range(value: &str) -> (f32, f32) {
    let mut angles = value.split_whitespace().skip(1).filter_map(|value| {
        for (suffix, scale) in [
            ("deg", 1.),
            ("grad", 0.9),
            ("rad", 180. / std::f32::consts::PI),
            ("turn", 360.),
        ] {
            if let Some(value) = value.strip_suffix(suffix) {
                return value
                    .parse::<f32>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(|value| value * scale);
            }
        }
        None
    });
    let first = angles.next().unwrap_or(14.);
    (first, angles.next().unwrap_or(first))
}
#[cfg(test)]
fn angle_distance(desired:f32,min:f32,max:f32)->[f32;2] {
    crate::cssom::native_import_graph::native_font_angle_distance(desired,min,max)
}
fn quote(name: &str) -> String {
    let mut result = String::new();
    let _ = cssparser::serialize_string(name, &mut result);
    result
}

/// Grapheme aliases preserve fallback order and prevent a font with a broad
/// cmap from covering characters excluded by its CSS unicode-range.
#[cfg(test)]
pub(super) fn families(
    faces: &[Face],
    style: &BTreeMap<String, String>,
    character: char,
) -> String {
    families_for_cluster(faces, style, &character.to_string())
}
pub(super) fn families_for_cluster(
    faces: &[Face],
    style: &BTreeMap<String, String>,
    cluster: &str,
) -> String {
    let mut output = Vec::new();
    for family in cssom::font_family_names(style_value(style, "font-family", DEFAULT_FONT_FAMILY)) {
        let candidates = faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(&family))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            if !family
                .to_ascii_lowercase()
                .starts_with("oryn private face ")
            {
                output.push(quote(&family));
            }
            continue;
        }
        let best = candidates
            .iter()
            .map(|face| face.rank(style))
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        for face in candidates
            .into_iter()
            .rev()
            .filter(|face| face.rank(style) == best && !face.pending)
        {
            if face.covers_cluster(cluster) {
                output.push(quote(&face.alias));
            }
        }
    }
    output.push(quote(DEFAULT_FONT_FAMILY));
    output.join(",")
}
pub(super) fn blocked_for_cluster(
    faces: &[Face],
    style: &BTreeMap<String, String>,
    cluster: &str,
) -> bool {
    for family in cssom::font_family_names(style_value(style, "font-family", DEFAULT_FONT_FAMILY)) {
        let candidates = faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(&family))
            .collect::<Vec<_>>();
        let Some(best) = candidates
            .iter()
            .map(|face| face.rank(style))
            .min_by(|a, b| a.partial_cmp(b).unwrap())
        else {
            continue;
        };
        for face in candidates
            .into_iter()
            .rev()
            .filter(|face| face.rank(style) == best)
        {
            if face.covers_cluster(cluster) {
                if face.pending {
                    return face.blocked;
                }
                return false;
            }
        }
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_display_block_preserves_fallback_measurements_and_suppresses_ink() {
        let style = BTreeMap::from([("font-family".into(), "Pending, serif".into())]);
        let mut blocked =
            TextEngine::new(&[("Pending".into(), Vec::new(), String::new(), true, true)]);
        let hidden = blocked.shape_styled(
            "Visible after timeout",
            500.,
            16.,
            &style,
            ViewportState::default(),
        );
        let mut swapped =
            TextEngine::new(&[("Pending".into(), Vec::new(), String::new(), true, false)]);
        let visible = swapped.shape_styled(
            "Visible after timeout",
            500.,
            16.,
            &style,
            ViewportState::default(),
        );
        assert_eq!(hidden.width, visible.width);
        assert_eq!(hidden.height, visible.height);
        assert!(
            hidden
                .glyph_runs
                .iter()
                .flatten()
                .all(|run| run.color.is_some_and(|color| color[3] == 0))
        );
        assert!(
            visible
                .glyph_runs
                .iter()
                .flatten()
                .any(|run| run.color.is_none_or(|color| color[3] != 0))
        );
    }
    #[test]
    fn descriptor_metrics_and_size_adjust_apply_at_actual_font_selection() {
        let plain = (
            "Demo".into(),
            DEFAULT_FONT_BYTES.to_vec(),
            String::new(),
            false,
            false,
        );
        let adjusted=("Demo".into(),DEFAULT_FONT_BYTES.to_vec(),"size-adjust:200%;ascent-override:70%;descent-override:20%;line-gap-override:10%;font-feature-settings:'liga' 0".into(),false,false);
        let style = BTreeMap::from([
            ("font-family".into(), "Demo".into()),
            ("font-size".into(), "20px".into()),
        ]);
        let mut ordinary = TextEngine::new(&[plain]);
        let normal = ordinary.shape_styled("MM", 500., 20., &style, ViewportState::default());
        let mut modified = TextEngine::new(&[adjusted]);
        let modified = modified.shape_styled("MM", 500., 20., &style, ViewportState::default());
        assert!((modified.width - normal.width * 2.).abs() < 0.05);
        assert!((modified.height - 40.).abs() < 0.05);
    }
    #[test]
    fn grapheme_coverage_normalizes_and_does_not_split_combining_or_variation_sequences() {
        let faces = vec![
            Face::new(0, "Demo", "unicode-range:U+00E9"),
            Face::new(1, "Emoji", "unicode-range:U+2764"),
        ];
        let style = BTreeMap::from([("font-family".into(), "Demo, Emoji".into())]);
        assert!(families_for_cluster(&faces, &style, "e\u{301}").contains("private face 0"));
        assert!(families_for_cluster(&faces, &style, "❤\u{fe0f}").contains("private face 1"));
        assert!(!families_for_cluster(&faces, &style, "a\u{301}").contains("private face"));
    }
    #[test]
    fn bundled_local_resolution_accepts_full_names_and_does_not_probe_host_fonts() {
        use skrifa::{MetadataProvider, string::StringId};
        let font = FontRef::new(DEFAULT_FONT_BYTES).unwrap();
        let name = font
            .localized_strings(StringId::FULL_NAME)
            .next()
            .unwrap()
            .to_string();
        assert_eq!(local_font_bytes(&name), Some(DEFAULT_FONT_BYTES));
        assert!(local_font_bytes("Definitely Uninstalled Face").is_none());
    }
    #[test]
    fn registered_unicode_faces_reach_native_shaping_without_family_leakage() {
        let mut engine = TextEngine::new(&[(
            "Demo".into(),
            DEFAULT_FONT_BYTES.to_vec(),
            "unicode-range:U+0041;font-weight:700".into(),
            false,
            false,
        )]);
        assert_eq!(engine.web_faces.len(), 1);
        let style = BTreeMap::from([
            ("font-family".into(), "Demo".into()),
            ("font-weight".into(), "700".into()),
        ]);
        let shaped = engine.shape_styled("AB", 200., 16., &style, ViewportState::default());
        assert!(shaped.width > 0.);
        assert!(!shaped.glyph_runs.is_empty());
        assert!(families(&engine.web_faces, &style, 'A').contains("private face 0"));
        assert!(!families(&engine.web_faces, &style, 'B').contains("private face"));
    }
    #[test]
    fn oblique_angle_ranges_prefer_the_css_direction_and_exact_containment() {
        assert!(angle_distance(20., 25., 30.) < angle_distance(20., 10., 15.));
        assert!(angle_distance(5., 0., 3.) < angle_distance(5., 10., 15.));
        assert_eq!(angle_distance(-20., -30., -10.), [0., 0.]);
        let faces = vec![
            Face::new(0, "Demo", "font-style:oblique 0deg 10deg"),
            Face::new(1, "Demo", "font-style:oblique 15deg 30deg"),
        ];
        let style = BTreeMap::from([
            ("font-family".into(), "Demo".into()),
            ("font-style".into(), "oblique 20deg".into()),
        ]);
        assert!(families(&faces, &style, 'A').starts_with("\"Oryn private face 1\""));
    }
    #[test]
    fn width_precedes_weight_and_four_hundred_searches_five_hundred_first() {
        let faces = vec![
            Face::new(0, "Demo", "font-weight:300"),
            Face::new(1, "Demo", "font-weight:500"),
            Face::new(2, "Demo", "font-weight:400;font-stretch:condensed"),
        ];
        let style = BTreeMap::from([
            ("font-family".into(), "Demo".into()),
            ("font-weight".into(), "400".into()),
        ]);
        let value = families(&faces, &style, 'A');
        assert!(value.contains("face 1"));
        assert!(!value.contains("face 0"));
        assert!(!value.contains("face 2"));
    }
    #[test]
    fn unicode_exclusions_and_later_composite_faces_preserve_family_fallback() {
        let faces = vec![
            Face::new(0, "Demo", "unicode-range:U+0041"),
            Face::new(1, "Demo", "unicode-range:U+1F600"),
        ];
        let style = BTreeMap::from([("font-family".into(), "Demo, Fallback".into())]);
        assert!(families(&faces, &style, 'A').starts_with("\"Oryn private face 0\""));
        assert!(families(&faces, &style, '😀').starts_with("\"Oryn private face 1\""));
        assert_eq!(
            families(&faces, &style, 'B'),
            format!("\"Fallback\",\"{DEFAULT_FONT_FAMILY}\"")
        );
    }
}
