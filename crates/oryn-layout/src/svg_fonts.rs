//! Independently owned SVG embedded font admission. Never performs resource IO.
use super::*;
use base64::Engine as _;
const LIMIT: usize = 32 * 1024 * 1024;
pub fn data_bytes(url: &str) -> Option<Vec<u8>> {
    let (metadata, data) = url.strip_prefix("data:")?.split_once(',')?;
    if data.len() > LIMIT * 3 {
        return None;
    }
    let mut decoded = Vec::with_capacity(data.len().min(LIMIT));
    let raw = data.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let (Some(a), Some(b)) = (
                (raw[i + 1] as char).to_digit(16),
                (raw[i + 2] as char).to_digit(16),
            )
        {
            decoded.push((a * 16 + b) as u8);
            i += 3;
            continue;
        }
        decoded.push(raw[i]);
        i += 1;
    }
    let bytes = if metadata
        .split(';')
        .next_back()
        .is_some_and(|value| value.eq_ignore_ascii_case("base64"))
    {
        decoded.retain(|byte| !matches!(*byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'));
        base64::engine::general_purpose::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            base64::engine::general_purpose::GeneralPurposeConfig::new()
                .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
                .with_decode_allow_trailing_bits(true),
        )
        .decode(decoded)
        .ok()?
    } else {
        decoded
    };
    (bytes.len() <= LIMIT).then_some(bytes)
}
/// Admit only data sources in active local stylesheets. Network and local()
/// candidates fail independently, allowing a later embedded source to win.
pub fn embedded(
    sheets: &[String],
    base: &str,
    viewport: ViewportState,
    color: ColorScheme,
    reduced: bool,
) -> Result<Vec<WebFont>, String> {
    let mut output = Vec::new();
    let mut total = 0usize;
    let mut count = 0usize;
    let mut pending = Vec::new();
    for sheet in sheets.iter().rev() {
        pending.extend(
            cssom::read_stylesheet(sheet, base, false, false)?
                .into_iter()
                .rev(),
        );
    }
    while let Some(rule) = pending.pop() {
        count += 1;
        if count > 8192 {
            return Err("SVG font rule budget".into());
        }
        if rule.rule_type == 4
            && !media::matches(
                rule.properties
                    .get("conditionText")
                    .map(String::as_str)
                    .unwrap_or(""),
                base,
                false,
                viewport,
                color,
                reduced,
                false,
            )?
        {
            continue;
        }
        if rule.rule_type == 12
            && rule
                .properties
                .get("matches")
                .is_some_and(|value| value == "false")
        {
            continue;
        }
        // Container-dependent faces are invalid in CSS and may not become globally
        // active by flattening a malformed or unsupported grouping rule.
        if !rule.conditions.is_empty() {
            continue;
        }
        if rule.rule_type == 5 {
            if let Some(descriptors) = rule.declaration_text {
                let family =
                    cssom::read_font_face_declaration(&descriptors, "font-family", base, false)?
                        .value;
                let Some(family) = cssom::font_family_names(&family).into_iter().next() else {
                    continue;
                };
                for source in cssom::font_face_sources(&descriptors, base)? {
                    let cssom::FontFaceSource::Url(url) = source else {
                        continue;
                    };
                    let Some(bytes) = data_bytes(&url) else {
                        continue;
                    };
                    let Ok(bytes) = decode_web_font(&bytes, LIMIT) else {
                        continue;
                    };
                    total = total
                        .checked_add(bytes.len())
                        .filter(|total| *total <= LIMIT)
                        .ok_or("SVG embedded font byte budget")?;
                    if output.len() >= 256 {
                        return Err("SVG embedded font face budget".into());
                    }
                    use sha2::Digest;
                    let sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
                    output.push(WebFont {
                        pending: false,
                        blocked: false,
                        order: output.len() as u64,
                        descriptors,
                        family,
                        source_url: url,
                        sha256,
                        bytes,
                    });
                    break;
                }
            }
        } else {
            pending.extend(rule.children.into_iter().rev());
        }
    }
    Ok(output)
}
/// Register CSS aliases and descriptors rather than the font file's intrinsic
/// family. Private aliases avoid leaking a data face under unrelated names.
pub fn install(options: &mut usvg::Options<'_>, fonts: &[WebFont]) -> Result<(), String> {
    let mut total = 0usize;
    let mut faces = Vec::new();
    let mut private_ids = std::collections::BTreeSet::new();
    for (index, font) in fonts.iter().enumerate() {
        if font.pending || font.blocked {
            continue;
        }
        total = total
            .checked_add(font.bytes.len())
            .filter(|total| *total <= LIMIT)
            .ok_or("SVG font byte budget")?;
        let descriptor = web_font_faces::Face::new(index, &font.family, &font.descriptors);
        let db = options.fontdb_mut();
        let ids = db.load_font_source(usvg::fontdb::Source::Binary(std::sync::Arc::new(
            font.bytes.clone(),
        )));
        let mut admitted = Vec::new();
        for id in ids {
            let Some(mut info) = db.face(id).cloned() else {
                continue;
            };
            db.remove_face(id);
            let language = info
                .families
                .first()
                .map(|(_, language)| *language)
                .unwrap_or(usvg::fontdb::Language::English_UnitedStates);
            info.families = vec![(descriptor.alias.clone(), language)];
            info.weight = usvg::fontdb::Weight(descriptor.weight.0.clamp(1., 1000.) as u16);
            info.style = if descriptor.style.starts_with("italic") {
                usvg::fontdb::Style::Italic
            } else if descriptor.style.starts_with("oblique") {
                usvg::fontdb::Style::Oblique
            } else {
                usvg::fontdb::Style::Normal
            };
            let id = db.push_face_info(info);
            private_ids.insert(id);
            admitted.push(id);
        }
        faces.push((descriptor, admitted));
    }
    let settings_faces = faces.clone();
    options.font_resolver.face_settings = Some(Box::new(move |id, font| {
        let face = settings_faces
            .iter()
            .find(|(_, ids)| ids.contains(&id))
            .map(|(face, _)| face);
        let overrides = face
            .map(|face| face.shaping_overrides())
            .unwrap_or_default();
        let tag = |value: String| -> Option<[u8; 4]> { value.as_bytes().try_into().ok() };
        let width: f32 = match font.stretch() {
            usvg::FontStretch::UltraCondensed => 50.,
            usvg::FontStretch::ExtraCondensed => 62.5,
            usvg::FontStretch::Condensed => 75.,
            usvg::FontStretch::SemiCondensed => 87.5,
            usvg::FontStretch::Normal => 100.,
            usvg::FontStretch::SemiExpanded => 112.5,
            usvg::FontStretch::Expanded => 125.,
            usvg::FontStretch::ExtraExpanded => 150.,
            usvg::FontStretch::UltraExpanded => 200.,
        };
        let mut variations = BTreeMap::from([
            (
                *b"wght",
                f32::from(font.weight()).clamp(
                    face.map_or(1., |face| face.weight.0),
                    face.map_or(1000., |face| face.weight.1),
                ),
            ),
            (
                *b"wdth",
                width.clamp(
                    face.map_or(50., |face| face.width.0),
                    face.map_or(200., |face| face.width.1),
                ),
            ),
            (
                *b"ital",
                if font.style() == usvg::FontStyle::Italic {
                    1.
                } else {
                    0.
                },
            ),
            (
                *b"slnt",
                if font.style() == usvg::FontStyle::Oblique {
                    -14.
                } else {
                    0.
                },
            ),
        ]);
        for (axis, min, max) in &overrides.axis_limits {
            if let Some(value) = variations.get_mut(axis) {
                *value = (*value).clamp(*min, *max);
            }
        }
        for setting in overrides.variations {
            if let Some(tag) = tag(setting.tag.to_string()) {
                variations.insert(tag, setting.value);
            }
        }
        for setting in
            parley::FontVariation::parse_css_list(font.variation_settings()).filter_map(Result::ok)
        {
            if let Some(tag) = tag(setting.tag.to_string()) {
                variations.insert(tag, setting.value);
            }
        }
        let features = overrides
            .features
            .into_iter()
            .filter_map(|feature| {
                tag(feature.tag.to_string()).map(|tag| (tag, u32::from(feature.value)))
            })
            .collect::<Vec<_>>();
        let feature_overrides = parley::FontFeature::parse_css_list(font.feature_settings())
            .filter_map(Result::ok)
            .filter_map(|feature| {
                tag(feature.tag.to_string()).map(|tag| (tag, u32::from(feature.value)))
            })
            .collect();
        let language = if font.language_override() == "normal" {
            overrides.language
        } else {
            text_features::language_override(&BTreeMap::from([(
                "font-language-override".into(),
                font.language_override().into(),
            )]))
        };
        usvg::FontFaceSettings {
            variations: variations.into_iter().collect(),
            features,
            feature_overrides,
            size_adjust: overrides.size_adjust.unwrap_or(1.),
            ascent: overrides.ascent,
            descent: overrides.descent,
            line_gap: overrides.line_gap,
            language,
        }
    }));
    let default = usvg::FontResolver::default_font_selector();
    type ClusterFontSelector = dyn Fn(
            &usvg::Font,
            &str,
            &mut std::sync::Arc<usvg::fontdb::Database>,
        ) -> Option<usvg::fontdb::ID>
        + Send
        + Sync;
    let select: std::sync::Arc<ClusterFontSelector> = std::sync::Arc::new(
        move |font, cluster, db| {
            let width: f32 = match font.stretch() {
                usvg::FontStretch::UltraCondensed => 50.,
                usvg::FontStretch::ExtraCondensed => 62.5,
                usvg::FontStretch::Condensed => 75.,
                usvg::FontStretch::SemiCondensed => 87.5,
                usvg::FontStretch::Normal => 100.,
                usvg::FontStretch::SemiExpanded => 112.5,
                usvg::FontStretch::Expanded => 125.,
                usvg::FontStretch::ExtraExpanded => 150.,
                usvg::FontStretch::UltraExpanded => 200.,
            };
            let style = BTreeMap::from([
                ("font-weight".into(), font.weight().to_string()),
                ("font-stretch".into(), format!("{width}%")),
                (
                    "font-style".into(),
                    match font.style() {
                        usvg::FontStyle::Normal => "normal",
                        usvg::FontStyle::Italic => "italic",
                        usvg::FontStyle::Oblique => "oblique",
                    }
                    .into(),
                ),
            ]);
            let supports = |id, db: &usvg::fontdb::Database| {
                db.with_face_data(id, |bytes, index| {
                    use skrifa::MetadataProvider;
                    let Ok(face) = skrifa::FontRef::from_index(bytes, index) else {
                        return false;
                    };
                    let map = face.charmap();
                    use unicode_normalization::UnicodeNormalization;
                    let supported = |ch: char| {
                        matches!(u32::from(ch),0x200c..=0x200d|0xfe00..=0xfe0f|0xe0100..=0xe01ef)
                            || map.map(ch).is_some()
                    };
                    cluster.chars().all(supported)
                        || cluster.nfc().all(supported)
                        || cluster.nfd().all(supported)
                })
                .unwrap_or(false)
            };
            for family in font.families() {
                let candidates=faces.iter().filter(|(face,_)|matches!(family,usvg::FontFamily::Named(name)if face.family.eq_ignore_ascii_case(name))).collect::<Vec<_>>();
                if candidates.is_empty() {
                    let family = match family {
                        usvg::FontFamily::Named(name) => usvg::fontdb::Family::Name(name),
                        usvg::FontFamily::Serif => usvg::fontdb::Family::Serif,
                        usvg::FontFamily::SansSerif => usvg::fontdb::Family::SansSerif,
                        usvg::FontFamily::Cursive => usvg::fontdb::Family::Cursive,
                        usvg::FontFamily::Fantasy => usvg::fontdb::Family::Fantasy,
                        usvg::FontFamily::Monospace => usvg::fontdb::Family::Monospace,
                    };
                    if let Some(id) = db.query(&usvg::fontdb::Query {
                        families: &[family],
                        weight: usvg::fontdb::Weight(font.weight()),
                        stretch: font.stretch().into(),
                        style: font.style().into(),
                    }) && !private_ids.contains(&id)
                        && supports(id, db)
                    {
                        return Some(id);
                    }
                    continue;
                }
                let Some(best) = candidates
                    .iter()
                    .map(|(face, _)| face.rank(&style))
                    .min_by(|a, b| a.partial_cmp(b).unwrap())
                else {
                    continue;
                };
                for (_, ids) in candidates
                    .into_iter()
                    .rev()
                    .filter(|(face, _)| face.rank(&style) == best && face.covers_cluster(cluster))
                {
                    if let Some(id) = ids.iter().copied().find(|id| supports(*id, db)) {
                        return Some(id);
                    }
                }
            }
            let fallback = default(font, db);
            if let Some(id) = fallback
                && supports(id, db)
            {
                return Some(id);
            }
            db.faces()
                .find(|face| !private_ids.contains(&face.id) && supports(face.id, db))
                .map(|face| face.id)
                .or(fallback)
                // Missing glyphs still have source ranges and bidi direction. If
                // the generic requested family is absent, keep the admitted public
                // fallback's .notdef glyph instead of dropping the whole cluster.
                .or_else(|| {
                    db.faces()
                        .find(|face| !private_ids.contains(&face.id))
                        .map(|face| face.id)
                })
        },
    );
    let initial = select.clone();
    options.font_resolver.select_font = Box::new(move |font, db| initial(font, "", db));
    options.font_resolver.select_cluster_font =
        Some(Box::new(move |font, cluster, db| select(font, cluster, db)));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_generic_family_retains_missing_clusters_in_admitted_font() {
        let mut options = usvg::Options::default();
        options
            .fontdb_mut()
            .load_font_data(DEFAULT_FONT_BYTES.to_vec());
        // This deliberately names no installed family. Both Hebrew characters
        // are absent from the bundled Latin font; they must retain .notdef
        // glyph/source identity rather than disappear from shaping.
        install(&mut options, &[]).unwrap();
        let tree = usvg::Tree::from_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="30"><text x="0" y="20" font-family="NoSuchNativeFont">אב A</text></svg>"#,
            &options,
        ).unwrap();
        fn inspect(group: &usvg::Group, seen: &mut Vec<(std::ops::Range<usize>, bool)>) {
            for node in group.children() {
                match node {
                    usvg::Node::Group(group) => inspect(group, seen),
                    usvg::Node::Text(text) => seen.extend(
                        text.layouted()
                            .iter()
                            .flat_map(|span| &span.positioned_glyphs)
                            .map(|glyph| (glyph.source.clone(), glyph.rtl)),
                    ),
                    _ => {}
                }
            }
        }
        let mut seen = Vec::new();
        inspect(tree.root(), &mut seen);
        assert!(
            seen.iter()
                .any(|(range, rtl)| *rtl && range.start == 0 && range.end == 2)
        );
        assert!(
            seen.iter()
                .any(|(range, rtl)| *rtl && range.start == 2 && range.end == 4)
        );
        assert!(
            seen.iter()
                .any(|(range, rtl)| !*rtl && range.start == 5 && range.end == 6)
        );
        assert_eq!(
            tree.fontdb().faces().count(),
            1,
            "no external fallback font was loaded"
        );
    }

    #[test]
    fn secure_embedded_faces_respect_conditions_fallback_aliases_and_original_bytes() {
        let data = base64::engine::general_purpose::STANDARD.encode(DEFAULT_FONT_BYTES);
        let source = format!(
            "@font-face{{font-family:Owned;src:url(https://blocked.test/font),url(data:font/ttf;base64,{data});font-weight:700}} @media (min-width:9999px){{@font-face{{font-family:Inactive;src:url(data:font/ttf;base64,{data})}}}} @supports (unknown-property:bad){{@font-face{{font-family:Unsupported;src:url(data:font/ttf;base64,{data})}}}}"
        );
        let fonts = embedded(
            &[source],
            "https://example.test/image.svg",
            ViewportState::default(),
            ColorScheme::Light,
            false,
        )
        .unwrap();
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].family, "Owned");
        assert_eq!(fonts[0].bytes, DEFAULT_FONT_BYTES);
        let mut options = usvg::Options::default();
        install(&mut options, &fonts).unwrap();
        assert!(options.fontdb.faces().all(|face| {
            face.families[0].0.starts_with("Oryn private face ") && face.weight.0 == 700
        }));
        assert!(
            embedded(
                &[
                    "@font-face{font-family:No;src:local(Arial),url(https://example.test/no.woff)}"
                        .into()
                ],
                "https://example.test/image.svg",
                ViewportState::default(),
                ColorScheme::Light,
                false
            )
            .unwrap()
            .is_empty()
        );
        assert!(data_bytes("data:font/ttf;base64,invalid!").is_none());
        assert_eq!(data_bytes("data:font/ttf;base64,YQ").unwrap(), b"a");
        assert_eq!(
            data_bytes("data:application/octet-stream,%00%2b+").unwrap(),
            vec![0, b'+', b'+']
        );
    }
}

#[cfg(test)]
mod cluster_tests {
    use super::*;
    #[test]
    fn svg_faces_select_whole_clusters_by_unicode_range_and_css_weight() {
        let font = |family: &str, descriptors: &str| WebFont {
            pending: false,
            blocked: false,
            order: 0,
            family: family.into(),
            descriptors: descriptors.into(),
            source_url: "data:font/ttf;base64,native-test".into(),
            sha256: String::new(),
            bytes: DEFAULT_FONT_BYTES.to_vec(),
        };
        let faces = vec![
            font("Owned", "font-weight:400;unicode-range:U+41"),
            font("Owned", "font-weight:400;unicode-range:U+42"),
            font("Owned", "font-weight:700;unicode-range:U+41-42"),
        ];
        let mut options = usvg::Options::default();
        options
            .fontdb_mut()
            .load_font_data(DEFAULT_FONT_BYTES.to_vec());
        install(&mut options, &faces).unwrap();
        for (weight, expected) in [
            (400, vec!["Oryn private face 0", "Oryn private face 1"]),
            (700, vec!["Oryn private face 2", "Oryn private face 2"]),
        ] {
            let xml = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='40'><text x='0' y='20' font-family='Owned' font-weight='{weight}'>AB</text></svg>"
            );
            let tree = usvg::Tree::from_str(&xml, &options).unwrap();
            fn collect(group: &usvg::Group, out: &mut Vec<usvg::fontdb::ID>) {
                for node in group.children() {
                    match node {
                        usvg::Node::Group(group) => collect(group, out),
                        usvg::Node::Text(text) => {
                            for span in text.layouted() {
                                out.extend(span.positioned_glyphs.iter().map(|glyph| glyph.font));
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut ids = Vec::new();
            collect(tree.root(), &mut ids);
            let names = ids
                .iter()
                .map(|id| tree.fontdb().face(*id).unwrap().families[0].0.as_str())
                .collect::<Vec<_>>();
            assert_eq!(names, expected);
        }
        assert!(
            !web_font_faces::Face::new(0, "Owned", "unicode-range:U+41").covers_cluster("A\u{301}")
        );
    }
}

#[cfg(test)]
mod descriptor_tests {
    use super::*;
    fn tree(bytes: &[u8], descriptors: &str, weight: u16, text: &str) -> usvg::Tree {
        let face = WebFont {
            pending: false,
            blocked: false,
            order: 0,
            family: "Owned".into(),
            descriptors: descriptors.into(),
            source_url: "data:font/ttf;base64,native-regression".into(),
            sha256: String::new(),
            bytes: bytes.to_vec(),
        };
        let mut options = usvg::Options::default();
        options
            .fontdb_mut()
            .load_font_data(DEFAULT_FONT_BYTES.to_vec());
        install(&mut options, &[face]).unwrap();
        usvg::Tree::from_str(&format!("<svg xmlns='http://www.w3.org/2000/svg' width='300' height='200'><text x='0' y='100' font-size='50' font-family='Owned' font-weight='{weight}'>{text}</text></svg>"),&options).unwrap()
    }
    #[test]
    fn variable_face_axes_reach_native_outlines_against_pinned_static_references() {
        // Unmodified WPT fixture: css/css-fonts/matching/resources at pinned revision.
        let bytes = include_bytes!("../../oryn-render/tests/assets/variabletest_matching.ttf");
        let raster = |tree: &usvg::Tree, scale: f32, shift: f32| {
            let mut pixels =
                resvg::tiny_skia::Pixmap::new((300. * scale) as u32, (200. * scale) as u32)
                    .unwrap();
            resvg::render(
                tree,
                resvg::tiny_skia::Transform::from_row(scale, 0., 0., scale, shift, 0.),
                &mut pixels.as_mut(),
            );
            pixels.data().to_vec()
        };
        for (weight, reference) in [(100, "/"), (900, ",")] {
            let actual = tree(bytes, "font-weight:100 900", weight, "P");
            let expected = tree(bytes, "font-weight:100 900", weight, reference);
            // The pinned reference glyph may trace the same contour in reverse order.
            // Compare the actual rendered coverage, retaining placement and shape checks.
            for scale in [1., 2.] {
                let actual_pixels = raster(&actual, scale, 0.);
                assert!(actual_pixels.iter().any(|value| *value != 0));
                assert_eq!(
                    actual_pixels,
                    raster(&expected, scale, 0.),
                    "weight {weight}, scale {scale}"
                );
                assert_ne!(
                    actual_pixels,
                    raster(&expected, scale, scale),
                    "oracle must detect shifted glyphs"
                );
            }
        }
    }
    #[test]
    fn size_and_metric_descriptors_are_retained_in_positioned_glyphs() {
        let plain = tree(DEFAULT_FONT_BYTES, "", 400, "A");
        let adjusted = tree(
            DEFAULT_FONT_BYTES,
            "size-adjust:200%;ascent-override:80%;descent-override:20%;line-gap-override:10%;font-feature-settings:'kern' 0",
            400,
            "A",
        );
        fn glyph(group: &usvg::Group) -> Option<&usvg::layout::PositionedGlyph> {
            for node in group.children() {
                match node {
                    usvg::Node::Group(group) => {
                        if let Some(glyph) = glyph(group) {
                            return Some(glyph);
                        }
                    }
                    usvg::Node::Text(text) => {
                        if let Some(glyph) = text
                            .layouted()
                            .iter()
                            .flat_map(|span| &span.positioned_glyphs)
                            .next()
                        {
                            return Some(glyph);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let settings = &glyph(adjusted.root()).unwrap().face_settings;
        assert_eq!(settings.size_adjust, 2.);
        assert_eq!(settings.ascent, Some(0.8));
        assert_eq!(settings.descent, Some(0.2));
        assert_eq!(settings.line_gap, Some(0.1));
        assert!(settings.features.contains(&(*b"kern", 0)));
        assert!(
            (adjusted.root().abs_bounding_box().width()
                - plain.root().abs_bounding_box().width() * 2.)
                .abs()
                < 0.01
        );
    }
}

#[cfg(test)]
mod precedence_tests {
    use super::*;
    #[test]
    fn inherited_element_settings_override_face_defaults_and_survive_svg_roundtrip() {
        let face=WebFont{pending:false,blocked:false,order:0,family:"Owned".into(),descriptors:"font-weight:100 900;font-variation-settings:'wght' 300;font-feature-settings:'kern' 0;font-language-override:'ENG'".into(),source_url:"data:font/ttf;base64,native-regression".into(),sha256:String::new(),bytes:include_bytes!("../../oryn-render/tests/assets/variabletest_matching.ttf").to_vec()};
        let mut options = usvg::Options::default();
        options
            .fontdb_mut()
            .load_font_data(DEFAULT_FONT_BYTES.to_vec());
        install(&mut options, &[face]).unwrap();
        let tree=usvg::Tree::from_str("<svg xmlns='http://www.w3.org/2000/svg' width='200' height='100'><g style=\"font-family:Owned;font-variation-settings:'wght' 800;font-feature-settings:'kern' 1;font-language-override:'TRK'\"><text x='0' y='50'>P</text></g></svg>",&options).unwrap();
        fn settings(group: &usvg::Group) -> Option<&usvg::FontFaceSettings> {
            for node in group.children() {
                match node {
                    usvg::Node::Group(group) => {
                        if let Some(settings) = settings(group) {
                            return Some(settings);
                        }
                    }
                    usvg::Node::Text(text) => {
                        if let Some(glyph) = text
                            .layouted()
                            .iter()
                            .flat_map(|span| &span.positioned_glyphs)
                            .next()
                        {
                            return Some(&glyph.face_settings);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let settings = settings(tree.root()).unwrap();
        assert!(settings.variations.contains(&(*b"wght", 800.)));
        assert!(settings.features.contains(&(*b"kern", 0)));
        assert!(settings.feature_overrides.contains(&(*b"kern", 1)));
        assert_eq!(settings.language, Some(*b"TRK "));
        let written = tree.to_string(&usvg::WriteOptions {
            preserve_text: true,
            ..Default::default()
        });
        assert!(written.contains("font-feature-settings"));
        assert!(written.contains("font-variation-settings"));
        assert!(written.contains("font-language-override"));
    }
}
