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
