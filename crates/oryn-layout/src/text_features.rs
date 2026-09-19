//! CSS feature selection, followed by explicit OpenType feature overrides.
use parley::FontFeature;
use std::collections::BTreeMap;

pub(super) fn resolve(style: &BTreeMap<String, String>) -> Vec<FontFeature> {
    let mut features = BTreeMap::new();
    let mut add = |source: &str| {
        for feature in FontFeature::parse_css_list(source).filter_map(Result::ok) {
            features.insert(feature.tag, feature);
        }
    };
    match crate::style_value(style, "font-kerning", "auto") {
        "none" => add("\"kern\" 0"),
        "normal" => add("\"kern\" 1"),
        _ => {}
    }
    for property in [
        "font-variant-ligatures",
        "font-variant-caps",
        "font-variant-numeric",
        "font-variant-east-asian",
        "font-variant-position",
    ] {
        for value in crate::style_value(style, property, "normal").split_ascii_whitespace() {
            let settings = match value {
                "none" if property == "font-variant-ligatures" => {
                    "\"liga\" 0, \"clig\" 0, \"dlig\" 0, \"hlig\" 0, \"calt\" 0"
                }
                "common-ligatures" => "\"liga\" 1, \"clig\" 1",
                "no-common-ligatures" => "\"liga\" 0, \"clig\" 0",
                "discretionary-ligatures" => "\"dlig\" 1",
                "no-discretionary-ligatures" => "\"dlig\" 0",
                "historical-ligatures" => "\"hlig\" 1",
                "no-historical-ligatures" => "\"hlig\" 0",
                "contextual" => "\"calt\" 1",
                "no-contextual" => "\"calt\" 0",
                "small-caps" => "\"smcp\" 1",
                "all-small-caps" => "\"smcp\" 1, \"c2sc\" 1",
                "petite-caps" => "\"pcap\" 1",
                "all-petite-caps" => "\"pcap\" 1, \"c2pc\" 1",
                "unicase" => "\"unic\" 1",
                "titling-caps" => "\"titl\" 1",
                "lining-nums" => "\"lnum\" 1",
                "oldstyle-nums" => "\"onum\" 1",
                "proportional-nums" => "\"pnum\" 1",
                "tabular-nums" => "\"tnum\" 1",
                "diagonal-fractions" => "\"frac\" 1",
                "stacked-fractions" => "\"afrc\" 1",
                "ordinal" => "\"ordn\" 1",
                "slashed-zero" => "\"zero\" 1",
                "jis78" => "\"jp78\" 1",
                "jis83" => "\"jp83\" 1",
                "jis90" => "\"jp90\" 1",
                "jis04" => "\"jp04\" 1",
                "simplified" => "\"smpl\" 1",
                "traditional" => "\"trad\" 1",
                "full-width" => "\"fwid\" 1",
                "proportional-width" => "\"pwid\" 1",
                "ruby" => "\"ruby\" 1",
                "sub" => "\"subs\" 1",
                "super" => "\"sups\" 1",
                _ => continue,
            };
            add(settings);
        }
    }
    let explicit = crate::style_value(style, "font-feature-settings", "normal");
    if explicit != "normal" {
        add(explicit);
    }
    features.into_values().collect()
}

pub(super) fn language_override(style: &BTreeMap<String, String>) -> Option<[u8; 4]> {
    let raw = crate::style_value(style, "font-language-override", "normal");
    if raw == "normal" {
        return None;
    }
    let mut input = cssparser::ParserInput::new(raw);
    let mut parser = cssparser::Parser::new(&mut input);
    let value = parser.expect_string().ok()?.to_string();
    parser.expect_exhausted().ok()?;
    if value.is_empty()
        || value.len() > 4
        || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return None;
    }
    let mut tag = [b' '; 4];
    tag[..value.len()].copy_from_slice(value.as_bytes());
    Some(tag)
}

pub(super) fn variations(
    style: &BTreeMap<String, String>,
    font_size: f64,
) -> Vec<parley::FontVariation> {
    let mut values = BTreeMap::new();
    if crate::style_value(style, "font-optical-sizing", "auto") == "auto" {
        for setting in parley::FontVariation::parse_css_list(&format!("\"opsz\" {font_size}"))
            .filter_map(Result::ok)
        {
            values.insert(setting.tag, setting);
        }
    }
    let explicit = crate::style_value(style, "font-variation-settings", "normal");
    if explicit != "normal" {
        for setting in parley::FontVariation::parse_css_list(explicit).filter_map(Result::ok) {
            values.insert(setting.tag, setting);
        }
    }
    values.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn css_language_override_preserves_tag_bytes_and_decodes_escapes() {
        let parse = |value: &str| {
            language_override(&BTreeMap::from([(
                "font-language-override".into(),
                value.into(),
            )]))
        };
        assert_eq!(parse("\"TRK\""), Some(*b"TRK "));
        assert_eq!(parse("\"a? z\""), Some(*b"a? z"));
        assert_eq!(parse(r#""\54 RK""#), Some(*b"TRK "));
        assert_eq!(parse("normal"), None);
        assert_eq!(parse("\"ABCDE\""), None);
        assert_eq!(parse("\"é\""), None);
        assert_eq!(parse("\"TRK\" extra"), None);
    }

    #[test]
    fn optical_sizing_uses_font_size_and_respects_none_and_explicit_axes() {
        let automatic = variations(&BTreeMap::new(), 24.);
        assert_eq!(
            automatic
                .iter()
                .find(|axis| axis.tag.to_string() == "opsz")
                .unwrap()
                .value,
            24.
        );
        assert!(
            variations(
                &BTreeMap::from([("font-optical-sizing".into(), "none".into())]),
                24.
            )
            .is_empty()
        );
        let explicit = variations(
            &BTreeMap::from([(
                "font-variation-settings".into(),
                "\"opsz\" 9, \"opsz\" 12, \"wght\" 700".into(),
            )]),
            24.,
        );
        assert_eq!(
            explicit
                .iter()
                .find(|axis| axis.tag.to_string() == "opsz")
                .unwrap()
                .value,
            12.
        );
        assert_eq!(explicit.len(), 2);
    }

    #[test]
    fn explicit_settings_override_css_variants_and_duplicate_tags() {
        let style = BTreeMap::from([
            ("font-kerning".into(), "none".into()),
            ("font-variant-ligatures".into(), "none".into()),
            (
                "font-variant-numeric".into(),
                "tabular-nums slashed-zero".into(),
            ),
            (
                "font-feature-settings".into(),
                "\"liga\" 1, \"kern\" 1, \"liga\" 0".into(),
            ),
        ]);
        let features = resolve(&style);
        let value = |tag: &str| {
            features
                .iter()
                .find(|feature| feature.tag.to_string() == tag)
                .map(|feature| feature.value)
        };
        assert_eq!(value("liga"), Some(0));
        assert_eq!(value("kern"), Some(1));
        assert_eq!(value("tnum"), Some(1));
        assert_eq!(value("zero"), Some(1));
        assert_eq!(value("clig"), Some(0));
    }
}
