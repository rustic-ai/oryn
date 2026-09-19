//! Font-face descriptors use Stylo's descriptor grammar, not style properties.
use super::{DeclarationView, Origin, ParsingMode, QuirksMode, UrlExtraData};
use style::font_face::{DescriptorId, Descriptors, parse_font_face_block};
use style::parser::ParserContext;
use style::stylesheets::CssRuleType;
use style_traits::ToCss;

fn parse(text: &str, context: &ParserContext) -> Descriptors {
    let mut input = cssparser::ParserInput::new(text);
    let mut parser = cssparser::Parser::new(&mut input);
    let location = parser.current_source_location();
    parse_font_face_block(context, &mut parser, location).descriptors
}

fn context(url: &UrlExtraData, quirks: bool) -> ParserContext<'_> {
    ParserContext::new(
        Origin::Author,
        url,
        Some(CssRuleType::FontFace),
        ParsingMode::DEFAULT,
        if quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        },
        Default::default(),
        None,
        None,
        Default::default(),
    )
}

pub fn read_font_face_declaration(
    text: &str,
    property: &str,
    base_url: &str,
    quirks: bool,
) -> Result<DeclarationView, String> {
    let url = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let descriptors = parse(text, &context(&url, quirks));
    let mut value = String::new();
    if let Ok(id) = DescriptorId::from_ident(property) {
        descriptors.get(id, &mut value).map_err(|e| e.to_string())?;
    }
    Ok(DeclarationView {
        css_text: descriptors.to_css_string(),
        names: (0..descriptors.len())
            .filter_map(|i| descriptors.at(i))
            .map(|id| id.name().to_string())
            .collect(),
        value,
        important: false,
    })
}

/// Mutate only the descriptors of an existing native rule. Invalid values leave
/// the record unchanged; an empty value removes the descriptor.
pub(super) fn edit(
    descriptors: &mut Descriptors,
    property: Option<&str>,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
) -> Result<(String, bool), String> {
    let url = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let context = context(&url, quirks);
    let Some(property) = property else {
        let next = parse(value, &context);
        let changed = *descriptors != next;
        *descriptors = next;
        return Ok((String::new(), changed));
    };
    let Ok(id) = DescriptorId::from_ident(property) else {
        return Ok((String::new(), false));
    };
    let mut previous = String::new();
    descriptors
        .get(id, &mut previous)
        .map_err(|e| e.to_string())?;
    if value.is_empty() {
        return Ok((previous, descriptors.remove(id)));
    }
    // Font-face descriptors cannot carry !important.
    if !priority.is_empty() {
        return Ok((previous, false));
    }
    let mut input = cssparser::ParserInput::new(value);
    let mut parser = cssparser::Parser::new(&mut input);
    let changed = descriptors.set(id, &context, &mut parser).unwrap_or(false);
    Ok((previous, changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = "https://example.test/styles/main.css";

    #[test]
    fn font_descriptors_use_native_grammar() {
        let text = "font-family: Example; src: url('../a.woff2') format('woff2'); font-weight: 100 900; color: red";
        let view = read_font_face_declaration(text, "src", BASE, false).unwrap();
        assert!(view.value.contains("a.woff2"));
        assert!(!view.names.iter().any(|name| name == "color"));
        assert_eq!(
            read_font_face_declaration(text, "font-weight", BASE, false)
                .unwrap()
                .value,
            "100 900"
        );
        assert_eq!(
            read_font_face_declaration(text, "color", BASE, false)
                .unwrap()
                .value,
            ""
        );
        assert!(!view.important);
    }

    #[test]
    fn invalid_descriptor_edits_are_atomic_and_removal_returns_old_value() {
        let url = UrlExtraData::from(url::Url::parse(BASE).unwrap());
        let mut descriptors = parse(
            "font-family: Example; font-weight: 400",
            &context(&url, false),
        );
        let before = descriptors.clone();
        assert!(
            !edit(
                &mut descriptors,
                Some("font-weight"),
                "400 junk",
                "",
                BASE,
                false
            )
            .unwrap()
            .1
        );
        assert!(
            !edit(
                &mut descriptors,
                Some("font-weight"),
                "700",
                "important",
                BASE,
                false
            )
            .unwrap()
            .1
        );
        assert_eq!(descriptors, before);
        assert_eq!(
            edit(&mut descriptors, Some("font-weight"), "", "", BASE, false).unwrap(),
            ("400".into(), true)
        );
    }
}

/// Decoded family names in a computed font-family list, including quoted commas.
pub fn font_family_names(source: &str) -> Vec<String> {
    use cssparser::Token;
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut names = Vec::new();
    let mut words = Vec::new();
    while let Ok(token) = parser.next() {
        match token {
            Token::QuotedString(value) => words.push(value.to_string()),
            Token::Ident(value) => words.push(value.to_string()),
            Token::Comma => {
                if !words.is_empty() {
                    names.push(words.join(" "));
                    words.clear();
                }
            }
            _ => {}
        }
    }
    if !words.is_empty() {
        names.push(words.join(" "));
    }
    names
}

#[cfg(test)]
mod family_name_tests {
    #[test]
    fn quoted_commas_and_escapes_remain_single_decoded_families() {
        assert_eq!(
            super::font_family_names(r#""Custom, Face", Demo\20 Family, serif"#),
            vec!["Custom, Face", "Demo Family", "serif"]
        );
    }
}

/// Inclusive scalar ranges from the native descriptor grammar. An omitted or
/// invalid descriptor has the CSS initial value (all Unicode code points).
pub fn font_face_unicode_ranges(text: &str, base: &str) -> Result<Vec<(u32, u32)>, String> {
    let value = read_font_face_declaration(text, "unicode-range", base, false)?.value;
    if value.is_empty() {
        return Ok(vec![(0, 0x10ffff)]);
    }
    let mut input = cssparser::ParserInput::new(&value);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut ranges = Vec::new();
    loop {
        let range = cssparser::UnicodeRange::parse(&mut parser)
            .map_err(|error| format!("invalid serialized font range: {error:?}"))?;
        ranges.push((range.start, range.end));
        if parser.is_exhausted() {
            break;
        }
        parser
            .expect_comma()
            .map_err(|error| format!("invalid serialized font range separator: {error:?}"))?;
    }
    Ok(ranges)
}

#[cfg(test)]
mod unicode_demand_tests {
    use super::*;
    #[test]
    fn native_ranges_cover_wildcards_supplementary_and_invalid_initial_value() {
        let base = "https://example.test/font.css";
        assert_eq!(
            font_face_unicode_ranges("unicode-range: U+4??, U+1F600-1F64F", base).unwrap(),
            vec![(0x400, 0x4ff), (0x1f600, 0x1f64f)]
        );
        assert_eq!(
            font_face_unicode_ranges("unicode-range: invalid", base).unwrap(),
            vec![(0, 0x10ffff)]
        );
        assert_eq!(
            font_face_unicode_ranges("", base).unwrap(),
            vec![(0, 0x10ffff)]
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFaceSource {
    Local(String),
    Url(String),
}

/// Parse src through Stylo and remove only sources requiring an unavailable
/// decoder or shaping technology. Ordering survives filtering for fallback.
pub fn font_face_sources(text: &str, base: &str) -> Result<Vec<FontFaceSource>, String> {
    use style::font_face::{
        FontFaceSourceFormat, FontFaceSourceFormatKeyword as Format,
        FontFaceSourceTechFlags as Tech, Source, SourceList,
    };
    use style::parser::Parse;
    let value = read_font_face_declaration(text, "src", base, false)?.value;
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let url = UrlExtraData::from(url::Url::parse(base).map_err(|error| error.to_string())?);
    let mut input = cssparser::ParserInput::new(&value);
    let mut parser = cssparser::Parser::new(&mut input);
    let sources = SourceList::parse(&context(&url, false), &mut parser)
        .map_err(|error| format!("invalid serialized font sources: {error:?}"))?;
    let mut result = Vec::new();
    for source in sources.0 {
        match source {
            Source::Local(family) => {
                let names = font_family_names(&family.to_css_string());
                if let Some(name) = names.into_iter().next() {
                    result.push(FontFaceSource::Local(name));
                }
            }
            Source::Url(source) => {
                let supported = match &source.format_hint {
                    None => true,
                    Some(FontFaceSourceFormat::Keyword(format)) => matches!(
                        format,
                        Format::Opentype | Format::Truetype | Format::Woff | Format::Woff2
                    ),
                    Some(FontFaceSourceFormat::String(format)) => matches!(
                        format.to_ascii_lowercase().as_str(),
                        "opentype" | "truetype" | "woff" | "woff2"
                    ),
                };
                let supported_tech = Tech::FEATURES_OPENTYPE | Tech::VARIATIONS;
                if !supported || !supported_tech.contains(source.tech_flags) {
                    continue;
                }
                for reference in super::resource_references(&source.url.to_css_string()) {
                    result.push(FontFaceSource::Url(reference.url));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod source_selection_tests {
    use super::*;
    #[test]
    fn local_only_faces_are_not_discarded_as_resource_free_styles() {
        assert!(
            super::super::stylesheet_requires_resources(
                "@media screen{@font-face{font-family:Demo;src:local('Noto Sans Regular')}}",
                "https://example.test/style.css"
            )
            .unwrap()
        );
    }
    #[test]
    fn native_src_filter_preserves_local_and_usable_url_order() {
        let sources=font_face_sources("src: local('Missing Face'), url(a.woff2) format(woff2), url(no.svg) format(svg), url(no.woff2) tech(incremental), url(last.ttf) format(truetype)","https://example.test/style.css").unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0], FontFaceSource::Local("Missing Face".into()));
        assert!(matches!(&sources[1],FontFaceSource::Url(url) if url.ends_with("a.woff2")));
        assert!(matches!(&sources[2],FontFaceSource::Url(url) if url.ends_with("last.ttf")));
    }
}

/// Font demand includes canonical alternatives that the native shaper may use.
pub fn font_text_codepoints(text: &str) -> Vec<u32> {
    use unicode_normalization::UnicodeNormalization;
    text.chars()
        .chain(text.nfc())
        .chain(text.nfd())
        .map(u32::from)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
